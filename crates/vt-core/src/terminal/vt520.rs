//! VT510/VT520 control functions (EK-VT520-RM chapter 5): the extra cursor
//! functions, Set-Up selections the host can make and report, the VT500
//! private modes, cursor style, terminal ID and emulation mode, secure
//! reset, and session names.

use std::collections::BTreeMap;

use vt_parser::Params;

use super::{Emulator, Event};
use crate::Model;
use crate::color::{ColorMode, ColorOptions, hls_to_rgb, rgb_to_hls};

/// Cursor appearance selected with DECSCUSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    /// The DEC factory default.
    #[default]
    BlinkingBlock,
    SteadyBlock,
    BlinkingUnderline,
    SteadyUnderline,
}

impl CursorStyle {
    pub const fn is_block(self) -> bool {
        matches!(self, CursorStyle::BlinkingBlock | CursorStyle::SteadyBlock)
    }

    pub const fn blinks(self) -> bool {
        matches!(
            self,
            CursorStyle::BlinkingBlock | CursorStyle::BlinkingUnderline
        )
    }

    const fn code(self) -> u16 {
        match self {
            CursorStyle::BlinkingBlock => 1,
            CursorStyle::SteadyBlock => 2,
            CursorStyle::BlinkingUnderline => 3,
            CursorStyle::SteadyUnderline => 4,
        }
    }
}

/// VT500 private modes that are stored and reported but have no effect on the
/// screen model, with their factory defaults (EK-VT520-RM table 5-3).
const STORED_MODES: [(u16, bool); 24] = [
    (34, false),  // DECRLM right-to-left
    (35, false),  // DECHEBM Hebrew keyboard mapping
    (36, false),  // DECHEM Hebrew encoding
    (57, false),  // DECNAKB Greek keyboard mapping
    (73, false),  // DECXRLM transmit rate limiting
    (81, false),  // DECKPM key position reports
    (95, false),  // DECNCSM no clear on column change
    (96, false),  // DECRLCM right-to-left copy
    (97, true),   // DECCRTSM CRT saver
    (98, false),  // DECARSM auto resize
    (99, false),  // DECMCM modem control
    (100, false), // DECAAM auto answerback
    (101, false), // DECCANSM conceal answerback
    (102, true),  // DECNULM discard NUL
    (103, false), // DECHDPXM half duplex
    (104, false), // DECESKM secondary keyboard language
    (106, false), // DECOSCNM overscan
    (111, true),  // DECFWM framed windows
    (112, false), // DECRPL review previous lines
    (113, false), // DECHWUM host wake-up
    (114, false), // DECATCUM alternate text colour underline
    (115, false), // DECATCBM alternate text colour blink
    (116, false), // DECBBSM bold and blink style
    (117, false), // DECECM erase colour
];

/// Modes that can be set but that DECRQM does not report (not in table 5-3).
const UNREPORTED_MODES: [(u16, bool); 3] = [
    (108, false), // DECNUMLK
    (109, false), // DECCAPSLK
    (110, false), // DECKLHIM
];

pub(super) const DECKPM: u16 = 81;
pub(super) const DECNCSM: u16 = 95;
pub(super) const DECRLM: u16 = 34;
pub(super) const DECATCUM: u16 = 114;
pub(super) const DECATCBM: u16 = 115;
pub(super) const DECBBSM: u16 = 116;
pub(super) const DECECM: u16 = 117;

/// Host-selectable Set-Up features of a VT520. Each keeps the parameters of
/// the last valid selection, which DECRQSS reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SetUp {
    pub cursor_style: CursorStyle,
    pub modes: BTreeMap<u16, bool>,
    /// DECTID: the primary DA identity (0 VT100 … 10 VT520).
    pub terminal_id: u16,
    /// Simple selections keyed by their DECRQSS final characters.
    selections: BTreeMap<&'static [u8], String>,
    /// DECSCS speed for each communication line (index = Ps1 - 1).
    comm_speed: [u16; 5],
    /// DECSFC and DECSPP for the communication and printer ports.
    flow_control: [String; 2],
    port_parameters: [String; 2],
    /// DECSTRL rate for all, graphic and function keys.
    transmit_rate: [u16; 3],
    pub banner: Vec<u8>,
    pub time_of_day: (u16, u16),
    /// DECKBD keyboard layout (1 VT, 2 enhanced PC) and language code, once the
    /// host has selected one.
    pub keyboard: Option<(u16, u16)>,
    /// DECELF: copy and paste, panning, window resizing keys enabled.
    pub local_functions: [bool; 3],
    /// DECLFKC: what F1–F4 do.
    pub local_function_keys: [LocalKeyAction; 4],
    /// DECSMKR: modifier key functions (1 default, 2 report, 3 disabled).
    pub modifier_keys: [u8; 8],
    /// DECUS: 1 only when active, 2 when available, 3 at regular intervals.
    update_session: u16,
}

/// DECLFKC: what a local function key (F1 Hold, F2 Print, F3 Set-Up,
/// F4 Session) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalKeyAction {
    /// Performs its local function (factory setting).
    #[default]
    Local,
    /// Sends its function key sequence to the host.
    SendToHost,
    /// Does nothing.
    Disabled,
}

/// Factory values of the simple selections (EK-VT520-RM chapter 5).
const SELECTIONS: [(&[u8], &str); 17] = [
    (b" r", "5"),   // DECSKCV key click: high
    (b" t", "5"),   // DECSWBV warning bell: high
    (b" u", "1"),   // DECSMBV margin bell: off
    (b" p", "1"),   // DECSSCLS scroll speed: smooth 2
    (b" v", "1"),   // DECSLCK lock key: caps lock
    (b"-p", "30"),  // DECARR auto repeat: fast
    (b"-q", "15"),  // DECCRTST CRT saver: 15 minutes
    (b"-r", "15"),  // DECSEST energy saver: 15 minutes
    (b",{", "1"),   // DECSZS zero: oval
    (b"$s", "1"),   // DECSPRTT printer type: DEC ANSI
    (b"*p", "437"), // DECSPPCS ProPrinter set: PC International
    (b"(p", "1"),   // DECSDPT printed data: national only
    (b"$q", "3"),   // DECSDDT disconnect delay: 2 seconds
    (b"p", "1"),    // DECSSL Set-Up language: English
    (b"*u", "0;1"), // DECSCP no printer port, session on Comm1
    (b",z", "2"),   // DECDLDA two soft sets per session
    (b"\"t", "3"),  // DECSRFR refresh rate: 70 Hz or more (VT510 only)
];

impl Default for SetUp {
    fn default() -> Self {
        SetUp {
            cursor_style: CursorStyle::default(),
            modes: STORED_MODES
                .iter()
                .chain(UNREPORTED_MODES.iter())
                .copied()
                .collect(),
            terminal_id: 10,
            selections: SELECTIONS
                .iter()
                .map(|(k, v)| (*k, (*v).to_string()))
                .collect(),
            comm_speed: [6, 6, 5, 0, 0],
            flow_control: ["1;3;1;1".into(), "2;3;1;1".into()],
            port_parameters: ["1;1;1;1".into(), "2;1;1;1".into()],
            transmit_rate: [1, 1, 1],
            banner: Vec::new(),
            time_of_day: (8, 0),
            keyboard: None,
            local_functions: [true; 3],
            local_function_keys: [LocalKeyAction::Local; 4],
            modifier_keys: [1; 8],
            update_session: 2,
        }
    }
}

/// Accepts `value` when it lies in `range`, treating 0 or an omitted value as `default`.
fn select(p: &Params, i: usize, range: std::ops::RangeInclusive<u16>, default: u16) -> Option<u16> {
    match p.get_or(i, 0) {
        0 => Some(default),
        v if range.contains(&v) => Some(v),
        _ => None,
    }
}

impl SetUp {
    /// The parameters of a simple selection, by its DECRQSS final characters.
    pub(super) fn selection(&self, key: &[u8]) -> &str {
        self.selections.get(key).map_or("", String::as_str)
    }

    pub(super) fn set_selection(&mut self, key: &[u8], value: &str) {
        if let Some(v) = self.selections.get_mut(key) {
            *v = value.to_string();
        }
    }

    pub(super) fn update_session(&self) -> u16 {
        self.update_session
    }

    pub(super) fn set_update_session(&mut self, value: u16) {
        self.update_session = value;
    }

    pub(super) fn comm_speed(&self, line: usize) -> u16 {
        self.comm_speed[line]
    }

    pub(super) fn set_comm_speed(&mut self, line: usize, speed: u16) {
        self.comm_speed[line] = speed;
    }

    /// DECSPP for the communication port: data bits (1 eight, 2 seven),
    /// parity (1–7) and stop bits (1 or 2).
    pub(super) fn port_parameters(&self) -> [u16; 3] {
        let v: Vec<u16> = self.port_parameters[0]
            .split(';')
            .skip(1)
            .map(|n| n.parse().unwrap_or(1))
            .collect();
        [v[0], v[1], v[2]]
    }

    pub(super) fn set_port_parameters(&mut self, [bits, parity, stop]: [u16; 3]) {
        self.port_parameters[0] = format!("1;{bits};{parity};{stop}");
    }

    /// DECSFC for the communication port: direction (1 transmit, 2 receive,
    /// 3 both), type (1 XON/XOFF, 2 DTR, 3 both, 4 none) and threshold.
    pub(super) fn flow_control(&self) -> [u16; 3] {
        let v: Vec<u16> = self.flow_control[0]
            .split(';')
            .skip(1)
            .map(|n| n.parse().unwrap_or(1))
            .collect();
        [v[0], v[1], v[2]]
    }

    pub(super) fn set_flow_control(&mut self, [dir, kind, threshold]: [u16; 3]) {
        self.flow_control[0] = format!("1;{dir};{kind};{threshold}");
    }

    /// DECSTRL rate (1 150, 2 50, 3 30 cps) for all keys (0), graphic keys
    /// (1) or function keys (2).
    pub(super) fn transmit_rate(&self, keys: usize) -> u16 {
        self.transmit_rate[keys]
    }

    pub(super) fn set_transmit_rate(&mut self, keys: usize, rate: u16) {
        self.transmit_rate[keys] = rate;
    }
}

impl Emulator {
    pub(super) fn vt520_mode(&self, mode: u16) -> Option<bool> {
        STORED_MODES
            .iter()
            .any(|(m, _)| *m == mode)
            .then(|| self.setup.modes.get(&mode).copied().unwrap_or(false))
    }

    /// DECSET/DECRST for a VT500 mode; false if `mode` is not one.
    pub(super) fn set_vt520_mode(&mut self, mode: u16, on: bool) -> bool {
        match self.setup.modes.get_mut(&mode) {
            Some(value) => {
                *value = on;
                true
            }
            None => false,
        }
    }

    pub(super) fn vt520_flag(&self, mode: u16) -> bool {
        self.level >= 5 && self.setup.modes.get(&mode).copied().unwrap_or(false)
    }

    /// VT500 CSI sequences. Returns false for anything else.
    pub(super) fn vt520_csi(&mut self, intermediates: &[u8], final_byte: u8, p: &Params) -> bool {
        let n = |i: usize| usize::from(p.get_nonzero_or(i, 1));
        match (intermediates, final_byte) {
            // CNL, CPL: down or up, then to the start of the line.
            ([], b'E') => {
                self.cud(n(0));
                self.carriage_return();
            }
            ([], b'F') => {
                self.cuu(n(0));
                self.carriage_return();
            }
            // CHA and HPA are the same function (ECMA-48): in origin mode they
            // are relative to the left margin; otherwise margins do not apply
            // (vttest, from a VT510).
            ([], b'G' | b'`') => {
                let (row, col) = (self.cursor.row, n(0) - 1);
                if self.modes.origin && !self.status.active {
                    self.goto(row, (self.left + col).min(self.right));
                } else {
                    self.goto(row, col);
                }
            }
            // VPA likewise honours origin mode.
            ([], b'd') => {
                let (row, col) = (n(0) - 1, self.cursor.col);
                if self.modes.origin && !self.status.active {
                    self.goto((self.top + row).min(self.bottom), col);
                } else {
                    self.goto(row, col);
                }
            }
            // HPR and VPR move relative to the cursor, stopping at the page edge.
            ([], b'a') => self.goto(self.cursor.row, self.cursor.col + n(0)),
            ([], b'e') => self.goto(self.cursor.row + n(0), self.cursor.col),
            ([], b'I') => self.tab(n(0)),
            // CBT stops at the left margin only in origin mode (vttest, from a VT510).
            ([], b'Z') => {
                let stop = if self.modes.origin && self.cursor.col >= self.left {
                    self.left
                } else {
                    0
                };
                self.back_tab(n(0));
                if self.cursor.col < stop {
                    self.goto(self.cursor.row, stop);
                }
            }
            ([b' '], b'q') => {
                self.setup.cursor_style = match p.get_or(0, 0) {
                    0 | 1 => CursorStyle::BlinkingBlock,
                    2 => CursorStyle::SteadyBlock,
                    3 => CursorStyle::BlinkingUnderline,
                    4 => CursorStyle::SteadyUnderline,
                    _ => return true,
                }
            }
            ([b' '], b'r') => self.store(b" r", select(p, 0, 1..=8, 5)),
            ([b' '], b't') => self.store(b" t", select(p, 0, 1..=8, 5)),
            ([b' '], b'u') => self.store(b" u", select(p, 0, 1..=8, 1)),
            ([b' '], b'p') => self.store(b" p", select(p, 0, 1..=9, 1)),
            ([b' '], b'v') => self.store(b" v", select(p, 0, 1..=3, 1)),
            // DECARR: 0-5 off, 6-15 slow, 16-30 fast; other values are ignored.
            ([b'-'], b'p') => self.store(b"-p", Some(p.get_or(0, 0)).filter(|v| *v <= 30)),
            ([b'-'], b'q') => {
                let v = p.get_or(0, 0);
                if matches!(v, 0 | 5 | 15 | 30 | 60) {
                    self.store(b"-q", Some(v));
                }
            }
            ([b'-'], b'r') => {
                let v = p.get_or(0, 0);
                if matches!(v, 0 | 5 | 15 | 30) {
                    self.store(b"-r", Some(v));
                }
            }
            ([b','], b'{') => self.store(b",{", select(p, 0, 1..=3, 1)),
            ([b'$'], b's') => self.store(b"$s", select(p, 0, 1..=3, 1)),
            ([b'*'], b'p') => {
                let v = p.get_or(0, 0);
                if matches!(
                    v,
                    210 | 220 | 437 | 850 | 852 | 857 | 860 | 862 | 863 | 865 | 866
                ) {
                    self.store(b"*p", Some(v));
                }
            }
            ([b'"'], b't') if self.config.model == Model::Vt510 => {
                self.store(b"\"t", select(p, 0, 1..=3, 3))
            }
            ([b'('], b'p') => self.store(b"(p", select(p, 0, 1..=4, 1)),
            ([b'$'], b'q') => self.store(b"$q", select(p, 0, 1..=3, 3)),
            ([], b'p') if p.len() <= 1 => self.store(b"p", select(p, 0, 1..=5, 1)),
            ([b'*'], b'u') => {
                if let (Some(printer), Some(host)) = (
                    Some(p.get_or(0, 0)).filter(|v| matches!(v, 0 | 1 | 4)),
                    select(p, 1, 1..=3, 1),
                ) {
                    self.setup
                        .selections
                        .insert(b"*u", format!("{printer};{host}"));
                }
            }
            ([b','], b'z') if !self.config.model.has_color() => {
                self.store(b",z", select(p, 0, 1..=2, 2))
            }
            ([b'*'], b'r') => {
                if let (Some(line), Some(speed)) = (select(p, 0, 1..=5, 1), select(p, 1, 1..=11, 6))
                {
                    self.setup.comm_speed[usize::from(line) - 1] = speed;
                }
            }
            ([b'*'], b's') => {
                if let (Some(port), Some(dir), Some(kind), Some(threshold)) = (
                    select(p, 0, 1..=2, 1),
                    select(p, 1, 1..=3, 1),
                    select(p, 2, 1..=4, 1),
                    select(p, 3, 1..=2, 1),
                ) {
                    self.setup.flow_control[usize::from(port) - 1] =
                        format!("{port};{dir};{kind};{threshold}");
                }
            }
            ([b'+'], b'w') => {
                if let (Some(port), Some(bits), Some(parity), Some(stop)) = (
                    select(p, 0, 1..=2, 1),
                    select(p, 1, 1..=2, 1),
                    select(p, 2, 1..=7, 1),
                    select(p, 3, 1..=2, 1),
                ) {
                    self.setup.port_parameters[usize::from(port) - 1] =
                        format!("{port};{bits};{parity};{stop}");
                }
            }
            ([b'"'], b'u') => {
                if let (Some(keys), Some(rate)) = (select(p, 0, 1..=3, 1), select(p, 1, 1..=3, 1)) {
                    self.setup.transmit_rate[usize::from(keys) - 1] = rate;
                }
            }
            ([b','], b'q') => {
                let id = p.get_or(0, 0);
                if matches!(id, 0 | 1 | 2 | 5 | 7 | 9 | 10) {
                    self.setup.terminal_id = id;
                }
            }
            // DECAC, DECATC, DECSTGLT (VT525).
            ([b','], b'|') if self.color_terminal() => {
                if let (Some(item @ 1..=2), Some(fg @ 0..=15), Some(bg @ 0..=15)) =
                    (p.get(0), p.get(1), p.get(2))
                {
                    let pair = (fg as u8, bg as u8);
                    if item == 1 {
                        self.colors.normal = pair;
                    } else {
                        self.colors.frame = pair;
                    }
                }
            }
            ([b','], b'}') if self.color_terminal() => {
                if let (Some(ps1 @ 0..=15), Some(fg @ 0..=15), Some(bg @ 0..=15)) =
                    (p.get(0), p.get(1), p.get(2))
                {
                    self.colors.alternate[usize::from(ps1)] = (fg as u8, bg as u8);
                }
            }
            ([b')'], b'{') if self.color_terminal() => {
                if let Some(mode) = ColorMode::from_code(p.get_or(0, 0)) {
                    self.colors.mode = mode;
                }
            }
            // DECKBD: keyboard layout and language.
            ([b' '], b'}') => {
                let layout = p.get_or(0, 0).max(1);
                let language = p.get_or(1, 0).max(1);
                if layout <= 2 && KEYBOARD_LANGUAGES.contains(&language) {
                    self.setup.keyboard = Some((layout, language));
                }
            }
            // DECELF: pairs of local function group and enable (1) / disable (2).
            ([b'+'], b'q') => {
                for pair in pairs(p) {
                    let enable = match pair.1 {
                        0 | 1 => true,
                        2 => false,
                        _ => continue,
                    };
                    match pair.0 {
                        0 => self.setup.local_functions = [enable; 3],
                        g @ 1..=3 => self.setup.local_functions[usize::from(g) - 1] = enable,
                        _ => {}
                    }
                }
            }
            // DECLFKC: pairs of key (0 all, 1–4 F1–F4) and action.
            ([b'*'], b'}') => {
                for (key, action) in pairs(p) {
                    let action = match action {
                        0 | 1 => LocalKeyAction::Local,
                        2 => LocalKeyAction::SendToHost,
                        3 => LocalKeyAction::Disabled,
                        _ => continue,
                    };
                    match key {
                        0 => self.setup.local_function_keys = [action; 4],
                        k @ 1..=4 => self.setup.local_function_keys[usize::from(k) - 1] = action,
                        _ => {}
                    }
                }
            }
            // DECSMKR: pairs of modifier key (0 all, 1–8) and function.
            ([b'+'], b'r') => {
                for (key, function) in pairs(p) {
                    let function = match function {
                        0 => 1,
                        f @ 1..=3 => f as u8,
                        _ => continue,
                    };
                    match key {
                        0 => self.setup.modifier_keys = [function; 8],
                        k @ 1..=8 => self.setup.modifier_keys[usize::from(k) - 1] = function,
                        _ => {}
                    }
                }
            }
            // DECES: make this session active for keyboard input.
            ([b'&'], b'x') => self.events.push(Event::SessionActivated),
            ([b','], b'y') => {
                if let v @ 1..=3 = p.get_or(0, 0) {
                    self.setup.update_session = v;
                }
            }
            // DECPS: play one note.
            ([b','], b'~') => {
                let (volume, duration, note) = (p.get_or(0, 0), p.get_or(1, 0), p.get_or(2, 0));
                if volume <= 7 && note <= 25 {
                    self.events.push(Event::PlaySound {
                        volume: volume as u8,
                        duration_ms: u32::from(duration) * 1000 / 32,
                        note: note as u8,
                    });
                }
            }
            ([b','], b'p') => {
                let (hour, minute) = (p.get_or(0, 8), p.get_or(1, 0));
                if hour <= 23 && minute <= 59 {
                    self.setup.time_of_day = (hour, minute);
                }
            }
            _ => return false,
        }
        true
    }

    /// DECTME: VT500, VT100 or VT52 operation, with a soft reset. The other
    /// emulations (Wyse, TVI, ADDS, SCO) are not provided.
    pub(super) fn terminal_mode_emulation(&mut self, mode: u16) {
        match mode {
            0 | 1 => {
                self.level = self.config.model.max_level();
                self.soft_reset();
            }
            2 => {
                self.level = 1;
                self.c1_8bit = false;
                self.soft_reset();
            }
            3 => {
                self.soft_reset();
                self.enter_vt52();
            }
            _ => {}
        }
    }

    fn store(&mut self, key: &'static [u8], value: Option<u16>) {
        if let Some(v) = value {
            self.setup.selections.insert(key, v.to_string());
        }
    }

    /// DECST8C: tab stops at every eighth column from column 9.
    pub(super) fn tab_every_8(&mut self) {
        for (c, tab) in self.tabs.iter_mut().enumerate() {
            *tab = c > 0 && c % 8 == 0;
        }
    }

    /// The DECRPSS data for a VT500 selection, without the leading validity digit.
    pub(super) fn vt520_setting(&self, data: &[u8]) -> Option<String> {
        let s = &self.setup;
        if self.color_terminal() {
            if let Some(report) = self.color_setting(data) {
                return Some(report);
            }
        }
        // DECDLDA exists only on monochrome terminals.
        if (data == b"\"t" && self.config.model != Model::Vt510)
            || (data == b",z" && self.config.model.has_color())
        {
            return None;
        }
        if let Some(value) = s.selections.get(data) {
            let finals = std::str::from_utf8(data).ok()?;
            return Some(format!("{value}{finals}"));
        }
        Some(match data {
            b" q" => format!("{} q", s.cursor_style.code()),
            b"*r" => format!("1;{}*r", s.comm_speed[0]),
            b"*s" => format!("{}*s", s.flow_control[0]),
            b"+w" => format!("{}+w", s.port_parameters[0]),
            b"\"u" => format!("1;{}\"u", s.transmit_rate[0]),
            b",y" => format!("{},y", s.update_session),
            // 🔎 Page memory is not divided between sessions yet: all pages
            // belong to session 1.
            b",x" => format!("{};0;0;0,x", self.page_count()),
            b" ~" => format!("{} ~", if self.level == 1 { 2 } else { 1 }),
            _ => return None,
        })
    }

    pub(super) fn color_terminal(&self) -> bool {
        self.config.model.has_color() && self.level >= 5
    }

    pub(super) fn color_options(&self) -> ColorOptions {
        ColorOptions {
            bold_blink_background: self.vt520_flag(DECBBSM),
            alternate_underline: self.vt520_flag(DECATCUM),
            alternate_blink: self.vt520_flag(DECATCBM),
        }
    }

    /// DECRQSS for DECAC (`Ps1,|`), DECATC (`Ps1,}`) and DECSTGLT (`){`).
    fn color_setting(&self, data: &[u8]) -> Option<String> {
        let text = std::str::from_utf8(data).ok()?;
        let number = |prefix: &str, default: u16| -> Option<u16> {
            if prefix.is_empty() {
                Some(default)
            } else {
                prefix.parse().ok()
            }
        };
        if text == "){" {
            return Some(format!("{}){{", self.colors.mode.code()));
        }
        if let Some(prefix) = text.strip_suffix(",|") {
            let (fg, bg) = match number(prefix, 1)? {
                1 => self.colors.normal,
                2 => self.colors.frame,
                _ => return None,
            };
            let item = number(prefix, 1)?;
            return Some(format!("{item};{fg};{bg},|"));
        }
        if let Some(prefix) = text.strip_suffix(",}") {
            let ps1 = number(prefix, 0)?;
            let (fg, bg) = *self.colors.alternate.get(usize::from(ps1))?;
            return Some(format!("{ps1};{fg};{bg},}}"));
        }
        None
    }

    /// DECCTR: the colour map as DECTSR 2, in HLS (1) or RGB (2).
    pub(super) fn color_table_report(&mut self, space: u16) {
        if !matches!(space, 1 | 2) {
            return;
        }
        let entries: Vec<String> = self
            .colors
            .map
            .iter()
            .enumerate()
            .map(|(i, rgb)| {
                if space == 1 {
                    let (h, l, s) = rgb_to_hls(*rgb);
                    format!("{i};1;{h};{l};{s}")
                } else {
                    format!("{i};2;{};{};{}", rgb[0], rgb[1], rgb[2])
                }
            })
            .collect();
        self.reply_dcs(&format!("2$s{}", entries.join("/")));
    }

    /// DECRSTS 2: load colour map entries; only those given change.
    pub(super) fn restore_color_table(&mut self, data: &[u8]) {
        let Ok(text) = std::str::from_utf8(data) else {
            return;
        };
        for group in text.split('/') {
            let v: Vec<u16> = group
                .split(';')
                .filter_map(|f| f.trim().parse().ok())
                .collect();
            let [pc, pu, x, y, z] = v[..] else { continue };
            let Some(entry) = self.colors.map.get_mut(usize::from(pc)) else {
                continue;
            };
            match pu {
                1 if x <= 360 && y <= 100 && z <= 100 => *entry = hls_to_rgb(x, y as u8, z as u8),
                2 if x <= 100 && y <= 100 && z <= 100 => *entry = [x as u8, y as u8, z as u8],
                _ => {}
            }
        }
    }

    /// The primary DA response selected with DECTID (EK-VT520-RM DECTID) or,
    /// on a VT420, the General Set-Up terminal ID.
    pub(super) fn terminal_id_attributes(&self) -> Option<&'static str> {
        if self.config.model.max_level() < 4 {
            return None;
        }
        Some(match self.setup.terminal_id {
            0 => "1;2",
            1 => "1;0",
            2 => "6",
            5 => "62;1;2;7;8;9",
            7 => "63;1;2;7;8;9",
            9 => "64;1;2;7;8;9;15;18;21",
            _ => return None,
        })
    }

    /// DECSR: a reset to the power-up state without disconnecting, confirmed
    /// with DECSRC when a number is given.
    pub(super) fn secure_reset(&mut self, p: &Params) {
        self.secure_reset_confirming(p.get(0));
    }

    pub(super) fn secure_reset_confirming(&mut self, pr: Option<u16>) {
        let confirm = pr.filter(|v| *v <= 16383);
        self.full_reset();
        if let Some(pr) = confirm {
            self.reply_csi(&format!("{pr}*q"));
        }
    }

    /// DECLANS: the answerback message as hex pairs.
    pub(super) fn load_answerback(&mut self, encoding: u16, data: &[u8]) {
        if encoding != 1 {
            return;
        }
        if let Some(bytes) = hex_pairs(data).filter(|b| b.len() <= 30) {
            self.config.answerback = bytes;
        }
    }

    /// DECLBAN: the power-up banner, as hex pairs (1) or text (0, 2).
    pub(super) fn load_banner(&mut self, encoding: u16, data: &[u8]) {
        let banner = match encoding {
            1 => hex_pairs(data),
            0 | 2 => Some(data.to_vec()),
            _ => None,
        };
        if let Some(b) = banner {
            self.setup.banner = b.into_iter().take(30).collect();
        }
    }

    /// OSC strings: DECSWT (`21;name`) and DECSIN (`2L;name`); with xterm
    /// compatibility also OSC 0 and 2 titles.
    pub(super) fn operating_system_command(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        let vt500 = self.config.model.max_level() >= 5;
        let xterm = self.config.extensions.xterm_compat;
        if let Some(name) = text.strip_prefix("21;").filter(|_| vt500) {
            let title: String = name.chars().take(30).collect();
            self.events.push(Event::TitleChanged(title));
        } else if let Some(name) = text.strip_prefix("2L;").filter(|_| vt500) {
            let icon: String = name.chars().take(12).collect();
            self.events.push(Event::IconNameChanged(icon));
        } else if let Some(title) = text
            .strip_prefix("2;")
            .or(text.strip_prefix("0;"))
            .filter(|_| xterm)
        {
            self.events.push(Event::TitleChanged(title.to_string()));
        }
    }
}

/// Keyboard language codes (EK-VT520-RM table 5-11).
pub(super) const KEYBOARD_LANGUAGES: [u16; 30] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 19, 22, 28, 29, 30, 31, 33, 34, 35, 36,
    38, 39, 40, 0,
];

/// Parameter pairs (Ps1;Ps2;Ps3;Ps4…); an odd trailing parameter is ignored.
fn pairs(p: &Params) -> impl Iterator<Item = (u16, u16)> + '_ {
    (0..p.len() / 2).map(move |i| (p.get_or(2 * i, 0), p.get_or(2 * i + 1, 0)))
}

fn hex_pairs(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() % 2 != 0 {
        return None;
    }
    data.chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}
