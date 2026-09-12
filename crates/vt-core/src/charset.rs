//! Graphic character sets and the ISO 2022 / DEC code extension state
//! (G0–G3, GL/GR invocation, single shifts).
//!
//! M1 covers the VT100 sets plus DEC Supplemental and ISO Latin-1. NRCS,
//! DEC Technical, DRCS and the VT510 sets arrive in M2.

/// A designatable graphic set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Charset {
    /// US ASCII (`B`).
    Ascii,
    /// United Kingdom NRCS (`A`): `#` is `£`.
    British,
    /// DEC Special Graphics (`0`), the VT100 line-drawing set.
    DecSpecialGraphics,
    /// DEC Supplemental Graphic (`<` or `%5`), the GR half of DEC Multinational.
    DecSupplemental,
    /// ISO Latin-1 Supplemental, a 96-character set (`- A`).
    IsoLatin1,
}

/// DEC Special Graphics for 0x5F–0x7E.
const SPECIAL_GRAPHICS: [char; 32] = [
    '\u{00A0}', // 5F blank
    '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼',
    '⎺', // 6F scan line 1
    '⎻', // 70 scan line 3
    '─', // 71 scan line 5
    '⎼', // 72 scan line 7
    '⎽', // 73 scan line 9
    '├', '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
];

impl Charset {
    /// 94-character sets have fixed SPACE and DEL positions; 96-character sets do not.
    pub const fn is_96(self) -> bool {
        matches!(self, Charset::IsoLatin1)
    }

    /// Maps a GL-relative code (0x20–0x7F) to its character.
    pub fn map(self, code: u8) -> Option<char> {
        debug_assert!((0x20..=0x7F).contains(&code));
        if !self.is_96() {
            match code {
                0x20 => return Some(' '),
                0x7F => return None,
                _ => {}
            }
        }
        Some(match self {
            Charset::Ascii => char::from(code),
            Charset::British => match code {
                b'#' => '£',
                _ => char::from(code),
            },
            Charset::DecSpecialGraphics => match code {
                0x5F..=0x7E => SPECIAL_GRAPHICS[usize::from(code - 0x5F)],
                _ => char::from(code),
            },
            Charset::DecSupplemental => match code | 0x80 {
                0xA8 => '¤',
                0xD7 => 'Œ',
                0xDD => 'Ÿ',
                0xF7 => 'œ',
                0xFD => 'ÿ',
                // Reserved positions.
                0xA4 | 0xA6 | 0xAC..=0xAF | 0xB4 | 0xB8 | 0xBE | 0xD0 | 0xDE | 0xF0 | 0xFE => {
                    char::REPLACEMENT_CHARACTER
                }
                c => char::from(c),
            },
            Charset::IsoLatin1 => char::from(code | 0x80),
        })
    }

    /// Resolves an SCS final (with optional second intermediate) for a 94-character set.
    pub fn from_94(intermediate: Option<u8>, final_byte: u8) -> Option<Charset> {
        Some(match (intermediate, final_byte) {
            (None, b'B') => Charset::Ascii,
            (None, b'A') => Charset::British,
            (None, b'0') => Charset::DecSpecialGraphics,
            // VT100 alternate character ROM: standard and special graphics.
            (None, b'1') => Charset::Ascii,
            (None, b'2') => Charset::DecSpecialGraphics,
            (None, b'<') | (Some(b'%'), b'5') => Charset::DecSupplemental,
            _ => return None,
        })
    }

    /// Resolves an SCS final for a 96-character set.
    pub fn from_96(intermediate: Option<u8>, final_byte: u8) -> Option<Charset> {
        match (intermediate, final_byte) {
            (None, b'A') => Some(Charset::IsoLatin1),
            _ => None,
        }
    }
}

/// Encodes a typed character as an 8-bit DEC Multinational (ASCII + DEC
/// Supplemental) code, the VT220+ factory keyboard encoding.
pub fn encode_dec_multinational(ch: char) -> Option<u8> {
    if ch.is_ascii() {
        return Some(ch as u8);
    }
    (0xA0..=0xFF).find(|&b| {
        Charset::DecSupplemental.map(b & 0x7F) == Some(ch) && ch != char::REPLACEMENT_CHARACTER
    })
}

/// Which G-set a byte is taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CharsetState {
    pub g: [Charset; 4],
    /// Index of the G-set invoked into GL (0x20–0x7F).
    pub gl: u8,
    /// Index of the G-set invoked into GR (0xA0–0xFF).
    pub gr: u8,
    /// Pending SS2 (2) or SS3 (3).
    pub single_shift: Option<u8>,
}

impl CharsetState {
    /// VT100 power-up state: ASCII everywhere, G0 in GL.
    pub const VT100: CharsetState = CharsetState {
        g: [Charset::Ascii; 4],
        gl: 0,
        gr: 2,
        single_shift: None,
    };

    /// VT220+ power-up state: ASCII in G0/G1, DEC Supplemental (the factory
    /// user-preferred supplemental set) in G2/G3, G0 in GL and G2 in GR.
    pub const VT220: CharsetState = CharsetState {
        g: [
            Charset::Ascii,
            Charset::Ascii,
            Charset::DecSupplemental,
            Charset::DecSupplemental,
        ],
        gl: 0,
        gr: 2,
        single_shift: None,
    };

    /// Translates a received graphic byte, consuming any single shift.
    pub fn translate(&mut self, byte: u8) -> Option<char> {
        let set = match self.single_shift.take() {
            Some(g) => self.g[usize::from(g)],
            None if byte < 0x80 => self.g[usize::from(self.gl)],
            None => self.g[usize::from(self.gr)],
        };
        set.map(byte & 0x7F)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_graphics_line_drawing() {
        let s = Charset::DecSpecialGraphics;
        let box_chars: String = b"lqkxmj".iter().map(|&b| s.map(b).unwrap()).collect();
        assert_eq!(box_chars, "┌─┐│└┘");
        assert_eq!(s.map(b'A'), Some('A'));
        assert_eq!(s.map(0x7F), None);
    }

    #[test]
    fn ninety_six_sets_use_space_and_del_positions() {
        assert_eq!(Charset::IsoLatin1.map(0x20), Some('\u{A0}'));
        assert_eq!(Charset::IsoLatin1.map(0x7F), Some('ÿ'));
    }

    #[test]
    fn single_shift_applies_once() {
        let mut st = CharsetState::VT220;
        st.g[3] = Charset::DecSpecialGraphics;
        st.single_shift = Some(3);
        assert_eq!(st.translate(b'q'), Some('─'));
        assert_eq!(st.translate(b'q'), Some('q'));
    }

    #[test]
    fn dec_multinational_keyboard_encoding() {
        assert_eq!(encode_dec_multinational('é'), Some(0xE9));
        assert_eq!(encode_dec_multinational('Œ'), Some(0xD7));
        assert_eq!(encode_dec_multinational('¤'), Some(0xA8));
        assert_eq!(encode_dec_multinational('×'), None);
        assert_eq!(encode_dec_multinational('─'), None);
    }

    #[test]
    fn gr_uses_g2_supplemental() {
        let mut st = CharsetState::VT220;
        assert_eq!(st.translate(0xD7), Some('Œ'));
        assert_eq!(st.translate(0xE9), Some('é'));
    }
}
