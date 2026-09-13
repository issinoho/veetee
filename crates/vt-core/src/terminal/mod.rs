use std::collections::VecDeque;

use vt_parser::{InputMode, Params, Parser, Perform, Sequence, StringEnd};

use crate::cell::{Attrs, Cell, Color, Flags};
use crate::charset::{self, Charset, CharsetState};
use crate::config::{Config, Model, StatusDisplay};
use crate::grid::{Grid, Line, LineSize, Region};
use crate::keyboard::{self, Key, KeyContext};
use crate::modes::Modes;
use crate::softfont::{SoftFonts, SoftGlyph};
use crate::udk::UserKeys;

mod dcs;
mod reports;

/// Something the host application (GUI, headless driver) must act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Bell,
    /// DECCOLM changed the page width; the window should follow.
    ColumnsChanged(usize),
    /// DECLL: bit 0 = L1 … bit 3 = L4.
    LedsChanged(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub attrs: Attrs,
    /// DEC "last column flag": a character was written in the last column
    /// and the next graphic character will wrap first (if DECAWM is set).
    pub pending_wrap: bool,
}

/// State saved by DECSC and restored by DECRC (VT510 RM, DECSC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SavedCursor {
    row: usize,
    col: usize,
    /// SGR rendition and the DECSCA selective erase attribute.
    attrs: Attrs,
    /// G0–G3, GL/GR and pending single shifts.
    charsets: CharsetState,
    origin: bool,
    /// "Wrap flag (autowrap or no autowrap)".
    autowrap: bool,
}

/// Which display receives host output (DECSASD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusLine {
    kind: StatusDisplay,
    active: bool,
    /// Cursor position on the main display while the status line is active.
    main_cursor: (usize, usize, bool),
    col: usize,
}

/// A DEC VT terminal: feed it host output, read back its screen and replies.
#[derive(Debug, Clone)]
pub struct Terminal {
    parser: Parser,
    emu: Emulator,
}

impl Terminal {
    pub fn new(config: Config) -> Terminal {
        let mut t = Terminal {
            parser: Parser::new(),
            emu: Emulator::new(config),
        };
        t.sync_parser();
        t
    }

    /// Processes bytes received from the host.
    pub fn advance(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while !rest.is_empty() {
            let n = self.parser.advance_until_pause(&mut self.emu, rest);
            rest = &rest[n..];
            self.sync_parser();
        }
    }

    /// Takes the bytes the terminal wants to send to the host (reports, answerback).
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.emu.output)
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.emu.events)
    }

    pub fn config(&self) -> &Config {
        &self.emu.config
    }

    pub fn grid(&self) -> &Grid {
        &self.emu.grid
    }

    pub fn scrollback(&self) -> &VecDeque<Line> {
        &self.emu.scrollback
    }

    pub fn cursor(&self) -> &Cursor {
        &self.emu.cursor
    }

    pub fn modes(&self) -> &Modes {
        &self.emu.modes
    }

    /// Current conformance level (1 = VT100 mode … 5 = VT500 mode).
    pub fn level(&self) -> u8 {
        self.emu.level
    }

    pub fn leds(&self) -> u8 {
        self.emu.leds
    }

    /// Top and bottom margins (DECSTBM), zero-based inclusive.
    pub fn margins(&self) -> (usize, usize) {
        (self.emu.top, self.emu.bottom)
    }

    /// Whether the host selected 8-bit C1 controls for replies (S8C1T).
    pub fn eight_bit_replies(&self) -> bool {
        self.emu.c1_8bit
    }

    /// The status line type in effect (always `None` before the VT320).
    pub fn status_display(&self) -> StatusDisplay {
        self.emu.status.kind
    }

    /// Contents of the host-writable status line.
    pub fn status_line(&self) -> &Line {
        &self.emu.status_line
    }

    /// Host output is going to the status line (DECSASD 1).
    pub fn status_active(&self) -> bool {
        self.emu.status.active
    }

    /// Column of the cursor on the status line while it is active.
    pub fn status_cursor_col(&self) -> usize {
        self.emu.status.col
    }

    pub fn user_keys(&self) -> &UserKeys {
        &self.emu.udk
    }

    /// Unlocks user-defined keys (the Set-Up "Unlocked" setting).
    pub fn unlock_user_keys(&mut self) {
        self.emu.udk.locked = false;
    }

    /// The downloaded glyph for a soft character, in the current column mode.
    pub fn soft_glyph(&self, ch: char) -> Option<&SoftGlyph> {
        self.emu.soft.glyph(ch, self.emu.modes.columns_132)
    }

    /// Incremented whenever soft fonts change, so renderers can refresh glyphs.
    pub fn soft_font_generation(&self) -> u64 {
        self.emu.soft_generation
    }

    /// A DEC key was pressed. Its codes are queued for the host (see
    /// [`Terminal::take_output`]) and echoed locally when SRM is reset.
    pub fn key(&mut self, key: Key) {
        if self.emu.modes.keyboard_locked {
            return;
        }
        let mut bytes = Vec::new();
        if let Key::UserDefined(f) = key {
            if self.emu.level >= 2 {
                if let Some(s) = self.emu.udk.get(f) {
                    bytes.extend_from_slice(s);
                }
            }
        } else {
            let e = &self.emu;
            let cx = KeyContext {
                ansi: e.modes.ansi,
                level: e.level,
                eight_bit: e.c1_8bit,
                cursor_app: e.modes.cursor_keys_application,
                keypad_app: e.modes.keypad_application,
                new_line: e.modes.new_line,
                backarrow_bs: e.modes.backarrow_sends_bs,
            };
            keyboard::encode(key, cx, &mut bytes);
        }
        self.transmit(&bytes);
    }

    /// Characters typed on the main keypad, including C0 controls produced
    /// with Ctrl. Characters the current mode cannot transmit are dropped.
    pub fn type_text(&mut self, text: &str) {
        if self.emu.modes.keyboard_locked {
            return;
        }
        let e = &self.emu;
        let national = e
            .modes
            .national
            .then_some(e.config.keyboard_language)
            .flatten();
        let mut bytes = Vec::new();
        for ch in text.chars() {
            if e.config.extensions.utf8 {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            } else if ch.is_ascii_control() {
                bytes.push(ch as u8);
            } else if let Some(nrc) = national {
                bytes.extend(nrc.encode(ch));
            } else if ch.is_ascii() {
                bytes.push(ch as u8);
            } else if e.level >= 2 && !e.modes.national {
                let encoded = match e.upss {
                    Charset::IsoLatin1 => charset::encode_latin1(ch),
                    _ => charset::encode_dec_multinational(ch),
                };
                bytes.extend(encoded);
            }
        }
        self.transmit(&bytes);
    }

    /// The Ctrl+Break local function: transmits the Set-Up answerback message.
    pub fn send_answerback(&mut self) {
        let answerback = self.emu.config.answerback.clone();
        self.transmit(&answerback);
    }

    fn transmit(&mut self, bytes: &[u8]) {
        self.emu.output.extend_from_slice(bytes);
        if !self.emu.modes.send_receive {
            self.advance(bytes);
        }
    }

    /// The window was resized by the user. DEC terminals never reflow text.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.emu.resize(rows.max(1), cols.max(2));
    }

    fn sync_parser(&mut self) {
        let e = &self.emu;
        self.parser.set_vt52(!e.modes.ansi);
        let mode = if e.config.extensions.utf8 {
            InputMode::Utf8
        } else if e.config.model.max_level() == 1 || !e.modes.ansi {
            // VT100-class terminals, and any terminal in VT52 mode, are 7-bit.
            InputMode::SevenBit
        } else {
            InputMode::EightBit
        };
        if self.parser.input_mode() != mode {
            self.parser.set_input_mode(mode);
        }
    }
}

#[derive(Debug, Clone)]
struct Emulator {
    config: Config,
    grid: Grid,
    scrollback: VecDeque<Line>,
    cursor: Cursor,
    saved: Option<SavedCursor>,
    charsets: CharsetState,
    /// Current user-preferred supplemental set (DECAUPSS).
    upss: Charset,
    modes: Modes,
    /// Scrolling margins, zero-based inclusive.
    top: usize,
    bottom: usize,
    tabs: Vec<bool>,
    level: u8,
    c1_8bit: bool,
    vt52_graphics: bool,
    leds: u8,
    status: StatusLine,
    status_line: Line,
    udk: UserKeys,
    soft: SoftFonts,
    soft_generation: u64,
    dcs: dcs::DcsState,
    output: Vec<u8>,
    events: Vec<Event>,
    pause: bool,
}

const TAB_WIDTH: usize = 8;

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c > 0 && c % TAB_WIDTH == 0).collect()
}

impl Emulator {
    fn new(config: Config) -> Emulator {
        let rows = config.rows.max(1);
        let cols = config.cols.max(2);
        let model = config.model;
        let upss = config.supplemental.charset();
        let status_kind = if model.has_status_line() {
            config.status_display
        } else {
            StatusDisplay::None
        };
        Emulator {
            grid: Grid::new(rows, cols),
            scrollback: VecDeque::new(),
            cursor: Cursor {
                row: 0,
                col: 0,
                attrs: Attrs::default(),
                pending_wrap: false,
            },
            saved: None,
            charsets: initial_charsets(model, upss),
            upss,
            modes: Modes::power_up(
                config.autowrap,
                config.new_line,
                config.national_mode && model.max_level() >= 2,
            ),
            top: 0,
            bottom: rows - 1,
            tabs: default_tabs(cols),
            level: model.max_level(),
            c1_8bit: false,
            vt52_graphics: false,
            leds: 0,
            status: StatusLine {
                kind: status_kind,
                active: false,
                main_cursor: (0, 0, false),
                col: 0,
            },
            status_line: Line::new(cols, Cell::BLANK),
            udk: UserKeys::new(config.udk_locked),
            soft: SoftFonts::default(),
            soft_generation: 0,
            dcs: dcs::DcsState::None,
            output: Vec::new(),
            events: Vec::new(),
            pause: false,
            config,
        }
    }

    // ------------------------------------------------------------ geometry

    fn rows(&self) -> usize {
        self.grid.rows()
    }

    fn cols(&self) -> usize {
        self.grid.cols()
    }

    fn line_width(&self, row: usize) -> usize {
        self.grid.line(row).width()
    }

    /// The line the cursor is on: the status line while it is active.
    fn cursor_line_mut(&mut self) -> &mut Line {
        if self.status.active {
            &mut self.status_line
        } else {
            self.grid.line_mut(self.cursor.row)
        }
    }

    fn cursor_line(&self) -> &Line {
        if self.status.active {
            &self.status_line
        } else {
            self.grid.line(self.cursor.row)
        }
    }

    fn last_col(&self) -> usize {
        self.cursor_line().width() - 1
    }

    fn blank(&self) -> Cell {
        Cell::erased(self.cursor.attrs.bg)
    }

    fn region(&self, top: usize) -> Region {
        Region {
            top,
            bottom: self.bottom,
            left: 0,
            right: self.cols() - 1,
        }
    }

    fn within_margins(&self) -> bool {
        (self.top..=self.bottom).contains(&self.cursor.row)
    }

    // ------------------------------------------------------- cursor motion

    /// Moves the cursor to an absolute position, clamped to the page and line
    /// width. On the status line only the column applies.
    fn goto(&mut self, row: usize, col: usize) {
        if self.status.active {
            self.cursor.col = col.min(self.status_line.width() - 1);
            self.cursor.pending_wrap = false;
            return;
        }
        let row = row.min(self.rows() - 1);
        self.cursor.row = row;
        self.cursor.col = col.min(self.line_width(row) - 1);
        self.cursor.pending_wrap = false;
    }

    /// CUP/HVP with one-based coordinates, honouring DECOM.
    fn cup(&mut self, row: usize, col: usize) {
        let (row, col) = (row.max(1) - 1, col.max(1) - 1);
        if self.modes.origin {
            let row = (self.top + row).min(self.bottom);
            self.goto(row, col);
        } else {
            self.goto(row, col);
        }
    }

    fn home(&mut self) {
        self.cup(1, 1);
    }

    fn cuu(&mut self, n: usize) {
        let limit = if self.cursor.row >= self.top {
            self.top
        } else {
            0
        };
        let row = self.cursor.row.saturating_sub(n).max(limit);
        self.goto(row, self.cursor.col);
    }

    fn cud(&mut self, n: usize) {
        let limit = if self.cursor.row <= self.bottom {
            self.bottom
        } else {
            self.rows() - 1
        };
        let row = (self.cursor.row + n).min(limit);
        self.goto(row, self.cursor.col);
    }

    fn cuf(&mut self, n: usize) {
        let col = (self.cursor.col + n).min(self.last_col());
        self.goto(self.cursor.row, col);
    }

    fn cub(&mut self, n: usize) {
        let col = self.cursor.col.saturating_sub(n);
        self.goto(self.cursor.row, col);
    }

    fn carriage_return(&mut self) {
        self.goto(self.cursor.row, 0);
    }

    /// IND: down one line, scrolling the region if at the bottom margin.
    fn index(&mut self) {
        if self.status.active {
            return;
        }
        if self.cursor.row == self.bottom {
            self.scroll_up(1);
        } else if self.cursor.row + 1 < self.rows() {
            self.cursor.row += 1;
        }
        let col = self.cursor.col;
        self.goto(self.cursor.row, col);
    }

    /// RI: up one line, scrolling the region down if at the top margin.
    fn reverse_index(&mut self) {
        if self.status.active {
            return;
        }
        if self.cursor.row == self.top {
            let region = self.region(self.top);
            let blank = self.blank();
            self.grid.scroll_down(region, 1, blank);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
        let col = self.cursor.col;
        self.goto(self.cursor.row, col);
    }

    fn line_feed(&mut self) {
        self.index();
        if self.modes.new_line {
            self.carriage_return();
        }
    }

    fn scroll_up(&mut self, n: usize) {
        let region = self.region(self.top);
        let blank = self.blank();
        let gone = self.grid.scroll_up(region, n, blank);
        if self.top == 0 && self.config.scrollback_lines > 0 {
            for line in gone {
                if self.scrollback.len() == self.config.scrollback_lines {
                    self.scrollback.pop_front();
                }
                self.scrollback.push_back(line);
            }
        }
    }

    fn tab(&mut self, n: usize) {
        let last = self.last_col();
        let mut col = self.cursor.col;
        for _ in 0..n {
            col = (col + 1..last).find(|&c| self.tabs[c]).unwrap_or(last);
        }
        self.goto(self.cursor.row, col);
    }

    fn back_tab(&mut self, n: usize) {
        let mut col = self.cursor.col;
        for _ in 0..n {
            col = (1..col).rev().find(|&c| self.tabs[c]).unwrap_or(0);
        }
        self.goto(self.cursor.row, col);
    }

    // ------------------------------------------------------------ graphics

    fn put_char(&mut self, ch: char) {
        if self.cursor.pending_wrap && self.modes.autowrap && !self.status.active {
            self.grid.line_mut(self.cursor.row).wrapped = true;
            self.cursor.col = 0;
            self.index();
        }
        let col = self.cursor.col;
        let last = self.last_col();
        let cell = Cell {
            ch,
            attrs: self.cursor.attrs,
        };
        let blank = self.blank();
        let insert = self.modes.insert;
        let line = self.cursor_line_mut();
        if insert {
            line.insert(col, last, 1, blank);
        }
        line.cells_mut()[col] = cell;
        if col >= last {
            self.cursor.col = last;
            self.cursor.pending_wrap = self.modes.autowrap && !self.status.active;
        } else {
            self.cursor.col = col + 1;
            self.cursor.pending_wrap = false;
        }
    }

    fn print_byte(&mut self, byte: u8) {
        let ch = if !self.modes.ansi {
            let code = byte & 0x7F;
            if self.vt52_graphics {
                Charset::DecSpecialGraphics.map(code)
            } else {
                Charset::Ascii.map(code)
            }
        } else if self.modes.national {
            // National mode is a 7-bit mode: GR is not available.
            self.charsets.translate(byte & 0x7F)
        } else {
            self.charsets.translate(byte)
        };
        if let Some(ch) = ch {
            self.put_char(ch);
        }
    }

    // -------------------------------------------------------------- erasing

    /// ED, or DECSED when `selective` (protected characters survive).
    fn erase_display(&mut self, mode: u16, selective: bool) {
        if self.status.active {
            // The status line is a single line: erasing the display clears it.
            return self.erase_line(2, selective);
        }
        let blank = self.blank();
        let (row, col) = (self.cursor.row, self.cursor.col);
        let last = self.line_width(row) - 1;
        let rows: Box<dyn Iterator<Item = (usize, std::ops::Range<usize>)>> = match mode {
            0 => Box::new(
                std::iter::once((row, col..usize::MAX))
                    .chain((row + 1..self.rows()).map(|r| (r, 0..usize::MAX))),
            ),
            1 => Box::new(
                (0..row)
                    .map(|r| (r, 0..usize::MAX))
                    .chain(std::iter::once((row, 0..col + 1))),
            ),
            2 => Box::new((0..self.rows()).map(|r| (r, 0..usize::MAX))),
            _ => return,
        };
        let rows: Vec<_> = rows.collect();
        for (r, range) in rows {
            let line = self.grid.line_mut(r);
            if selective {
                erase_unprotected(line, range, blank);
            } else if range.start == 0 && (range.end == usize::MAX || (r == row && col >= last)) {
                // Completely erased lines become single width (VT510 RM, ED).
                line.clear(blank);
            } else {
                line.erase(range, blank);
            }
        }
        self.cursor.pending_wrap = false;
    }

    /// EL, or DECSEL when `selective`.
    fn erase_line(&mut self, mode: u16, selective: bool) {
        let blank = self.blank();
        let col = self.cursor.col;
        let range = match mode {
            0 => col..usize::MAX,
            1 => 0..col + 1,
            2 => 0..usize::MAX,
            _ => return,
        };
        let line = self.cursor_line_mut();
        if selective {
            erase_unprotected(line, range, blank);
        } else {
            line.erase(range, blank);
        }
        self.cursor.pending_wrap = false;
    }

    // -------------------------------------------------------- VT102 editing

    fn insert_lines(&mut self, n: usize) {
        if self.status.active || !self.within_margins() {
            return;
        }
        let region = self.region(self.cursor.row);
        let blank = self.blank();
        self.grid.scroll_down(region, n, blank);
        self.carriage_return();
    }

    fn delete_lines(&mut self, n: usize) {
        if self.status.active || !self.within_margins() {
            return;
        }
        let region = self.region(self.cursor.row);
        let blank = self.blank();
        self.grid.scroll_up(region, n, blank);
        self.carriage_return();
    }

    fn insert_chars(&mut self, n: usize) {
        let (col, last) = (self.cursor.col, self.last_col());
        let blank = self.blank();
        self.cursor_line_mut().insert(col, last, n, blank);
        self.cursor.pending_wrap = false;
    }

    fn delete_chars(&mut self, n: usize) {
        let (col, last) = (self.cursor.col, self.last_col());
        let blank = self.blank();
        self.cursor_line_mut().delete(col, last, n, blank);
        self.cursor.pending_wrap = false;
    }

    fn erase_chars(&mut self, n: usize) {
        let col = self.cursor.col;
        let blank = self.blank();
        self.cursor_line_mut()
            .erase(col..col.saturating_add(n), blank);
        self.cursor.pending_wrap = false;
    }

    // ---------------------------------------------------------------- modes

    fn set_ansi_mode(&mut self, mode: u16, on: bool) {
        match mode {
            2 => self.modes.keyboard_locked = on,
            4 => self.modes.insert = on,
            12 => self.modes.send_receive = on,
            20 => self.modes.new_line = on,
            _ => {}
        }
    }

    fn set_dec_mode(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.modes.cursor_keys_application = on,
            2 if !on => self.enter_vt52(),
            3 => self.set_columns(if on { 132 } else { 80 }),
            4 => self.modes.smooth_scroll = on,
            5 => self.modes.reverse_screen = on,
            6 => {
                self.modes.origin = on;
                self.home();
            }
            7 => self.modes.autowrap = on,
            8 => self.modes.auto_repeat = on,
            18 => self.modes.print_form_feed = on,
            19 => self.modes.print_extent_full = on,
            25 if self.level >= 2 => self.modes.cursor_visible = on,
            42 if self.level >= 3 => self.modes.national = on,
            66 if self.level >= 3 => self.modes.keypad_application = on,
            67 if self.level >= 3 => self.modes.backarrow_sends_bs = on,
            _ => {}
        }
    }

    /// DECCOLM: the page is cleared, margins reset and the cursor homed.
    fn set_columns(&mut self, cols: usize) {
        self.exit_status_line();
        self.modes.columns_132 = cols == 132;
        let rows = self.rows();
        self.grid.resize(rows, cols);
        self.grid.clear(Cell::BLANK);
        self.status_line.resize(cols, Cell::BLANK);
        // Tab stops are not reset (DEC STD 070); new columns get the default stops.
        self.resize_tabs(cols);
        self.top = 0;
        self.bottom = rows - 1;
        self.goto(0, 0);
        self.events.push(Event::ColumnsChanged(cols));
    }

    fn resize_tabs(&mut self, cols: usize) {
        let old = self.tabs.len();
        self.tabs.resize(cols, false);
        for c in old..cols {
            self.tabs[c] = c > 0 && c % TAB_WIDTH == 0;
        }
    }

    fn enter_vt52(&mut self) {
        self.exit_status_line();
        self.modes.ansi = false;
        self.vt52_graphics = false;
        self.pause = true;
    }

    fn exit_vt52(&mut self) {
        self.modes.ansi = true;
        // A real VT220/VT420 returns to VT100 mode, not its previous level.
        self.level = 1;
        self.c1_8bit = false;
        self.pause = true;
    }

    // ------------------------------------------------------------ status line

    /// DECSSDT: select the status line type.
    fn set_status_type(&mut self, ps: u16) {
        if !self.config.model.has_status_line() {
            return;
        }
        let kind = match ps {
            0 => StatusDisplay::None,
            1 => StatusDisplay::Indicator,
            2 => StatusDisplay::HostWritable,
            _ => return,
        };
        if kind != StatusDisplay::HostWritable {
            self.exit_status_line();
        }
        if kind != self.status.kind {
            // A new host-writable status line starts empty.
            self.status_line.clear(Cell::BLANK);
        }
        self.status.kind = kind;
    }

    /// DECSASD: direct output to the main display (0) or the status line (1).
    fn set_active_display(&mut self, ps: u16) {
        match ps {
            0 => self.exit_status_line(),
            1 if self.status.kind == StatusDisplay::HostWritable && !self.status.active => {
                self.status.main_cursor =
                    (self.cursor.row, self.cursor.col, self.cursor.pending_wrap);
                self.status.active = true;
                self.cursor.col = self.status.col.min(self.status_line.width() - 1);
                self.cursor.pending_wrap = false;
            }
            _ => {}
        }
    }

    fn exit_status_line(&mut self) {
        if !self.status.active {
            return;
        }
        self.status.col = self.cursor.col;
        self.status.active = false;
        let (row, col, pending) = self.status.main_cursor;
        self.goto(row, col);
        self.cursor.pending_wrap = pending;
    }

    // --------------------------------------------------------- save/restore

    fn save_cursor(&mut self) {
        self.saved = Some(SavedCursor {
            row: self.cursor.row,
            col: self.cursor.col,
            attrs: self.cursor.attrs,
            charsets: self.charsets,
            origin: self.modes.origin,
            autowrap: self.modes.autowrap,
        });
    }

    fn restore_cursor(&mut self) {
        match self.saved {
            Some(saved) => {
                self.modes.origin = saved.origin;
                self.modes.autowrap = saved.autowrap;
                self.cursor.attrs = saved.attrs;
                self.charsets = saved.charsets;
                self.goto(saved.row, saved.col);
            }
            None => {
                // Nothing saved: home the cursor with default rendition and sets.
                self.modes.origin = false;
                self.cursor.attrs = Attrs::default();
                self.charsets = initial_charsets(self.config.model, self.upss);
                self.goto(0, 0);
            }
        }
    }

    // ---------------------------------------------------------------- reset

    /// RIS: return to power-up state, including Set-Up defaults.
    fn full_reset(&mut self) {
        let config = self.config.clone();
        let rows = self.rows();
        let scrollback = std::mem::take(&mut self.scrollback);
        let columns_changed = self.cols() != config.cols;
        let generation = self.soft_generation + 1;
        *self = Emulator::new(Config { rows, ..config });
        self.scrollback = scrollback;
        self.soft_generation = generation;
        if columns_changed {
            self.events.push(Event::ColumnsChanged(self.cols()));
        }
        self.pause = true;
    }

    /// DECSTR, per VT510 RM table 5-9.
    fn soft_reset(&mut self) {
        self.exit_status_line();
        self.modes.cursor_visible = true;
        self.modes.insert = false;
        self.modes.origin = false;
        self.modes.autowrap = false;
        self.modes.national = false;
        self.modes.keyboard_locked = false;
        self.modes.keypad_application = false;
        self.modes.cursor_keys_application = false;
        self.top = 0;
        self.bottom = self.rows() - 1;
        self.upss = self.config.supplemental.charset();
        self.charsets = initial_charsets(self.config.model, self.upss);
        // SGR normal rendition and DECSCA erasable.
        self.cursor.attrs = Attrs::default();
        self.saved = None;
        self.cursor.pending_wrap = false;
    }

    /// DECSCL: select conformance level. The terminal performs a hard reset
    /// (VT510 RM, DECSCL) and then operates at the new level.
    fn set_conformance_level(&mut self, params: &Params) {
        let max = self.config.model.max_level();
        if max < 2 {
            return;
        }
        let level = match params.get_or(0, 0) {
            v @ 61..=65 => (v - 60) as u8,
            _ => return,
        };
        let level = level.min(max);
        let eight_bit = level >= 2 && params.get_or(1, 0) != 1;
        self.full_reset();
        self.level = level;
        self.c1_8bit = eight_bit;
    }

    fn screen_alignment(&mut self) {
        let fill = Cell {
            ch: 'E',
            attrs: Attrs::default(),
        };
        self.grid.clear(fill);
        self.top = 0;
        self.bottom = self.rows() - 1;
        self.modes.origin = false;
        self.goto(0, 0);
    }

    fn set_line_size(&mut self, size: LineSize) {
        if self.status.active {
            return;
        }
        let row = self.cursor.row;
        let line = self.grid.line_mut(row);
        if size.is_double_width() && !line.size.is_double_width() {
            // Characters in the right half of the line are lost.
            let half = line.len() / 2;
            line.erase(half..usize::MAX, Cell::BLANK);
        }
        line.size = size;
        let col = self.cursor.col;
        let pending = self.cursor.pending_wrap;
        self.goto(row, col);
        self.cursor.pending_wrap = pending && self.cursor.col == col;
    }

    fn resize(&mut self, rows: usize, cols: usize) {
        self.exit_status_line();
        if self.cursor.row >= rows {
            let excess = self.cursor.row + 1 - rows;
            for line in self.grid.drain_top(excess) {
                self.scrollback.push_back(line);
            }
            while self.scrollback.len() > self.config.scrollback_lines {
                self.scrollback.pop_front();
            }
            self.cursor.row -= excess;
        }
        self.grid.resize(rows, cols);
        self.status_line.resize(cols, Cell::BLANK);
        self.resize_tabs(cols);
        self.top = 0;
        self.bottom = rows - 1;
        let (row, col) = (self.cursor.row, self.cursor.col);
        self.goto(row, col);
    }

    // ------------------------------------------------------------------ SGR

    fn select_graphic_rendition(&mut self, params: &Params) {
        let colors = self.config.model.has_color() || self.config.extensions.xterm_sgr;
        let xterm = self.config.extensions.xterm_sgr;
        let vt220 = self.level >= 2;
        let a = &mut self.cursor.attrs;
        // SGR never changes the DECSCA protection attribute.
        let normal = |a: &Attrs| {
            let mut n = Attrs::default();
            n.flags
                .set(Flags::PROTECTED, a.flags.contains(Flags::PROTECTED));
            n
        };
        if params.is_empty() {
            *a = normal(a);
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let p = params.get_or(i, 0);
            match p {
                0 => *a = normal(a),
                1 => a.flags.set(Flags::BOLD, true),
                4 => a.flags.set(Flags::UNDERLINE, true),
                5 => a.flags.set(Flags::BLINK, true),
                7 => a.flags.set(Flags::REVERSE, true),
                8 if vt220 => a.flags.set(Flags::INVISIBLE, true),
                22 if vt220 => a.flags.set(Flags::BOLD, false),
                24 if vt220 => a.flags.set(Flags::UNDERLINE, false),
                25 if vt220 => a.flags.set(Flags::BLINK, false),
                27 if vt220 => a.flags.set(Flags::REVERSE, false),
                28 if vt220 => a.flags.set(Flags::INVISIBLE, false),
                30..=37 if colors => a.fg = Color::Indexed((p - 30) as u8),
                39 if colors => a.fg = Color::Default,
                40..=47 if colors => a.bg = Color::Indexed((p - 40) as u8),
                49 if colors => a.bg = Color::Default,
                90..=97 if xterm => a.fg = Color::Indexed((p - 90 + 8) as u8),
                100..=107 if xterm => a.bg = Color::Indexed((p - 100 + 8) as u8),
                38 | 48 if xterm => {
                    let (color, used) = extended_color(params, i + 1);
                    if let Some(color) = color {
                        if p == 38 {
                            a.fg = color;
                        } else {
                            a.bg = color;
                        }
                    }
                    i += used;
                }
                _ => {}
            }
            i += 1;
        }
    }

    // ------------------------------------------------------------------ VT52

    fn vt52_escape(&mut self, final_byte: u8) {
        match final_byte {
            b'A' => self.cuu(1),
            b'B' => self.cud(1),
            b'C' => self.cuf(1),
            b'D' => self.cub(1),
            b'F' => self.vt52_graphics = true,
            b'G' => self.vt52_graphics = false,
            b'H' => self.goto(0, 0),
            b'I' => self.reverse_index(),
            b'J' => self.erase_display(0, false),
            b'K' => self.erase_line(0, false),
            b'Z' => self.output.extend_from_slice(b"\x1b/Z"),
            b'=' => self.modes.keypad_application = true,
            b'>' => self.modes.keypad_application = false,
            b'<' => self.exit_vt52(),
            // Printer functions (ESC ^ _ W X ] V) are handled with printer support.
            _ => {}
        }
    }

    // ----------------------------------------------------------- designation

    /// SCS: designate a graphic set into G0–G3.
    fn designate(&mut self, g: usize, is_96: bool, intermediate: Option<u8>, final_byte: u8) {
        let level = self.level;
        let mut designation = Vec::with_capacity(2);
        designation.extend(intermediate);
        designation.push(final_byte);
        // Soft sets are selected by the designator they were loaded with.
        if level >= 2 {
            if let Some(slot) = self.soft.find(&designation, is_96) {
                self.charsets.g[g] = Charset::Soft { slot, is_96 };
                return;
            }
        }
        let set = if is_96 {
            match (intermediate, final_byte) {
                (None, b'A') if level >= 3 => Charset::IsoLatin1,
                (None, b'<') if level >= 3 && self.upss.is_96() => self.upss,
                _ => return,
            }
        } else {
            match (intermediate, final_byte) {
                (None, b'B' | b'1') => Charset::Ascii,
                (None, b'0' | b'2') => Charset::DecSpecialGraphics,
                (None, b'<') if level == 2 => Charset::DecSupplemental,
                (None, b'<') if level >= 3 && !self.upss.is_96() => self.upss,
                (Some(b'%'), b'5') if level >= 3 => Charset::DecSupplemental,
                (None, b'>') if level >= 3 => Charset::DecTechnical,
                (None, b'A') if level == 1 => Charset::National(charset::Nrc::British),
                _ => match Charset::national(intermediate, final_byte) {
                    // National sets need national mode (DECNRCM) on VT220 and later.
                    Some(nrc) if self.modes.national => Charset::National(nrc),
                    _ => return,
                },
            }
        };
        self.charsets.g[g] = set;
    }
}

/// Clears cells in `range` that are not protected by DECSCA.
fn erase_unprotected(line: &mut Line, range: std::ops::Range<usize>, blank: Cell) {
    let end = range.end.min(line.len());
    for cell in line.cells_mut()[range.start.min(end)..end].iter_mut() {
        if !cell.attrs.flags.contains(Flags::PROTECTED) {
            *cell = blank;
        }
    }
}

fn initial_charsets(model: Model, upss: Charset) -> CharsetState {
    if model.max_level() >= 2 {
        CharsetState::vt220(upss)
    } else {
        CharsetState::VT100
    }
}

/// Parses the tail of SGR 38/48 starting at `start`. Returns the colour and
/// the number of parameters consumed.
fn extended_color(params: &Params, start: usize) -> (Option<Color>, usize) {
    let byte = |i: usize| params.get(i).map(|v| v.min(255) as u8);
    match params.get(start) {
        Some(5) => (byte(start + 1).map(Color::Indexed), 2),
        Some(2) => {
            let rgb = (byte(start + 1), byte(start + 2), byte(start + 3));
            match rgb {
                (Some(r), Some(g), Some(b)) => (Some(Color::Rgb(r, g, b)), 4),
                _ => (None, 4),
            }
        }
        _ => (None, 1),
    }
}

impl Perform for Emulator {
    fn print(&mut self, byte: u8) {
        self.print_byte(byte);
    }

    fn print_run(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.print_byte(b);
        }
    }

    fn print_char(&mut self, ch: char) {
        self.put_char(ch);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x05 => {
                let answerback = self.config.answerback.clone();
                self.output.extend_from_slice(&answerback);
            }
            0x07 => self.events.push(Event::Bell),
            0x08 => self.cub(1),
            0x09 => self.tab(1),
            0x0A..=0x0C => self.line_feed(),
            0x0D => self.carriage_return(),
            0x0E => self.charsets.gl = 1,
            0x0F => self.charsets.gl = 0,
            0x1A => {
                // SUB displays the error character: a checkerboard on the
                // VT100, a reversed question mark on later terminals.
                let error = if self.config.model.max_level() == 1 {
                    '▒'
                } else {
                    charset::ERROR_CHARACTER
                };
                self.put_char(error);
            }
            0x84 => self.index(),
            0x85 => {
                self.index();
                self.carriage_return();
            }
            0x88 => {
                let col = self.cursor.col;
                self.tabs[col] = true;
            }
            0x8D => self.reverse_index(),
            0x8E if self.level >= 2 => self.charsets.single_shift = Some(2),
            0x8F if self.level >= 2 => self.charsets.single_shift = Some(3),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], final_byte: u8) {
        if !self.modes.ansi {
            self.vt52_escape(final_byte);
            return;
        }
        let vt220 = self.level >= 2;
        match (intermediates, final_byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'=') => self.modes.keypad_application = true,
            ([], b'>') => self.modes.keypad_application = false,
            ([], b'D') => self.index(),
            ([], b'E') => {
                self.index();
                self.carriage_return();
            }
            ([], b'H') => {
                let col = self.cursor.col;
                self.tabs[col] = true;
            }
            ([], b'M') => self.reverse_index(),
            ([], b'Z') => self.device_attributes(),
            ([], b'c') => self.full_reset(),
            ([], b'N') if vt220 => self.charsets.single_shift = Some(2),
            ([], b'O') if vt220 => self.charsets.single_shift = Some(3),
            ([], b'n') if vt220 => self.charsets.gl = 2,
            ([], b'o') if vt220 => self.charsets.gl = 3,
            ([], b'~') if vt220 => self.charsets.gr = 1,
            ([], b'}') if vt220 => self.charsets.gr = 2,
            ([], b'|') if vt220 => self.charsets.gr = 3,
            ([b'#'], b'3') => self.set_line_size(LineSize::DoubleHeightTop),
            ([b'#'], b'4') => self.set_line_size(LineSize::DoubleHeightBottom),
            ([b'#'], b'5') => self.set_line_size(LineSize::Single),
            ([b'#'], b'6') => self.set_line_size(LineSize::DoubleWidth),
            ([b'#'], b'8') => self.screen_alignment(),
            ([b' '], b'F') if vt220 => self.c1_8bit = false,
            ([b' '], b'G') if vt220 => self.c1_8bit = true,
            ([g @ (b'(' | b')' | b'*' | b'+'), rest @ ..], f) if rest.len() <= 1 => {
                let g = usize::from(g - b'(');
                if g >= 2 && !vt220 {
                    return;
                }
                self.designate(g, false, rest.first().copied(), f);
            }
            ([g @ (b'-' | b'.' | b'/'), rest @ ..], f) if rest.len() <= 1 => {
                let g = usize::from(g - b',');
                self.designate(g, true, rest.first().copied(), f);
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, seq: Sequence<'_>) {
        if !self.modes.ansi {
            return;
        }
        let p = seq.params;
        if p.has_subparams() && !self.config.extensions.xterm_sgr {
            // A DEC terminal ignores any sequence containing ':'.
            return;
        }
        let n = |i: usize| usize::from(p.get_nonzero_or(i, 1));
        let vt102 = self.config.model >= Model::Vt102;
        let vt220 = self.level >= 2;
        let vt320 = self.level >= 3;
        let vt420 = self.level >= 4;
        let vt510 = self.level >= 5;
        match (seq.private, seq.intermediates, seq.final_byte) {
            (None, [], b'@') if vt220 => self.insert_chars(n(0)),
            (None, [], b'E') if vt510 => {
                self.cud(n(0));
                self.carriage_return();
            }
            (None, [], b'F') if vt510 => {
                self.cuu(n(0));
                self.carriage_return();
            }
            (None, [], b'G' | b'`') if vt510 => self.goto(self.cursor.row, n(0) - 1),
            (None, [], b'd') if vt510 => {
                let col = self.cursor.col + 1;
                self.cup(n(0), col);
            }
            (None, [], b'I') if vt510 => self.tab(n(0)),
            (None, [], b'Z') if vt510 => self.back_tab(n(0)),
            (None, [], b'S') if vt420 && !self.status.active => self.scroll_up(n(0)),
            (None, [], b'T') if vt420 && p.len() <= 1 && !self.status.active => {
                let region = self.region(self.top);
                let blank = self.blank();
                self.grid.scroll_down(region, n(0), blank);
            }
            (None, [], b'A') => self.cuu(n(0)),
            (None, [], b'B') => self.cud(n(0)),
            (None, [], b'C') => self.cuf(n(0)),
            (None, [], b'D') => self.cub(n(0)),
            (None, [], b'H' | b'f') => self.cup(n(0), n(1)),
            (None, [], b'J') => self.erase_display(p.get_or(0, 0), false),
            (None, [], b'K') => self.erase_line(p.get_or(0, 0), false),
            (Some(b'?'), [], b'J') if vt220 => self.erase_display(p.get_or(0, 0), true),
            (Some(b'?'), [], b'K') if vt220 => self.erase_line(p.get_or(0, 0), true),
            (None, [], b'L') if vt102 => self.insert_lines(n(0)),
            (None, [], b'M') if vt102 => self.delete_lines(n(0)),
            (None, [], b'P') if vt102 => self.delete_chars(n(0)),
            (None, [], b'X') if vt220 => self.erase_chars(n(0)),
            (None, [], b'c') if p.get_or(0, 0) == 0 => self.device_attributes(),
            (Some(b'>'), [], b'c') if p.get_or(0, 0) == 0 => self.secondary_attributes(),
            (Some(b'='), [], b'c') if vt420 && p.get_or(0, 0) == 0 => self.tertiary_attributes(),
            (None, [], b'g') => match p.get_or(0, 0) {
                0 => {
                    let col = self.cursor.col;
                    self.tabs[col] = false;
                }
                3 => self.tabs.fill(false),
                _ => {}
            },
            (None, [], b'h' | b'l') => {
                for m in p.iter().filter_map(|m| m.value) {
                    self.set_ansi_mode(m, seq.final_byte == b'h');
                }
            }
            (Some(b'?'), [], b'h' | b'l') => {
                for m in p.iter().filter_map(|m| m.value) {
                    self.set_dec_mode(m, seq.final_byte == b'h');
                }
            }
            (None, [], b'm') => self.select_graphic_rendition(p),
            (None | Some(b'?'), [], b'n') => self.device_status(seq.private, p.get_or(0, 0)),
            (None, [], b'q') => {
                for v in p.iter().map(|v| v.value.unwrap_or(0)) {
                    match v {
                        0 => self.leds = 0,
                        1..=4 => self.leds |= 1 << (v - 1),
                        21..=24 => self.leds &= !(1 << (v - 21)),
                        _ => {}
                    }
                }
                self.events.push(Event::LedsChanged(self.leds));
            }
            (None, [], b'r') if !self.status.active => {
                let top = n(0) - 1;
                let bottom =
                    usize::from(p.get_nonzero_or(1, self.rows() as u16)).min(self.rows()) - 1;
                if top < bottom {
                    self.top = top;
                    self.bottom = bottom;
                    self.home();
                }
            }
            (None, [], b'x') if self.config.model.max_level() <= 3 => {
                // DECREQTPARM: no parity, 8 bits, 9600 baud, clock multiplier 1.
                let kind = p.get_or(0, 0);
                if kind <= 1 {
                    self.reply_csi(&format!("{};1;1;112;112;1;0x", kind + 2));
                }
            }
            (None, [], b'y') if p.get_or(0, 0) == 4 => {
                // DECTST: the self-test ends with the terminal reset.
                self.full_reset();
            }
            (None, [b'!'], b'p') if vt220 => self.soft_reset(),
            (None, [b'"'], b'p') => self.set_conformance_level(p),
            (None, [b'"'], b'q') if vt220 => {
                // DECSCA: 1 protects subsequent characters; 0 and 2 do not.
                let protect = p.get_or(0, 0) == 1;
                self.cursor.attrs.flags.set(Flags::PROTECTED, protect);
            }
            (None, [b'$'], b'p') if vt320 => self.request_mode(None, p.get_or(0, 0)),
            (Some(b'?'), [b'$'], b'p') if vt320 => self.request_mode(Some(b'?'), p.get_or(0, 0)),
            (None, [b'$'], b'w') if vt320 => self.presentation_state_report(p.get_or(0, 0)),
            (None, [b'&'], b'u') if vt320 => self.user_preferred_supplemental_report(),
            (None, [b'$'], b'~') if vt320 => self.set_status_type(p.get_or(0, 0)),
            (None, [b'$'], b'}') if vt320 => self.set_active_display(p.get_or(0, 0)),
            _ => {}
        }
    }

    fn dcs_hook(&mut self, seq: Sequence<'_>) {
        self.dcs = if self.modes.ansi {
            dcs::DcsState::hook(self.level, &seq)
        } else {
            dcs::DcsState::None
        };
    }

    fn dcs_put(&mut self, byte: u8) {
        self.dcs.put(byte);
    }

    fn dcs_unhook(&mut self, end: StringEnd) {
        let state = std::mem::replace(&mut self.dcs, dcs::DcsState::None);
        if end == StringEnd::Terminated {
            self.finish_dcs(state);
        }
    }

    fn vt52_cursor(&mut self, line: u8, column: u8) {
        // A VT100-family terminal in VT52 mode ignores an out-of-range line or
        // column and keeps the current value; a genuine VT52 would clamp.
        let (line, column) = (usize::from(line), usize::from(column));
        let row = if line < self.rows() {
            line
        } else {
            self.cursor.row
        };
        let col = if column < self.line_width(row) {
            column
        } else {
            self.cursor.col
        };
        self.goto(row, col);
    }

    fn pause_requested(&mut self) -> bool {
        std::mem::take(&mut self.pause)
    }
}
