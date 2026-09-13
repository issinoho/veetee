//! Graphic character sets and the ISO 2022 / DEC code extension state
//! (G0–G3, GL/GR invocation, single shifts).
//!
//! Sources: VT220 Programmer Reference tables 2-5 to 2-15 (NRC sets), VT510
//! Programmer Information chapter 5 (SCS designators) and the DEC Technical
//! character set chart at vt100.net.

/// A national replacement character set (7-bit, 94 characters).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Nrc {
    British,
    Dutch,
    Finnish,
    French,
    FrenchCanadian,
    German,
    Italian,
    NorwegianDanish,
    Portuguese,
    Spanish,
    Swedish,
    Swiss,
}

/// ASCII positions replaced by NRC sets.
const NRC_POSITIONS: [u8; 12] = *b"#@[\\]^_`{|}~";

impl Nrc {
    /// Replacement characters for [`NRC_POSITIONS`].
    fn table(self) -> [char; 12] {
        use Nrc::*;
        match self {
            British => ['£', '@', '[', '\\', ']', '^', '_', '`', '{', '|', '}', '~'],
            // vt100.net's transcription shows £ at 7/13; DEC documentation
            // elsewhere (and Kermit, xterm) give ¼ at 7/13 and ´ at 7/14.
            Dutch => ['£', '¾', 'ĳ', '½', '|', '^', '_', '`', '¨', 'ƒ', '¼', '´'],
            Finnish => ['#', '@', 'Ä', 'Ö', 'Å', 'Ü', '_', 'é', 'ä', 'ö', 'å', 'ü'],
            French => ['£', 'à', '°', 'ç', '§', '^', '_', '`', 'é', 'ù', 'è', '¨'],
            FrenchCanadian => ['#', 'à', 'â', 'ç', 'ê', 'î', '_', 'ô', 'é', 'ù', 'è', 'û'],
            German => ['#', '§', 'Ä', 'Ö', 'Ü', '^', '_', '`', 'ä', 'ö', 'ü', 'ß'],
            Italian => ['£', '§', '°', 'ç', 'é', '^', '_', 'ù', 'à', 'ò', 'è', 'ì'],
            NorwegianDanish => ['#', 'Ä', 'Æ', 'Ø', 'Å', 'Ü', '_', 'ä', 'æ', 'ø', 'å', 'ü'],
            Portuguese => ['#', '@', 'Ã', 'Ç', 'Õ', '^', '_', '`', 'ã', 'ç', 'õ', '~'],
            Spanish => ['£', '§', '¡', 'Ñ', '¿', '^', '_', '`', '°', 'ñ', 'ç', '~'],
            Swedish => ['#', 'É', 'Ä', 'Ö', 'Å', 'Ü', '_', 'é', 'ä', 'ö', 'å', 'ü'],
            Swiss => ['ù', 'à', 'é', 'ç', 'ê', 'î', 'è', 'ô', 'ä', 'ö', 'ü', 'û'],
        }
    }

    fn map(self, code: u8) -> char {
        match NRC_POSITIONS.iter().position(|&p| p == code) {
            Some(i) => self.table()[i],
            None => char::from(code),
        }
    }

    /// The 7-bit code that displays `ch` in this set, if any.
    pub fn encode(self, ch: char) -> Option<u8> {
        if let Some(i) = self.table().iter().position(|&c| c == ch) {
            return Some(NRC_POSITIONS[i]);
        }
        (ch.is_ascii_graphic() || ch == ' ')
            .then_some(ch as u8)
            .filter(|b| !NRC_POSITIONS.contains(b) || self.map(*b) == ch)
    }
}

/// A designatable graphic set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Charset {
    /// US ASCII (`B`).
    Ascii,
    /// A national replacement set; `A` (British) is also a VT100 set.
    National(Nrc),
    /// DEC Special Graphics (`0`), the VT100 line-drawing set.
    DecSpecialGraphics,
    /// DEC Supplemental Graphic (`%5`; `<` on the VT220), the GR half of DEC Multinational.
    DecSupplemental,
    /// DEC Technical (`>`), VT320 and later.
    DecTechnical,
    /// ISO Latin-1 Supplemental, a 96-character set (`- A`).
    IsoLatin1,
    /// A downloaded soft set (DECDLD), by font slot.
    Soft { slot: u8, is_96: bool },
    /// A VT510/VT520 national or supplemental set.
    Vt500(Vt500Set),
}

/// Character sets added by the VT510 and VT520 (EK-VT520-RM table 5-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Vt500Set {
    DecGreek,
    DecHebrew,
    DecTurkish,
    DecCyrillic,
    IsoLatin2,
    IsoGreek,
    IsoHebrew,
    IsoCyrillic,
    IsoLatin5,
    /// National replacement sets, available only in NRC mode.
    NrcGreek,
    NrcHebrew,
    NrcTurkish,
    NrcSerboCroatian,
    NrcRussian,
}

impl Vt500Set {
    fn table(self) -> &'static [Option<char>; 96] {
        use crate::charset_tables::*;
        match self {
            Vt500Set::DecGreek => &DEC_GREEK,
            Vt500Set::DecHebrew => &DEC_HEBREW,
            Vt500Set::DecTurkish => &DEC_TURKISH,
            Vt500Set::DecCyrillic => &DEC_CYRILLIC,
            Vt500Set::IsoLatin2 => &ISO_LATIN_2,
            Vt500Set::IsoGreek => &ISO_GREEK,
            Vt500Set::IsoHebrew => &ISO_HEBREW,
            Vt500Set::IsoCyrillic => &ISO_CYRILLIC,
            Vt500Set::IsoLatin5 => &ISO_LATIN_5,
            Vt500Set::NrcGreek => &NRC_GREEK,
            Vt500Set::NrcHebrew => &NRC_HEBREW,
            Vt500Set::NrcTurkish => &NRC_TURKISH,
            Vt500Set::NrcSerboCroatian => &NRC_SERBO_CROATIAN,
            Vt500Set::NrcRussian => &NRC_RUSSIAN,
        }
    }

    pub const fn is_96(self) -> bool {
        matches!(
            self,
            Vt500Set::IsoLatin2
                | Vt500Set::IsoGreek
                | Vt500Set::IsoHebrew
                | Vt500Set::IsoCyrillic
                | Vt500Set::IsoLatin5
        )
    }

    pub const fn is_national(self) -> bool {
        matches!(
            self,
            Vt500Set::NrcGreek
                | Vt500Set::NrcHebrew
                | Vt500Set::NrcTurkish
                | Vt500Set::NrcSerboCroatian
                | Vt500Set::NrcRussian
        )
    }

    /// The set selected by an SCS designator.
    pub fn from_designator(
        is_96: bool,
        intermediate: Option<u8>,
        final_byte: u8,
    ) -> Option<Vt500Set> {
        use Vt500Set::*;
        Some(match (is_96, intermediate, final_byte) {
            (false, Some(b'"'), b'?') => DecGreek,
            (false, Some(b'"'), b'4') => DecHebrew,
            (false, Some(b'%'), b'0') => DecTurkish,
            (false, Some(b'&'), b'4') => DecCyrillic,
            (false, Some(b'"'), b'>') => NrcGreek,
            (false, Some(b'%'), b'=') => NrcHebrew,
            (false, Some(b'%'), b'2') => NrcTurkish,
            (false, Some(b'%'), b'3') => NrcSerboCroatian,
            (false, Some(b'&'), b'5') => NrcRussian,
            (true, None, b'B') => IsoLatin2,
            (true, None, b'F') => IsoGreek,
            (true, None, b'H') => IsoHebrew,
            (true, None, b'L') => IsoCyrillic,
            (true, None, b'M') => IsoLatin5,
            _ => return None,
        })
    }

    pub const fn designator(self) -> &'static str {
        use Vt500Set::*;
        match self {
            DecGreek => "\"?",
            DecHebrew => "\"4",
            DecTurkish => "%0",
            DecCyrillic => "&4",
            IsoLatin2 => "B",
            IsoGreek => "F",
            IsoHebrew => "H",
            IsoCyrillic => "L",
            IsoLatin5 => "M",
            NrcGreek => "\">",
            NrcHebrew => "%=",
            NrcTurkish => "%2",
            NrcSerboCroatian => "%3",
            NrcRussian => "&5",
        }
    }
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

/// First Private Use code point for DEC Technical pieces with no Unicode
/// equivalent (the large sigma parts at 3/1–3/7). The font draws these.
pub const TECHNICAL_PUA: u32 = 0xF7E0;

/// DEC Technical for 0x21–0x7E. `None` marks positions DEC left undefined.
const TECHNICAL: [Option<char>; 94] = {
    const U: Option<char> = None;
    const fn p(n: u32) -> Option<char> {
        char::from_u32(TECHNICAL_PUA + n)
    }
    [
        // 2/1 – 2/15
        Some('⎷'),
        Some('┌'),
        Some('─'),
        Some('⌠'),
        Some('⌡'),
        Some('│'),
        Some('⎡'),
        Some('⎣'),
        Some('⎤'),
        Some('⎦'),
        Some('⎛'),
        Some('⎝'),
        Some('⎞'),
        Some('⎠'),
        Some('⎨'),
        // 3/0 – 3/15
        Some('⎬'),
        p(1),
        p(2),
        p(3),
        p(4),
        p(5),
        p(6),
        p(7),
        U,
        U,
        U,
        U,
        Some('≤'),
        Some('≠'),
        Some('≥'),
        Some('∫'),
        // 4/0 – 4/15
        Some('∴'),
        Some('∝'),
        Some('∞'),
        Some('÷'),
        Some('Δ'),
        Some('∇'),
        Some('Φ'),
        Some('Γ'),
        Some('∼'),
        Some('≃'),
        Some('Θ'),
        Some('×'),
        Some('Λ'),
        Some('⇔'),
        Some('⇒'),
        Some('≡'),
        // 5/0 – 5/15
        Some('Π'),
        Some('Ψ'),
        U,
        Some('Σ'),
        U,
        U,
        Some('√'),
        Some('Ω'),
        Some('Ξ'),
        Some('Υ'),
        Some('⊂'),
        Some('⊃'),
        Some('∩'),
        Some('∪'),
        Some('∧'),
        Some('∨'),
        // 6/0 – 6/15
        Some('¬'),
        Some('α'),
        Some('β'),
        Some('χ'),
        Some('δ'),
        Some('ε'),
        Some('φ'),
        Some('γ'),
        Some('η'),
        Some('ι'),
        Some('θ'),
        Some('κ'),
        Some('λ'),
        U,
        Some('ν'),
        Some('∂'),
        // 7/0 – 7/14
        Some('π'),
        Some('ψ'),
        Some('ρ'),
        Some('σ'),
        Some('τ'),
        U,
        Some('ƒ'),
        Some('ω'),
        Some('ξ'),
        Some('υ'),
        Some('ζ'),
        Some('←'),
        Some('↑'),
        Some('→'),
        Some('↓'),
    ]
};

/// Character DEC terminals show for undefined positions and unloaded soft characters.
pub const ERROR_CHARACTER: char = '\u{2426}';

/// First code point used for soft (DRCS) characters: plane 16 private use,
/// `SOFT_BASE + slot * 256 + code`.
pub const SOFT_BASE: u32 = 0x10_0000;

impl Charset {
    /// 94-character sets have fixed SPACE and DEL positions; 96-character sets do not.
    pub const fn is_96(self) -> bool {
        match self {
            Charset::IsoLatin1 => true,
            Charset::Soft { is_96, .. } => is_96,
            Charset::Vt500(set) => set.is_96(),
            _ => false,
        }
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
            Charset::National(nrc) => nrc.map(code),
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
                    ERROR_CHARACTER
                }
                c => char::from(c),
            },
            Charset::DecTechnical => TECHNICAL[usize::from(code - 0x21)].unwrap_or(ERROR_CHARACTER),
            Charset::IsoLatin1 => char::from(code | 0x80),
            Charset::Vt500(set) => set.table()[usize::from(code - 0x20)].unwrap_or(ERROR_CHARACTER),
            Charset::Soft { slot, .. } => {
                char::from_u32(SOFT_BASE + u32::from(slot) * 256 + u32::from(code))
                    .unwrap_or(ERROR_CHARACTER)
            }
        })
    }

    /// The NRC set for a 94-character SCS designator, per the VT510 table.
    pub fn national(intermediate: Option<u8>, final_byte: u8) -> Option<Nrc> {
        use Nrc::*;
        Some(match (intermediate, final_byte) {
            (None, b'A') => British,
            (None, b'4') => Dutch,
            (None, b'C' | b'5') => Finnish,
            (None, b'R' | b'f') => French,
            (None, b'Q' | b'9') => FrenchCanadian,
            (None, b'K') => German,
            (None, b'Y') => Italian,
            (None, b'`' | b'E' | b'6') => NorwegianDanish,
            (Some(b'%'), b'6') => Portuguese,
            (None, b'Z') => Spanish,
            (None, b'H' | b'7') => Swedish,
            (None, b'=') => Swiss,
            _ => return None,
        })
    }

    /// The SCS designator (intermediate and final) that selects this set.
    pub fn designator(self) -> &'static str {
        use Nrc::*;
        match self {
            Charset::Ascii => "B",
            Charset::DecSpecialGraphics => "0",
            Charset::DecSupplemental => "%5",
            Charset::DecTechnical => ">",
            Charset::IsoLatin1 => "A",
            Charset::Soft { .. } => " @",
            Charset::Vt500(set) => set.designator(),
            Charset::National(n) => match n {
                British => "A",
                Dutch => "4",
                Finnish => "C",
                French => "R",
                FrenchCanadian => "Q",
                German => "K",
                Italian => "Y",
                NorwegianDanish => "E",
                Portuguese => "%6",
                Spanish => "Z",
                Swedish => "H",
                Swiss => "=",
            },
        }
    }
}

/// Encodes a typed character as an 8-bit DEC Multinational (ASCII + DEC
/// Supplemental) code, the VT220+ factory keyboard encoding.
pub fn encode_dec_multinational(ch: char) -> Option<u8> {
    if ch.is_ascii() {
        return Some(ch as u8);
    }
    if ch == ERROR_CHARACTER {
        return None;
    }
    (0xA0..=0xFF).find(|&b| Charset::DecSupplemental.map(b & 0x7F) == Some(ch))
}

/// Encodes a typed character as an ISO Latin-1 code.
pub fn encode_latin1(ch: char) -> Option<u8> {
    u8::try_from(u32::from(ch))
        .ok()
        .filter(|b| *b < 0x80 || *b >= 0xA0)
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

    /// VT220+ power-up state: ASCII in G0/G1, the user-preferred supplemental
    /// set in G2/G3, G0 in GL and G2 in GR.
    pub const fn vt220(upss: Charset) -> CharsetState {
        CharsetState {
            g: [Charset::Ascii, Charset::Ascii, upss, upss],
            gl: 0,
            gr: 2,
            single_shift: None,
        }
    }

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
        let mut st = CharsetState::vt220(Charset::DecSupplemental);
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
        let mut st = CharsetState::vt220(Charset::DecSupplemental);
        assert_eq!(st.translate(0xD7), Some('Œ'));
        assert_eq!(st.translate(0xE9), Some('é'));
    }

    #[test]
    fn national_sets_follow_dec_tables() {
        let german = Charset::National(Nrc::German);
        let s: String = b"[\\]{|}~@"
            .iter()
            .map(|&b| german.map(b).unwrap())
            .collect();
        assert_eq!(s, "ÄÖÜäöüß§");
        let swiss = Charset::National(Nrc::Swiss);
        assert_eq!(swiss.map(b'#'), Some('ù'));
        assert_eq!(Charset::national(None, b'7'), Some(Nrc::Swedish));
        assert_eq!(Charset::national(Some(b'%'), b'6'), Some(Nrc::Portuguese));
    }

    #[test]
    fn national_keyboard_encoding() {
        assert_eq!(Nrc::German.encode('ß'), Some(b'~'));
        assert_eq!(Nrc::German.encode('a'), Some(b'a'));
        assert_eq!(Nrc::German.encode('['), None, "replaced by Ä in this set");
        assert_eq!(Nrc::British.encode('£'), Some(b'#'));
    }

    #[test]
    fn dec_technical() {
        let t = Charset::DecTechnical;
        assert_eq!(t.map(0x44), Some('Δ'));
        assert_eq!(t.map(0x7B), Some('←'));
        assert_eq!(t.map(0x6D), Some(ERROR_CHARACTER), "undefined");
        assert_eq!(t.map(0x31), char::from_u32(TECHNICAL_PUA + 1));
    }

    #[test]
    fn soft_sets_map_to_private_use() {
        let s = Charset::Soft {
            slot: 1,
            is_96: false,
        };
        assert_eq!(s.map(0x21), char::from_u32(SOFT_BASE + 256 + 0x21));
    }
}
