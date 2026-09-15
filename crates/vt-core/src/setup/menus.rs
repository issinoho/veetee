//! The VT500-series menu Set-Up (EK-VT520-RM chapter 2), used for the VT510,
//! VT520 and VT525 models: pull-right menus from a main menu, check boxes
//! and radio buttons for the settings, dialog boxes for the answerback
//! message and tab stops, and a Set-Up summary line in place of the status
//! line. Features veetee does not have are dimmed, as the terminal dims
//! features that cannot be selected (table 2-2).

use super::*;

/// The menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Menu {
    Main,
    Actions,
    Session,
    SelectSession,
    UpdateSession,
    Display,
    LinesPerScreen,
    LinesPerPage,
    Columns,
    Status,
    Scrolling,
    Background,
    CursorDisplay,
    CursorCoupling,
    WritingDirection,
    Zero,
    CrtSaver,
    EnergySaver,
    Color,
    TerminalType,
    Emulation,
    TerminalId,
    CharacterSet,
    AsciiEmulation,
    Keyboard,
    VtLanguage,
    CapsLock,
    Keyclick,
    WarningBell,
    MarginBell,
    Encoding,
    AutoRepeat,
    Communication,
    WordSize,
    Parity,
    StopBits,
    TransmitSpeed,
    ReceiveSpeed,
    TransmitFlow,
    ReceiveFlow,
    Threshold,
    TransmitRate,
    FkeyRate,
    Modem,
    Disconnect,
    ModemHigh,
    ModemLow,
    Printer,
    Language,
}

/// What an item does when chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Action(Action),
    ClearDisplay,
    Exit,
    Align,
    Answerback,
    Tabs,
    /// A feature veetee does not have (always dimmed).
    Unavailable,
}

type Apply = Box<dyn Fn(&mut Features)>;

enum Kind {
    Sub(Menu),
    Check(bool, Apply),
    Radio(bool, Apply),
    Do(Command),
    Separator,
}

struct Item {
    label: String,
    kind: Kind,
    enabled: bool,
}

impl Item {
    fn when(mut self, enabled: bool) -> Item {
        self.enabled &= enabled;
        self
    }

    fn dim(self) -> Item {
        self.when(false)
    }

    fn selectable(&self) -> bool {
        self.enabled && !matches!(self.kind, Kind::Separator)
    }
}

fn sub(label: &str, menu: Menu) -> Item {
    Item {
        label: label.into(),
        kind: Kind::Sub(menu),
        enabled: true,
    }
}

fn check(label: &str, on: bool, apply: impl Fn(&mut Features) + 'static) -> Item {
    Item {
        label: label.into(),
        kind: Kind::Check(on, Box::new(apply)),
        enabled: true,
    }
}

fn radio(label: impl Into<String>, on: bool, apply: impl Fn(&mut Features) + 'static) -> Item {
    Item {
        label: label.into(),
        kind: Kind::Radio(on, Box::new(apply)),
        enabled: true,
    }
}

fn cmd(label: &str, command: Command) -> Item {
    Item {
        label: label.into(),
        kind: Kind::Do(command),
        enabled: command != Command::Unavailable,
    }
}

fn unavailable(label: &str) -> Item {
    cmd(label, Command::Unavailable)
}

/// A check box for a feature veetee keeps but cannot change.
fn fixed(label: &str, on: bool) -> Item {
    check(label, on, |_| {}).dim()
}

fn separator() -> Item {
    Item {
        label: String::new(),
        kind: Kind::Separator,
        enabled: false,
    }
}

fn volume_items(v: Volume, set: fn(&mut Features, Volume)) -> Vec<Item> {
    [
        ("High", Volume::High),
        ("Low", Volume::Low),
        ("Off", Volume::Off),
    ]
    .into_iter()
    .map(|(label, value)| radio(label, v == value, move |f| set(f, value)))
    .collect()
}

fn speed_name(speed: u32) -> String {
    let text = if speed >= 19200 {
        format!("{}.{}K", speed / 1000, speed % 1000 / 100)
    } else {
        speed.to_string()
    };
    format!("{text:>6} baud")
}

fn minutes_items(current: Option<u16>, set: fn(&mut Features, u16)) -> Vec<Item> {
    SAVER_MINUTES
        .iter()
        .map(|&m| {
            let label = if m == 0 {
                "Never".to_string()
            } else {
                format!("{m} minutes")
            };
            radio(label, current == Some(m), move |f| set(f, m))
        })
        .collect()
}

fn mode_name(mode: TerminalMode, model: Model) -> String {
    match mode {
        TerminalMode::Vt52 => "VT52".into(),
        TerminalMode::Vt100 => "VT100".into(),
        TerminalMode::Level { level: 5, .. } => model_name(model),
        TerminalMode::Level { level, .. } => format!("VT{level}00"),
    }
}

fn items(menu: Menu, f: &Features, model: Model, session: u8) -> Vec<Item> {
    use Menu as M;
    let level = match f.terminal_mode {
        TerminalMode::Level { level, .. } => level,
        _ => 1,
    };
    let rate = |label: &str, on: bool, value: u8, fkey: bool| {
        radio(label, on, move |f: &mut Features| {
            if fkey {
                f.fkey_rate = value;
            } else if value == 0 {
                f.limited_transmit = false;
            } else {
                f.limited_transmit = true;
                f.transmit_rate = value;
            }
        })
    };
    match menu {
        M::Main => vec![
            sub("Actions", M::Actions),
            sub("Session", M::Session),
            sub("Display", M::Display),
            sub("Color", M::Color),
            sub("Terminal type", M::TerminalType),
            sub("ASCII emulation", M::AsciiEmulation),
            sub("Keyboard", M::Keyboard),
            sub("Communication", M::Communication),
            sub("Modem", M::Modem),
            sub("Printer", M::Printer),
            cmd("Tabs...", Command::Tabs),
            sub("Set-Up language", M::Language),
            separator(),
            check("On-line", f.on_line, |f| f.on_line = !f.on_line),
            cmd("Save settings", Command::Action(Action::Save)),
            cmd("Restore settings", Command::Action(Action::Recall)),
            cmd("Exit Set-Up", Command::Exit),
        ],
        M::Actions => vec![
            cmd("Clear display", Command::ClearDisplay),
            cmd("Clear communications", Command::Action(Action::ClearComm)),
            cmd("Reset this session", Command::Action(Action::ResetSession)),
            cmd("Restore factory defaults", Command::Action(Action::Default)),
            separator(),
            unavailable("Clock"),
            unavailable("Calculator"),
            unavailable("Show character sets"),
            unavailable("Banner message..."),
        ],
        M::Session => vec![
            sub("Select session", M::SelectSession),
            unavailable("Session name..."),
            unavailable("Pages per session..."),
            sub("Soft char sets/session", M::Session).dim(),
            unavailable("Save settings for all"),
            unavailable("Restore settings for all"),
            sub("Copy settings from", M::Session).dim(),
            sub("Update session", M::UpdateSession),
        ],
        M::SelectSession => (1..=4)
            .map(|n| radio(format!("Session {n}"), n == session, |_| {}).when(n == session))
            .collect(),
        M::UpdateSession => [
            ("Only when active", 1),
            ("When available", 2),
            ("At regular intervals", 3),
        ]
        .into_iter()
        .map(|(label, v)| radio(label, f.update == v, move |f| f.update = v))
        .collect(),
        M::Display => vec![
            sub("Lines per screen", M::LinesPerScreen),
            sub("Lines per page", M::LinesPerPage),
            fixed("Save lines off top", false),
            sub("Columns per page", M::Columns),
            sub("Status display", M::Status).when(model.has_status_line()),
            sub("Scrolling mode", M::Scrolling),
            sub("Screen background", M::Background),
            sub("Cursor display", M::CursorDisplay),
            sub("Cursor coupling", M::CursorCoupling),
            sub("Writing direction", M::WritingDirection),
            sub("Zero font", M::Zero),
            check("Auto wrap", f.autowrap, |f| f.autowrap = !f.autowrap),
            check("New line mode", f.new_line, |f| f.new_line = !f.new_line),
            check("Lock user preferences", f.user_features_locked, |f| {
                f.user_features_locked = !f.user_features_locked
            }),
            check("Show control characters", f.display_controls, |f| {
                f.display_controls = !f.display_controls
            }),
            sub("CRT saver", M::CrtSaver),
            sub("Energy saver", M::EnergySaver).when(f.crt_saver && f.crt_saver_minutes > 0),
            check("Overscan", f.overscan, |f| f.overscan = !f.overscan).when(model != Model::Vt525),
            fixed("Framed windows", false),
            cmd("Screen alignment...", Command::Align),
        ],
        M::LinesPerScreen => {
            let mut v: Vec<Item> = screen_line_choices(model)
                .iter()
                .map(|&n| {
                    radio(format!("{n} lines"), f.screen_lines == n, move |f| {
                        f.screen_lines = n;
                        f.page_length = f.page_length.max(n);
                    })
                })
                .collect();
            v.push(check("Auto resize", f.auto_resize, |f| {
                f.auto_resize = !f.auto_resize
            }));
            v
        }
        M::LinesPerPage => page_lengths(model)
            .iter()
            .map(|&n| {
                let pages = match n {
                    0..=24 => 3,
                    25..=36 => 2,
                    _ => 1,
                };
                let label = format!(
                    "{n} lines x {pages} page{}",
                    if pages == 1 { "" } else { "s" }
                );
                radio(label, f.page_length == n, move |f| {
                    f.page_length = n;
                    f.screen_lines = f.screen_lines.min(super::screens::auto_screen_lines(n));
                })
            })
            .collect(),
        M::Columns => vec![
            radio("80 columns", !f.columns_132, |f| set_columns(f, false)),
            radio("132 columns", f.columns_132, |f| set_columns(f, true)),
            check("Clear on change", f.clear_on_column_change, |f| {
                f.clear_on_column_change = !f.clear_on_column_change
            }),
        ],
        M::Status => [
            ("Local status", StatusDisplay::Indicator),
            ("Host writable", StatusDisplay::HostWritable),
            ("None", StatusDisplay::None),
        ]
        .into_iter()
        .map(|(label, v)| radio(label, f.status == v, move |f| f.status = v))
        .collect(),
        M::Scrolling => [
            ("Slow smooth", Scroll::Smooth2),
            ("Fast smooth", Scroll::Smooth4),
            ("Jump", Scroll::Jump),
        ]
        .into_iter()
        .map(|(label, v)| radio(label, f.scroll == v, move |f| f.scroll = v))
        .collect(),
        M::Background => vec![
            radio("Dark", !f.light_screen, |f| f.light_screen = false),
            radio("Light", f.light_screen, |f| f.light_screen = true),
        ],
        M::CursorDisplay => {
            let block = f.cursor_style.is_block();
            let blink = f.cursor_style.blinks();
            vec![
                radio("Block", block, move |f| {
                    f.cursor_style = cursor_style(true, f.cursor_style.blinks())
                }),
                radio("Underline", !block, move |f| {
                    f.cursor_style = cursor_style(false, f.cursor_style.blinks())
                }),
                check("Blink", blink, |f| {
                    f.cursor_style =
                        cursor_style(f.cursor_style.is_block(), !f.cursor_style.blinks())
                }),
                check("Enable cursor", f.cursor, |f| f.cursor = !f.cursor),
            ]
        }
        M::CursorCoupling => vec![
            check("Vertical coupling", f.vertical_coupling, |f| {
                f.vertical_coupling = !f.vertical_coupling
            }),
            check("Page coupling", f.page_coupling, |f| {
                f.page_coupling = !f.page_coupling
            }),
        ],
        // Right to left needs a Hebrew keyboard.
        M::WritingDirection => vec![
            radio("Left to right", true, |_| {}),
            radio("Right to left", false, |_| {}).dim(),
        ],
        M::Zero => [("Oval zero", 1), ("Slashed zero", 2), ("Dotted zero", 3)]
            .into_iter()
            .map(|(label, v)| radio(label, f.zero_style == v, move |f| f.zero_style = v))
            .collect(),
        M::CrtSaver => {
            let current = if f.crt_saver { f.crt_saver_minutes } else { 0 };
            let mut v = minutes_items(Some(current), |f, m| {
                f.crt_saver = m > 0;
                if m > 0 {
                    f.crt_saver_minutes = m;
                }
            });
            v.push(check("Host wake-up", f.host_wake_up, |f| {
                f.host_wake_up = !f.host_wake_up
            }));
            v
        }
        M::EnergySaver => minutes_items(Some(f.energy_saver_minutes), |f, m| {
            f.energy_saver_minutes = m
        }),
        M::Color => vec![
            unavailable("Assign colors..."),
            unavailable("Alternate text colors..."),
            unavailable("Define colors..."),
            sub("Select color mode", M::Color).dim(),
            sub("ASCII color mode", M::Color).dim(),
            sub("Bold and blink style", M::Color).dim(),
            sub("Erase color", M::Color).dim(),
            sub("Reverse and blank attributes", M::Color).dim(),
            fixed("Intensity attributes", true),
        ],
        M::TerminalType => {
            let seven_bit = !matches!(
                f.terminal_mode,
                TerminalMode::Level {
                    eight_bit: true,
                    ..
                }
            );
            vec![
                sub("Emulation mode", M::Emulation),
                sub("Terminal ID to host", M::TerminalId)
                    .when(f.terminal_mode != TerminalMode::Vt52),
                sub("VT default char set", M::CharacterSet),
                sub("PCTerm character set", M::CharacterSet).dim(),
                check("7-bit NRCS characters", f.national, |f| {
                    f.national = !f.national
                })
                .when(level >= 2 && f.keyboard_language.is_some()),
                check("Transmit 7-bit controls", seven_bit, |f| {
                    if let TerminalMode::Level { eight_bit, .. } = &mut f.terminal_mode {
                        *eight_bit = !*eight_bit;
                    }
                })
                .when(level >= 2),
            ]
        }
        M::Emulation => terminal_modes(model)
            .into_iter()
            .filter(|m| {
                !matches!(
                    m,
                    TerminalMode::Level {
                        eight_bit: true,
                        ..
                    }
                )
            })
            .map(|m| {
                let on = match (m, f.terminal_mode) {
                    (
                        TerminalMode::Level { level: a, .. },
                        TerminalMode::Level { level: b, .. },
                    ) => a == b,
                    (a, b) => a == b,
                };
                radio(format!("{} mode", mode_name(m, model)), on, move |f| {
                    f.terminal_mode = match (m, f.terminal_mode) {
                        (
                            TerminalMode::Level { level, .. },
                            TerminalMode::Level { eight_bit, .. },
                        ) => TerminalMode::Level { level, eight_bit },
                        _ => m,
                    }
                })
            })
            .collect(),
        M::TerminalId => terminal_ids(model)
            .into_iter()
            .map(|id| {
                let name = match id {
                    0 => "VT100".into(),
                    1 => "VT101".into(),
                    2 => "VT102".into(),
                    5 => "VT220".into(),
                    7 => "VT320".into(),
                    9 => "VT420".into(),
                    _ => model_name(model),
                };
                radio(name, f.terminal_id == id, move |f| f.terminal_id = id)
            })
            .collect(),
        M::CharacterSet => vec![
            radio("ISO Latin-1", f.upss == Supplemental::IsoLatin1, |f| {
                f.upss = Supplemental::IsoLatin1
            }),
            radio(
                "DEC Multinational",
                f.upss == Supplemental::DecSupplemental,
                |f| f.upss = Supplemental::DecSupplemental,
            ),
        ],
        M::AsciiEmulation => [
            "Data lines",
            "Pages",
            "Attribute",
            "Write protect attributes",
            "Page edit",
            "Received CR",
            "Recognize DEL",
            "Enhance",
            "Autoscroll",
            "Autopage",
            "Send ACK",
            "Answerback mode",
            "TVI page-flip",
            "Font load",
            "Block mode",
            "Block end",
        ]
        .into_iter()
        .map(unavailable)
        .collect(),
        M::Keyboard => vec![
            sub("VT keyboard language", M::VtLanguage),
            sub("PC keyboard language", M::VtLanguage).dim(),
            unavailable("Define key..."),
            unavailable("Save key definitions"),
            unavailable("Recall key definitions"),
            check("Lock key definitions", f.udk_locked, |f| {
                f.udk_locked = !f.udk_locked
            }),
            sub("Caps lock function", M::CapsLock),
            sub("Keyclick volume", M::Keyclick),
            sub("Warning bell volume", M::WarningBell),
            sub("Margin bell volume", M::MarginBell),
            sub("Keyboard encoding", M::Encoding),
            sub("Auto repeat", M::AutoRepeat),
            check("Data processing keys", f.data_processing_keys, |f| {
                f.data_processing_keys = !f.data_processing_keys
            }),
            check("Application cursor keys", f.cursor_keys_application, |f| {
                f.cursor_keys_application = !f.cursor_keys_application
            }),
            check("Application keypad mode", f.keypad_application, |f| {
                f.keypad_application = !f.keypad_application
            }),
            fixed("Map PC keyboard to VT", false),
            fixed("Ignore missing keyboard", false),
        ],
        M::VtLanguage => DIALECTS
            .iter()
            .map(|&(nrc, name)| {
                let name = name.trim_end_matches(" Keyboard");
                radio(name, f.keyboard_language == nrc, move |f| {
                    f.keyboard_language = nrc;
                    f.national &= nrc.is_some();
                })
            })
            .collect(),
        M::CapsLock => vec![
            radio("Caps lock", !f.shift_lock, |f| f.shift_lock = false),
            radio("Shift lock", f.shift_lock, |f| f.shift_lock = true),
            radio("Reverse lock", false, |_| {}).dim(),
        ],
        M::Keyclick => volume_items(f.keyclick, |f, v| f.keyclick = v),
        M::WarningBell => volume_items(f.warning_bell, |f, v| f.warning_bell = v),
        M::MarginBell => volume_items(f.margin_bell, |f, v| f.margin_bell = v),
        M::Encoding => vec![
            radio("Character (ASCII)", !f.position_mode, |f| {
                f.position_mode = false
            }),
            radio("Scancode", false, |_| {}).dim(),
            radio("Key position", f.position_mode, |f| f.position_mode = true),
        ],
        M::AutoRepeat => [
            ("Fast (30/sec)", Some(30)),
            ("Slow (10/sec)", Some(10)),
            ("Off", None),
        ]
        .into_iter()
        .map(|(label, rate)| {
            let on = match rate {
                Some(r) => f.auto_repeat && f.auto_repeat_rate == r,
                None => !f.auto_repeat,
            };
            radio(label, on, move |f| match rate {
                Some(r) => {
                    f.auto_repeat = true;
                    f.auto_repeat_rate = r;
                }
                None => f.auto_repeat = false,
            })
        })
        .collect(),
        M::Communication => vec![
            unavailable("Port select..."),
            sub("Word size", M::WordSize),
            sub("Parity", M::Parity),
            sub("Stop bits", M::StopBits),
            sub("Transmit speed", M::TransmitSpeed),
            sub("Receive speed", M::ReceiveSpeed),
            sub("Transmit flow control", M::TransmitFlow),
            sub("Receive flow control", M::ReceiveFlow),
            sub("Flow control threshold", M::Threshold),
            sub("Transmit rate limit", M::TransmitRate),
            sub("Fkey rate limit", M::FkeyRate),
            check("Ignore Null character", f.ignore_null, |f| {
                f.ignore_null = !f.ignore_null
            }),
            check("Local echo", f.local_echo, |f| f.local_echo = !f.local_echo),
            check("Half duplex", f.half_duplex, |f| {
                f.half_duplex = !f.half_duplex
            }),
            check("Auto answerback", f.auto_answerback, |f| {
                f.auto_answerback = !f.auto_answerback
            }),
            cmd("Answerback message...", Command::Answerback),
            // Only a new message clears it (2.13.16).
            check("Answerback concealed", f.conceal_answerback, |f| {
                f.conceal_answerback = true
            }),
        ],
        M::WordSize => vec![
            radio("8 bits", !f.seven_bit_data, |f| f.seven_bit_data = false),
            radio("7 bits", f.seven_bit_data, |f| f.seven_bit_data = true),
        ],
        M::Parity => {
            let names = [
                "None",
                "Even",
                "Odd",
                "Even, unchecked",
                "Odd, unchecked",
                "Mark",
                "Space",
            ];
            Parity::ALL
                .into_iter()
                .zip(names)
                .map(|(p, name)| radio(name, f.parity == p, move |f| f.parity = p))
                .collect()
        }
        M::StopBits => vec![
            radio("1 bit", !f.two_stop_bits, |f| f.two_stop_bits = false),
            radio("2 bits", f.two_stop_bits, |f| f.two_stop_bits = true),
        ],
        M::TransmitSpeed => speeds(model)
            .iter()
            .rev()
            .map(|&s| {
                radio(speed_name(s), f.transmit_speed == s, move |f| {
                    f.transmit_speed = s
                })
            })
            .collect(),
        M::ReceiveSpeed => std::iter::once(None)
            .chain(speeds(model).iter().rev().map(|&s| Some(s)))
            .map(|s| {
                let label = s.map_or("Transmit speed".into(), speed_name);
                radio(label, f.receive_speed == s, move |f| f.receive_speed = s)
            })
            .collect(),
        M::TransmitFlow => ["None", "XON/XOFF", "DSR", "Both"]
            .into_iter()
            .zip(0..)
            .map(|(label, v)| radio(label, f.transmit_flow == v, move |f| f.transmit_flow = v))
            .collect(),
        M::ReceiveFlow => ["None", "XON/XOFF or XPC", "DTR", "Both"]
            .into_iter()
            .zip(0..)
            .map(|(label, v)| radio(label, f.receive_flow == v, move |f| f.receive_flow = v))
            .collect(),
        M::Threshold => vec![
            radio("Low (64)", !f.flow_threshold_high, |f| {
                f.flow_threshold_high = false
            }),
            radio("High (768)", f.flow_threshold_high, |f| {
                f.flow_threshold_high = true
            }),
        ],
        M::TransmitRate => vec![
            rate("None", !f.limited_transmit, 0, false),
            rate(
                "150 cps",
                f.limited_transmit && f.transmit_rate == 1,
                1,
                false,
            ),
            rate(
                " 50 cps",
                f.limited_transmit && f.transmit_rate == 2,
                2,
                false,
            ),
            rate(
                " 30 cps",
                f.limited_transmit && f.transmit_rate == 3,
                3,
                false,
            ),
        ],
        M::FkeyRate => vec![
            rate("None", f.fkey_rate == 0, 0, true),
            rate("150 cps", f.fkey_rate == 1, 1, true),
            rate(" 50 cps", f.fkey_rate == 2, 2, true),
            rate(" 30 cps", f.fkey_rate == 3, 3, true),
        ],
        M::Modem => vec![
            check("Enable modem control", f.modem_control, |f| {
                f.modem_control = !f.modem_control
            }),
            sub("Disconnect delay", M::Disconnect).when(f.modem_control),
            sub("Modem high speed", M::ModemHigh).when(f.modem_control),
            sub("Modem low speed", M::ModemLow).when(f.modem_control),
        ],
        M::Disconnect => [("2 seconds", 0), ("60 ms", 1), ("No disconnect", 2)]
            .into_iter()
            .map(|(label, v)| radio(label, f.disconnect == v, move |f| f.disconnect = v))
            .collect(),
        M::ModemHigh | M::ModemLow => {
            let high = menu == M::ModemHigh;
            let current = if high {
                f.modem_high_speed
            } else {
                f.modem_low_speed
            };
            std::iter::once(None)
                .chain(speeds(model).iter().rev().map(|&s| Some(s)))
                .map(|s| {
                    let label = s.map_or("Ignore".into(), speed_name);
                    radio(label, current == s, move |f| {
                        if high {
                            f.modem_high_speed = s;
                        } else {
                            f.modem_low_speed = s;
                        }
                    })
                })
                .collect()
        }
        M::Printer => {
            let mut v: Vec<Item> = [
                "Port select...",
                "Print mode",
                "Printer type",
                "DEC/ISO char sets",
                "PC character sets",
                "Print extent",
                "Print terminator",
            ]
            .into_iter()
            .map(unavailable)
            .collect();
            v.push(separator());
            v.extend(
                [
                    "Serial print speed",
                    "2-way communication",
                    "Transmit flow control",
                    "Receive flow control",
                    "Word size",
                    "Parity",
                    "Stop bits",
                ]
                .into_iter()
                .map(unavailable),
            );
            v
        }
        M::Language => ["English", "French", "German", "Italian", "Spanish"]
            .into_iter()
            .map(|name| radio(name, name == "English", |_| {}).when(name == "English"))
            .collect(),
    }
}

fn set_columns(f: &mut Features, wide: bool) {
    if f.columns_132 != wide {
        f.columns_132 = wide;
        let cols = if wide { 132 } else { 80 };
        f.tabs.resize(cols, false);
    }
}

fn cursor_style(block: bool, blink: bool) -> CursorStyle {
    match (block, blink) {
        (true, true) => CursorStyle::BlinkingBlock,
        (true, false) => CursorStyle::SteadyBlock,
        (false, true) => CursorStyle::BlinkingUnderline,
        (false, false) => CursorStyle::SteadyUnderline,
    }
}

/// An open dialog box.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Dialog {
    /// Focus: 0 the message, 1 OK, 2 Cancel.
    Answerback { text: Vec<u8>, focus: u8 },
    /// Focus: 0 the ruler, then the OK, Cancel, Set 8 column tabs and
    /// Clear all tabs buttons.
    Tabs {
        tabs: Vec<bool>,
        col: usize,
        focus: u8,
    },
}

const ANSWERBACK_BUTTONS: [&str; 2] = ["OK", "Cancel"];
const TAB_BUTTONS: [&str; 4] = ["OK", "Cancel", "Set 8 column tabs", "Clear all tabs"];

/// An open VT500 Set-Up.
#[derive(Debug, Clone)]
pub(super) struct Menus {
    model: Model,
    version: String,
    features: Features,
    session: u8,
    /// The open menus from the main menu down, each with its cursor.
    path: Vec<(Menu, usize)>,
    dialog: Option<Dialog>,
    message: Option<String>,
    clear_display: bool,
    aligning: bool,
}

/// A menu box on the screen: its top border line and left border column
/// (1-based) and the width inside the borders.
struct Frame {
    top: usize,
    left: usize,
    inner: usize,
}

/// The last screen line a box may use.
const BOTTOM: usize = 24;

impl Menus {
    pub fn new(model: Model, version: &str, features: Features) -> Menus {
        Menus {
            model,
            version: version.to_string(),
            features,
            session: 1,
            path: vec![(Menu::Main, 0)],
            dialog: None,
            message: None,
            clear_display: false,
            aligning: false,
        }
    }

    pub fn set_session(&mut self, session: u8) {
        self.session = session;
    }

    pub fn features(&self) -> &Features {
        &self.features
    }

    pub fn set_features(&mut self, features: Features) {
        self.features = features;
    }

    pub fn clear_display_on_exit(&self) -> bool {
        self.clear_display
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn done(&mut self) {
        self.message = Some("Done".into());
    }

    fn items(&self, menu: Menu) -> Vec<Item> {
        items(menu, &self.features, self.model, self.session)
    }

    fn cols(&self) -> usize {
        if self.features.columns_132 { 132 } else { 80 }
    }

    /// The Set-Up summary line (2.1.6): the communication port and line
    /// settings, the default character set, the keyboard, the emulation and
    /// the firmware version. A message replaces it until the next key.
    pub fn summary_line(&self) -> String {
        let cols = self.cols();
        if let Some(message) = &self.message {
            return format!(" {message:<w$}", w = cols - 1);
        }
        let f = &self.features;
        let mut line = vec![' '; cols];
        let mut put = |col: usize, text: &str| {
            for (i, ch) in text.chars().enumerate() {
                if let Some(c) = line.get_mut(col + i) {
                    *c = ch;
                }
            }
        };
        put(1, &format!("S{}=comm1", self.session));
        put(
            14,
            &format!(
                "{}{}{}{}",
                f.transmit_speed,
                f.parity.letter(),
                if f.seven_bit_data { 7 } else { 8 },
                if f.two_stop_bits { 2 } else { 1 }
            ),
        );
        put(
            27,
            match f.upss {
                Supplemental::IsoLatin1 => "ISO Latin-1",
                Supplemental::DecSupplemental => "DEC Multinational",
            },
        );
        let keyboard = DIALECTS
            .iter()
            .find(|d| d.0 == f.keyboard_language)
            .map_or(DIALECTS[0].1, |d| d.1)
            .trim_end_matches(" Keyboard");
        put(46, keyboard);
        put(64, &mode_name(f.terminal_mode, self.model));
        let version = format!("V{}", self.version);
        put(cols.saturating_sub(version.len() + 1), &version);
        line.into_iter().collect()
    }

    /// Handles a key. Every key clears the last message.
    pub fn input(&mut self, input: Input) -> Outcome {
        self.message = None;
        if self.aligning {
            self.aligning = false;
            return Outcome::Redraw;
        }
        if let Some(dialog) = self.dialog.take() {
            return self.dialog_input(dialog, input);
        }
        let depth = self.path.len() - 1;
        let (menu, at) = self.path[depth];
        let items = self.items(menu);
        match input {
            Input::SetUp => return Outcome::Exit,
            Input::Up => self.path[depth].1 = step(&items, at, false),
            Input::Down | Input::Tab => self.path[depth].1 = step(&items, at, true),
            Input::Left => {
                if depth > 0 {
                    self.path.pop();
                }
            }
            Input::Right => {
                if let Some(Kind::Sub(child)) = items.get(at).map(|i| &i.kind) {
                    self.open(*child);
                }
            }
            Input::Enter => {
                if let Some(item) = items.into_iter().nth(at) {
                    return self.activate(item);
                }
            }
            Input::Backspace | Input::Text(_) => {}
        }
        Outcome::Redraw
    }

    fn open(&mut self, menu: Menu) {
        let items = self.items(menu);
        // The cursor starts on the current setting, or the first item.
        let at = items
            .iter()
            .position(|i| i.selectable() && matches!(i.kind, Kind::Radio(true, _)))
            .or_else(|| items.iter().position(Item::selectable));
        match at {
            Some(at) => self.path.push((menu, at)),
            None => self.message = Some("Not available".into()),
        }
    }

    fn activate(&mut self, item: Item) -> Outcome {
        if !item.enabled {
            return Outcome::Redraw;
        }
        match item.kind {
            Kind::Sub(menu) => self.open(menu),
            Kind::Check(_, apply) => apply(&mut self.features),
            Kind::Radio(_, apply) => {
                apply(&mut self.features);
                // A choice closes its menu, unless check boxes share it.
                let (menu, _) = self.path[self.path.len() - 1];
                let has_checks = self
                    .items(menu)
                    .iter()
                    .any(|i| matches!(i.kind, Kind::Check(..)));
                if self.path.len() > 1 && !has_checks {
                    self.path.pop();
                }
            }
            Kind::Do(command) => match command {
                Command::Action(action) => return Outcome::Action(action),
                Command::ClearDisplay => {
                    self.clear_display = true;
                    self.done();
                }
                Command::Exit => return Outcome::Exit,
                Command::Align => self.aligning = true,
                Command::Answerback => {
                    let f = &self.features;
                    self.dialog = Some(Dialog::Answerback {
                        text: if f.conceal_answerback {
                            Vec::new()
                        } else {
                            f.answerback.clone()
                        },
                        focus: 0,
                    });
                }
                Command::Tabs => {
                    let cols = self.cols();
                    let mut tabs = self.features.tabs.clone();
                    tabs.resize(cols, false);
                    self.dialog = Some(Dialog::Tabs {
                        tabs,
                        col: 0,
                        focus: 0,
                    });
                }
                Command::Unavailable => {}
            },
            Kind::Separator => {}
        }
        Outcome::Redraw
    }

    fn dialog_input(&mut self, dialog: Dialog, input: Input) -> Outcome {
        match dialog {
            Dialog::Answerback {
                mut text,
                mut focus,
            } => {
                let accept = |m: &mut Menus, text: Vec<u8>| {
                    m.features.answerback = text;
                    m.features.conceal_answerback = false;
                };
                match input {
                    // The Set-Up key presses OK and leaves Set-Up (2.3).
                    Input::SetUp => {
                        accept(self, text);
                        return Outcome::Exit;
                    }
                    Input::Enter if focus == 1 => {
                        accept(self, text);
                        return Outcome::Redraw;
                    }
                    Input::Enter if focus == 2 => return Outcome::Redraw,
                    // Return enters a CR in the message (2.13.15).
                    Input::Enter => push_answerback(&mut text, b'\r'),
                    Input::Tab if focus == 0 => push_answerback(&mut text, b'\t'),
                    Input::Text(typed) => {
                        if focus == 0 {
                            for b in typed.bytes().filter(u8::is_ascii) {
                                push_answerback(&mut text, b);
                            }
                        }
                    }
                    Input::Backspace if focus == 0 => {
                        text.pop();
                    }
                    Input::Down | Input::Tab if focus == 0 => focus = 1,
                    Input::Up => focus = 0,
                    Input::Left if focus == 2 => focus = 1,
                    Input::Right | Input::Tab if focus == 1 => focus = 2,
                    Input::Tab => focus = 0,
                    _ => {}
                }
                self.dialog = Some(Dialog::Answerback { text, focus });
            }
            Dialog::Tabs {
                mut tabs,
                mut col,
                mut focus,
            } => {
                let cols = tabs.len();
                let row = cols / 2;
                match input {
                    Input::SetUp => {
                        self.features.tabs = tabs;
                        return Outcome::Exit;
                    }
                    Input::Enter if focus == 0 => {
                        // No tab stop in column 1.
                        if col > 0 {
                            tabs[col] = !tabs[col];
                        }
                    }
                    Input::Enter if focus == 1 => {
                        self.features.tabs = tabs;
                        return Outcome::Redraw;
                    }
                    Input::Enter if focus == 2 => return Outcome::Redraw,
                    Input::Enter if focus == 3 => tabs = default_tabs(cols),
                    Input::Enter => tabs.iter_mut().for_each(|t| *t = false),
                    Input::Left if focus == 0 => col = (col + cols - 1) % cols,
                    Input::Right if focus == 0 => col = (col + 1) % cols,
                    Input::Down if focus == 0 && col + row < cols => col += row,
                    Input::Down if focus == 0 => focus = 1,
                    Input::Up if focus == 0 => col = col.saturating_sub(row),
                    Input::Up => focus = 0,
                    Input::Left => focus = (focus - 1).max(1),
                    Input::Right => focus = (focus + 1).min(4),
                    Input::Tab => focus = (focus + 1) % 5,
                    _ => {}
                }
                self.dialog = Some(Dialog::Tabs { tabs, col, focus });
            }
        }
        Outcome::Redraw
    }

    /// The Set-Up screen as terminal output for a blank 24-line terminal of
    /// the session's width that accepts UTF-8 and SGR 2 (dim) for Set-Up's
    /// check boxes, radio buttons and dimmed items.
    pub fn render(&self) -> Vec<u8> {
        let mut out = String::from("\x1b[?7l\x1b[?25l\x1b[H\x1b[2J\x1b[m");
        if self.aligning {
            out.push_str("\x1b#8");
            return out.into_bytes();
        }
        out.push_str("\x1b[1;1H\x1b#6Set-Up");
        let boxes = self.layout();
        for (menu, cursor, frame) in &boxes {
            self.draw_menu(&mut out, frame, &self.items(*menu), *cursor);
        }
        match &self.dialog {
            Some(Dialog::Answerback { text, focus }) => {
                let (_, cursor, frame) = &boxes[boxes.len() - 1];
                let line = frame.top + 1 + cursor.unwrap_or(0);
                self.draw_answerback(&mut out, frame, line, text, *focus);
            }
            Some(Dialog::Tabs { tabs, col, focus }) => {
                self.draw_tabs(&mut out, tabs, *col, *focus);
            }
            None => {}
        }
        out.into_bytes()
    }

    /// The menu boxes to draw: the open menus with their cursors, then a
    /// preview of the submenu under the cursor. Each submenu's first item
    /// is level with the item that opened it, moved up to fit the screen.
    fn layout(&self) -> Vec<(Menu, Option<usize>, Frame)> {
        let cols = self.cols();
        let mut shown: Vec<(Menu, Option<usize>)> =
            self.path.iter().map(|&(m, at)| (m, Some(at))).collect();
        let (menu, at) = self.path[self.path.len() - 1];
        if self.dialog.is_none() {
            if let Some(Item {
                kind: Kind::Sub(child),
                enabled: true,
                ..
            }) = self.items(menu).get(at)
            {
                shown.push((*child, None));
            }
        }
        let mut boxes: Vec<(Menu, Option<usize>, Frame)> = Vec::new();
        for (menu, cursor) in shown {
            let items = self.items(menu);
            let inner = box_width(&items);
            let frame = match boxes.last() {
                None => Frame {
                    top: 2,
                    left: 1,
                    inner,
                },
                Some((_, at, p)) => Frame {
                    top: (p.top + at.unwrap_or(0))
                        .min(BOTTOM - items.len() - 1)
                        .max(2),
                    left: (p.left + p.inner + 1).min(cols - inner - 1),
                    inner,
                },
            };
            boxes.push((menu, cursor, frame));
        }
        boxes
    }

    fn draw_menu(&self, out: &mut String, frame: &Frame, items: &[Item], cursor: Option<usize>) {
        let w = frame.inner;
        draw_border(out, frame.top, frame.left, w, items.len());
        for (i, item) in items.iter().enumerate() {
            let line = frame.top + 1 + i;
            let _ = write!(out, "\x1b[{line};{}H", frame.left + 1);
            if let Kind::Separator = item.kind {
                out.push_str(&"-".repeat(w));
                continue;
            }
            let mark = match item.kind {
                Kind::Check(on, _) => {
                    if on {
                        '☒'
                    } else {
                        '☐'
                    }
                }
                Kind::Radio(on, _) => {
                    if on {
                        '◆'
                    } else {
                        '◇'
                    }
                }
                _ => ' ',
            };
            let arrow = if let Kind::Sub(_) = item.kind {
                "▶"
            } else {
                " "
            };
            let mut sgr = String::new();
            if cursor == Some(i) {
                sgr.push_str("\x1b[7m");
            }
            if !item.enabled {
                sgr.push_str("\x1b[2m");
            }
            let _ = write!(
                out,
                "{sgr}{mark} {:<lw$}{arrow}\x1b[m",
                item.label,
                lw = w - 3
            );
        }
    }

    fn draw_answerback(
        &self,
        out: &mut String,
        menu: &Frame,
        item_line: usize,
        text: &[u8],
        focus: u8,
    ) {
        let cols = self.cols();
        // Wide enough for the 30-character message, beside the menu.
        let inner = 31;
        let height = 7;
        let bottom = (item_line + 2).min(BOTTOM);
        let top = bottom - height - 1;
        let left = (menu.left + menu.inner + 1).min(cols - inner - 1);
        draw_border(out, top, left, inner, height);
        clear_inside(out, top, left, inner, height);
        let _ = write!(
            out,
            "\x1b[{};{}HEnter answerback message",
            top + 1,
            left + 2
        );
        let shown: String = printable(text)
            .chars()
            .rev()
            .take(30)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let rev = if focus == 0 { "\x1b[7m" } else { "" };
        let _ = write!(
            out,
            "\x1b[{};{}H\x1b[4m{rev}{shown:<30}\x1b[m",
            top + 3,
            left + 2
        );
        let mut x = left + 2;
        for (i, label) in ANSWERBACK_BUTTONS.iter().enumerate() {
            let width = label.len().max(6) + 2;
            draw_button(out, top + 5, x, width, label, focus == i as u8 + 1);
            x += width + 6;
        }
    }

    fn draw_tabs(&self, out: &mut String, tabs: &[bool], col: usize, focus: u8) {
        let cols = self.cols();
        let row = cols / 2;
        let inner = cols - 2;
        let height = 9 + 3 * cols.div_ceil(row) - 3;
        let top = BOTTOM - height - 1;
        draw_border(out, top, 1, inner, height);
        clear_inside(out, top, 1, inner, height);
        let title = "Tab Set-Up";
        let _ = write!(
            out,
            "\x1b[{};{}H{title}",
            top + 1,
            (cols - title.len()) / 2 + 1
        );
        let _ = write!(
            out,
            "\x1b[{};3HUse arrows to move the cursor and RETURN or ENTER to toggle:",
            top + 2
        );
        let x0 = (cols - row) / 2 + 1;
        for (r, start) in (0..cols).step_by(row).enumerate() {
            let line = top + 4 + 3 * r;
            let _ = write!(out, "\x1b[{line};{x0}H");
            for c in start..(start + row).min(cols) {
                let n = c + 1;
                out.push(if n % 10 == 0 {
                    char::from(b'0' + (n / 10 % 10) as u8)
                } else if n % 5 == 0 {
                    ','
                } else {
                    '.'
                });
            }
            let _ = write!(out, "\x1b[{};{x0}H\x1b[7m", line + 1);
            for (c, &tab) in tabs.iter().enumerate().skip(start).take(row) {
                let t = if tab { 'T' } else { ' ' };
                if focus == 0 && c == col {
                    let _ = write!(out, "\x1b[27m{t}\x1b[7m");
                } else {
                    out.push(t);
                }
            }
            out.push_str("\x1b[m");
        }
        let line = top + height - 2;
        let gap = (inner
            - TAB_BUTTONS
                .iter()
                .map(|b| b.len().max(6) + 2)
                .sum::<usize>())
            / 5;
        let mut x = 2 + gap;
        for (i, label) in TAB_BUTTONS.iter().enumerate() {
            let width = label.len().max(6) + 2;
            draw_button(out, line, x, width, label, focus == i as u8 + 1);
            x += width + gap;
        }
    }
}

fn push_answerback(text: &mut Vec<u8>, b: u8) {
    if text.len() < 30 {
        text.push(b);
    }
}

/// The next (or previous) selectable item, wrapping around.
fn step(items: &[Item], at: usize, forward: bool) -> usize {
    let n = items.len();
    (1..=n)
        .map(|k| {
            if forward {
                (at + k) % n
            } else {
                (at + n - k) % n
            }
        })
        .find(|&i| items[i].selectable())
        .unwrap_or(at)
}

/// Inside width: a mark column, a space, the label, a space and an arrow.
fn box_width(items: &[Item]) -> usize {
    items
        .iter()
        .map(|i| i.label.chars().count())
        .max()
        .unwrap_or(0)
        + 4
}

fn draw_border(out: &mut String, top: usize, left: usize, inner: usize, height: usize) {
    let bar = "─".repeat(inner);
    let _ = write!(out, "\x1b[{top};{left}H┌{bar}┐");
    for line in top + 1..=top + height {
        let _ = write!(
            out,
            "\x1b[{line};{left}H│\x1b[{line};{}H│",
            left + inner + 1
        );
    }
    let _ = write!(out, "\x1b[{};{left}H└{bar}┘", top + height + 1);
}

fn clear_inside(out: &mut String, top: usize, left: usize, inner: usize, height: usize) {
    for line in top + 1..=top + height {
        let _ = write!(out, "\x1b[{line};{}H{:inner$}", left + 1, "");
    }
}

/// A button: a box three lines high with its label centred.
fn draw_button(
    out: &mut String,
    top: usize,
    left: usize,
    width: usize,
    label: &str,
    focused: bool,
) {
    let inner = width - 2;
    draw_border(out, top, left, inner, 1);
    let rev = if focused { "\x1b[7m" } else { "" };
    let _ = write!(
        out,
        "\x1b[{};{}H{rev}{label:^inner$}\x1b[m",
        top + 1,
        left + 1
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Extensions, Terminal};

    fn menus() -> Menus {
        Menus::new(Model::Vt520, "0.8", Features::factory(Model::Vt520))
    }

    fn screen_text(m: &Menus) -> Vec<String> {
        let mut term = Terminal::new(Config {
            model: Model::Vt520,
            cols: m.cols(),
            extensions: Extensions {
                utf8: true,
                xterm_sgr: true,
                ..Extensions::default()
            },
            ..Config::default()
        });
        term.advance(&m.render());
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

    fn find(m: &Menus, label: &str) -> usize {
        let (menu, _) = m.path[m.path.len() - 1];
        m.items(menu)
            .iter()
            .position(|i| i.label == label)
            .unwrap_or_else(|| panic!("no {label}"))
    }

    /// Moves the cursor to `label` in the active menu and presses Enter.
    fn choose(m: &mut Menus, label: &str) -> Outcome {
        let at = find(m, label);
        let depth = m.path.len() - 1;
        m.path[depth].1 = at;
        m.input(Input::Enter)
    }

    #[test]
    fn main_menu_shows_the_actions_submenu() {
        let m = menus();
        let text = screen_text(&m);
        assert!(text[0].starts_with("Set-Up"));
        assert!(text[1].starts_with("┌"));
        assert!(text[2].contains("Actions") && text[2].contains("▶"));
        assert!(text[2].contains("Clear display"), "{}", text[2]);
        assert!(text[15].contains("☒ On-line"), "{}", text[15]);
        assert!(text[18].contains("Exit Set-Up"));
        assert!(text[19].starts_with("└"));
    }

    #[test]
    fn arrows_walk_the_menus_and_enter_chooses() {
        let mut m = menus();
        // Down skips nothing on the main menu until the separator.
        for _ in 0..2 {
            m.input(Input::Down);
        }
        assert_eq!(m.path, [(Menu::Main, 2)]);
        m.input(Input::Right);
        assert_eq!(m.path[1].0, Menu::Display);
        choose(&mut m, "Columns per page");
        assert_eq!(
            m.path[2],
            (Menu::Columns, 0),
            "cursor on the current setting"
        );
        choose(&mut m, "132 columns");
        assert!(m.features().columns_132);
        assert_eq!(m.features().tabs.len(), 132);
        // Check boxes share this menu, so it stays open.
        assert_eq!(m.path.len(), 3);
        m.input(Input::Left);
        choose(&mut m, "Scrolling mode");
        assert_eq!(m.path[2], (Menu::Scrolling, 2), "jump by default");
        choose(&mut m, "Fast smooth");
        assert_eq!(m.features().scroll, Scroll::Smooth4);
        assert_eq!(m.path.len(), 2, "a radio choice closes its menu");
        let wrapped = m.features().autowrap;
        choose(&mut m, "Auto wrap");
        assert_eq!(
            m.features().autowrap,
            !wrapped,
            "a toggle flips its feature"
        );
    }

    #[test]
    fn dimmed_items_are_skipped() {
        let mut m = menus();
        m.input(Input::Right);
        m.input(Input::Down);
        m.input(Input::Down);
        m.input(Input::Down);
        m.input(Input::Down);
        // Past the separator and the dimmed desktop features, back to the top.
        assert_eq!(m.path[1], (Menu::Actions, 0));
        m.input(Input::Up);
        assert_eq!(m.path[1], (Menu::Actions, 3));
    }

    #[test]
    fn actions_and_exit_are_reported() {
        let mut m = menus();
        m.input(Input::Right);
        assert_eq!(
            choose(&mut m, "Restore factory defaults"),
            Outcome::Action(Action::Default)
        );
        m.input(Input::Left);
        assert_eq!(
            choose(&mut m, "Save settings"),
            Outcome::Action(Action::Save)
        );
        m.done();
        assert!(m.summary_line().starts_with(" Done"));
        m.input(Input::Up);
        assert!(m.summary_line().contains("9600N81"));
        assert_eq!(choose(&mut m, "Exit Set-Up"), Outcome::Exit);
    }

    #[test]
    fn summary_line_follows_the_communication_settings() {
        let mut m = menus();
        let mut f = m.features().clone();
        f.transmit_speed = 19200;
        f.parity = Parity::Even;
        f.seven_bit_data = true;
        f.keyboard_language = Some(Nrc::German);
        m.set_features(f);
        m.set_session(2);
        let line = m.summary_line();
        assert_eq!(line.chars().count(), 80);
        assert!(line.starts_with(" S2=comm1     19200E71"), "{line}");
        assert!(
            line.contains("DEC Multinational") && line.contains("German"),
            "{line}"
        );
        assert!(line.contains("VT520") && line.trim_end().ends_with("V0.8"));
    }

    #[test]
    fn answerback_dialog() {
        let mut m = menus();
        choose(&mut m, "Communication");
        choose(&mut m, "Answerback message...");
        m.input(Input::Text("VMS".into()));
        m.input(Input::Enter);
        let text = screen_text(&m);
        assert!(text.iter().any(|l| l.contains("Enter answerback message")));
        if std::env::var_os("SETUP_SCREENS").is_some() {
            println!("{}", text.join("\n"));
        }
        assert!(text.iter().any(|l| l.contains("VMS·")), "{text:#?}");
        m.input(Input::Down);
        m.input(Input::Enter);
        assert_eq!(m.features().answerback, b"VMS\r");
        assert!(m.dialog.is_none());
        // Cancel keeps the old message.
        choose(&mut m, "Answerback message...");
        m.input(Input::Backspace);
        m.input(Input::Down);
        m.input(Input::Right);
        m.input(Input::Enter);
        assert_eq!(m.features().answerback, b"VMS\r");
    }

    #[test]
    fn tabs_dialog() {
        let mut m = menus();
        choose(&mut m, "Tabs...");
        let text = screen_text(&m);
        assert!(text.iter().any(|l| l.contains("Tab Set-Up")));
        if std::env::var_os("SETUP_SCREENS").is_some() {
            println!("{}", text.join("\n"));
        }
        assert!(text.iter().any(|l| l.contains("....,....1....,....2")));
        for _ in 0..3 {
            m.input(Input::Right);
        }
        m.input(Input::Enter);
        // OK.
        m.input(Input::Down);
        m.input(Input::Down);
        m.input(Input::Enter);
        assert!(m.features().tabs[3] && m.features().tabs[8]);
        choose(&mut m, "Tabs...");
        m.input(Input::Tab);
        m.input(Input::Right);
        m.input(Input::Right);
        m.input(Input::Right);
        m.input(Input::Enter);
        assert!(m.dialog.is_some(), "Clear all tabs stays open");
        m.input(Input::Left);
        m.input(Input::Left);
        m.input(Input::Left);
        m.input(Input::Enter);
        assert!(m.features().tabs.iter().all(|t| !t));
    }

    #[test]
    fn every_menu_fits_the_screen() {
        for model in [Model::Vt510, Model::Vt520, Model::Vt525] {
            for wide in [false, true] {
                let mut m = Menus::new(model, "0.8", Features::factory(model));
                let mut f = m.features().clone();
                f.modem_control = true;
                set_columns(&mut f, wide);
                m.set_features(f);
                let main = m.items(Menu::Main);
                for (i, item) in main.iter().enumerate() {
                    let Kind::Sub(menu) = item.kind else { continue };
                    m.path = vec![(Menu::Main, i), (menu, 0)];
                    for (j, child) in m.items(menu).iter().enumerate() {
                        m.path.truncate(2);
                        m.path[1].1 = j;
                        if let Kind::Sub(grandchild) = child.kind {
                            m.path.push((grandchild, 0));
                        }
                        for (menu, _, frame) in m.layout() {
                            let n = m.items(menu).len();
                            assert!(
                                frame.top >= 2
                                    && frame.top + n < BOTTOM
                                    && frame.left + frame.inner < m.cols(),
                                "{model:?} {menu:?} at {}, {}",
                                frame.top,
                                frame.left
                            );
                        }
                    }
                    if std::env::var_os("SETUP_SCREENS").is_some() && !wide {
                        m.path.truncate(2);
                        println!("{}", screen_text(&m).join("\n"));
                    }
                }
            }
        }
    }
}
