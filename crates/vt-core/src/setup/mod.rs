//! Set-Up: the terminal's local configuration (F3).
//!
//! The VT100 to VT420 models show the VT420's Set-Up screens (Installing and
//! Using the VT420 Video Terminal, chapter 5): a Set-Up Directory and the
//! Global, Display, General, Communications, Printer, Keyboard and Tab
//! screens, with a field cursor. The VT510, VT520 and VT525 show the VT500
//! series' pull-right menus (EK-VT520-RM chapter 2). [`Features`] holds
//! every setting; the terminal reads its current features with
//! [`crate::Terminal::setup_features`] and applies them with
//! [`crate::Terminal::apply_setup_features`] when Set-Up is left.
//!
//! [`SetupMenu::render`] draws Set-Up as terminal output for a scratch
//! 24-line terminal, so Set-Up is drawn with the same fonts as the session.

use std::fmt::Write as _;

use crate::charset::Nrc;
use crate::{CursorStyle, LocalKeyAction, Model, StatusDisplay, Supplemental};

mod menus;
mod screens;

pub use screens::Screen;

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

/// Communication parity (VT520 Communication Set-Up; the VT420 offers
/// fewer combinations with the word size).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Parity {
    #[default]
    None,
    Even,
    Odd,
    EvenUnchecked,
    OddUnchecked,
    Mark,
    Space,
}

impl Parity {
    pub const ALL: [Parity; 7] = [
        Parity::None,
        Parity::Even,
        Parity::Odd,
        Parity::EvenUnchecked,
        Parity::OddUnchecked,
        Parity::Mark,
        Parity::Space,
    ];

    fn name(self) -> &'static str {
        match self {
            Parity::None => "none",
            Parity::Even => "even",
            Parity::Odd => "odd",
            Parity::EvenUnchecked => "even-unchecked",
            Parity::OddUnchecked => "odd-unchecked",
            Parity::Mark => "mark",
            Parity::Space => "space",
        }
    }

    /// The letter shown in the VT520 Set-Up summary line.
    pub fn letter(self) -> char {
        match self {
            Parity::None => 'N',
            Parity::Even | Parity::EvenUnchecked => 'E',
            Parity::Odd | Parity::OddUnchecked => 'O',
            Parity::Mark => 'M',
            Parity::Space => 'S',
        }
    }
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
    /// VT500 Display: DECCOLM clears the page (DECNCSM reset).
    pub clear_on_column_change: bool,
    /// VT500 Display: DECSZS 1 oval, 2 slashed, 3 dotted.
    pub zero_style: u8,
    /// VT500 Display: DECCRTST and DECSEST, in minutes.
    pub crt_saver_minutes: u16,
    pub energy_saver_minutes: u16,
    /// VT500 Display: DECHWUM, DECOSCNM.
    pub host_wake_up: bool,
    pub overscan: bool,
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
    pub seven_bit_data: bool,
    pub parity: Parity,
    pub two_stop_bits: bool,
    /// VT500: transmit flow control 0 none, 1 XON/XOFF, 2 DSR, 3 both;
    /// receive 0 none, 1 XON/XOFF, 2 DTR, 3 both.
    pub transmit_flow: u8,
    pub receive_flow: u8,
    /// VT500: XOFF at 768 characters instead of 64.
    pub flow_threshold_high: bool,
    /// VT500 transmit rate limits (DECSTRL): 1 150, 2 50, 3 30 characters
    /// per second; function keys may also be 0, no separate limit.
    pub transmit_rate: u8,
    pub fkey_rate: u8,
    /// VT500: DECNULM, DECHDPXM.
    pub ignore_null: bool,
    pub half_duplex: bool,
    /// VT500 Modem: speeds chosen by the speed indicator; `None` ignores it.
    pub modem_high_speed: Option<u32>,
    pub modem_low_speed: Option<u32>,
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
    /// VT500: DECARR, 30 (fast) or 10 (slow) keystrokes per second.
    pub auto_repeat_rate: u8,
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

/// VT420 Communications Set-Up character formats: label, 7-bit, parity.
const DATA_FORMATS: [(&str, bool, Parity); 7] = [
    ("8 Bits, No Parity", false, Parity::None),
    ("8 Bits, Even Parity", false, Parity::Even),
    ("8 Bits, Odd Parity", false, Parity::Odd),
    ("7 Bits, Even Parity", true, Parity::Even),
    ("7 Bits, Odd Parity", true, Parity::Odd),
    ("7 Bits, Mark Parity", true, Parity::Mark),
    ("7 Bits, Space Parity", true, Parity::Space),
];

/// Line speeds a model offers.
fn speeds(model: Model) -> &'static [u32] {
    if model.max_level() >= 5 {
        &[
            300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 76800, 115200,
        ]
    } else {
        &[300, 600, 1200, 2400, 4800, 9600, 19200, 38400]
    }
}

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
            scroll: if model.smooth_scroll_default() {
                Scroll::Smooth2
            } else {
                Scroll::Jump
            },
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
            clear_on_column_change: true,
            zero_style: 1,
            crt_saver_minutes: 15,
            energy_saver_minutes: 15,
            host_wake_up: false,
            overscan: false,
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
            seven_bit_data: false,
            parity: Parity::None,
            two_stop_bits: false,
            transmit_flow: 1,
            receive_flow: 1,
            flow_threshold_high: false,
            transmit_rate: 1,
            fkey_rate: 1,
            ignore_null: true,
            half_duplex: false,
            modem_high_speed: None,
            modem_low_speed: None,
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
            auto_repeat_rate: 30,
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
            "clear-on-column-change",
            b(self.clear_on_column_change).to_string(),
        );
        line("zero-style", self.zero_style.to_string());
        line("crt-saver-minutes", self.crt_saver_minutes.to_string());
        line(
            "energy-saver-minutes",
            self.energy_saver_minutes.to_string(),
        );
        line("host-wake-up", b(self.host_wake_up).to_string());
        line("overscan", b(self.overscan).to_string());
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
        line(
            "word-size",
            if self.seven_bit_data { "7" } else { "8" }.into(),
        );
        line("parity", self.parity.name().into());
        line(
            "stop-bits",
            if self.two_stop_bits { "2" } else { "1" }.into(),
        );
        line("transmit-flow", self.transmit_flow.to_string());
        line("receive-flow", self.receive_flow.to_string());
        line(
            "flow-threshold-high",
            b(self.flow_threshold_high).to_string(),
        );
        line("transmit-rate", self.transmit_rate.to_string());
        line("fkey-rate", self.fkey_rate.to_string());
        line("ignore-null", b(self.ignore_null).to_string());
        line("half-duplex", b(self.half_duplex).to_string());
        let speed = |s: Option<u32>| s.map_or("ignore".into(), |v| v.to_string());
        line("modem-high-speed", speed(self.modem_high_speed));
        line("modem-low-speed", speed(self.modem_low_speed));
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
        line("auto-repeat-rate", self.auto_repeat_rate.to_string());
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
                "clear-on-column-change" => f.clear_on_column_change = flag,
                "zero-style" => f.zero_style = num().unwrap_or(1).clamp(1, 3) as u8,
                "crt-saver-minutes" => {
                    if let Some(n) = num().filter(|n| SAVER_MINUTES.contains(&(*n as u16))) {
                        f.crt_saver_minutes = n as u16;
                    }
                }
                "energy-saver-minutes" => {
                    if let Some(n) = num().filter(|n| SAVER_MINUTES.contains(&(*n as u16))) {
                        f.energy_saver_minutes = n as u16;
                    }
                }
                "host-wake-up" => f.host_wake_up = flag,
                "overscan" => f.overscan = flag,
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
                    if let Some(s) = num()
                        .map(|n| n as u32)
                        .filter(|s| speeds(model).contains(s))
                    {
                        f.transmit_speed = s;
                    }
                }
                "receive-speed" => {
                    f.receive_speed = v.parse::<u32>().ok().filter(|s| speeds(model).contains(s))
                }
                "modem-high-speed" => {
                    f.modem_high_speed = v.parse::<u32>().ok().filter(|s| speeds(model).contains(s))
                }
                "modem-low-speed" => {
                    f.modem_low_speed = v.parse::<u32>().ok().filter(|s| speeds(model).contains(s))
                }
                "transmit-flow" => f.transmit_flow = num().unwrap_or(1).min(3) as u8,
                "receive-flow" => f.receive_flow = num().unwrap_or(1).min(3) as u8,
                "flow-threshold-high" => f.flow_threshold_high = flag,
                "transmit-rate" => f.transmit_rate = num().unwrap_or(1).clamp(1, 3) as u8,
                "fkey-rate" => f.fkey_rate = num().unwrap_or(1).min(3) as u8,
                "ignore-null" => f.ignore_null = flag,
                "half-duplex" => f.half_duplex = flag,
                "word-size" => f.seven_bit_data = v == "7",
                "parity" => {
                    if let Some(p) = Parity::ALL.into_iter().find(|p| p.name() == v) {
                        f.parity = p;
                    }
                }
                "xoff" => f.xoff = v.parse::<u16>().ok().filter(|x| [64, 128].contains(x)),
                // Written by veetee 0.7.
                "data-format" => {
                    let (_, seven, parity) = DATA_FORMATS[num().unwrap_or(0).min(6)];
                    f.seven_bit_data = seven;
                    f.parity = parity;
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
                "auto-repeat-rate" => {
                    f.auto_repeat_rate = if v == "10" { 10 } else { 30 };
                }
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

/// VT500 CRT saver and energy saver intervals in minutes; 0 is never.
const SAVER_MINUTES: [u16; 5] = [0, 5, 15, 30, 60];

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

/// An open Set-Up: the VT420 screens or the VT500 menus, by model.
#[derive(Debug, Clone)]
pub struct SetupMenu(Style);

#[derive(Debug, Clone)]
enum Style {
    Screens(Box<screens::Screens>),
    Menus(Box<menus::Menus>),
}

impl SetupMenu {
    /// Opens Set-Up for `model`. `version` is the firmware version shown in
    /// Set-Up (veetee's major and minor version).
    pub fn new(model: Model, version: &str, features: Features) -> SetupMenu {
        SetupMenu(if model.max_level() >= 5 {
            Style::Menus(Box::new(menus::Menus::new(model, version, features)))
        } else {
            Style::Screens(Box::new(screens::Screens::new(model, version, features)))
        })
    }

    /// The session Set-Up was opened in (1 or 2), shown by the VT500
    /// summary line.
    pub fn set_session(&mut self, session: u8) {
        if let Style::Menus(m) = &mut self.0 {
            m.set_session(session);
        }
    }

    pub fn features(&self) -> &Features {
        match &self.0 {
            Style::Screens(m) => m.features(),
            Style::Menus(m) => m.features(),
        }
    }

    /// Replaces the features shown, after an action changed them.
    pub fn set_features(&mut self, features: Features) {
        match &mut self.0 {
            Style::Screens(m) => m.set_features(features),
            Style::Menus(m) => m.set_features(features),
        }
    }

    /// True if Clear Display was selected: clear the page on leaving.
    pub fn clear_display_on_exit(&self) -> bool {
        match &self.0 {
            Style::Screens(m) => m.clear_display_on_exit(),
            Style::Menus(m) => m.clear_display_on_exit(),
        }
    }

    /// The last status message ("Done"), until the next key.
    pub fn message(&self) -> Option<&str> {
        match &self.0 {
            Style::Screens(m) => m.message(),
            Style::Menus(m) => m.message(),
        }
    }

    /// Reports that an action finished.
    pub fn done(&mut self) {
        match &mut self.0 {
            Style::Screens(m) => m.done(),
            Style::Menus(m) => m.done(),
        }
    }

    /// What the status line shows in Set-Up: `None` keeps the keyboard
    /// indicator line (VT420); the VT500s show the Set-Up summary line, or
    /// a message in its place.
    pub fn status_line(&self) -> Option<String> {
        match &self.0 {
            Style::Screens(_) => None,
            Style::Menus(m) => Some(m.summary_line()),
        }
    }

    /// Handles a key.
    pub fn input(&mut self, input: Input) -> Outcome {
        match &mut self.0 {
            Style::Screens(m) => m.input(input),
            Style::Menus(m) => m.input(input),
        }
    }

    /// Set-Up as terminal output for a blank 24-line terminal of the
    /// session's width, with the UTF-8 and xterm SGR extensions (for the
    /// VT500 menus' boxes, check marks and dimmed items).
    pub fn render(&self) -> Vec<u8> {
        match &self.0 {
            Style::Screens(m) => m.render(),
            Style::Menus(m) => m.render(),
        }
    }
}
