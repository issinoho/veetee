//! The VT420 Set-Up screens (Installing and Using the VT420 Video Terminal,
//! chapter 5), used for the VT100 to VT420 models.

use super::*;

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
    // Printer
    PrintMode,
    PrintExtent,
    PrintTerminator,
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
    /// The tab ruler; the column is [`Screens::tab_col`].
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
        // The three that act on veetee's print jobs; speed, flow control
        // and data format are for a printer's cable, which veetee has none
        // of. 🔎 Their places on the screen are from Table 8-1's order, not
        // the figure.
        Screen::Printer => vec![
            with_nav(&[F::PrintMode]),
            vec![F::PrintExtent, F::PrintTerminator],
        ],
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
        // Installing and Using the VT420, table 8-1.
        F::PrintMode => match f.print_mode {
            PrintMode::Normal => "Normal Print Mode",
            PrintMode::Auto => "Auto Print Mode",
            PrintMode::Controller => "Controller Mode",
        }
        .into(),
        F::PrintExtent => on_off(f.print_full_page, "Print Full Page", "Print Scroll Region"),
        F::PrintTerminator => on_off(f.print_form_feed, "Terminator = FF", "No Terminator"),
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
        F::DataFormat => DATA_FORMATS
            .iter()
            .find(|d| (d.1, d.2) == (f.seven_bit_data, f.parity))
            .map_or(DATA_FORMATS[0].0, |d| d.0)
            .into(),
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
        F::PrintMode => {
            f.print_mode = match f.print_mode {
                PrintMode::Normal => PrintMode::Auto,
                PrintMode::Auto => PrintMode::Controller,
                PrintMode::Controller => PrintMode::Normal,
            }
        }
        F::PrintExtent => f.print_full_page = !f.print_full_page,
        F::PrintTerminator => f.print_form_feed = !f.print_form_feed,
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
        F::Transmit => f.transmit_speed = next_of(speeds(model), f.transmit_speed),
        F::Receive => {
            let mut options = vec![None];
            options.extend(speeds(model).iter().map(|s| Some(*s)));
            f.receive_speed = next_of(&options, f.receive_speed);
        }
        F::Xoff => f.xoff = next_of(&[Some(64), Some(128), None], f.xoff),
        F::DataFormat => {
            let at = DATA_FORMATS
                .iter()
                .position(|d| (d.1, d.2) == (f.seven_bit_data, f.parity))
                .map_or(0, |i| (i + 1) % DATA_FORMATS.len());
            (_, f.seven_bit_data, f.parity) = DATA_FORMATS[at];
        }
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
pub(super) fn auto_screen_lines(page_length: usize) -> usize {
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

/// An open VT420 Set-Up.
#[derive(Debug, Clone)]
pub(super) struct Screens {
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

impl Screens {
    /// Opens Set-Up on the Set-Up Directory, with the field cursor on
    /// Global. `version` is shown after the model name.
    pub fn new(model: Model, version: &str, features: Features) -> Screens {
        Screens {
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

    #[cfg(test)]
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
                if self.tab_col > 0
                    && let Some(t) = self.features.tabs.get_mut(self.tab_col)
                {
                    *t = !*t;
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
                "\x1b[{};3HPrint jobs go to a PDF or a printer, chosen in the window menu.",
                row_line(2)
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

    fn screen_text(menu: &Screens) -> Vec<String> {
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

    fn menu() -> Screens {
        Screens::new(Model::Vt420, "0.6", Features::factory(Model::Vt420))
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
    fn printer_set_up_has_print_mode_extent_and_terminator() {
        let mut m = menu();
        for _ in 0..4 {
            m.input(Input::Right);
        }
        m.input(Input::Enter);
        assert_eq!(m.screen(), Screen::Printer);
        let text = screen_text(&m);
        assert!(text[2].contains("Normal Print Mode"), "{}", text[2]);
        assert!(text[4].contains("Print Full Page"), "{}", text[4]);
        assert!(text[4].contains("No Terminator"), "{}", text[4]);
        assert!(text[6].contains("chosen in the window menu"), "{}", text[6]);
        // To Next Set-Up, To Directory, then the print mode.
        m.input(Input::Right);
        m.input(Input::Right);
        m.input(Input::Enter);
        assert_eq!(m.features().print_mode, PrintMode::Auto);
        assert!(screen_text(&m)[2].contains("Auto Print Mode"));
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
            let mut m = Screens::new(model, "0.6", Features::factory(model));
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
