//! Pasting and copying: translating between the desktop's Unicode text and
//! what the terminal can send and show.
//!
//! Pasted text is sent as if typed, through the character sets in use.
//! Characters those sets lack are replaced by the nearest ones they have —
//! typographic quotes and dashes by ASCII, accented letters without an
//! encoding by their base letter — and anything else by `?`, rather than
//! being lost silently.

use super::Terminal;
use crate::cell::Cell;
use crate::charset::{C1_CONTROL_PICTURES, SOFT_BASE, TECHNICAL_PUA};

/// Precomposed Latin letters: the letter, its base and combining mark.
const COMPOSED: &[(char, char, char)] = &{
    const GRAVE: char = '\u{300}';
    const ACUTE: char = '\u{301}';
    const CIRCUMFLEX: char = '\u{302}';
    const TILDE: char = '\u{303}';
    const MACRON: char = '\u{304}';
    const BREVE: char = '\u{306}';
    const DOT: char = '\u{307}';
    const DIAERESIS: char = '\u{308}';
    const RING: char = '\u{30A}';
    const DOUBLE_ACUTE: char = '\u{30B}';
    const CARON: char = '\u{30C}';
    const CEDILLA: char = '\u{327}';
    const OGONEK: char = '\u{328}';
    [
        ('À', 'A', GRAVE),
        ('È', 'E', GRAVE),
        ('Ì', 'I', GRAVE),
        ('Ò', 'O', GRAVE),
        ('Ù', 'U', GRAVE),
        ('à', 'a', GRAVE),
        ('è', 'e', GRAVE),
        ('ì', 'i', GRAVE),
        ('ò', 'o', GRAVE),
        ('ù', 'u', GRAVE),
        ('Á', 'A', ACUTE),
        ('É', 'E', ACUTE),
        ('Í', 'I', ACUTE),
        ('Ó', 'O', ACUTE),
        ('Ú', 'U', ACUTE),
        ('Ý', 'Y', ACUTE),
        ('á', 'a', ACUTE),
        ('é', 'e', ACUTE),
        ('í', 'i', ACUTE),
        ('ó', 'o', ACUTE),
        ('ú', 'u', ACUTE),
        ('ý', 'y', ACUTE),
        ('Ć', 'C', ACUTE),
        ('ć', 'c', ACUTE),
        ('Ĺ', 'L', ACUTE),
        ('ĺ', 'l', ACUTE),
        ('Ń', 'N', ACUTE),
        ('ń', 'n', ACUTE),
        ('Ŕ', 'R', ACUTE),
        ('ŕ', 'r', ACUTE),
        ('Ś', 'S', ACUTE),
        ('ś', 's', ACUTE),
        ('Ź', 'Z', ACUTE),
        ('ź', 'z', ACUTE),
        ('Â', 'A', CIRCUMFLEX),
        ('Ê', 'E', CIRCUMFLEX),
        ('Î', 'I', CIRCUMFLEX),
        ('Ô', 'O', CIRCUMFLEX),
        ('Û', 'U', CIRCUMFLEX),
        ('â', 'a', CIRCUMFLEX),
        ('ê', 'e', CIRCUMFLEX),
        ('î', 'i', CIRCUMFLEX),
        ('ô', 'o', CIRCUMFLEX),
        ('û', 'u', CIRCUMFLEX),
        ('Ĉ', 'C', CIRCUMFLEX),
        ('ĉ', 'c', CIRCUMFLEX),
        ('Ĝ', 'G', CIRCUMFLEX),
        ('ĝ', 'g', CIRCUMFLEX),
        ('Ĥ', 'H', CIRCUMFLEX),
        ('ĥ', 'h', CIRCUMFLEX),
        ('Ĵ', 'J', CIRCUMFLEX),
        ('ĵ', 'j', CIRCUMFLEX),
        ('Ŝ', 'S', CIRCUMFLEX),
        ('ŝ', 's', CIRCUMFLEX),
        ('Ŵ', 'W', CIRCUMFLEX),
        ('ŵ', 'w', CIRCUMFLEX),
        ('Ŷ', 'Y', CIRCUMFLEX),
        ('ŷ', 'y', CIRCUMFLEX),
        ('Ã', 'A', TILDE),
        ('Ñ', 'N', TILDE),
        ('Õ', 'O', TILDE),
        ('ã', 'a', TILDE),
        ('ñ', 'n', TILDE),
        ('õ', 'o', TILDE),
        ('Ĩ', 'I', TILDE),
        ('ĩ', 'i', TILDE),
        ('Ũ', 'U', TILDE),
        ('ũ', 'u', TILDE),
        ('Ā', 'A', MACRON),
        ('ā', 'a', MACRON),
        ('Ē', 'E', MACRON),
        ('ē', 'e', MACRON),
        ('Ī', 'I', MACRON),
        ('ī', 'i', MACRON),
        ('Ō', 'O', MACRON),
        ('ō', 'o', MACRON),
        ('Ū', 'U', MACRON),
        ('ū', 'u', MACRON),
        ('Ă', 'A', BREVE),
        ('ă', 'a', BREVE),
        ('Ğ', 'G', BREVE),
        ('ğ', 'g', BREVE),
        ('Ŭ', 'U', BREVE),
        ('ŭ', 'u', BREVE),
        ('Ċ', 'C', DOT),
        ('ċ', 'c', DOT),
        ('Ė', 'E', DOT),
        ('ė', 'e', DOT),
        ('Ġ', 'G', DOT),
        ('ġ', 'g', DOT),
        ('İ', 'I', DOT),
        ('Ż', 'Z', DOT),
        ('ż', 'z', DOT),
        ('Ä', 'A', DIAERESIS),
        ('Ë', 'E', DIAERESIS),
        ('Ï', 'I', DIAERESIS),
        ('Ö', 'O', DIAERESIS),
        ('Ü', 'U', DIAERESIS),
        ('Ÿ', 'Y', DIAERESIS),
        ('ä', 'a', DIAERESIS),
        ('ë', 'e', DIAERESIS),
        ('ï', 'i', DIAERESIS),
        ('ö', 'o', DIAERESIS),
        ('ü', 'u', DIAERESIS),
        ('ÿ', 'y', DIAERESIS),
        ('Å', 'A', RING),
        ('å', 'a', RING),
        ('Ů', 'U', RING),
        ('ů', 'u', RING),
        ('Ő', 'O', DOUBLE_ACUTE),
        ('ő', 'o', DOUBLE_ACUTE),
        ('Ű', 'U', DOUBLE_ACUTE),
        ('ű', 'u', DOUBLE_ACUTE),
        ('Č', 'C', CARON),
        ('č', 'c', CARON),
        ('Ď', 'D', CARON),
        ('ď', 'd', CARON),
        ('Ě', 'E', CARON),
        ('ě', 'e', CARON),
        ('Ň', 'N', CARON),
        ('ň', 'n', CARON),
        ('Ř', 'R', CARON),
        ('ř', 'r', CARON),
        ('Š', 'S', CARON),
        ('š', 's', CARON),
        ('Ť', 'T', CARON),
        ('ť', 't', CARON),
        ('Ž', 'Z', CARON),
        ('ž', 'z', CARON),
        ('Ç', 'C', CEDILLA),
        ('ç', 'c', CEDILLA),
        ('Ş', 'S', CEDILLA),
        ('ş', 's', CEDILLA),
        ('Ţ', 'T', CEDILLA),
        ('ţ', 't', CEDILLA),
        ('Ģ', 'G', CEDILLA),
        ('ģ', 'g', CEDILLA),
        ('Ķ', 'K', CEDILLA),
        ('ķ', 'k', CEDILLA),
        ('Ļ', 'L', CEDILLA),
        ('ļ', 'l', CEDILLA),
        ('Ņ', 'N', CEDILLA),
        ('ņ', 'n', CEDILLA),
        ('Ŗ', 'R', CEDILLA),
        ('ŗ', 'r', CEDILLA),
        ('Ą', 'A', OGONEK),
        ('ą', 'a', OGONEK),
        ('Ę', 'E', OGONEK),
        ('ę', 'e', OGONEK),
        ('Į', 'I', OGONEK),
        ('į', 'i', OGONEK),
        ('Ų', 'U', OGONEK),
        ('ų', 'u', OGONEK),
    ]
};

/// Stand-ins for characters with no encoding and no accent to drop.
fn substitute(ch: char) -> Option<&'static str> {
    Some(match ch {
        '‘' | '’' | '‚' | '‛' | '′' | 'ʼ' => "'",
        '“' | '”' | '„' | '‟' | '″' => "\"",
        '‐' | '‑' | '‒' | '–' | '—' | '―' | '−' => "-",
        '…' => "...",
        '•' | '∙' | '◦' | '‣' => "*",
        '‹' => "<",
        '›' => ">",
        '«' => "<<",
        '»' => ">>",
        '←' => "<-",
        '→' => "->",
        '⇐' => "<=",
        '⇒' => "=>",
        '≤' => "<=",
        '≥' => ">=",
        '≠' => "/=",
        '×' => "x",
        '÷' => "/",
        '€' => "EUR",
        '™' => "TM",
        '©' => "(C)",
        '®' => "(R)",
        '°' => "o",
        '\u{A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => " ",
        'Ł' => "L",
        'Đ' => "D",
        'Ø' => "O",
        'ł' => "l",
        'đ' => "d",
        'ø' => "o",
        'ı' => "i",
        'ß' => "ss",
        'Æ' => "AE",
        'æ' => "ae",
        'Œ' => "OE",
        'œ' => "oe",
        'Þ' => "Th",
        'þ' => "th",
        _ => return None,
    })
}

/// Characters that paste as nothing.
fn invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}'
    ) || ('\u{300}'..='\u{36F}').contains(&ch)
}

/// Joins letters and combining accents written separately.
fn compose(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(&mark) = chars.peek() {
            if let Some(&(composed, ..)) = COMPOSED.iter().find(|&&(_, b, m)| b == ch && m == mark)
            {
                chars.next();
                out.push(composed);
                continue;
            }
        }
        out.push(ch);
    }
    out
}

impl Terminal {
    /// Sends pasted text as typed. Line breaks become Return; tabs are kept
    /// and other control characters dropped, so pasted text cannot send
    /// control sequences. See the module notes for characters the host's
    /// character sets lack.
    pub fn paste(&mut self, text: &str) {
        if self.emu.modes.keyboard_locked {
            return;
        }
        let text = compose(&text.replace("\r\n", "\r").replace('\n', "\r"));
        let utf8 = self.emu.config.extensions.utf8;
        let mut bytes = Vec::new();
        for ch in text.chars() {
            match ch {
                '\r' | '\t' => bytes.push(ch as u8),
                _ if ch.is_control() || invisible(ch) => {}
                _ if utf8 => {
                    let mut buf = [0u8; 4];
                    bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                _ => {
                    if let Some(encoded) = self.encode_typed(ch) {
                        bytes.extend(encoded);
                    } else if let Some(stand_in) =
                        substitute(ch).map(str::to_string).or_else(|| {
                            COMPOSED
                                .iter()
                                .find(|&&(c, ..)| c == ch)
                                .map(|&(_, base, _)| base.to_string())
                        })
                    {
                        for c in stand_in.chars() {
                            bytes.extend(self.encode_typed(c).unwrap_or_else(|| vec![b'?']));
                        }
                    } else {
                        bytes.push(b'?');
                    }
                }
            }
        }
        self.transmit(&bytes);
    }
}

/// A screen cell as clipboard text: private-use characters veetee draws with
/// become their nearest Unicode characters.
pub(crate) fn copied_char(cell: &Cell) -> CopiedChar {
    let code = u32::from(cell.ch);
    if code >= SOFT_BASE {
        // A soft (DRCS) character: the code the host sent for it.
        let c = char::from(cell.code & 0x7F);
        return CopiedChar::One(if c.is_ascii_graphic() { c } else { '?' });
    }
    if (TECHNICAL_PUA..TECHNICAL_PUA + 0x20).contains(&code) {
        // DEC Technical summation and bracket pieces.
        return CopiedChar::One(match code - TECHNICAL_PUA {
            1 | 3 | 5 => '⎲',
            2 | 4 | 6 => '⎳',
            _ => '>',
        });
    }
    if (C1_CONTROL_PICTURES..C1_CONTROL_PICTURES + 0x21).contains(&code) {
        // Display Controls symbols for C1 controls: their names.
        const NAMES: [&str; 33] = [
            "PAD", "HOP", "BPH", "NBH", "IND", "NEL", "SSA", "ESA", "HTS", "HTJ", "VTS", "PLD",
            "PLU", "RI", "SS2", "SS3", "DCS", "PU1", "PU2", "STS", "CCH", "MW", "SPA", "EPA",
            "SOS", "SGC", "SCI", "CSI", "ST", "OSC", "PM", "APC", "NS",
        ];
        return CopiedChar::Name(NAMES[(code - C1_CONTROL_PICTURES) as usize]);
    }
    CopiedChar::One(if cell.ch == '\u{A0}' { ' ' } else { cell.ch })
}

pub(crate) enum CopiedChar {
    One(char),
    /// A C1 control shown by Display Controls, copied as `<NAME>`.
    Name(&'static str),
}

#[cfg(test)]
mod tests {
    use crate::{Config, Extensions, Model, Terminal};

    fn pasted(term: &mut Terminal, text: &str) -> Vec<u8> {
        term.paste(text);
        term.take_output()
    }

    #[test]
    fn quotes_dashes_and_missing_letters_are_replaced() {
        let mut t = Terminal::new(Config::default());
        assert_eq!(
            pasted(&mut t, "“Don’t” – ok… 5€"),
            b"\"Don't\" - ok... 5EUR"
        );
        // DEC Multinational has é but not ł or ő.
        assert_eq!(pasted(&mut t, "é ł ő 中"), b"\xe9 l o ?");
        // Decomposed accents are joined first.
        assert_eq!(pasted(&mut t, "e\u{301}"), b"\xe9");
    }

    #[test]
    fn line_breaks_become_return_and_controls_are_dropped() {
        let mut t = Terminal::new(Config::default());
        assert_eq!(
            pasted(&mut t, "$ DIR\r\n$ SHOW\n\x1b[2J\tx\u{200B}y"),
            b"$ DIR\r$ SHOW\r[2J\txy"
        );
    }

    #[test]
    fn a_vt100_sends_seven_bits() {
        let mut t = Terminal::new(Config {
            model: Model::Vt100,
            ..Config::default()
        });
        assert_eq!(pasted(&mut t, "café"), b"cafe");
    }

    #[test]
    fn national_mode_uses_the_national_set() {
        let mut t = Terminal::new(Config {
            keyboard_language: Some(crate::charset::Nrc::German),
            national_mode: true,
            ..Config::default()
        });
        assert_eq!(pasted(&mut t, "Grüße ł"), b"Gr}~e l");
    }

    #[test]
    fn utf8_extension_sends_unicode() {
        let mut t = Terminal::new(Config {
            extensions: Extensions {
                utf8: true,
                ..Extensions::default()
            },
            ..Config::default()
        });
        assert_eq!(pasted(&mut t, "ł “x”\n"), "ł “x”\r".as_bytes());
    }

    #[test]
    fn copied_text_uses_unicode_for_drawn_characters() {
        let mut t = Terminal::new(Config::default());
        // DEC Technical summation pieces, and Display Controls symbols.
        t.advance(b"\x1b*>\x1bn12\x1bo");
        let mut f = t.setup_features();
        f.display_controls = true;
        t.apply_setup_features(&f);
        t.advance(b"\x9b\x1b");
        let all = crate::Selection::new(
            crate::Point { row: 0, col: 0 },
            crate::Point { row: 0, col: 79 },
        );
        assert_eq!(t.selection_text(&all), "⎲⎳<CSI>\u{241B}");
    }
}
