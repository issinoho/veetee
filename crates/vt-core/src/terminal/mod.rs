use std::collections::VecDeque;

use vt_parser::{InputMode, Params, Parser, Perform, Sequence, StringEnd};

use crate::cell::{Attrs, Cell, Color, Flags};
use crate::charset::{self, Charset, CharsetState};
use crate::config::{Config, Model, StatusDisplay};
use crate::grid::{Grid, Line, LineSize, Region};
use crate::keyboard::{self, Key, KeyContext, KeyMods};
use crate::modes::Modes;
use crate::softfont::{SoftFonts, SoftGlyph};
use crate::udk::UserKeys;

mod capture;
mod crm;
mod dcs;
mod history;
pub(crate) mod paste;
mod rect;
mod reports;
mod setup;
pub use history::Found;
pub use setup::SoundVolumes;
mod vt520;

pub use vt520::{CursorStyle, LocalKeyAction};

/// Something the host application (GUI, headless driver) must act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// BEL: sound the warning bell.
    Bell,
    /// A printed character brought the cursor eight columns from the right
    /// margin with the margin bell on (Installing and Using the VT420, chapter 4).
    MarginBell,
    /// DECCOLM changed the page width; the window should follow.
    ColumnsChanged(usize),
    /// DECLL: bit 0 = L1 … bit 3 = L4.
    LedsChanged(u8),
    /// DECSLPP changed the number of lines per page.
    LinesChanged(usize),
    /// DECSNLS changed the number of lines the screen displays.
    ScreenLinesChanged(usize),
    /// DECSWT (or an xterm title with xterm compatibility) named the session.
    TitleChanged(String),
    /// DECSIN named the session icon.
    IconNameChanged(String),
    /// DECES: the host made this session the active one.
    SessionActivated,
    /// DECPS: play a note (1 = C5 … 25 = C7) at volume 0–7.
    PlaySound {
        volume: u8,
        duration_ms: u32,
        note: u8,
    },
}

/// One line of smooth scrolling (DECSCLM), for the display to animate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmoothScroll {
    /// Page rows and columns of the scrolling region, inclusive.
    pub top: usize,
    pub bottom: usize,
    pub left: usize,
    pub right: usize,
    /// True when the lines moved up (LF at the bottom margin).
    pub up: bool,
    /// The line that left the region, for drawing as it slides out.
    pub outgoing: crate::grid::Line,
}

/// What a key press did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Codes were sent (or nothing needed to be).
    Handled,
    /// The host programmed the key to perform local function `n`
    /// (EK-VT520-RM table 8-6), which the frontend carries out.
    LocalFunction(u16),
    /// An alphanumeric key without a program: type as usual.
    NotProgrammed,
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
        self.advance_nested(bytes, 0);
        self.emu.smooth = None;
        self.emu.couple();
    }

    fn advance_nested(&mut self, bytes: &[u8], depth: usize) {
        let mut rest = bytes;
        while !rest.is_empty() {
            if self.emu.stored.display_controls {
                let n = self.emu.show_controls(rest);
                rest = &rest[n..];
                continue;
            }
            let n = self.parser.advance_until_pause(&mut self.emu, rest);
            rest = &rest[n..];
            self.sync_parser();
            let invoked = std::mem::take(&mut self.emu.pending_input);
            // A macro runs as if received at this point; nesting is bounded
            // so a macro that invokes itself cannot hang the terminal.
            if !invoked.is_empty() && depth < 16 {
                self.advance_nested(&invoked, depth + 1);
            }
        }
    }

    /// Like [`Terminal::advance`], but in smooth scroll mode stops after the
    /// first line that scrolls, so the caller can show it at DEC speed.
    /// Returns how many bytes were processed; the rest must be passed again.
    pub fn advance_paced(&mut self, bytes: &[u8]) -> usize {
        self.emu.pacing = true;
        let used = self.advance_paced_inner(bytes);
        self.emu.pacing = false;
        used
    }

    fn advance_paced_inner(&mut self, bytes: &[u8]) -> usize {
        let mut used = 0;
        while used < bytes.len() {
            let n = if self.emu.stored.display_controls {
                self.emu.show_controls(&bytes[used..])
            } else {
                self.parser
                    .advance_until_pause(&mut self.emu, &bytes[used..])
            };
            used += n;
            self.sync_parser();
            let invoked = std::mem::take(&mut self.emu.pending_input);
            if !invoked.is_empty() {
                self.advance_nested(&invoked, 1);
            }
            if self.emu.smooth.is_some() {
                break;
            }
        }
        self.emu.couple();
        used
    }

    /// The smooth scroll that stopped [`Terminal::advance_paced`], if any.
    pub fn take_smooth_scroll(&mut self) -> Option<SmoothScroll> {
        self.emu.smooth.take()
    }

    /// Smooth scrolling speed in lines per second, or `None` for jump
    /// scrolling: Smooth 2 is 9 lines a second and Smooth 4 is 18
    /// (EK-VT520-RM DECSSCLS, section 2.8.6).
    pub fn smooth_scroll_rate(&self) -> Option<u32> {
        if !self.emu.modes.smooth_scroll {
            return None;
        }
        match self.emu.setup.selection(b" p").parse::<u8>().unwrap_or(0) {
            0..=3 => Some(9),
            4..=8 => Some(18),
            _ => None,
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

    /// The VT525 colour table and colour modes; `None` on monochrome models.
    pub fn colors(&self) -> Option<(&crate::color::ColorTable, crate::color::ColorOptions)> {
        self.emu
            .color_terminal()
            .then(|| (&self.emu.colors, self.emu.color_options()))
    }

    /// Local panning (Ctrl with the cursor keys): moves the user window
    /// `lines` down (negative: up) within the displayed page.
    pub fn pan_view(&mut self, lines: isize) {
        self.emu.pan(lines);
    }

    /// Local page view (Ctrl with Prev/Next): shows another page without
    /// moving the cursor.
    pub fn view_page(&mut self, delta: isize) {
        let count = self.emu.page_count() as isize;
        let page = (self.emu.display_page as isize + delta).clamp(0, count - 1);
        self.emu.display_page = page as usize;
    }

    /// Tells the terminal how many sessions share its window, for DSR ?85.
    pub fn set_sessions(&mut self, count: u8) {
        self.emu.sessions = count.max(1);
    }

    /// DECLFKC: what local function key F1–F4 (`key` 1–4) does.
    pub fn local_function_key(&self, key: u8) -> LocalKeyAction {
        match key {
            1..=4 if self.emu.level >= 5 => {
                self.emu.setup.local_function_keys[usize::from(key) - 1]
            }
            _ => LocalKeyAction::Local,
        }
    }

    /// DECELF: whether the copy and paste keys (group 1) are enabled.
    pub fn copy_paste_keys_enabled(&self) -> bool {
        self.emu.level < 5 || self.emu.setup.local_functions[0]
    }

    /// The cursor style selected with DECSCUSR.
    pub fn cursor_style(&self) -> CursorStyle {
        self.emu.setup.cursor_style
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

    /// The page the cursor is on (zero-based) and the number of pages.
    pub fn page(&self) -> (usize, usize) {
        (self.emu.page, self.emu.page_count())
    }

    /// The page shown on the screen, which differs from the cursor's page
    /// when page cursor coupling (DECPCCM) is off.
    pub fn display_grid(&self) -> &Grid {
        self.emu.page_grid(self.emu.display_page)
    }

    /// Whether the cursor is on the displayed page.
    pub fn cursor_on_display(&self) -> bool {
        self.emu.display_page == self.emu.page
    }

    /// The user window: first displayed page line and the number of screen lines.
    pub fn window(&self) -> (usize, usize) {
        (self.emu.window_top, self.emu.screen_lines)
    }

    /// Left and right margins (DECSLRM), zero-based inclusive.
    pub fn lr_margins(&self) -> (usize, usize) {
        (self.emu.left, self.emu.right)
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
        self.key_with(key, KeyMods::NONE);
    }

    /// A DEC key pressed with modifiers. A VT520 key the host has programmed
    /// (DECPFK) sends its sequence or asks for a local function instead.
    pub fn key_with(&mut self, key: Key, mods: KeyMods) -> KeyOutcome {
        if self.emu.modes.keyboard_locked {
            return KeyOutcome::Handled;
        }
        let vt500 = self.emu.config.model.max_level() >= 5 && self.emu.modes.ansi;
        let station = crate::keyprog::station_of(key).filter(|_| vt500);
        // Shifted F6–F20 are the function keys with Shift.
        let (key, mods) = match key {
            Key::UserDefined(n) if station.is_some() => (
                Key::UserDefined(n),
                KeyMods {
                    shift: true,
                    ..mods
                },
            ),
            k => (k, mods),
        };
        if let Some(station) = station {
            if let Some(program) = self.emu.keyprog.function(station, mods).cloned() {
                return self.run_program(&program);
            }
            // DECCKD: the key behaves as another key's default.
            let source = self.emu.keyprog.default_source(station);
            if source != station {
                if let Some(other) = crate::keyprog::key_at(source) {
                    let bytes = self.key_bytes(other, mods);
                    self.transmit(&bytes);
                    return KeyOutcome::Handled;
                }
            }
        }
        let bytes = self.key_bytes(key, mods);
        self.transmit(&bytes);
        KeyOutcome::Handled
    }

    /// The codes a key sends by default in the current modes.
    fn key_bytes(&self, key: Key, mods: KeyMods) -> Vec<u8> {
        let mut bytes = Vec::new();
        let e = &self.emu;
        if let Key::UserDefined(f) = key {
            if e.level >= 2 {
                match e.udk.get(f) {
                    Some(s) => bytes.extend_from_slice(s),
                    // A VT500's undefined shifted F6–F20 send their DECFNK
                    // sequence (EK-VT510-RM DECFNK notes).
                    None if e.level >= 5 && e.modes.ansi => {
                        if let Some(n) = keyboard::function_key_number(f) {
                            if e.c1_8bit {
                                bytes.push(0x9B);
                            } else {
                                bytes.extend_from_slice(b"\x1b[");
                            }
                            bytes.extend_from_slice(format!("{n};2~").as_bytes());
                        }
                    }
                    None => {}
                }
            }
        } else {
            let cx = KeyContext {
                ansi: e.modes.ansi,
                level: e.level,
                eight_bit: e.c1_8bit,
                cursor_app: e.modes.cursor_keys_application,
                keypad_app: e.modes.keypad_application,
                new_line: e.modes.new_line,
                backarrow_bs: e.modes.backarrow_sends_bs,
            };
            keyboard::encode_with(key, mods, cx, &mut bytes);
        }
        bytes
    }

    fn run_program(&mut self, program: &crate::keyprog::Program) -> KeyOutcome {
        use crate::keyprog::{Direction, SEND_SEQUENCE};
        match program.function {
            0 => KeyOutcome::Handled,
            SEND_SEQUENCE => {
                match program.direction {
                    Direction::Local => self.advance(&program.uds),
                    Direction::Remote => self.emu.output.extend_from_slice(&program.uds),
                    Direction::Normal => self.transmit(&program.uds),
                }
                KeyOutcome::Handled
            }
            // BS, CAN, ESC and DEL (EK-VT520-RM table 8-6).
            91 => self.send_code(0x08),
            92 => self.send_code(0x18),
            93 => self.send_code(0x1B),
            94 => self.send_code(0x7F),
            n => KeyOutcome::LocalFunction(n),
        }
    }

    fn send_code(&mut self, code: u8) -> KeyOutcome {
        self.transmit(&[code]);
        KeyOutcome::Handled
    }

    /// A key on the main keypad, by LK411 station. When the host has
    /// programmed it (DECPAK) the programmed codes are sent and `Handled`
    /// is returned; otherwise `NotProgrammed` lets typing proceed normally.
    pub fn alphanumeric_key(&mut self, station: u8, mods: KeyMods, alt_graph: bool) -> KeyOutcome {
        if self.emu.config.model.max_level() < 5 || !self.emu.modes.ansi {
            return KeyOutcome::NotProgrammed;
        }
        let Some(definition) = self.emu.keyprog.alphanumeric(station).cloned() else {
            return KeyOutcome::NotProgrammed;
        };
        if mods.alt && !alt_graph {
            return match definition.alt {
                Some(program) => self.run_program(&program),
                None => KeyOutcome::NotProgrammed,
            };
        }
        let state = match (mods.ctrl, alt_graph, mods.shift) {
            (true, _, _) => 6,
            (false, false, false) => 0,
            (false, false, true) => 1,
            (false, true, false) => 3,
            (false, true, true) => 4,
        };
        match &definition.codes[state] {
            Some(codes) => {
                let codes = codes.clone();
                self.transmit(&codes);
                KeyOutcome::Handled
            }
            None => KeyOutcome::NotProgrammed,
        }
    }

    /// Characters typed on the main keypad, including C0 controls produced
    /// with Ctrl. Characters the current mode cannot transmit are dropped.
    pub fn type_text(&mut self, text: &str) {
        if self.emu.modes.keyboard_locked {
            return;
        }
        let e = &self.emu;
        let mut bytes = Vec::new();
        for ch in text.chars() {
            if e.config.extensions.utf8 {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            } else if ch.is_ascii_control() {
                bytes.push(ch as u8);
            } else if let Some(encoded) = self.encode_typed(ch) {
                bytes.extend(encoded);
            }
        }
        self.transmit(&bytes);
    }

    /// The bytes a typed graphic character is sent as, or `None` if the
    /// character sets in use cannot send it.
    fn encode_typed(&self, ch: char) -> Option<Vec<u8>> {
        let e = &self.emu;
        let national = e
            .modes
            .national
            .then_some(e.config.keyboard_language)
            .flatten();
        if let Some(nrc) = national {
            return nrc.encode(ch).map(|b| vec![b]);
        }
        if ch.is_ascii() {
            return Some(vec![ch as u8]);
        }
        if e.level < 2 || e.modes.national {
            return None;
        }
        let encoded = match e.upss {
            Charset::IsoLatin1 => charset::encode_latin1(ch),
            // A VT500 supplemental set as GR, with ASCII as GL.
            set @ Charset::Vt500(_) => (0x20..=0x7F)
                .find(|&c| set.map(c) == Some(ch) && ch != charset::ERROR_CHARACTER)
                .map(|c| c | 0x80),
            _ => charset::encode_dec_multinational(ch),
        };
        encoded.map(|b| vec![b])
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
    /// The page the cursor is on. Other pages live in `pages`.
    grid: Grid,
    /// Page memory; the slot for `page` is an empty placeholder while that
    /// page is in `grid`.
    pages: Vec<Grid>,
    page: usize,
    /// The page shown in the user window.
    display_page: usize,
    /// First page line shown in the user window.
    window_top: usize,
    /// Lines the screen displays (DECSNLS).
    screen_lines: usize,
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
    left: usize,
    right: usize,
    /// DECSACE: DECCARA/DECRARA affect the whole rectangle (true) or a stream.
    sace_rectangle: bool,
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
    /// DECDMAC definitions, indexed by macro ID (0–63).
    macros: Vec<Vec<u8>>,
    /// Macro text waiting to be processed as input (DECINVM).
    pending_input: Vec<u8>,
    /// VT500 Set-Up selections and modes.
    setup: vt520::SetUp,
    /// VT525 colour map and colour assignments.
    colors: crate::color::ColorTable,
    /// OSC string being received.
    osc: Vec<u8>,
    /// Sessions open in the terminal window (for the multiple session report).
    sessions: u8,
    /// VT520 programmed keys.
    keyprog: crate::keyprog::KeyPrograms,
    /// Set-Up features with no other home in the terminal.
    stored: crate::setup::Features,
    /// A smooth scroll that has happened and not yet been taken.
    smooth: Option<SmoothScroll>,
    /// Processing is paced ([`Terminal::advance_paced`]), so smooth scrolls
    /// are recorded and stop the parser.
    pacing: bool,
    /// The last bytes shown in Display Controls mode, to recognise DECSR.
    crm_tail: Vec<u8>,
    /// Text written to the page, for a session log.
    capture: Option<String>,
    output: Vec<u8>,
    events: Vec<Event>,
    pause: bool,
}

const TAB_WIDTH: usize = 8;

/// Pages available for a page length with one session: EK-VT420-RM table 6-1;
/// for the VT500s, EK-VT520-RM DECSLPP.
// 🔎 The VT520 manual gives both "6 pages of 24 lines" and a list starting
// "3 pages x 24 lines"; the list is used.
fn pages_for_length(model: Model, lines: usize) -> usize {
    if model.max_level() >= 5 {
        return match lines {
            0..=24 => 3,
            25..=36 => 2,
            _ => 1,
        };
    }
    match lines {
        0..=24 => 6,
        25 => 5,
        26..=36 => 4,
        37..=48 => 3,
        49..=72 => 2,
        _ => 1,
    }
}

/// Page lengths DECSLPP accepts.
fn page_lengths(model: Model) -> &'static [usize] {
    if model.max_level() >= 5 {
        &[24, 25, 36, 41, 42, 48, 52, 53, 72]
    } else {
        &[24, 25, 36, 48, 72, 144]
    }
}

/// Screen heights the terminal can display (DECSNLS): the VT420's 24, 36
/// and 48 lines, or the VT500s' 26, 42 and 53 data lines.
fn screen_heights(model: Model) -> &'static [usize] {
    if model.max_level() >= 5 {
        &[26, 42, 53]
    } else {
        &[24, 36, 48]
    }
}

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
        // A VT420 with one session has 6 pages of 24 lines (EK-VT420-RM, DECSLPP).
        let page_count = if model.max_level() >= 4 {
            pages_for_length(model, rows)
        } else {
            1
        };
        let mut emu = Emulator {
            grid: Grid::new(rows, cols),
            pages: (0..page_count)
                .map(|i| {
                    if i == 0 {
                        Grid::new(0, 0)
                    } else {
                        Grid::new(rows, cols)
                    }
                })
                .collect(),
            page: 0,
            display_page: 0,
            window_top: 0,
            screen_lines: rows,
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
            modes: Modes {
                smooth_scroll: model.smooth_scroll_default(),
                ..Modes::power_up(
                    config.autowrap,
                    config.new_line,
                    config.national_mode && model.max_level() >= 2,
                )
            },
            top: 0,
            bottom: rows - 1,
            left: 0,
            right: cols - 1,
            sace_rectangle: false,
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
            macros: vec![Vec::new(); 64],
            pending_input: Vec::new(),
            setup: vt520::SetUp::default(),
            colors: crate::color::ColorTable::default(),
            osc: Vec::new(),
            sessions: 1,
            keyprog: crate::keyprog::KeyPrograms::default(),
            stored: crate::setup::Features::factory(model),
            smooth: None,
            pacing: false,
            crm_tail: Vec::new(),
            capture: None,
            output: Vec::new(),
            events: Vec::new(),
            pause: false,
            config,
        };
        if let Some(features) = emu.config.setup.clone() {
            emu.apply_features(&features);
            emu.events.clear();
        }
        emu
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

    /// Attributes for newly written characters (VT525 colour modes apply).
    fn writing_attrs(&self) -> Attrs {
        if self.color_terminal() {
            self.colors.attrs_for_writing(self.cursor.attrs)
        } else {
            self.cursor.attrs
        }
    }

    /// The cell left by erasing or scrolling: the current text background,
    /// or the screen background with DECECM set.
    fn blank(&self) -> Cell {
        if self.vt520_flag(vt520::DECECM) {
            Cell::erased(Color::Default)
        } else {
            Cell::erased(self.cursor.attrs.bg)
        }
    }

    fn region(&self, top: usize) -> Region {
        Region {
            top,
            bottom: self.bottom,
            left: self.left,
            right: self.right,
        }
    }

    /// Cursor inside both the top/bottom and left/right margins.
    fn within_margins(&self) -> bool {
        (self.top..=self.bottom).contains(&self.cursor.row) && self.within_lr_margins()
    }

    fn within_lr_margins(&self) -> bool {
        (self.left..=self.right).contains(&self.cursor.col)
    }

    /// Resets all four margins to the page borders.
    fn reset_margins(&mut self) {
        self.top = 0;
        self.bottom = self.rows() - 1;
        self.left = 0;
        self.right = self.cols() - 1;
    }

    /// The rightmost column the cursor can write or move to from where it is:
    /// the right margin when inside the margins, otherwise the line end.
    fn right_limit(&self) -> usize {
        let last = self.last_col();
        if !self.status.active && self.within_lr_margins() {
            self.right.min(last)
        } else {
            last
        }
    }

    // ------------------------------------------------------------- pages

    fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn page_grid(&self, page: usize) -> &Grid {
        if page == self.page {
            &self.grid
        } else {
            &self.pages[page]
        }
    }

    fn page_grid_mut(&mut self, page: usize) -> &mut Grid {
        if page == self.page {
            &mut self.grid
        } else {
            &mut self.pages[page]
        }
    }

    /// Moves the cursor to another page, keeping its position.
    fn switch_page(&mut self, page: usize) {
        let page = page.min(self.page_count() - 1);
        if page != self.page {
            let current = std::mem::replace(&mut self.grid, Grid::new(0, 0));
            self.pages[self.page] = current;
            self.grid = std::mem::replace(&mut self.pages[page], Grid::new(0, 0));
            self.page = page;
        }
        if self.modes.page_coupling {
            self.display_page = page;
        }
        let (row, col) = (self.cursor.row, self.cursor.col);
        self.goto(row, col);
    }

    /// NP/PP (`home`) or PPA/PPR/PPB.
    fn move_page(&mut self, target: isize, home: bool) {
        if self.page_count() < 2 || self.status.active {
            return;
        }
        let page = target.clamp(0, self.page_count() as isize - 1) as usize;
        self.switch_page(page);
        if home {
            self.goto(0, 0);
        }
    }

    /// Applies `f` to every page in page memory.
    fn for_each_page(&mut self, mut f: impl FnMut(&mut Grid)) {
        f(&mut self.grid);
        let current = self.page;
        for (i, page) in self.pages.iter_mut().enumerate() {
            if i != current {
                f(page);
            }
        }
    }

    /// DECSLPP: lines per page; the number of pages follows the page length.
    fn set_page_length(&mut self, requested: usize) {
        let model = self.config.model;
        let lengths = page_lengths(model);
        let lines = if model.max_level() >= 5 {
            // VT500s use the next supported length, or the longest (EK-VT520-RM).
            lengths
                .iter()
                .copied()
                .find(|&l| l >= requested)
                .unwrap_or(lengths[lengths.len() - 1])
        } else if lengths.contains(&requested) {
            requested
        } else {
            return;
        };
        if lines == self.rows() {
            return;
        }
        self.exit_status_line();
        let cols = self.cols();
        let count = pages_for_length(model, lines);
        // Margins at the page limits (the default) stay at the page limits.
        let full_page = self.top == 0 && self.bottom == self.rows() - 1;
        self.for_each_page(|g| g.resize(lines, cols));
        if self.page >= count {
            self.switch_page(count - 1);
        }
        self.pages.resize_with(count, || Grid::new(lines, cols));
        self.pages.truncate(count);
        self.display_page = self.display_page.min(count - 1);
        // Otherwise DECSLPP keeps the margins unless they no longer fit (RM420).
        if full_page || self.bottom >= lines {
            self.top = 0;
            self.bottom = lines - 1;
        }
        let (row, col) = (self.cursor.row, self.cursor.col);
        self.goto(row, col);
        self.couple();
        self.events.push(Event::LinesChanged(lines));
    }

    /// DECSCPP: 80 or 132 columns without clearing page memory.
    fn set_page_width(&mut self, cols: usize) {
        if cols == self.cols() {
            return;
        }
        self.exit_status_line();
        let rows = self.rows();
        self.modes.columns_132 = cols == 132;
        self.for_each_page(|g| g.resize(rows, cols));
        self.status_line.resize(cols, Cell::BLANK);
        self.resize_tabs(cols);
        if self.right >= cols {
            self.left = 0;
            self.right = cols - 1;
        }
        let (row, col) = (self.cursor.row, self.cursor.col);
        self.goto(row, col);
        self.events.push(Event::ColumnsChanged(cols));
    }

    /// DECSNLS: the terminal uses the next supported screen height.
    fn set_screen_lines(&mut self, requested: usize) {
        let heights = screen_heights(self.config.model);
        // A VT500 status line takes one of the screen's data lines.
        let status = usize::from(
            self.config.model.max_level() >= 5 && self.status.kind != StatusDisplay::None,
        );
        let lines = heights
            .iter()
            .map(|h| h - status)
            .find(|&l| l >= requested)
            .unwrap_or(heights[heights.len() - 1] - status);
        if lines != self.screen_lines {
            self.screen_lines = lines;
            self.couple();
            self.events.push(Event::ScreenLinesChanged(lines));
        }
    }

    /// SU/SD on DEC terminals: move the user window within the page.
    fn pan(&mut self, delta: isize) {
        let max_top = self.rows().saturating_sub(self.screen_lines);
        self.window_top = (self.window_top as isize + delta).clamp(0, max_top as isize) as usize;
    }

    /// DECVCCM: pan the user window to keep the cursor in view.
    fn couple(&mut self) {
        let max_top = self.rows().saturating_sub(self.screen_lines);
        if self.modes.vertical_coupling && self.display_page == self.page {
            let row = self.cursor.row;
            if row < self.window_top {
                self.window_top = row;
            } else if row >= self.window_top + self.screen_lines {
                self.window_top = row + 1 - self.screen_lines;
            }
        }
        self.window_top = self.window_top.min(max_top);
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
        if row != self.cursor.row {
            self.capture_line_break();
        }
        self.cursor.row = row;
        self.cursor.col = col.min(self.line_width(row) - 1);
        self.cursor.pending_wrap = false;
    }

    /// CUP/HVP with one-based coordinates, honouring DECOM.
    fn cup(&mut self, row: usize, col: usize) {
        let (row, col) = (row.max(1) - 1, col.max(1) - 1);
        if self.modes.origin {
            let row = (self.top + row).min(self.bottom);
            let col = (self.left + col).min(self.right);
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

    // 🔎 The VT420/VT510 manuals say CUF and CUB stop at the page border;
    // like xterm, they stop at the left/right margin when starting inside it.
    fn cuf(&mut self, n: usize) {
        let col = (self.cursor.col + n).min(self.right_limit());
        self.goto(self.cursor.row, col);
    }

    fn cub(&mut self, n: usize) {
        let limit = if self.cursor.col >= self.left && !self.status.active {
            self.left
        } else {
            0
        };
        let col = self.cursor.col.saturating_sub(n).max(limit);
        self.goto(self.cursor.row, col);
    }

    /// CR returns to the left margin, or column 1 from left of it.
    fn carriage_return(&mut self) {
        let col = if self.cursor.col >= self.left && !self.status.active {
            self.left
        } else {
            0
        };
        self.goto(self.cursor.row, col);
    }

    /// IND: down one line, scrolling the region if at the bottom margin.
    fn index(&mut self) {
        if self.status.active {
            return;
        }
        if self.cursor.row == self.bottom {
            // Outside the left/right margins nothing scrolls.
            if self.within_lr_margins() {
                self.note_smooth_scroll(true);
                self.scroll_up(1);
            }
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
            if self.within_lr_margins() {
                self.note_smooth_scroll(false);
                let region = self.region(self.top);
                let blank = self.blank();
                self.grid.scroll_down(region, 1, blank);
            }
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

    /// In smooth scroll mode, records the line about to scroll and stops the
    /// parser, so the display can show the scroll before more is processed.
    fn note_smooth_scroll(&mut self, up: bool) {
        if !self.modes.smooth_scroll || !self.pacing {
            return;
        }
        let row = if up { self.top } else { self.bottom };
        self.smooth = Some(SmoothScroll {
            top: self.top,
            bottom: self.bottom,
            left: self.left,
            right: self.right,
            up,
            outgoing: self.grid.line(row).clone(),
        });
        self.pause = true;
    }

    fn scroll_up(&mut self, n: usize) {
        let region = self.region(self.top);
        let blank = self.blank();
        let capacity = self.config.scrollback_lines;
        let keep = self.top == 0 && self.page == 0 && capacity > 0;
        let scrollback = &mut self.scrollback;
        // Lines leaving the page go to the scrollback; the blank lines that
        // replace them reuse the oldest scrollback line, or the line itself.
        self.grid.scroll_up_with(region, n, blank, |line| {
            if !keep {
                return Some(line);
            }
            let spare = if scrollback.len() >= capacity {
                scrollback.pop_front()
            } else {
                None
            };
            scrollback.push_back(line);
            spare
        });
    }

    fn tab(&mut self, n: usize) {
        let last = if self.cursor.col <= self.right && !self.status.active {
            self.right.min(self.last_col())
        } else {
            self.last_col()
        };
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

    fn put_char(&mut self, ch: char, code: u8) {
        if self.cursor.pending_wrap && self.modes.autowrap && !self.status.active {
            self.grid.line_mut(self.cursor.row).wrapped = true;
            // Wrap to the left margin of the next line (column 1 if the cursor
            // was right of the right margin).
            let inside = self.within_lr_margins();
            self.index();
            self.cursor.col = if inside { self.left } else { 0 };
        }
        let col = self.cursor.col;
        let last = self.right_limit();
        if !self.status.active {
            self.capture_text(ch);
        }
        let cell = Cell {
            ch,
            attrs: self.writing_attrs(),
            code: code.max(1),
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
            if last >= 8 && self.cursor.col == last - 8 && self.setup.selection(b" u") != "1" {
                self.events.push(Event::MarginBell);
            }
        }
    }

    /// Writes the GL characters at the start of `bytes` in one go, when that
    /// is exactly what writing them a character at a time would do: ASCII or
    /// DEC Special Graphics in GL, no single shift, insert mode or national
    /// mode, and no character reaching the right margin (where autowrap
    /// applies). Returns how many characters were written; 0 leaves them to
    /// [`Emulator::print_byte`].
    fn print_ascii_run(&mut self, bytes: &[u8]) -> usize {
        let cs = &self.charsets;
        if !self.modes.ansi
            || self.modes.national
            || self.modes.insert
            || self.cursor.pending_wrap
            || cs.single_shift.is_some()
        {
            return 0;
        }
        let set = cs.g[usize::from(cs.gl)];
        if !matches!(set, Charset::Ascii | Charset::DecSpecialGraphics) {
            return 0;
        }
        let col = self.cursor.col;
        let last = self.right_limit();
        // Left of the left margin, the right margin applies once the cursor
        // enters the margins: stop there and look again.
        let stop = if !self.status.active && self.modes.lr_margins && col < self.left {
            self.left.min(last)
        } else {
            last
        };
        let n = bytes
            .iter()
            .take(stop.saturating_sub(col))
            .take_while(|b| (0x20..0x7F).contains(*b))
            .count();
        if n == 0 {
            return 0;
        }
        let attrs = self.writing_attrs();
        // Every GL code of these sets maps to a character.
        let glyph = |code: u8| match set {
            Charset::Ascii => char::from(code),
            _ => set.map(code).unwrap_or(char::from(code)),
        };
        if !self.status.active {
            if let Some(text) = &mut self.capture {
                text.extend(bytes[..n].iter().map(|&b| glyph(b)));
            }
        }
        let line = self.cursor_line_mut();
        for (cell, &code) in line.cells_mut()[col..col + n].iter_mut().zip(bytes) {
            *cell = Cell {
                ch: glyph(code),
                attrs,
                code,
            };
        }
        self.cursor.col = col + n;
        // The margin bell rings as the cursor reaches eight columns from the margin.
        if last >= 8
            && (col + 1..=col + n).contains(&(last - 8))
            && self.setup.selection(b" u") != "1"
        {
            self.events.push(Event::MarginBell);
        }
        n
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
        let code = if self.modes.national || !self.modes.ansi {
            byte & 0x7F
        } else {
            byte
        };
        if let Some(ch) = ch {
            self.put_char(ch, code);
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

    /// ICH: ignored when the cursor is outside the left/right margins.
    fn insert_chars(&mut self, n: usize) {
        if !self.status.active && !self.within_lr_margins() {
            return;
        }
        let (col, last) = (self.cursor.col, self.right_limit());
        let blank = self.blank();
        self.cursor_line_mut().insert(col, last, n, blank);
        self.cursor.pending_wrap = false;
    }

    fn delete_chars(&mut self, n: usize) {
        if !self.status.active && !self.within_lr_margins() {
            return;
        }
        let (col, last) = (self.cursor.col, self.right_limit());
        let blank = self.blank();
        self.cursor_line_mut().delete(col, last, n, blank);
        self.cursor.pending_wrap = false;
    }

    /// DECIC/DECDC: insert or delete columns at the cursor within the
    /// scrolling margins. No effect outside them.
    fn insert_columns(&mut self, n: usize, insert: bool) {
        if self.status.active || !self.within_margins() {
            return;
        }
        let (col, right) = (self.cursor.col, self.right);
        let blank = Cell::BLANK;
        for row in self.top..=self.bottom {
            let line = self.grid.line_mut(row);
            if insert {
                line.insert(col, right, n, blank);
            } else {
                line.delete(col, right, n, blank);
            }
        }
        self.cursor.pending_wrap = false;
    }

    /// DECBI (ESC 6) moves left; at the left margin the region's contents shift right.
    fn back_index(&mut self) {
        if self.status.active {
            return;
        }
        let col = self.cursor.col;
        if col == self.left && (self.top..=self.bottom).contains(&self.cursor.row) {
            let (left, right) = (self.left, self.right);
            for row in self.top..=self.bottom {
                self.grid.line_mut(row).insert(left, right, 1, Cell::BLANK);
            }
        } else if col > 0 {
            self.goto(self.cursor.row, col - 1);
        }
    }

    /// DECFI (ESC 9) moves right; at the right margin the region's contents shift left.
    fn forward_index(&mut self) {
        if self.status.active {
            return;
        }
        let col = self.cursor.col;
        if col == self.right && (self.top..=self.bottom).contains(&self.cursor.row) {
            let (left, right) = (self.left, self.right);
            for row in self.top..=self.bottom {
                self.grid.line_mut(row).delete(left, right, 1, Cell::BLANK);
            }
        } else if col < self.last_col() {
            self.goto(self.cursor.row, col + 1);
        }
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
            61 if self.level >= 4 => self.modes.vertical_coupling = on,
            64 if self.level >= 4 => self.modes.page_coupling = on,
            68 if self.level >= 4 => self.modes.data_processing_keys = on,
            69 if self.level >= 4 => {
                self.modes.lr_margins = on;
                if on {
                    // Line attributes in page memory become single width (EK-VT420-RM).
                    self.for_each_page(|grid| {
                        for row in 0..grid.rows() {
                            grid.line_mut(row).size = LineSize::Single;
                        }
                    });
                } else {
                    self.left = 0;
                    self.right = self.cols() - 1;
                }
            }
            _ if self.level >= 5 => {
                self.set_vt520_mode(mode, on);
            }
            _ => {}
        }
    }

    /// DECCOLM: the page is cleared, margins reset and the cursor homed.
    fn set_columns(&mut self, cols: usize) {
        self.exit_status_line();
        self.modes.columns_132 = cols == 132;
        let rows = self.rows();
        // DECCOLM erases all of page memory (EK-VT420-RM) unless DECNCSM is set.
        let clear = !self.vt520_flag(vt520::DECNCSM);
        self.for_each_page(|g| {
            g.resize(rows, cols);
            if clear {
                g.clear(Cell::BLANK);
            }
        });
        self.status_line.resize(cols, Cell::BLANK);
        // Tab stops are not reset (DEC STD 070); new columns get the default stops.
        self.resize_tabs(cols);
        // DECCOLM resets all margins and makes left/right margins unavailable.
        self.modes.lr_margins = false;
        self.reset_margins();
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
        let output = std::mem::take(&mut self.output);
        let events = std::mem::take(&mut self.events);
        let capture = self.capture.take();
        let pacing = self.pacing;
        *self = Emulator::new(Config { rows, ..config });
        self.capture = capture;
        self.pacing = pacing;
        self.scrollback = scrollback;
        self.soft_generation = generation;
        self.output = output;
        self.events = events;
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
        // DEC's table resets DECAWM here. veetee returns it to the Set-Up
        // value instead: EDT sends DECSTR as it exits, and a terminal left
        // with no auto wrap cannot draw the next EDT session (see
        // Config::autowrap).
        self.modes.autowrap = self.config.autowrap;
        self.modes.national = false;
        self.modes.keyboard_locked = false;
        self.modes.keypad_application = false;
        self.modes.cursor_keys_application = false;
        // DEC STD 070 also resets left/right margin mode.
        self.modes.lr_margins = false;
        // VT520: key position mode and cursor direction (EK-VT520-RM table 5-6).
        self.set_vt520_mode(vt520::DECKPM, false);
        self.set_vt520_mode(vt520::DECRLM, false);
        self.reset_margins();
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
            code: b'E',
        };
        self.grid.clear(fill);
        self.reset_margins();
        self.modes.origin = false;
        self.goto(0, 0);
    }

    fn set_line_size(&mut self, size: LineSize) {
        // DECDWL/DECDHL are ignored while left/right margins are available.
        if self.status.active || (self.modes.lr_margins && size != LineSize::Single) {
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
        self.for_each_page(|g| g.resize(rows, cols));
        self.screen_lines = rows;
        self.window_top = 0;
        self.status_line.resize(cols, Cell::BLANK);
        self.resize_tabs(cols);
        self.reset_margins();
        let (row, col) = (self.cursor.row, self.cursor.col);
        self.goto(row, col);
    }

    // ------------------------------------------------------------------ SGR

    fn select_graphic_rendition(&mut self, params: &Params) {
        let colors = self.color_terminal() || self.config.extensions.xterm_sgr;
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
                2 if xterm => a.flags.set(Flags::DIM, true),
                4 => a.flags.set(Flags::UNDERLINE, true),
                5 => a.flags.set(Flags::BLINK, true),
                7 => a.flags.set(Flags::REVERSE, true),
                8 if vt220 => a.flags.set(Flags::INVISIBLE, true),
                22 if vt220 => {
                    a.flags.set(Flags::BOLD, false);
                    a.flags.set(Flags::DIM, false);
                }
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
        if level >= 5 {
            if let Some(set) = charset::Vt500Set::from_designator(is_96, intermediate, final_byte) {
                if !set.is_national() || self.modes.national {
                    self.charsets.g[g] = Charset::Vt500(set);
                }
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

    fn print_run(&mut self, mut bytes: &[u8]) {
        while let Some(&first) = bytes.first() {
            let n = self.print_ascii_run(bytes);
            if n == 0 {
                self.print_byte(first);
                bytes = &bytes[1..];
            } else {
                bytes = &bytes[n..];
            }
        }
    }

    fn print_char(&mut self, ch: char) {
        let code = u8::try_from(u32::from(ch)).unwrap_or(b'?');
        self.put_char(ch, code);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x05 => {
                let answerback = self.config.answerback.clone();
                self.output.extend_from_slice(&answerback);
            }
            0x07 => self.events.push(Event::Bell),
            0x08 => {
                self.capture_backspace();
                self.cub(1);
            }
            0x09 => {
                self.capture_text('\t');
                self.tab(1);
            }
            0x0A..=0x0C => {
                self.capture_newline();
                self.line_feed();
            }
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
                self.put_char(error, b'?');
            }
            0x84 => {
                self.capture_newline();
                self.index();
            }
            0x85 => {
                self.capture_newline();
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
            ([], b'6') if self.level >= 4 => self.back_index(),
            ([], b'9') if self.level >= 4 => self.forward_index(),
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
        let xterm = self.config.extensions.xterm_compat;
        let vt510 = self.level >= 5;
        if vt510 && seq.private.is_none() && self.vt520_csi(seq.intermediates, seq.final_byte, p) {
            return;
        }
        match (seq.private, seq.intermediates, seq.final_byte) {
            (None, [], b'@') if vt220 => self.insert_chars(n(0)),
            (Some(b'?'), [], b'W') if vt510 && p.get_or(0, 0) == 5 => self.tab_every_8(),
            // DECSR: EK-VT420-RM chapter 13 and EK-VT520-RM.
            (None, [b'+'], b'p') if self.config.model.max_level() >= 4 => self.secure_reset(p),
            (None, [b'+'], b'z') if self.config.model.max_level() >= 5 => {
                self.keyprog.key_action(p.get_or(0, 0))
            }
            (None, [b'+'], b'x') if self.config.model.max_level() >= 5 => {
                let free = self.keyprog.free();
                self.reply_csi(&format!("{};{free}+y", crate::keyprog::MEMORY));
            }
            (None, [b','], b'w') if self.config.model.max_level() >= 5 => {
                self.key_definition_report(p.get_or(0, 0), p.get_or(1, 0))
            }
            (None, [b','], b'u') if self.config.model.max_level() >= 5 => {
                let station = p.get_or(0, 0);
                let kind = match u8::try_from(station) {
                    Ok(s) if crate::keyprog::key_at(s).is_some() => 1,
                    Ok(s) if crate::keyprog::is_alphanumeric(s) => 0,
                    _ => return,
                };
                self.reply_csi(&format!("{station};{kind},v"));
            }
            (None, [b' '], b'~') if self.config.model.max_level() >= 5 => {
                self.terminal_mode_emulation(p.get_or(0, 0))
            }
            (None, [], b'S') if vt420 && !self.status.active => {
                if self.config.extensions.xterm_compat {
                    self.scroll_up(n(0));
                } else {
                    self.pan(n(0) as isize);
                }
            }
            (None, [], b'T') if vt420 && p.len() <= 1 && !self.status.active => {
                if self.config.extensions.xterm_compat {
                    let region = self.region(self.top);
                    let blank = self.blank();
                    self.grid.scroll_down(region, n(0), blank);
                } else {
                    self.pan(-(n(0) as isize));
                }
            }
            // SCOSC/SCORC as xterm and the SCO console use them.
            (None, [], b's') if xterm && !self.modes.lr_margins && p.is_empty() => {
                self.save_cursor()
            }
            (None, [], b'u') if xterm && p.is_empty() => self.restore_cursor(),
            (None, [], b's') if vt420 && self.modes.lr_margins && !self.status.active => {
                let left = n(0) - 1;
                let right =
                    usize::from(p.get_nonzero_or(1, self.cols() as u16)).min(self.cols()) - 1;
                if left < right {
                    self.left = left;
                    self.right = right;
                    self.home();
                }
            }
            (None, [], b'U') if vt420 => self.move_page((self.page + n(0)) as isize, true),
            (None, [], b'V') if vt420 => self.move_page(self.page as isize - n(0) as isize, true),
            (None, [b' '], b'P') if vt420 => self.move_page(n(0) as isize - 1, false),
            (None, [b' '], b'Q') if vt420 => self.move_page((self.page + n(0)) as isize, false),
            (None, [b' '], b'R') if vt420 => {
                self.move_page(self.page as isize - n(0) as isize, false)
            }
            // xterm window operation 18: report the text area size.
            (None, [], b't') if xterm && p.get_or(0, 0) == 18 => {
                let (rows, cols) = (self.rows(), self.cols());
                self.reply_csi(&format!("8;{rows};{cols}t"));
            }
            (None, [], b't') if vt420 && p.len() == 1 => {
                self.set_page_length(usize::from(p.get_or(0, 0)));
            }
            (None, [b'$'], b'|') if vt420 => match p.get_or(0, 0) {
                0 | 80 => self.set_page_width(80),
                132 => self.set_page_width(132),
                _ => {}
            },
            (None, [b'*'], b'|') if vt420 => self.set_screen_lines(n(0)),
            (None, [b'"'], b'v') if vt420 => {
                self.couple();
                let shown = self.screen_lines.min(self.rows());
                let body = format!(
                    "{shown};{};1;{};{}\"w",
                    self.cols(),
                    self.window_top + 1,
                    self.display_page + 1
                );
                self.reply_csi(&body);
            }
            (None, [b'$'], b'v') if vt420 => self.copy_rectangle(p),
            (None, [b'$'], b'z') if vt420 => self.erase_rectangle(p),
            (None, [b'$'], b'{') if vt420 => self.selective_erase_rectangle(p),
            (None, [b'$'], b'x') if vt420 => self.fill_rectangle(p),
            (None, [b'$'], b'r') if vt420 => self.change_rectangle_attributes(p, false),
            (None, [b'$'], b't') if vt420 => self.change_rectangle_attributes(p, true),
            (None, [b'*'], b'x') if vt420 => self.select_attribute_change_extent(p.get_or(0, 0)),
            (None, [b'*'], b'y') if vt420 => self.request_checksum(p),
            (None, [b'*'], b'z') if vt420 => self.invoke_macro(p.get_or(0, 0)),
            (None, [b'$'], b'u') if vt420 && p.get_or(0, 0) == 1 => self.terminal_state_report(),
            (None, [b'$'], b'u') if self.color_terminal() && p.get_or(0, 0) == 2 => {
                self.color_table_report(p.get_or(1, 0))
            }
            (None, [b'\''], b'}') if vt420 => self.insert_columns(n(0), true),
            (None, [b'\''], b'~') if vt420 => self.insert_columns(n(0), false),
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
            (Some(b'?'), [], b'n') if p.get_or(0, 0) == 63 && vt420 => {
                self.memory_checksum(p.get_or(1, 0))
            }
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

    fn osc_start(&mut self) {
        self.osc.clear();
    }

    fn osc_put(&mut self, byte: u8) {
        if self.osc.len() < 512 {
            self.osc.push(byte);
        }
    }

    fn osc_end(&mut self, end: StringEnd) {
        let data = std::mem::take(&mut self.osc);
        if end == StringEnd::Terminated && self.modes.ansi {
            self.operating_system_command(&data);
        }
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
