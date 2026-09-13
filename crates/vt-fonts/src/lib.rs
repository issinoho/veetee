//! Bitmap fonts for veetee.
//!
//! Fonts are plain-text dot matrices (see `fonts/*.vtfont`) so they can be
//! edited by hand and reviewed in diffs. They are parsed once at startup.

use std::collections::HashMap;

/// The default 80/132-column font, drawn on a 10×10 dot cell.
pub const VEETEE_10X10: &str = include_str!("../fonts/veetee-10x10.vtfont");

/// A parsed bitmap font. Glyph rows are bit masks, bit 0 = leftmost dot.
#[derive(Debug, Clone)]
pub struct Font {
    pub width: u8,
    pub height: u8,
    glyphs: Vec<Glyph>,
    index: HashMap<char, u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    pub ch: char,
    pub rows: Vec<u16>,
}

impl Glyph {
    pub fn dot(&self, x: usize, y: usize) -> bool {
        self.rows.get(y).is_some_and(|r| r & (1 << x) != 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

impl Font {
    /// Parses a `.vtfont` source. The first glyph is the fallback for
    /// characters the font lacks unless U+FFFD is present.
    pub fn parse(source: &str) -> Result<Font, ParseError> {
        let err = |line: usize, message: String| ParseError {
            line: line + 1,
            message,
        };
        // Blank lines are ignored everywhere; `#` comments only between glyphs,
        // since glyph rows themselves start with `#` or `.`.
        let mut lines = source
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty());
        fn next_directive<'a>(
            lines: &mut impl Iterator<Item = (usize, &'a str)>,
        ) -> Option<(usize, &'a str)> {
            lines.find(|(_, l)| !l.starts_with('#'))
        }

        let (n, header) = next_directive(&mut lines).ok_or_else(|| err(0, "empty font".into()))?;
        let dims: Vec<u8> = header
            .strip_prefix("cell ")
            .ok_or_else(|| err(n, "expected `cell W H`".into()))?
            .split_whitespace()
            .map(|v| {
                v.parse()
                    .map_err(|_| err(n, format!("bad dimension {v:?}")))
            })
            .collect::<Result<_, _>>()?;
        let [width, height] = dims[..] else {
            return Err(err(n, "expected `cell W H`".into()));
        };
        if width == 0 || width > 16 || height == 0 {
            return Err(err(
                n,
                "cell width must be 1–16 and height at least 1".into(),
            ));
        }

        let mut font = Font {
            width,
            height,
            glyphs: Vec::new(),
            index: HashMap::new(),
        };
        while let Some((n, line)) = next_directive(&mut lines) {
            let code = line
                .split_whitespace()
                .next()
                .and_then(|w| w.strip_prefix("U+"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .and_then(char::from_u32)
                .ok_or_else(|| err(n, format!("expected `U+XXXX`, found {line:?}")))?;
            let mut rows = Vec::with_capacity(usize::from(height));
            for _ in 0..height {
                let (n, row) = lines
                    .next()
                    .ok_or_else(|| err(n, format!("glyph U+{:04X} is truncated", code as u32)))?;
                if row.chars().count() != usize::from(width) {
                    return Err(err(n, format!("row must be {width} dots wide")));
                }
                let mut bits = 0u16;
                for (x, c) in row.chars().enumerate() {
                    match c {
                        '#' => bits |= 1 << x,
                        '.' => {}
                        _ => return Err(err(n, format!("unexpected {c:?} in glyph row"))),
                    }
                }
                rows.push(bits);
            }
            if font.index.insert(code, font.glyphs.len() as u16).is_some() {
                return Err(err(n, format!("duplicate glyph U+{:04X}", code as u32)));
            }
            font.glyphs.push(Glyph { ch: code, rows });
        }
        if font.glyphs.is_empty() {
            return Err(err(0, "font has no glyphs".into()));
        }
        Ok(font)
    }

    pub fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }

    /// Index of the glyph for `ch`, or of the fallback glyph.
    pub fn index_of(&self, ch: char) -> u16 {
        self.index
            .get(&ch)
            .or_else(|| self.index.get(&char::REPLACEMENT_CHARACTER))
            .copied()
            .unwrap_or(0)
    }

    pub fn contains(&self, ch: char) -> bool {
        self.index.contains_key(&ch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin() -> Font {
        Font::parse(VEETEE_10X10).expect("built-in font parses")
    }

    #[test]
    fn covers_every_character_the_emulator_can_produce() {
        use vt_charset_coverage::*;
        let font = builtin();
        let missing: Vec<String> = required()
            .filter(|c| !font.contains(*c))
            .map(|c| format!("U+{:04X}", c as u32))
            .collect();
        assert!(missing.is_empty(), "missing glyphs: {missing:?}");
    }

    /// Characters produced by the VT100/VT220 graphic sets and error glyphs.
    mod vt_charset_coverage {
        pub fn required() -> impl Iterator<Item = char> {
            let ascii = (0x20u32..0x7F).filter_map(char::from_u32);
            let latin1 = (0xA0u32..=0xFF).filter_map(char::from_u32);
            let dec = "◆▒␉␌␍␊␤␋┘┐┌└┼⎺⎻─⎼⎽├┤┴┬│≤≥π≠£·ŒœŸ⸮\u{FFFD}".chars();
            ascii.chain(latin1).chain(dec)
        }
    }

    #[test]
    fn box_drawing_meets_at_the_cell_centre() {
        let font = builtin();
        let cross = &font.glyphs()[usize::from(font.index_of('┼'))];
        assert!((0..10).all(|y| cross.dot(4, y)));
        assert!((0..10).all(|x| cross.dot(x, 4)));
        let h = &font.glyphs()[usize::from(font.index_of('─'))];
        assert!(
            h.dot(0, 4) && h.dot(9, 4),
            "horizontal lines join neighbouring cells"
        );
    }

    #[test]
    fn unknown_characters_use_the_replacement_glyph() {
        let font = builtin();
        assert_eq!(font.index_of('漢'), font.index_of('\u{FFFD}'));
    }

    #[test]
    fn parse_errors_are_located() {
        let e = Font::parse("cell 3 2\nU+0041\n#.#\n##\n").unwrap_err();
        assert_eq!(e.line, 4);
        assert!(Font::parse("cell 3 1\nU+0041\n#.#\nU+0041\n###\n").is_err());
    }
}
