//! Set-Up: the terminal's local configuration screens (F3).
//!
//! The screens follow the VT420 (Installing and Using the VT420 Video
//! Terminal, chapter 5): a Set-Up Directory and the Global, Display, General,
//! Communications, Printer, Keyboard and Tab screens. A field cursor moves
//! with the arrow keys; Enter performs an action field or steps a feature to
//! its next setting. [`Features`] holds every setting; the terminal reads its
//! current features with [`crate::Terminal::setup_features`] and applies them
//! with [`crate::Terminal::apply_setup_features`] when Set-Up is left.
//!
//! [`SetupMenu::render`] draws the screen as VT420 output (a double-width
//! title, the field cursor in reverse video) for a scratch 24-line terminal,
//! so Set-Up is drawn with the same fonts as the session.

use std::fmt::Write as _;

use crate::charset::Nrc;
use crate::{CursorStyle, LocalKeyAction, Model, StatusDisplay, Supplemental};

/// Smooth or jump scrolling (Display Set-Up).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scroll {
    /// The factory setting (EK-VT520-RM table 2-10).
    #[default]
    Jump,
    Smooth2,
    Smooth4,
}

/// Keyclick and bell volumes (Keyboard Set-Up).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Volume {
    Off,
    Low,
    High,
}

/// The terminal's operating level (General Set-Up "terminal mode").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalMode {
    Vt52,
    Vt100,
    /// VT200 (2) to VT500 (5) mode, with 7-bit or 8-bit controls.
    Level {
        level: u8,
        eight_bit: bool,
    },
}

/// What F5 does (Keyboard Set-Up).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakKey {
    #[default]
    Break,
    NoBreak,
    FunctionKey,
    Ignore,
}

/// What the Compose Character key does (Keyboard Set-Up).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComposeKey {
    #[default]
    Local,
    Report,
    Ignore,
}

/// Every Set-Up feature of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Features {
    // Global
    pub on_line: bool,
    pub two_sessions: bool,
    pub crt_saver: bool,
    pub comm1_dec423: bool,
    pub refresh_60hz: bool,
    /// Printer assignment: 0 shared, 1 session 1, 2 session 2.
    pub printer_session: u8,
    // Display
    pub columns_132: bool,
    pub display_controls: bool,
    pub autowrap: bool,
    pub scroll: Scroll,
    pub light_screen: bool,
    pub cursor: bool,
    pub cursor_style: CursorStyle,
    pub status: StatusDisplay,
    pub page_length: usize,
    pub screen_lines: usize,
    pub vertical_coupling: bool,
    pub page_coupling: bool,
    pub auto_resize: bool,
    // General
    pub terminal_mode: TerminalMode,
    pub udk_locked: bool,
    pub user_features_locked: bool,
    pub national: bool,
    pub keypad_application: bool,
    pub cursor_keys_application: bool,
    pub new_line: bool,
    pub upss: Supplemental,
    /// DECTID code; 10 is the model's own identity.
    pub terminal_id: u16,
    /// Inactive session updates: 1 never, 2 when available, 3 shared.
    pub update: u16,
    // Communications
    pub transmit_speed: u32,
    /// `None` receives at the transmit speed.
    pub receive_speed: Option<u32>,
    /// `None` sends no XOFF.
    pub xoff: Option<u16>,
    /// Index into [`DATA_FORMATS`].
    pub data_format: u8,
    pub two_stop_bits: bool,
    pub local_echo: bool,
    pub modem_control: bool,
    /// 0 two seconds, 1 60 ms, 2 no disconnect.
    pub disconnect: u8,
    pub limited_transmit: bool,
    pub auto_answerback: bool,
    pub answerback: Vec<u8>,
    pub conceal_answerback: bool,
    // Keyboard
    pub data_processing_keys: bool,
    pub shift_lock: bool,
    pub auto_repeat: bool,
    pub keyclick: Volume,
    pub margin_bell: Volume,
    pub warning_bell: Volume,
    pub position_mode: bool,
    pub backarrow_bs: bool,
    pub compose: ComposeKey,
    pub report_alt: bool,
    /// F1 Hold, F2 Print, F3 Set-Up, F4 Session.
    pub local_keys: [LocalKeyAction; 4],
    pub break_key: BreakKey,
    pub comma_period_same: bool,
    pub angle_sends_tilde: bool,
    pub tilde_sends_escape: bool,
    pub keyboard_language: Option<Nrc>,
    // Tab
    pub tabs: Vec<bool>,
}

/// Communications Set-Up character formats.
pub const DATA_FORMATS: [&str; 7] = [
    "8 Bits, No Parity",
    "8 Bits, Even Parity",
    "8 Bits, Odd Parity",
    "7 Bits, Even Parity",
    "7 Bits, Odd Parity",
    "7 Bits, Mark Parity",
    "7 Bits, Space Parity",
];

const SPEEDS: [u32; 8] = [300, 600, 1200, 2400, 4800, 9600, 19200, 38400];

/// Keyboard dialects offered in the Set-Up Directory, `None` first.
const DIALECTS: [(Option<Nrc>, &str); 13] = [
    (None, "North American Keyboard"),
    (Some(Nrc::British), "British Keyboard"),
    (Some(Nrc::Dutch), "Dutch Keyboard"),
    (Some(Nrc::Finnish), "Finnish Keyboard"),
    (Some(Nrc::French), "French/Belgian Keyboard"),
    (Some(Nrc::FrenchCanadian), "Canadian (French) Keyboard"),
    (Some(Nrc::German), "German Keyboard"),
    (Some(Nrc::Italian), "Italian Keyboard"),
    (Some(Nrc::NorwegianDanish), "Norwegian/Danish Keyboard"),
    (Some(Nrc::Portuguese), "Portuguese Keyboard"),
    (Some(Nrc::Spanish), "Spanish Keyboard"),
    (Some(Nrc::Swedish), "Swedish Keyboard"),
    (Some(Nrc::Swiss), "Swiss Keyboard"),
];

impl Features {
    /// The factory settings of `model` (Installing and Using the VT420,
    /// tables 5-2 to 5-7).
    pub fn factory(model: Model) -> Features {
        let level = model.max_level();
        Features {
            on_line: true,
            two_sessions: false,
            crt_saver: true,
            comm1_dec423: false,
            refresh_60hz: false,
            printer_session: 0,
            columns_132: false,
            display_controls: false,
            autowrap: false,
            scroll: Scroll::Jump,
            light_screen: false,
            cursor: true,
            cursor_style: CursorStyle::default(),
            status: if model.has_status_line() {
                StatusDisplay::Indicator
            } else {
                StatusDisplay::None
            },
            page_length: 24,
            screen_lines: 24,
            vertical_coupling: true,
            page_coupling: true,
            auto_resize: false,
            terminal_mode: if level >= 2 {
                TerminalMode::Level {
                    level,
                    eight_bit: false,
                }
            } else {
                TerminalMode::Vt100
            },
            udk_locked: false,
            user_features_locked: false,
            national: false,
            keypad_application: false,
            cursor_keys_application: false,
            new_line: false,
            upss: Supplemental::DecSupplemental,
            terminal_id: 10,
            update: 2,
            transmit_speed: 9600,
            receive_speed: None,
            xoff: Some(64),
            data_format: 0,
            two_stop_bits: false,
            local_echo: false,
            modem_control: false,
            disconnect: 0,
            limited_transmit: false,
            auto_answerback: false,
            answerback: Vec::new(),
            conceal_answerback: false,
            data_processing_keys: false,
            shift_lock: false,
            auto_repeat: true,
            keyclick: Volume::High,
            margin_bell: Volume::Off,
            warning_bell: Volume::High,
            position_mode: false,
            backarrow_bs: false,
            compose: ComposeKey::Local,
            report_alt: true,
            local_keys: [LocalKeyAction::Local; 4],
            break_key: BreakKey::Break,
            comma_period_same: false,
            angle_sends_tilde: false,
            tilde_sends_escape: false,
            keyboard_language: None,
            tabs: default_tabs(80),
        }
    }

    /// The features as `name=value` lines, for saving.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let b = |v: bool| u8::from(v);
        let mut line = |k: &str, v: String| {
            let _ = writeln!(s, "{k}={v}");
        };
        line("on-line", b(self.on_line).to_string());
        line("crt-saver", b(self.crt_saver).to_string());
        line("comm1-dec423", b(self.comm1_dec423).to_string());
        line("refresh-60hz", b(self.refresh_60hz).to_string());
        line("printer-session", self.printer_session.to_string());
        line(
            "columns",
            if self.columns_132 { "132" } else { "80" }.into(),
        );
        line("display-controls", b(self.display_controls).to_string());
        line("autowrap", b(self.autowrap).to_string());
        line(
            "scroll",
            match self.scroll {
                Scroll::Jump => "jump",
                Scroll::Smooth2 => "smooth-2",
                Scroll::Smooth4 => "smooth-4",
            }
            .into(),
        );
        line("light-screen", b(self.light_screen).to_string());
        line("cursor", b(self.cursor).to_string());
        line(
            "cursor-style",
            match self.cursor_style {
                CursorStyle::BlinkingBlock => "blinking-block",
                CursorStyle::SteadyBlock => "steady-block",
                CursorStyle::BlinkingUnderline => "blinking-underline",
                CursorStyle::SteadyUnderline => "steady-underline",
            }
            .into(),
        );
        line(
            "status",
            match self.status {
                StatusDisplay::None => "none",
                StatusDisplay::Indicator => "indicator",
                StatusDisplay::HostWritable => "host",
            }
            .into(),
        );
        line("page-length", self.page_length.to_string());
        line("screen-lines", self.screen_lines.to_string());
        line("vertical-coupling", b(self.vertical_coupling).to_string());
        line("page-coupling", b(self.page_coupling).to_string());
        line("auto-resize", b(self.auto_resize).to_string());
        line(
            "terminal-mode",
            match self.terminal_mode {
                TerminalMode::Vt52 => "vt52".into(),
                TerminalMode::Vt100 => "vt100".into(),
                TerminalMode::Level { level, eight_bit } => {
                    format!("level{level}-{}bit", if eight_bit { 8 } else { 7 })
                }
            },
        );
        line("udk-locked", b(self.udk_locked).to_string());
        line(
            "user-features-locked",
            b(self.user_features_locked).to_string(),
        );
        line("national", b(self.national).to_string());
        line("new-line", b(self.new_line).to_string());
        line(
            "upss",
            match self.upss {
                Supplemental::DecSupplemental => "dec",
                Supplemental::IsoLatin1 => "latin1",
            }
            .into(),
        );
        line("terminal-id", self.terminal_id.to_string());
        line("update", self.update.to_string());
        line("transmit-speed", self.transmit_speed.to_string());
        line(
            "receive-speed",
            self.receive_speed
                .map_or("transmit".into(), |v| v.to_string()),
        );
        line("xoff", self.xoff.map_or("none".into(), |v| v.to_string()));
        line("data-format", self.data_format.to_string());
        line(
            "stop-bits",
            if self.two_stop_bits { "2" } else { "1" }.into(),
        );
        line("local-echo", b(self.local_echo).to_string());
        line("modem-control", b(self.modem_control).to_string());
        line("disconnect", self.disconnect.to_string());
        line("limited-transmit", b(self.limited_transmit).to_string());
        line("auto-answerback", b(self.auto_answerback).to_string());
        line("answerback", hex(&self.answerback));
        line("conceal-answerback", b(self.conceal_answerback).to_string());
        line(
            "data-processing-keys",
            b(self.data_processing_keys).to_string(),
        );
        line("shift-lock", b(self.shift_lock).to_string());
        line("auto-repeat", b(self.auto_repeat).to_string());
        line("keyclick", volume_name(self.keyclick).into());
        line("margin-bell", volume_name(self.margin_bell).into());
        line("warning-bell", volume_name(self.warning_bell).into());
        line("position-mode", b(self.position_mode).to_string());
        line("backarrow-bs", b(self.backarrow_bs).to_string());
        line(
            "compose",
            match self.compose {
                ComposeKey::Local => "local",
                ComposeKey::Report => "report",
                ComposeKey::Ignore => "ignore",
            }
            .into(),
        );
        line("report-alt", b(self.report_alt).to_string());
        line(
            "local-keys",
            self.local_keys
                .iter()
                .map(|k| match k {
                    LocalKeyAction::Local => 'l',
                    LocalKeyAction::SendToHost => 'h',
                    LocalKeyAction::Disabled => 'x',
                })
                .collect(),
        );
        line(
            "break-key",
            match self.break_key {
                BreakKey::Break => "break",
                BreakKey::NoBreak => "no-break",
                BreakKey::FunctionKey => "fkey",
                BreakKey::Ignore => "ignore",
            }
            .into(),
        );
        line("comma-period-same", b(self.comma_period_same).to_string());
        line("angle-sends-tilde", b(self.angle_sends_tilde).to_string());
        line("tilde-sends-escape", b(self.tilde_sends_escape).to_string());
        line(
            "keyboard",
            DIALECTS
                .iter()
                .position(|(n, _)| *n == self.keyboard_language)
                .unwrap_or(0)
                .to_string(),
        );
        line(
            "tabs",
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| **t)
                .map(|(i, _)| (i + 1).to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
        s
    }

    /// Reads [`Features::to_text`] output over the factory settings of
    /// `model`. Unknown names and bad values are ignored.
    pub fn from_text(model: Model, text: &str) -> Features {
        let mut f = Features::factory(model);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let flag = v == "1";
            let num = || v.parse::<usize>().ok();
            match k.trim() {
                "on-line" => f.on_line = flag,
                "crt-saver" => f.crt_saver = flag,
                "comm1-dec423" => f.comm1_dec423 = flag,
                "refresh-60hz" => f.refresh_60hz = flag,
                "printer-session" => f.printer_session = num().unwrap_or(0).min(2) as u8,
                "columns" => f.columns_132 = v == "132",
                "display-controls" => f.display_controls = flag,
                "autowrap" => f.autowrap = flag,
                "scroll" => {
                    f.scroll = match v {
                        "smooth-2" => Scroll::Smooth2,
                        "smooth-4" => Scroll::Smooth4,
                        _ => Scroll::Jump,
                    }
                }
                "light-screen" => f.light_screen = flag,
                "cursor" => f.cursor = flag,
                "cursor-style" => {
                    f.cursor_style = match v {
                        "steady-block" => CursorStyle::SteadyBlock,
                        "blinking-underline" => CursorStyle::BlinkingUnderline,
                        "steady-underline" => CursorStyle::SteadyUnderline,
                        _ => CursorStyle::BlinkingBlock,
                    }
                }
                "status" if model.has_status_line() => {
                    f.status = match v {
                        "none" => StatusDisplay::None,
                        "host" => StatusDisplay::HostWritable,
                        _ => StatusDisplay::Indicator,
                    }
                }
                "page-length" => {
                    if let Some(n) = num().filter(|n| page_lengths(model).contains(n)) {
                        f.page_length = n;
                    }
                }
                "screen-lines" => {
                    if let Some(n) = num().filter(|n| screen_line_choices(model).contains(n)) {
                        f.screen_lines = n;
                    }
                }
                "vertical-coupling" => f.vertical_coupling = flag,
                "page-coupling" => f.page_coupling = flag,
                "auto-resize" => f.auto_resize = flag,
                "terminal-mode" => {
                    if let Some(m) = terminal_modes(model).into_iter().find(|m| {
                        let name = match m {
                            TerminalMode::Vt52 => "vt52".to_string(),
                            TerminalMode::Vt100 => "vt100".to_string(),
                            TerminalMode::Level { level, eight_bit } => {
                                format!("level{level}-{}bit", if *eight_bit { 8 } else { 7 })
                            }
                        };
                        name == v
                    }) {
                        f.terminal_mode = m;
                    }
                }
                "udk-locked" => f.udk_locked = flag,
                "user-features-locked" => f.user_features_locked = flag,
                "national" => f.national = flag,
                "new-line" => f.new_line = flag,
                "upss" => {
                    f.upss = if v == "latin1" {
                        Supplemental::IsoLatin1
                    } else {
                        Supplemental::DecSupplemental
                    }
                }
                "terminal-id" => {
                    if let Some(id) = num().map(|n| n as u16) {
                        if terminal_ids(model).contains(&id) {
                            f.terminal_id = id;
                        }
                    }
                }
                "update" => f.update = num().unwrap_or(2).clamp(1, 3) as u16,
                "transmit-speed" => {
                    if let Some(s) = num().map(|n| n as u32).filter(|s| SPEEDS.contains(s)) {
                        f.transmit_speed = s;
                    }
                }
                "receive-speed" => {
                    f.receive_speed = v.parse::<u32>().ok().filter(|s| SPEEDS.contains(s))
                }
                "xoff" => f.xoff = v.parse::<u16>().ok().filter(|x| [64, 128].contains(x)),
                "data-format" => {
                    f.data_format = num().unwrap_or(0).min(DATA_FORMATS.len() - 1) as u8
                }
                "stop-bits" => f.two_stop_bits = v == "2",
                "local-echo" => f.local_echo = flag,
                "modem-control" => f.modem_control = flag,
                "disconnect" => f.disconnect = num().unwrap_or(0).min(2) as u8,
                "limited-transmit" => f.limited_transmit = flag,
                "auto-answerback" => f.auto_answerback = flag,
                "answerback" => f.answerback = unhex(v).unwrap_or_default(),
                "conceal-answerback" => f.conceal_answerback = flag,
                "data-processing-keys" => f.data_processing_keys = flag,
                "shift-lock" => f.shift_lock = flag,
                "auto-repeat" => f.auto_repeat = flag,
                "keyclick" => f.keyclick = volume_from(v),
                "margin-bell" => f.margin_bell = volume_from(v),
                "warning-bell" => f.warning_bell = volume_from(v),
                "position-mode" => f.position_mode = flag,
                "backarrow-bs" => f.backarrow_bs = flag,
                "compose" => {
                    f.compose = match v {
                        "report" => ComposeKey::Report,
                        "ignore" => ComposeKey::Ignore,
                        _ => ComposeKey::Local,
                    }
                }
                "report-alt" => f.report_alt = flag,
                "local-keys" => {
                    for (slot, c) in f.local_keys.iter_mut().zip(v.chars()) {
                        *slot = match c {
                            'h' => LocalKeyAction::SendToHost,
                            'x' => LocalKeyAction::Disabled,
                            _ => LocalKeyAction::Local,
                        };
                    }
                }
                "break-key" => {
                    f.break_key = match v {
                        "no-break" => BreakKey::NoBreak,
                        "fkey" => BreakKey::FunctionKey,
                        "ignore" => BreakKey::Ignore,
                        _ => BreakKey::Break,
                    }
                }
                "comma-period-same" => f.comma_period_same = flag,
                "angle-sends-tilde" => f.angle_sends_tilde = flag,
                "tilde-sends-escape" => f.tilde_sends_escape = flag,
                "keyboard" => {
                    f.keyboard_language = DIALECTS.get(num().unwrap_or(0)).and_then(|d| d.0)
                }
                "tabs" => {
                    let cols = if f.columns_132 { 132 } else { 80 };
                    let mut tabs = vec![false; cols];
                    for n in v.split(',').filter_map(|n| n.parse::<usize>().ok()) {
                        if (2..=cols).contains(&n) {
                            tabs[n - 1] = true;
                        }
                    }
                    f.tabs = tabs;
                }
                _ => {}
            }
        }
        if f.keyboard_language.is_none() {
            // National mode needs a national keyboard.
            f.national = false;
        }
        f
    }
}

pub(crate) fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c > 0 && c % 8 == 0).collect()
}

fn volume_name(v: Volume) -> &'static str {
    match v {
        Volume::Off => "off",
        Volume::Low => "low",
        Volume::High => "high",
    }
}

fn volume_from(v: &str) -> Volume {
    match v {
        "off" => Volume::Off,
        "low" => Volume::Low,
        _ => Volume::High,
    }
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn model_name(model: Model) -> String {
    model.term_name_exact().to_uppercase()
}

pub(crate) fn page_lengths(model: Model) -> &'static [usize] {
    if model.max_level() >= 5 {
        &[24, 25, 36, 41, 42, 48, 52, 53, 72]
    } else {
        &[24, 25, 36, 48, 72, 144]
    }
}

/// Lines per screen as Set-Up names them; a VT500 shows 26, 42 or 53 data
/// lines for these (EK-VT520-RM DECSNLS).
fn screen_line_choices(_model: Model) -> &'static [usize] {
    &[24, 36, 48]
}

fn terminal_modes(model: Model) -> Vec<TerminalMode> {
    let max = model.max_level();
    let mut modes = Vec::new();
    for level in (2..=max).rev() {
        modes.push(TerminalMode::Level {
            level,
            eight_bit: false,
        });
        modes.push(TerminalMode::Level {
            level,
            eight_bit: true,
        });
    }
    modes.push(TerminalMode::Vt100);
    modes.push(TerminalMode::Vt52);
    modes
}

/// DECTID codes a model offers, its own identity (10) first.
fn terminal_ids(model: Model) -> Vec<u16> {
    match model.max_level() {
        5 => vec![10, 9, 7, 5, 2, 1, 0],
        4 => vec![10, 0, 1, 2, 5, 7],
        _ => vec![10],
    }
}

// ------------------------------------------------------------------ menus

/// The Set-Up screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Directory,
    Global,
    Display,
    General,
    Communications,
    Printer,
    Keyboard,
    Tab,
}

impl Screen {
    fn title(self) -> &'static str {
        match self {
            Screen::Directory => "Set-Up Directory",
            Screen::Global => "Global Set-Up",
            Screen::Display => "Display Set-Up",
            Screen::General => "General Set-Up",
            Screen::Communications => "Communications Set-Up Comm1",
            Screen::Printer => "Printer Set-Up",
            Screen::Keyboard => "Keyboard Set-Up",
            Screen::Tab => "Tab Set-Up",
        }
    }

    fn next(self) -> Screen {
        match self {
            Screen::Directory => Screen::Global,
            Screen::Global => Screen::Display,
            Screen::Display => Screen::General,
            Screen::General => Screen::Communications,
            Screen::Communications => Screen::Printer,
            Screen::Printer => Screen::Keyboard,
            Screen::Keyboard => Screen::Tab,
            Screen::Tab => Screen::Global,
        }
    }
}

/// Actions the terminal or window performs for Set-Up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ClearComm,
    ResetSession,
    /// Recall the saved settings.
    Recall,
    Save,
    /// Recall the factory settings.
    Default,
}

/// What a Set-Up key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The screen changed (or nothing happened).
    Redraw,
    /// Leave Set-Up, applying the features.
    Exit,
    /// Perform an action, then report with [`SetupMenu::done`].
    Action(Action),
}

/// Keys Set-Up understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Tab,
    Backspace,
    /// Typed text (only used while entering an answerback message).
    Text(String),
    /// The Set-Up key: leaves Set-Up, or cancels answerback entry.
    SetUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Goto(Screen),
    NextSetUp,
    ToDirectory,
    Do(Action),
    ClearDisplay,
    Language,
    Dialect,
    EnableSessions,
    DisableSessions,
    Exit,
    ScreenAlign,
    // Global
    OnLine,
    CommPorts,
    CrtSaver,
    CommPort,
    Refresh,
    PrinterAssignment,
    // Display
    Columns,
    Controls,
    AutoWrap,
    Scroll,
    Screen,
    Cursor,
    CursorStyle,
    Status,
    CursorBlink,
    Pages,
    Lines,
    VerticalCoupling,
    PageCoupling,
    AutoResize,
    // General
    TerminalMode,
    UdkLock,
    UserFeatures,
    CharacterSetMode,
    Keypad,
    CursorKeys,
    NewLine,
    Upss,
    TerminalId,
    Update,
    // Communications
    Transmit,
    Receive,
    Xoff,
    DataFormat,
    StopBits,
    LocalEcho,
    ModemControl,
    Disconnect,
    TransmitLimit,
    AutoAnswerback,
    Answerback,
    Conceal,
    ModemHigh,
    ModemLow,
    // Keyboard
    Typewriter,
    Lock,
    AutoRepeat,
    Keyclick,
    MarginBell,
    WarningBell,
    CharacterMode,
    Backarrow,
    Compose,
    Alt,
    LocalKey(u8),
    BreakKey,
    CommaKeys,
    AngleKey,
    TildeKey,
    // Tab
    ClearTabs,
    SetTabs8,
    /// The tab ruler; the column is [`SetupMenu::tab_col`].
    Ruler,
}

fn rows(screen: Screen) -> Vec<Vec<Field>> {
    use Field as F;
    let nav = || vec![F::NextSetUp, F::ToDirectory];
    let with_nav = |rest: &[Field]| {
        let mut row = nav();
        row.extend_from_slice(rest);
        row
    };
    match screen {
        Screen::Directory => vec![
            vec![
                F::Goto(Screen::Global),
                F::Goto(Screen::Display),
                F::Goto(Screen::General),
                F::Goto(Screen::Communications),
                F::Goto(Screen::Printer),
                F::Goto(Screen::Keyboard),
                F::Goto(Screen::Tab),
            ],
            vec![
                F::ClearDisplay,
                F::Do(Action::ClearComm),
                F::Do(Action::ResetSession),
                F::Do(Action::Recall),
                F::Do(Action::Save),
            ],
            vec![F::Language, F::Dialect, F::Do(Action::Default)],
            vec![
                F::EnableSessions,
                F::DisableSessions,
                F::Exit,
                F::ScreenAlign,
            ],
        ],
        Screen::Global => vec![
            nav(),
            vec![F::OnLine, F::CommPorts, F::CrtSaver],
            vec![F::CommPort, F::Refresh, F::PrinterAssignment],
        ],
        Screen::Display => vec![
            with_nav(&[F::Columns, F::Controls]),
            vec![F::AutoWrap, F::Scroll, F::Screen],
            vec![F::Cursor, F::CursorStyle, F::Status],
            vec![F::CursorBlink, F::Pages, F::Lines],
            vec![F::VerticalCoupling, F::PageCoupling, F::AutoResize],
        ],
        Screen::General => vec![
            with_nav(&[F::TerminalMode]),
            vec![F::UdkLock, F::UserFeatures, F::CharacterSetMode],
            vec![F::Keypad, F::CursorKeys, F::NewLine],
            vec![F::Upss, F::TerminalId],
            vec![F::Update],
        ],
        Screen::Communications => vec![
            with_nav(&[F::Transmit, F::Receive]),
            vec![F::Xoff, F::DataFormat, F::StopBits, F::LocalEcho],
            vec![F::ModemControl, F::Disconnect, F::TransmitLimit],
            vec![F::AutoAnswerback, F::Answerback, F::Conceal],
            vec![F::ModemHigh, F::ModemLow],
        ],
        Screen::Printer => vec![nav()],
        Screen::Keyboard => vec![
            with_nav(&[F::Typewriter, F::Lock]),
            vec![F::AutoRepeat, F::Keyclick, F::MarginBell, F::WarningBell],
            vec![F::CharacterMode, F::Backarrow, F::Compose, F::Alt],
            vec![
                F::LocalKey(0),
                F::LocalKey(1),
                F::LocalKey(2),
                F::LocalKey(3),
                F::BreakKey,
            ],
            vec![F::CommaKeys, F::AngleKey, F::TildeKey],
        ],
        Screen::Tab => vec![with_nav(&[F::ClearTabs]), vec![F::SetTabs8], vec![F::Ruler]],
    }
}

/// Steps `current` to the entry after it in `options`, wrapping round.
fn next_of<T: Copy + PartialEq>(options: &[T], current: T) -> T {
    let i = options.iter().position(|o| *o == current).unwrap_or(0);
    options[(i + 1) % options.len()]
}

fn on_off(on: bool, yes: &str, no: &str) -> String {
    if on { yes } else { no }.to_string()
}

fn volume(label: &str, v: Volume) -> String {
    match v {
        Volume::Off => format!("{label} Off"),
        Volume::Low => format!("{label} Low"),
        Volume::High => format!("{label} High"),
    }
}

/// Text shown for a field.
fn label(field: Field, f: &Features, model: Model) -> String {
    use Field as F;
    match field {
        F::Goto(s) => match s {
            Screen::Global => "Global",
            Screen::Display => "Display",
            Screen::General => "General",
            Screen::Communications => "Comm",
            Screen::Printer => "Printer",
            Screen::Keyboard => "Keyboard",
            Screen::Tab => "Tab",
            Screen::Directory => "Directory",
        }
        .into(),
        F::NextSetUp => "To Next Set-Up".into(),
        F::ToDirectory => "To Directory".into(),
        F::Do(a) => match a {
            Action::ClearComm => "Clear Comm",
            Action::ResetSession => "Reset Session",
            Action::Recall => "Recall",
            Action::Save => "Save",
            Action::Default => "Default",
        }
        .into(),
        F::ClearDisplay => "Clear Display".into(),
        F::Language => "Set-Up=English".into(),
        F::Dialect => DIALECTS
            .iter()
            .find(|(n, _)| *n == f.keyboard_language)
            .map_or(DIALECTS[0].1, |d| d.1)
            .into(),
        F::EnableSessions => "Enable Sessions".into(),
        F::DisableSessions => "Disable Sessions".into(),
        F::Exit => "Exit".into(),
        F::ScreenAlign => "Screen Align".into(),
        F::OnLine => on_off(f.on_line, "On Line", "Local"),
        F::CommPorts => on_off(f.two_sessions, "S1=Comm1,S2=Comm2", "S1=Comm1"),
        F::CrtSaver => on_off(f.crt_saver, "CRT Saver", "No CRT Saver"),
        F::CommPort => on_off(f.comm1_dec423, "Comm1=DEC-423", "Comm1=RS-232"),
        F::Refresh => on_off(f.refresh_60hz, "60 Hz", "70 Hz"),
        F::PrinterAssignment => match f.printer_session {
            1 => "Printer Session 1",
            2 => "Printer Session 2",
            _ => "Printer Shared",
        }
        .into(),
        F::Columns => on_off(f.columns_132, "132 Columns", "80 Columns"),
        F::Controls => on_off(f.display_controls, "Display Controls", "Interpret Controls"),
        F::AutoWrap => on_off(f.autowrap, "Auto Wrap", "No Auto Wrap"),
        F::Scroll => match f.scroll {
            Scroll::Jump => "Jump Scroll",
            Scroll::Smooth2 => "Smooth-2 Scroll",
            Scroll::Smooth4 => "Smooth-4 Scroll",
        }
        .into(),
        F::Screen => on_off(f.light_screen, "Light Screen", "Dark Screen"),
        F::Cursor => on_off(f.cursor, "Cursor", "No Cursor"),
        F::CursorStyle => on_off(
            f.cursor_style.is_block(),
            "Block Cursor Style",
            "Underline Cursor Style",
        ),
        F::Status => match f.status {
            StatusDisplay::None => "No Status Display",
            StatusDisplay::Indicator => "Indicator Status Display",
            StatusDisplay::HostWritable => "Host Writable Status Display",
        }
        .into(),
        F::CursorBlink => on_off(f.cursor_style.blinks(), "Cursor Blink", "Cursor Steady"),
        F::Pages => {
            let pages = if f.two_sessions { 72 } else { 144 };
            let count = if model.max_level() >= 5 {
                // The VT500s keep a fixed number of pages per length.
                match f.page_length {
                    0..=24 => 3,
                    25..=36 => 2,
                    _ => 1,
                }
            } else {
                (pages / f.page_length).max(1)
            };
            format!("{count}x{} Pages", f.page_length)
        }
        F::Lines => format!("{} Lines/Screen", f.screen_lines),
        F::VerticalCoupling => on_off(
            f.vertical_coupling,
            "Vertical Coupling",
            "No Vertical Coupling",
        ),
        F::PageCoupling => on_off(f.page_coupling, "Page Coupling", "No Page Coupling"),
        F::AutoResize => on_off(f.auto_resize, "Auto Resize Screen", "No Auto Resize Screen"),
        F::TerminalMode => match f.terminal_mode {
            TerminalMode::Vt52 => "VT52 Mode".into(),
            TerminalMode::Vt100 => "VT100 Mode".into(),
            TerminalMode::Level { level, eight_bit } => format!(
                "VT{level}00 Mode, {} Bit Controls",
                if eight_bit { 8 } else { 7 }
            ),
        },
        F::UdkLock => on_off(
            f.udk_locked,
            "User Defined Keys Locked",
            "User Defined Keys Unlocked",
        ),
        F::UserFeatures => on_off(
            f.user_features_locked,
            "User Features Locked",
            "User Features Unlocked",
        ),
        F::CharacterSetMode => on_off(f.national, "7-bit NRCS Characters", "8-bit Characters"),
        F::Keypad => on_off(f.keypad_application, "Application Keypad", "Numeric Keypad"),
        F::CursorKeys => on_off(
            f.cursor_keys_application,
            "Application Cursor Keys",
            "Normal Cursor Keys",
        ),
        F::NewLine => on_off(f.new_line, "New Line", "No New Line"),
        F::Upss => match f.upss {
            Supplemental::DecSupplemental => "UPSS DEC Supplemental",
            Supplemental::IsoLatin1 => "UPSS ISO Latin-1",
        }
        .into(),
        F::TerminalId => match f.terminal_id {
            0 => "VT100 ID".into(),
            1 => "VT101 ID".into(),
            2 => "VT102 ID".into(),
            5 => "VT220 ID".into(),
            7 => "VT320 ID".into(),
            9 => "VT420 ID".into(),
            _ => format!("{} ID", model_name(model)),
        },
        F::Update => match f.update {
            1 => "Never Update",
            3 => "Shared Update",
            _ => "When Available Update",
        }
        .into(),
        F::Transmit => format!("Transmit={}", f.transmit_speed),
        F::Receive => f
            .receive_speed
            .map_or("Receive=Transmit".into(), |s| format!("Receive={s}")),
        F::Xoff => f.xoff.map_or("No XOFF".into(), |x| format!("XOFF at {x}")),
        F::DataFormat => {
            DATA_FORMATS[usize::from(f.data_format).min(DATA_FORMATS.len() - 1)].into()
        }
        F::StopBits => on_off(f.two_stop_bits, "2 Stop Bits", "1 Stop Bit"),
        F::LocalEcho => on_off(f.local_echo, "Local Echo", "No Local Echo"),
        F::ModemControl => on_off(f.modem_control, "Modem Control", "Data Leads Only"),
        F::Disconnect => match f.disconnect {
            1 => "Disconnect, 60 ms Delay",
            2 => "No Disconnect",
            _ => "Disconnect, 2 s Delay",
        }
        .into(),
        F::TransmitLimit => on_off(f.limited_transmit, "Limited Transmit", "Unlimited Transmit"),
        F::AutoAnswerback => on_off(f.auto_answerback, "Auto Answerback", "No Auto Answerback"),
        // The message itself is typed and shown on the bottom line.
        F::Answerback => "Answerback=".into(),
        F::Conceal => on_off(f.conceal_answerback, "Concealed", "Not Concealed"),
        F::ModemHigh => "Modem High Speed = Ignore".into(),
        F::ModemLow => "Modem Low Speed = Ignore".into(),
        F::Typewriter => on_off(
            f.data_processing_keys,
            "Data Processing Keys",
            "Typewriter Keys",
        ),
        F::Lock => on_off(f.shift_lock, "Shift Lock", "Caps Lock"),
        F::AutoRepeat => on_off(f.auto_repeat, "Auto Repeat", "No Auto Repeat"),
        F::Keyclick => volume("Keyclick", f.keyclick),
        F::MarginBell => volume("Margin Bell", f.margin_bell),
        F::WarningBell => volume("Warning Bell", f.warning_bell),
        F::CharacterMode => on_off(f.position_mode, "Position Mode", "Character Mode"),
        F::Backarrow => on_off(f.backarrow_bs, "<X] Backspace", "<X] Delete"),
        F::Compose => match f.compose {
            ComposeKey::Local => "Local Compose",
            ComposeKey::Report => "Report Compose",
            ComposeKey::Ignore => "Ignore Compose",
        }
        .into(),
        F::Alt => on_off(f.report_alt, "Report Alt", "Ignore Alt"),
        F::LocalKey(k) => {
            let (n, name) =
                [(1, "Hold"), (2, "Print"), (3, "Set-Up"), (4, "Session")][usize::from(k)];
            match f.local_keys[usize::from(k)] {
                LocalKeyAction::Local => format!("F{n} = {name}"),
                LocalKeyAction::SendToHost => format!("F{n} = Fkey"),
                LocalKeyAction::Disabled => format!("F{n} = Ignore"),
            }
        }
        F::BreakKey => match f.break_key {
            BreakKey::Break => "F5 = Break",
            BreakKey::NoBreak => "F5 = No Break",
            BreakKey::FunctionKey => "F5 = Fkey",
            BreakKey::Ignore => "F5 = Ignore",
        }
        .into(),
        F::CommaKeys => on_off(f.comma_period_same, ",, and .. Keys", ",< and .> Keys"),
        F::AngleKey => on_off(f.angle_sends_tilde, "<> Key Sends `~", "<> Key"),
        F::TildeKey => on_off(f.tilde_sends_escape, "`~ Key Sends ESC", "`~ Key"),
        F::ClearTabs => "Clear all tabs".into(),
        F::SetTabs8 => "Set 8 column tabs".into(),
        F::Ruler => String::new(),
    }
}

/// Control characters of an answerback message shown as `·`.
fn printable(data: &[u8]) -> String {
    data.iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                char::from(b)
            } else {
                '·'
            }
        })
        .collect()
}

/// Steps a feature field to its next setting. Returns false for fields
/// that have none.
fn cycle(field: Field, f: &mut Features, model: Model) -> bool {
    use Field as F;
    let level = model.max_level();
    match field {
        F::Dialect => {
            let languages: Vec<Option<Nrc>> = DIALECTS.iter().map(|d| d.0).collect();
            f.keyboard_language = next_of(&languages, f.keyboard_language);
            if f.keyboard_language.is_none() {
                f.national = false;
            }
        }
        F::OnLine => f.on_line = !f.on_line,
        F::CommPorts => f.two_sessions = !f.two_sessions,
        F::CrtSaver => f.crt_saver = !f.crt_saver,
        F::CommPort => f.comm1_dec423 = !f.comm1_dec423,
        F::Refresh => f.refresh_60hz = !f.refresh_60hz,
        F::PrinterAssignment => f.printer_session = (f.printer_session + 1) % 3,
        F::Columns => {
            f.columns_132 = !f.columns_132;
            let cols = if f.columns_132 { 132 } else { 80 };
            let old = std::mem::take(&mut f.tabs);
            f.tabs = (0..cols)
                .map(|c| old.get(c).copied().unwrap_or(c % 8 == 0 && c > 0))
                .collect();
        }
        F::Controls => f.display_controls = !f.display_controls,
        F::AutoWrap => f.autowrap = !f.autowrap,
        F::Scroll => {
            f.scroll = next_of(&[Scroll::Smooth2, Scroll::Smooth4, Scroll::Jump], f.scroll)
        }
        F::Screen => f.light_screen = !f.light_screen,
        F::Cursor => f.cursor = !f.cursor,
        F::CursorStyle => {
            f.cursor_style = match f.cursor_style {
                CursorStyle::BlinkingBlock => CursorStyle::BlinkingUnderline,
                CursorStyle::SteadyBlock => CursorStyle::SteadyUnderline,
                CursorStyle::BlinkingUnderline => CursorStyle::BlinkingBlock,
                CursorStyle::SteadyUnderline => CursorStyle::SteadyBlock,
            }
        }
        F::Status if model.has_status_line() => {
            f.status = next_of(
                &[
                    StatusDisplay::None,
                    StatusDisplay::Indicator,
                    StatusDisplay::HostWritable,
                ],
                f.status,
            )
        }
        F::CursorBlink => {
            f.cursor_style = match f.cursor_style {
                CursorStyle::BlinkingBlock => CursorStyle::SteadyBlock,
                CursorStyle::SteadyBlock => CursorStyle::BlinkingBlock,
                CursorStyle::BlinkingUnderline => CursorStyle::SteadyUnderline,
                CursorStyle::SteadyUnderline => CursorStyle::BlinkingUnderline,
            }
        }
        F::Pages if level >= 4 => {
            f.page_length = next_of(page_lengths(model), f.page_length);
            if f.auto_resize {
                f.screen_lines = auto_screen_lines(f.page_length);
            }
        }
        F::Lines if level >= 4 => {
            f.screen_lines = next_of(screen_line_choices(model), f.screen_lines)
        }
        F::VerticalCoupling if level >= 4 => f.vertical_coupling = !f.vertical_coupling,
        F::PageCoupling if level >= 4 => f.page_coupling = !f.page_coupling,
        F::AutoResize if level >= 4 => f.auto_resize = !f.auto_resize,
        F::TerminalMode => f.terminal_mode = next_of(&terminal_modes(model), f.terminal_mode),
        F::UdkLock if level >= 2 => f.udk_locked = !f.udk_locked,
        F::UserFeatures => f.user_features_locked = !f.user_features_locked,
        // National mode needs a national keyboard dialect.
        F::CharacterSetMode if level >= 2 && f.keyboard_language.is_some() => {
            f.national = !f.national
        }
        F::Keypad => f.keypad_application = !f.keypad_application,
        F::CursorKeys => f.cursor_keys_application = !f.cursor_keys_application,
        F::NewLine => f.new_line = !f.new_line,
        F::Upss if level >= 3 => {
            f.upss = next_of(
                &[Supplemental::DecSupplemental, Supplemental::IsoLatin1],
                f.upss,
            )
        }
        F::TerminalId if level >= 4 => f.terminal_id = next_of(&terminal_ids(model), f.terminal_id),
        F::Update => f.update = next_of(&[2, 3, 1], f.update),
        F::Transmit => f.transmit_speed = next_of(&SPEEDS, f.transmit_speed),
        F::Receive => {
            let mut options = vec![None];
            options.extend(SPEEDS.iter().map(|s| Some(*s)));
            f.receive_speed = next_of(&options, f.receive_speed);
        }
        F::Xoff => f.xoff = next_of(&[Some(64), Some(128), None], f.xoff),
        F::DataFormat => f.data_format = (f.data_format + 1) % DATA_FORMATS.len() as u8,
        F::StopBits => f.two_stop_bits = !f.two_stop_bits,
        F::LocalEcho => f.local_echo = !f.local_echo,
        F::ModemControl => f.modem_control = !f.modem_control,
        F::Disconnect => f.disconnect = (f.disconnect + 1) % 3,
        F::TransmitLimit => f.limited_transmit = !f.limited_transmit,
        F::AutoAnswerback => f.auto_answerback = !f.auto_answerback,
        // A concealed message stays concealed until a new one is entered.
        F::Conceal if !f.conceal_answerback => f.conceal_answerback = true,
        F::Typewriter => f.data_processing_keys = !f.data_processing_keys,
        F::Lock => f.shift_lock = !f.shift_lock,
        F::AutoRepeat => f.auto_repeat = !f.auto_repeat,
        F::Keyclick => f.keyclick = next_of(&[Volume::High, Volume::Low, Volume::Off], f.keyclick),
        F::MarginBell => {
            f.margin_bell = next_of(&[Volume::Off, Volume::High, Volume::Low], f.margin_bell)
        }
        F::WarningBell => {
            f.warning_bell = next_of(&[Volume::High, Volume::Low, Volume::Off], f.warning_bell)
        }
        F::CharacterMode => f.position_mode = !f.position_mode,
        F::Backarrow => f.backarrow_bs = !f.backarrow_bs,
        F::Compose => {
            f.compose = next_of(
                &[ComposeKey::Local, ComposeKey::Report, ComposeKey::Ignore],
                f.compose,
            )
        }
        F::Alt => f.report_alt = !f.report_alt,
        F::LocalKey(k) => {
            let slot = &mut f.local_keys[usize::from(k)];
            *slot = next_of(
                &[
                    LocalKeyAction::Local,
                    LocalKeyAction::SendToHost,
                    LocalKeyAction::Disabled,
                ],
                *slot,
            );
        }
        F::BreakKey => {
            f.break_key = next_of(
                &[
                    BreakKey::Break,
                    BreakKey::NoBreak,
                    BreakKey::FunctionKey,
                    BreakKey::Ignore,
                ],
                f.break_key,
            )
        }
        F::CommaKeys => f.comma_period_same = !f.comma_period_same,
        F::AngleKey => f.angle_sends_tilde = !f.angle_sends_tilde,
        F::TildeKey => f.tilde_sends_escape = !f.tilde_sends_escape,
        _ => return false,
    }
    true
}

/// Lines per screen for a page length when Auto Resize Screen is on.
fn auto_screen_lines(page_length: usize) -> usize {
    match page_length {
        0..=25 => 24,
        26..=36 => 36,
        _ => 48,
    }
}

/// The widest text a field can show, so that fields keep their places as
/// their settings change.
fn width(field: Field, f: &Features, model: Model) -> usize {
    let mut probe = f.clone();
    let mut widest = label(field, &probe, model).chars().count();
    for _ in 0..16 {
        if !cycle(field, &mut probe, model) {
            break;
        }
        widest = widest.max(label(field, &probe, model).chars().count());
    }
    widest
}

/// An open Set-Up session.
#[derive(Debug, Clone)]
pub struct SetupMenu {
    model: Model,
    version: String,
    features: Features,
    screen: Screen,
    row: usize,
    col: usize,
    tab_col: usize,
    entry: Option<Vec<u8>>,
    message: Option<String>,
    clear_display: bool,
    aligning: bool,
}

/// Screen line of each field row (rows are two lines apart).
fn row_line(row: usize) -> usize {
    3 + 2 * row
}

impl SetupMenu {
    /// Opens Set-Up on the Set-Up Directory, with the field cursor on
    /// Global. `version` is shown after the model name.
    pub fn new(model: Model, version: &str, features: Features) -> SetupMenu {
        SetupMenu {
            model,
            version: version.to_string(),
            features,
            screen: Screen::Directory,
            row: 0,
            col: 0,
            tab_col: 8,
            entry: None,
            message: None,
            clear_display: false,
            aligning: false,
        }
    }

    pub fn features(&self) -> &Features {
        &self.features
    }

    /// Replaces the features shown, after an action changed them.
    pub fn set_features(&mut self, features: Features) {
        self.features = features;
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// True if Clear Display was selected: clear the page on leaving.
    pub fn clear_display_on_exit(&self) -> bool {
        self.clear_display
    }

    /// The message to show in place of the keyboard indicator line.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Reports that an action finished.
    pub fn done(&mut self) {
        self.message = Some("Done".into());
    }

    fn fields(&self) -> Vec<Vec<Field>> {
        rows(self.screen)
    }

    fn current(&self) -> Field {
        let rows = self.fields();
        let row = &rows[self.row.min(rows.len() - 1)];
        row[self.col.min(row.len() - 1)]
    }

    /// Column where each field of a row starts.
    fn positions(&self, row: &[Field]) -> Vec<usize> {
        let mut x = 2;
        row.iter()
            .map(|field| {
                let at = x;
                x += width(*field, &self.features, self.model) + 3;
                at
            })
            .collect()
    }

    fn go(&mut self, screen: Screen) {
        self.screen = screen;
        self.row = 0;
        self.col = 0;
    }

    fn cols(&self) -> usize {
        if self.features.columns_132 { 132 } else { 80 }
    }

    /// Handles a key. Every key clears the last message.
    pub fn input(&mut self, input: Input) -> Outcome {
        self.message = None;
        if self.aligning {
            self.aligning = false;
            return Outcome::Redraw;
        }
        if let Some(mut typed) = self.entry.take() {
            match input {
                Input::Enter => {
                    self.features.answerback = typed;
                    self.features.conceal_answerback = false;
                }
                Input::SetUp => {}
                Input::Backspace => {
                    typed.pop();
                    self.entry = Some(typed);
                }
                Input::Text(text) => {
                    for b in text.bytes().filter(|b| b.is_ascii()) {
                        if typed.len() < 30 {
                            typed.push(b);
                        }
                    }
                    self.entry = Some(typed);
                }
                Input::Tab => {
                    if typed.len() < 30 {
                        typed.push(b'\t');
                    }
                    self.entry = Some(typed);
                }
                _ => self.entry = Some(typed),
            }
            return Outcome::Redraw;
        }
        let rows = self.fields();
        let field = self.current();
        match input {
            Input::SetUp => return Outcome::Exit,
            Input::Left | Input::Right if field == Field::Ruler => {
                let cols = self.cols();
                self.tab_col = if input == Input::Left {
                    (self.tab_col + cols - 1) % cols
                } else {
                    (self.tab_col + 1) % cols
                };
            }
            Input::Tab if field == Field::Ruler => {
                let cols = self.cols();
                self.tab_col = (self.tab_col + 1..cols)
                    .find(|&c| self.features.tabs.get(c).copied().unwrap_or(false))
                    .unwrap_or(cols - 1);
            }
            Input::Left => self.col = self.col.saturating_sub(1),
            Input::Right => self.col = (self.col + 1).min(rows[self.row].len() - 1),
            Input::Tab => {
                self.col += 1;
                if self.col >= rows[self.row].len() {
                    self.col = 0;
                    self.row = (self.row + 1) % rows.len();
                }
            }
            Input::Up | Input::Down => {
                let target = if input == Input::Up {
                    self.row.checked_sub(1)
                } else {
                    (self.row + 1 < rows.len()).then_some(self.row + 1)
                };
                if let Some(target) = target {
                    // Keep the cursor over the same part of the screen.
                    let x = self.positions(&rows[self.row])[self.col];
                    let positions = self.positions(&rows[target]);
                    self.col = positions
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, p)| p.abs_diff(x))
                        .map_or(0, |(i, _)| i);
                    self.row = target;
                }
            }
            Input::Enter => return self.activate(field),
            Input::Backspace | Input::Text(_) => {}
        }
        Outcome::Redraw
    }

    fn activate(&mut self, field: Field) -> Outcome {
        match field {
            Field::Goto(screen) => self.go(screen),
            Field::NextSetUp => self.go(self.screen.next()),
            Field::ToDirectory => self.go(Screen::Directory),
            Field::Do(action) => return Outcome::Action(action),
            Field::ClearDisplay => {
                self.clear_display = true;
                self.done();
            }
            Field::Exit => return Outcome::Exit,
            Field::ScreenAlign => self.aligning = true,
            Field::EnableSessions | Field::DisableSessions => {
                self.message = Some("Sessions not selected".into())
            }
            Field::Answerback => self.entry = Some(Vec::new()),
            Field::ClearTabs => self.features.tabs.iter_mut().for_each(|t| *t = false),
            Field::SetTabs8 => self.features.tabs = default_tabs(self.cols()),
            Field::Ruler => {
                // No tab stop in column 1.
                if self.tab_col > 0 {
                    if let Some(t) = self.features.tabs.get_mut(self.tab_col) {
                        *t = !*t;
                    }
                }
            }
            other => {
                if !cycle(other, &mut self.features, self.model) {
                    self.message = Some("Not available".into());
                }
            }
        }
        Outcome::Redraw
    }

    /// The Set-Up screen as terminal output for a blank 24-line terminal of
    /// the session's width. The status line is left to the caller.
    pub fn render(&self) -> Vec<u8> {
        let mut out = String::from("\x1b[?7l\x1b[?25l\x1b[H\x1b[2J");
        if self.aligning {
            out.push_str("\x1b#8");
            return out.into_bytes();
        }
        let cols = self.cols();
        // Title: a double-width line with the model at the right.
        let model = format!("{} V{}", model_name(self.model), self.version);
        let half = cols / 2;
        let title = self.screen.title();
        let gap = half.saturating_sub(title.len() + model.len() + 1);
        let _ = write!(
            out,
            "\x1b[1;1H\x1b#6\x1b[1m{title}{:gap$}\x1b[4m{model}\x1b[m",
            ""
        );
        let rows = self.fields();
        for (r, row) in rows.iter().enumerate() {
            let line = row_line(r);
            if row.first() == Some(&Field::Ruler) {
                self.render_ruler(&mut out, line + 2, r == self.row);
                continue;
            }
            for ((c, field), x) in row.iter().enumerate().zip(self.positions(row)) {
                let text = label(*field, &self.features, self.model);
                if x + text.len() > cols {
                    continue;
                }
                let selected = r == self.row && c == self.col;
                let _ = write!(
                    out,
                    "\x1b[{};{}H{}{text}\x1b[m",
                    line,
                    x + 1,
                    if selected { "\x1b[7m" } else { "" }
                );
            }
        }
        if self.screen == Screen::Directory {
            let _ = write!(
                out,
                "\x1b[{};3Hveetee {} - free software under the MIT or Apache-2.0 licence",
                row_line(rows.len()),
                self.version
            );
        }
        if self.screen == Screen::Printer {
            let _ = write!(
                out,
                "\x1b[{};3HPrinting is not available in this version of veetee.",
                row_line(1)
            );
        }
        if let Some(typed) = &self.entry {
            // An answerback message is typed on the bottom line (VT420 table 5-5).
            let _ = write!(out, "\x1b[24;3HAnswerback={}_", printable(typed));
        } else if let Some(message) = &self.message {
            let _ = write!(out, "\x1b[24;3H\x1b[1m{message}\x1b[m");
        } else if self.screen == Screen::Communications
            && self.current() == Field::Answerback
            && !self.features.conceal_answerback
            && !self.features.answerback.is_empty()
        {
            let _ = write!(
                out,
                "\x1b[24;3HAnswerback={}",
                printable(&self.features.answerback)
            );
        }
        out.into_bytes()
    }

    fn render_ruler(&self, out: &mut String, line: usize, active: bool) {
        let cols = self.cols();
        for c in 0..cols {
            let tab = self.features.tabs.get(c).copied().unwrap_or(false);
            let selected = active && c == self.tab_col;
            let rev = if selected { "\x1b[7m" } else { "" };
            let _ = write!(
                out,
                "\x1b[{line};{}H{rev}{}\x1b[m\x1b[{};{}H{}",
                c + 1,
                if tab { 'T' } else { ' ' },
                line + 1,
                c + 1,
                (c + 1) % 10
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Terminal};

    fn screen_text(menu: &SetupMenu) -> Vec<String> {
        let mut term = Terminal::new(Config {
            cols: if menu.features.columns_132 { 132 } else { 80 },
            ..Config::default()
        });
        term.advance(&menu.render());
        (0..24)
            .map(|r| {
                term.grid()
                    .line(r)
                    .cells()
                    .iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn menu() -> SetupMenu {
        SetupMenu::new(Model::Vt420, "0.6", Features::factory(Model::Vt420))
    }

    #[test]
    fn directory_lists_the_vt420_screens() {
        let m = menu();
        let text = screen_text(&m);
        assert!(text[0].starts_with("Set-Up Directory"), "{}", text[0]);
        assert!(text[0].ends_with("VT420 V0.6"), "{}", text[0]);
        for word in [
            "Global", "Display", "General", "Comm", "Printer", "Keyboard", "Tab",
        ] {
            assert!(text[2].contains(word), "{word}: {}", text[2]);
        }
        assert!(text[4].contains("Clear Display") && text[4].contains("Save"));
        assert!(text[6].contains("North American Keyboard"));
        assert!(text[8].contains("Screen Align"));
    }

    #[test]
    fn enter_steps_a_feature_and_arrows_move() {
        let mut m = menu();
        m.input(Input::Right);
        assert_eq!(m.input(Input::Enter), Outcome::Redraw);
        assert_eq!(m.screen(), Screen::Display);
        // To Next Set-Up, To Directory, 80 Columns.
        m.input(Input::Right);
        m.input(Input::Right);
        m.input(Input::Enter);
        assert!(m.features().columns_132);
        assert!(screen_text(&m)[2].contains("132 Columns"));
        // Down keeps the cursor over the same part of the screen: from
        // 132 Columns to Dark Screen.
        m.input(Input::Down);
        m.input(Input::Enter);
        assert!(m.features().light_screen);
    }

    #[test]
    fn actions_and_exit_are_reported() {
        let mut m = menu();
        m.input(Input::Down);
        for _ in 0..4 {
            m.input(Input::Right);
        }
        assert_eq!(m.input(Input::Enter), Outcome::Action(Action::Save));
        m.done();
        assert_eq!(m.message(), Some("Done"));
        assert_eq!(m.input(Input::SetUp), Outcome::Exit);
    }

    #[test]
    fn answerback_is_typed_on_the_communications_screen() {
        let mut m = menu();
        m.go(Screen::Communications);
        m.row = 3;
        m.col = 1;
        m.input(Input::Enter);
        m.input(Input::Text("VMS1".into()));
        assert!(screen_text(&m)[23].contains("Answerback=VMS1_"));
        m.input(Input::Enter);
        assert_eq!(m.features().answerback, b"VMS1");
    }

    #[test]
    fn tab_ruler_toggles_stops() {
        let mut m = menu();
        m.go(Screen::Tab);
        m.row = 2;
        m.tab_col = 3;
        m.input(Input::Enter);
        assert!(m.features().tabs[3]);
        let text = screen_text(&m);
        assert_eq!(&text[8][..9], "   T    T");
        assert!(text[9].starts_with("1234567890"));
    }

    #[test]
    fn every_screen_fits_80_columns() {
        for model in [Model::Vt100, Model::Vt220, Model::Vt420, Model::Vt525] {
            let mut m = SetupMenu::new(model, "0.6", Features::factory(model));
            for screen in [
                Screen::Directory,
                Screen::Global,
                Screen::Display,
                Screen::General,
                Screen::Communications,
                Screen::Printer,
                Screen::Keyboard,
                Screen::Tab,
            ] {
                m.go(screen);
                for row in m.fields() {
                    let x = m.positions(&row);
                    let last = row.len() - 1;
                    let end = x[last] + width(row[last], &m.features, model);
                    assert!(end <= 80, "{model:?} {screen:?}: {row:?} ends at {end}");
                }
                if std::env::var_os("SETUP_SCREENS").is_some() {
                    println!("{}", screen_text(&m).join("\n"));
                }
            }
        }
    }

    #[test]
    fn features_survive_saving() {
        let mut f = Features::factory(Model::Vt525);
        f.columns_132 = true;
        f.tabs = default_tabs(132);
        f.tabs[4] = true;
        f.answerback = b"VMS\r".to_vec();
        f.terminal_mode = TerminalMode::Level {
            level: 4,
            eight_bit: true,
        };
        f.keyboard_language = Some(Nrc::German);
        f.national = true;
        f.page_length = 36;
        f.local_keys[2] = LocalKeyAction::Disabled;
        let back = Features::from_text(Model::Vt525, &f.to_text());
        assert_eq!(back, f);
    }
}
