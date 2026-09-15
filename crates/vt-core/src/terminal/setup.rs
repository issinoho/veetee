//! The terminal side of Set-Up: reading the current features, applying
//! changed ones, and the Set-Up Directory actions (Installing and Using the
//! VT420, chapter 5).

use super::{Emulator, Terminal, vt520};
use crate::Model;
use crate::cell::Cell;
use crate::charset::Charset;
use crate::config::{Config, StatusDisplay, Supplemental};
use crate::setup::{Features, Parity, Scroll, TerminalMode, Volume};

/// VT500 private modes a VT420 also keeps in Set-Up.
const DECXRLM: u16 = 73;
const DECCRTSM: u16 = 97;
const DECMCM: u16 = 99;
const DECAAM: u16 = 100;
const DECCANSM: u16 = 101;
const DECNULM: u16 = 102;
const DECHDPXM: u16 = 103;
const DECOSCNM: u16 = 106;
const DECHWUM: u16 = 113;

impl Terminal {
    /// The session's current Set-Up features.
    pub fn setup_features(&self) -> Features {
        self.emu.setup_features()
    }

    /// Applies features changed in Set-Up, as when leaving Set-Up.
    pub fn apply_setup_features(&mut self, features: &Features) {
        self.emu.apply_features(features);
        self.sync_parser();
    }

    /// Save: the current features become the power-up settings (used by
    /// RIS and Recall).
    pub fn save_setup_features(&mut self) {
        let features = self.emu.setup_features();
        self.emu.config.set_saved_features(features);
    }

    /// Global Set-Up "On Line"; off (Local), typed characters go to the
    /// screen instead of the host.
    pub fn on_line(&self) -> bool {
        self.emu.stored.on_line
    }

    /// Keyclick, warning bell and margin bell volumes (Keyboard Set-Up,
    /// DECSKCV, DECSWBV, DECSMBV).
    pub fn sound_volumes(&self) -> SoundVolumes {
        let s = &self.emu.setup;
        SoundVolumes {
            keyclick: volume_of(s.selection(b" r")),
            warning_bell: volume_of(s.selection(b" t")),
            margin_bell: volume_of(s.selection(b" u")),
        }
    }

    /// How long the terminal waits, with no keys pressed and nothing
    /// received, before blanking the screen; `None` when the CRT saver is
    /// off. A VT420 waits 30 minutes (Global Set-Up); a VT500 uses DECCRTST
    /// (minutes, 0 never; factory 15).
    pub fn crt_saver_timeout(&self) -> Option<std::time::Duration> {
        let e = &self.emu;
        if !e.setup.modes.get(&DECCRTSM).copied().unwrap_or(false) {
            return None;
        }
        let minutes = if e.config.model.max_level() >= 5 {
            e.setup.selection(b"-q").parse::<u64>().unwrap_or(15)
        } else {
            30
        };
        (minutes > 0).then(|| std::time::Duration::from_secs(minutes * 60))
    }

    /// The power-up settings, or `None` for the factory settings.
    pub fn saved_setup_features(&self) -> Option<&Features> {
        self.emu.config.setup.as_ref()
    }

    /// Recall: every feature returns to its saved value and the screen clears.
    pub fn recall_setup_features(&mut self) {
        self.emu.full_reset();
        self.sync_parser();
    }

    /// Default: the factory settings replace the saved ones, then Recall.
    pub fn restore_factory_setup(&mut self) {
        let c = &self.emu.config;
        self.emu.config = Config {
            model: c.model,
            rows: c.rows,
            cols: c.cols,
            scrollback_lines: c.scrollback_lines,
            extensions: c.extensions,
            ..Config::default()
        };
        self.recall_setup_features();
    }

    /// Clear Comm: abandons any control sequence or string being received
    /// and unlocks the keyboard. The screen is not cleared.
    pub fn clear_comm(&mut self) {
        self.parser = vt_parser::Parser::new();
        self.emu.dcs = super::dcs::DcsState::None;
        self.emu.osc.clear();
        self.emu.pending_input.clear();
        self.emu.modes.keyboard_locked = false;
        self.sync_parser();
    }

    /// Reset Session: a soft reset (DECSTR) of the active session.
    pub fn reset_session(&mut self) {
        self.emu.soft_reset();
        self.sync_parser();
    }

    /// Clear Display: erases the page and homes the cursor.
    pub fn clear_display(&mut self) {
        self.emu.exit_status_line();
        self.emu.grid.clear(Cell::BLANK);
        self.emu.goto(0, 0);
    }
}

/// The volumes of the terminal's sounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundVolumes {
    pub keyclick: Volume,
    pub warning_bell: Volume,
    pub margin_bell: Volume,
}

impl Config {
    /// Records `features` as the saved (power-up) settings, keeping the
    /// individual configuration fields in step.
    pub fn set_saved_features(&mut self, features: Features) {
        self.autowrap = features.autowrap;
        self.new_line = features.new_line;
        self.answerback = features.answerback.clone();
        self.status_display = features.status;
        self.keyboard_language = features.keyboard_language;
        self.national_mode = features.national;
        self.supplemental = features.upss;
        self.udk_locked = features.udk_locked;
        self.setup = Some(features);
    }
}

fn volume_of(value: &str) -> Volume {
    // VT520 volumes: 1 off, 2–4 low, 0 and 5–8 high.
    match value.parse::<u8>().unwrap_or(5) {
        1 => Volume::Off,
        2..=4 => Volume::Low,
        _ => Volume::High,
    }
}

fn volume_code(v: Volume) -> &'static str {
    match v {
        Volume::Off => "1",
        Volume::Low => "3",
        Volume::High => "5",
    }
}

/// DECSCS speeds by code (EK-VT520-RM); 0 ignores the modem speed
/// indicator.
const SPEEDS: [u32; 11] = [
    300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 76800, 115200,
];

fn speed_code(speed: u32) -> u16 {
    SPEEDS
        .iter()
        .position(|&s| s == speed)
        .map_or(6, |i| i as u16 + 1)
}

fn speed_of(code: u16) -> u32 {
    SPEEDS
        .get(usize::from(code).wrapping_sub(1))
        .copied()
        .unwrap_or(9600)
}

fn modem_speed_of(code: u16) -> Option<u32> {
    (code > 0).then(|| speed_of(code))
}

/// DECSPP parity codes, in [`Parity::ALL`] order.
fn parity_code(parity: Parity) -> u16 {
    Parity::ALL.iter().position(|&p| p == parity).unwrap_or(0) as u16 + 1
}

/// DECSFC flow control types for Set-Up's 0 none, 1 XON/XOFF, 2 DSR/DTR,
/// 3 both.
fn flow_code(flow: u8) -> u16 {
    match flow {
        0 => 4,
        n => u16::from(n),
    }
}

fn flow_of(code: u16) -> u8 {
    match code {
        4 => 0,
        n => n.min(3) as u8,
    }
}

impl Emulator {
    pub(super) fn setup_features(&self) -> Features {
        let s = &self.setup;
        let flag = |mode| s.modes.get(&mode).copied().unwrap_or(false);
        let stored = &self.stored;
        Features {
            columns_132: self.modes.columns_132,
            autowrap: self.modes.autowrap,
            scroll: match (
                self.modes.smooth_scroll,
                s.selection(b" p").parse::<u8>().unwrap_or(0),
            ) {
                (false, _) | (true, 9..) => Scroll::Jump,
                (true, 4..=8) => Scroll::Smooth4,
                (true, _) => Scroll::Smooth2,
            },
            light_screen: self.modes.reverse_screen,
            cursor: self.modes.cursor_visible,
            cursor_style: s.cursor_style,
            status: self.status.kind,
            page_length: self.rows(),
            screen_lines: match self.screen_lines {
                0..=30 => 24,
                31..=45 => 36,
                _ => 48,
            },
            vertical_coupling: self.modes.vertical_coupling,
            page_coupling: self.modes.page_coupling,
            terminal_mode: if !self.modes.ansi {
                TerminalMode::Vt52
            } else if self.level <= 1 {
                TerminalMode::Vt100
            } else {
                TerminalMode::Level {
                    level: self.level,
                    eight_bit: self.c1_8bit,
                }
            },
            udk_locked: self.udk.locked,
            national: self.modes.national,
            keypad_application: self.modes.keypad_application,
            cursor_keys_application: self.modes.cursor_keys_application,
            new_line: self.modes.new_line,
            upss: if self.upss == Charset::IsoLatin1 {
                Supplemental::IsoLatin1
            } else {
                Supplemental::DecSupplemental
            },
            terminal_id: s.terminal_id,
            update: s.update_session(),
            transmit_speed: speed_of(s.comm_speed(0)),
            modem_high_speed: modem_speed_of(s.comm_speed(3)),
            modem_low_speed: modem_speed_of(s.comm_speed(4)),
            seven_bit_data: s.port_parameters()[0] == 2,
            parity: Parity::ALL[usize::from(s.port_parameters()[1].clamp(1, 7)) - 1],
            two_stop_bits: s.port_parameters()[2] == 2,
            transmit_flow: match s.flow_control() {
                [1 | 3, kind, _] => flow_of(kind),
                _ => stored.transmit_flow,
            },
            receive_flow: match s.flow_control() {
                [2 | 3, kind, _] => flow_of(kind),
                _ => stored.receive_flow,
            },
            flow_threshold_high: s.flow_control()[2] == 2,
            transmit_rate: s.transmit_rate(0) as u8,
            fkey_rate: if s.transmit_rate(2) == s.transmit_rate(0) {
                stored.fkey_rate
            } else {
                s.transmit_rate(2) as u8
            },
            ignore_null: flag(DECNULM),
            half_duplex: flag(DECHDPXM),
            clear_on_column_change: !flag(vt520::DECNCSM),
            overscan: flag(DECOSCNM),
            host_wake_up: flag(DECHWUM),
            zero_style: s.selection(b",{").parse().unwrap_or(1),
            crt_saver_minutes: s.selection(b"-q").parse().unwrap_or(15),
            energy_saver_minutes: s.selection(b"-r").parse().unwrap_or(15),
            auto_repeat_rate: if s.selection(b"-p") == "10" { 10 } else { 30 },
            local_echo: !self.modes.send_receive,
            modem_control: flag(DECMCM),
            limited_transmit: flag(DECXRLM),
            crt_saver: flag(DECCRTSM),
            auto_answerback: flag(DECAAM),
            conceal_answerback: flag(DECCANSM),
            answerback: self.config.answerback.clone(),
            data_processing_keys: self.modes.data_processing_keys,
            auto_repeat: self.modes.auto_repeat,
            keyclick: volume_of(s.selection(b" r")),
            warning_bell: volume_of(s.selection(b" t")),
            margin_bell: volume_of(s.selection(b" u")),
            position_mode: flag(vt520::DECKPM),
            backarrow_bs: self.modes.backarrow_sends_bs,
            local_keys: s.local_function_keys,
            keyboard_language: self.config.keyboard_language,
            tabs: self.tabs.clone(),
            ..stored.clone()
        }
    }

    pub(super) fn apply_features(&mut self, f: &Features) {
        let model = self.config.model;
        let level = model.max_level();
        // The operating level first: it affects which features apply.
        match f.terminal_mode {
            TerminalMode::Vt52 if self.modes.ansi => self.enter_vt52(),
            TerminalMode::Vt100 => {
                self.modes.ansi = true;
                self.level = 1;
                self.c1_8bit = false;
            }
            TerminalMode::Level {
                level: l,
                eight_bit,
            } => {
                self.modes.ansi = true;
                self.level = l.min(level).max(2);
                self.c1_8bit = eight_bit;
            }
            TerminalMode::Vt52 => {}
        }
        self.pause = true;

        if f.columns_132 != self.modes.columns_132 {
            self.set_columns(if f.columns_132 { 132 } else { 80 });
        }
        self.modes.autowrap = f.autowrap;
        self.modes.smooth_scroll = f.scroll != Scroll::Jump;
        self.modes.reverse_screen = f.light_screen;
        self.modes.cursor_visible = f.cursor;
        self.setup.cursor_style = f.cursor_style;
        if model.has_status_line() {
            self.set_status_type(match f.status {
                StatusDisplay::None => 0,
                StatusDisplay::Indicator => 1,
                StatusDisplay::HostWritable => 2,
            });
        }
        if level >= 4 {
            self.set_page_length(f.page_length);
            self.set_screen_lines(f.screen_lines);
            self.modes.vertical_coupling = f.vertical_coupling;
            self.modes.page_coupling = f.page_coupling;
        }
        self.udk.locked = f.udk_locked;
        self.config.udk_locked = f.udk_locked;
        self.config.keyboard_language = f.keyboard_language;
        self.modes.national = f.national && f.keyboard_language.is_some() && level >= 2;
        self.modes.keypad_application = f.keypad_application;
        self.modes.cursor_keys_application = f.cursor_keys_application;
        self.modes.new_line = f.new_line;
        if level >= 3 {
            self.upss = f.upss.charset();
            self.charsets = super::initial_charsets(model, self.upss);
        }
        self.setup.terminal_id = f.terminal_id;
        self.setup.set_update_session(f.update);
        self.setup.set_comm_speed(0, speed_code(f.transmit_speed));
        self.setup
            .set_comm_speed(3, f.modem_high_speed.map_or(0, speed_code));
        self.setup
            .set_comm_speed(4, f.modem_low_speed.map_or(0, speed_code));
        self.setup.set_port_parameters([
            if f.seven_bit_data { 2 } else { 1 },
            parity_code(f.parity),
            if f.two_stop_bits { 2 } else { 1 },
        ]);
        let threshold = if f.flow_threshold_high { 2 } else { 1 };
        self.setup
            .set_flow_control(if f.transmit_flow == f.receive_flow {
                [3, flow_code(f.receive_flow), threshold]
            } else {
                [2, flow_code(f.receive_flow), threshold]
            });
        let rate = u16::from(f.transmit_rate.clamp(1, 3));
        self.setup.set_transmit_rate(0, rate);
        self.setup.set_transmit_rate(1, rate);
        self.setup.set_transmit_rate(
            2,
            if f.fkey_rate == 0 {
                rate
            } else {
                u16::from(f.fkey_rate)
            },
        );
        self.setup.set_selection(b",{", &f.zero_style.to_string());
        self.setup
            .set_selection(b"-q", &f.crt_saver_minutes.to_string());
        self.setup
            .set_selection(b"-r", &f.energy_saver_minutes.to_string());
        self.setup
            .set_selection(b"-p", &f.auto_repeat_rate.to_string());
        self.modes.send_receive = !f.local_echo;
        let modes = [
            (DECMCM, f.modem_control),
            (DECXRLM, f.limited_transmit),
            (DECCRTSM, f.crt_saver),
            (DECAAM, f.auto_answerback),
            (DECCANSM, f.conceal_answerback),
            (vt520::DECKPM, f.position_mode),
            (DECNULM, f.ignore_null),
            (DECHDPXM, f.half_duplex),
            (vt520::DECNCSM, !f.clear_on_column_change),
            (DECOSCNM, f.overscan),
            (DECHWUM, f.host_wake_up),
        ];
        for (mode, on) in modes {
            self.set_vt520_mode(mode, on);
        }
        self.config.answerback = f.answerback.clone();
        self.modes.data_processing_keys = f.data_processing_keys;
        self.modes.auto_repeat = f.auto_repeat;
        self.setup.set_selection(b" r", volume_code(f.keyclick));
        self.setup.set_selection(b" t", volume_code(f.warning_bell));
        self.setup.set_selection(b" u", volume_code(f.margin_bell));
        self.setup.set_selection(
            b" p",
            match f.scroll {
                Scroll::Smooth4 => "4",
                _ => "1",
            },
        );
        self.modes.backarrow_sends_bs = f.backarrow_bs;
        self.setup.local_function_keys = f.local_keys;
        let cols = self.cols();
        self.tabs = (0..cols)
            .map(|c| c > 0 && f.tabs.get(c).copied().unwrap_or(c % 8 == 0))
            .collect();
        self.stored = f.clone();
    }
}

impl Model {
    /// Set-Up's name for the model, e.g. `VT420`.
    pub fn setup_name(self) -> String {
        self.term_name_exact().to_uppercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::Features;

    #[test]
    fn a_new_terminal_has_the_factory_features() {
        for model in [Model::Vt100, Model::Vt220, Model::Vt420, Model::Vt525] {
            let term = Terminal::new(Config {
                model,
                ..Config::default()
            });
            assert_eq!(term.setup_features(), Features::factory(model), "{model:?}");
        }
    }

    #[test]
    fn applied_features_take_effect_and_read_back() {
        let mut term = Terminal::new(Config::default());
        let mut f = term.setup_features();
        f.columns_132 = true;
        f.tabs = crate::setup::default_tabs(132);
        f.autowrap = true;
        f.light_screen = true;
        f.page_length = 36;
        f.keyclick = Volume::Off;
        f.answerback = b"VMS1".to_vec();
        f.terminal_mode = TerminalMode::Level {
            level: 4,
            eight_bit: true,
        };
        f.crt_saver = false;
        term.apply_setup_features(&f);
        assert_eq!(term.grid().cols(), 132);
        assert_eq!(term.grid().rows(), 36);
        assert!(term.modes().autowrap && term.modes().reverse_screen);
        assert!(term.eight_bit_replies());
        assert_eq!(term.setup_features(), f);
    }

    #[test]
    fn vt500_features_take_effect_and_read_back() {
        let mut term = Terminal::new(Config {
            model: Model::Vt520,
            ..Config::default()
        });
        let mut f = term.setup_features();
        f.transmit_speed = 115200;
        f.modem_high_speed = Some(57600);
        f.seven_bit_data = true;
        f.parity = Parity::OddUnchecked;
        f.two_stop_bits = true;
        f.transmit_flow = 0;
        f.receive_flow = 2;
        f.flow_threshold_high = true;
        f.limited_transmit = true;
        f.transmit_rate = 3;
        f.fkey_rate = 2;
        f.ignore_null = false;
        f.clear_on_column_change = false;
        f.zero_style = 2;
        f.crt_saver_minutes = 60;
        f.auto_repeat_rate = 10;
        term.apply_setup_features(&f);
        assert_eq!(term.setup_features(), f);
        // The host sees them in its reports.
        for (request, reply) in [
            (&b"*r"[..], &b"\x1bP1$r1;11*r\x1b\\"[..]),
            (b"+w", b"\x1bP1$r1;2;5;2+w\x1b\\"),
            (b"-q", b"\x1bP1$r60-q\x1b\\"),
        ] {
            term.advance(b"\x1bP$q");
            term.advance(request);
            term.advance(b"\x1b\\");
            assert_eq!(
                term.take_output(),
                reply,
                "{}",
                String::from_utf8_lossy(request)
            );
        }
        term.advance(b"\x1b[?95$p");
        assert_eq!(term.take_output(), b"\x1b[?95;1$y", "DECNCSM set");
        // And host changes show in Set-Up.
        term.advance(b"\x1b[1;1;4+w\x1b[?102h");
        let f = term.setup_features();
        assert!(!f.seven_bit_data && f.parity == Parity::EvenUnchecked && f.ignore_null);
    }

    #[test]
    fn saved_features_return_on_recall() {
        let mut term = Terminal::new(Config::default());
        let mut f = term.setup_features();
        f.autowrap = true;
        f.new_line = true;
        term.apply_setup_features(&f);
        term.save_setup_features();
        let mut changed = f.clone();
        changed.autowrap = false;
        term.apply_setup_features(&changed);
        term.recall_setup_features();
        assert!(term.modes().autowrap && term.modes().new_line);
        term.restore_factory_setup();
        // New Line is off in the factory settings; Auto Wrap is the one
        // place veetee differs from DEC's table, so it stays on.
        assert!(!term.modes().new_line && term.modes().autowrap);
        assert_eq!(term.saved_setup_features(), None);
    }

    #[test]
    fn margin_bell_rings_eight_columns_from_the_margin() {
        let mut term = Terminal::new(Config::default());
        term.advance(&[b'x'; 72]);
        assert!(
            term.take_events().is_empty(),
            "margin bell is off by default"
        );
        let mut f = term.setup_features();
        f.margin_bell = Volume::High;
        term.apply_setup_features(&f);
        term.advance(b"\r");
        term.advance(&[b'x'; 70]);
        assert!(term.take_events().is_empty());
        term.advance(b"x");
        assert_eq!(term.take_events(), [crate::Event::MarginBell]);
        term.advance(b"xx");
        assert!(
            term.take_events().is_empty(),
            "only when the cursor reaches it"
        );
        assert_eq!(term.sound_volumes().margin_bell, Volume::High);
    }

    #[test]
    fn crt_saver_waits_as_set_up() {
        let term = Terminal::new(Config::default());
        assert_eq!(
            term.crt_saver_timeout(),
            Some(std::time::Duration::from_secs(1800))
        );
        let mut vt520 = Terminal::new(Config {
            model: Model::Vt520,
            ..Config::default()
        });
        assert_eq!(
            vt520.crt_saver_timeout(),
            Some(std::time::Duration::from_secs(900))
        );
        vt520.advance(b"\x1b[5-q");
        assert_eq!(
            vt520.crt_saver_timeout(),
            Some(std::time::Duration::from_secs(300))
        );
        vt520.advance(b"\x1b[0-q");
        assert_eq!(vt520.crt_saver_timeout(), None, "0 is never");
        let mut f = vt520.setup_features();
        f.crt_saver = false;
        vt520.apply_setup_features(&f);
        assert_eq!(vt520.crt_saver_timeout(), None);
    }

    #[test]
    fn vt420_terminal_id_changes_device_attributes() {
        let mut term = Terminal::new(Config::default());
        let mut f = term.setup_features();
        f.terminal_id = 7;
        term.apply_setup_features(&f);
        term.advance(b"\x1b[c");
        assert_eq!(term.take_output(), b"\x1b[?63;1;2;7;8;9c");
    }

    #[test]
    fn saved_features_apply_at_power_up() {
        let mut f = Features::factory(Model::Vt420);
        f.cursor_style = crate::CursorStyle::SteadyUnderline;
        f.tabs[4] = true;
        let mut config = Config::default();
        config.set_saved_features(f.clone());
        let term = Terminal::new(config);
        assert_eq!(term.cursor_style(), crate::CursorStyle::SteadyUnderline);
        assert_eq!(term.setup_features(), f);
    }
}
