//! Display Controls (CRM, "Show control characters" in VT500 Set-Up): the
//! terminal shows every character it receives instead of acting on it
//! (EK-VT420-RM section 2, "Display Controls Mode"; EK-VT520-RM CRM). Only
//! Set-Up can select it.

use super::Emulator;
use crate::charset::C1_CONTROL_PICTURES;

impl Emulator {
    /// Shows `bytes` in the display controls font. Returns how many were
    /// shown: fewer when a line scrolls smoothly or DECSR ends the mode.
    pub(super) fn show_controls(&mut self, bytes: &[u8]) -> usize {
        for (i, &byte) in bytes.iter().enumerate() {
            self.show_control(byte);
            if !self.stored.display_controls || std::mem::take(&mut self.pause) {
                return i + 1;
            }
        }
        bytes.len()
    }

    fn show_control(&mut self, byte: u8) {
        // VT52 and VT100 modes show only the left half of the font.
        let byte = if !self.modes.ansi || self.level <= 1 {
            byte & 0x7F
        } else {
            byte
        };
        let ch = match byte {
            0x00..=0x1F => char::from_u32(0x2400 + u32::from(byte)),
            0x7F => Some('\u{2421}'),
            0x80..=0xA0 => char::from_u32(C1_CONTROL_PICTURES + u32::from(byte - 0x80)),
            0x20..=0x7E => Some(char::from(byte)),
            _ => self.upss.map(byte & 0x7F),
        };
        // Auto wrap always happens in this mode.
        let autowrap = std::mem::replace(&mut self.modes.autowrap, true);
        if let Some(ch) = ch {
            self.put_char(ch, byte);
        }
        self.modes.autowrap = autowrap;
        // LF, VT and FF are shown, then start a new line.
        if matches!(byte, 0x0A..=0x0C) {
            self.carriage_return();
            self.index();
        }
        self.recognise_secure_reset(byte);
    }

    /// DECSR (CSI Pr + p) still works, and leaves Display Controls
    /// (EK-VT420-RM DECSR notes).
    fn recognise_secure_reset(&mut self, byte: u8) {
        let tail = &mut self.crm_tail;
        tail.push(byte);
        if tail.len() > 12 {
            tail.remove(0);
        }
        if byte != b'p' || self.config.model.max_level() < 4 {
            return;
        }
        let Some(body) = tail.strip_suffix(b"+p") else {
            return;
        };
        let digits = body.iter().rev().take_while(|b| b.is_ascii_digit()).count();
        let (intro, number) = body.split_at(body.len() - digits);
        if !(intro.ends_with(b"\x1b[") || intro.ends_with(&[0x9B])) {
            return;
        }
        let pr = std::str::from_utf8(number)
            .ok()
            .and_then(|n| n.parse().ok());
        self.crm_tail.clear();
        self.secure_reset_confirming(pr);
        self.stored.display_controls = false;
    }
}

#[cfg(test)]
mod tests {
    use crate::{Config, Model, Terminal};

    fn crm(model: Model) -> Terminal {
        let mut term = Terminal::new(Config {
            model,
            ..Config::default()
        });
        let mut f = term.setup_features();
        f.display_controls = true;
        term.apply_setup_features(&f);
        term
    }

    fn row(term: &Terminal, r: usize) -> String {
        term.grid()
            .line(r)
            .cells()
            .iter()
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .into()
    }

    #[test]
    fn controls_are_shown_not_performed() {
        let mut term = crm(Model::Vt420);
        term.advance(b"A\x1b[1mB\x07\r\nC\x9b\xe9");
        assert_eq!(row(&term, 0), "A\u{241B}[1mB\u{2407}\u{240D}\u{240A}");
        // LF starts a new line after it is shown.
        assert_eq!(row(&term, 1), "C\u{F79B}é");
        assert!(term.take_output().is_empty() && term.take_events().is_empty());
        let cell = term.grid().line(0).cells()[2];
        assert!(cell.attrs.flags.is_empty(), "SGR is not performed");
    }

    #[test]
    fn auto_wrap_always_happens() {
        let mut term = crm(Model::Vt420);
        term.advance(&[b'x'; 82]);
        assert_eq!(row(&term, 1), "xx");
    }

    #[test]
    fn vt100_mode_shows_seven_bits() {
        let mut term = crm(Model::Vt100);
        term.advance(b"\x9b");
        assert_eq!(row(&term, 0), "\u{241B}");
    }

    #[test]
    fn secure_reset_leaves_display_controls() {
        let mut term = crm(Model::Vt420);
        term.advance(b"\x1b[12+p");
        assert!(!term.setup_features().display_controls);
        assert_eq!(term.take_output(), b"\x1b[12*q");
        term.advance(b"\x1b[1mB");
        assert_eq!(row(&term, 0), "B");
    }
}

#[cfg(test)]
mod power_up {
    use crate::setup::Features;
    use crate::{Config, Model, Terminal};

    #[test]
    fn saved_display_controls_apply_at_power_up() {
        let mut f = Features::factory(Model::Vt420);
        f.display_controls = true;
        let mut config = Config::default();
        config.set_saved_features(f);
        let mut term = Terminal::new(config);
        term.advance(b"\x1b[2J");
        assert_eq!(term.grid().line(0).cells()[0].ch, '\u{241B}');
    }
}
