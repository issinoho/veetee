//! The display controls font: the symbols Set-Up's Display Controls (CRM)
//! shows for control characters. On a 24-line screen each symbol is the
//! control's name in three small letters stepping down the cell; on 36- and
//! 48-line screens it is two letters (EK-VT420-RM section 2, "Display
//! Controls Mode", table 2-5). The symbols are generated from a 3×5 dot
//! alphabet for every cell size.

use crate::{Font, Glyph};

/// Where the C1 control symbols (0x80–0x9F) and NS (0xA0) are, in the
/// Private Use Area. C0 controls use Unicode's Control Pictures
/// (U+2400–U+241F) and DEL U+2421. vt-core uses the same code points.
pub const C1_CONTROL_PICTURES: u32 = 0xF780;

/// Names of 0x00–0x1F, DEL, 0x80–0x9F and 0xA0: three letters, then the
/// two-letter form. Unnamed C1 positions show their ISO 6429 names, or
/// their code in two letters.
const NAMES: [(&str, &str); 66] = [
    ("NUL", "NL"),
    ("SOH", "SH"),
    ("STX", "SX"),
    ("ETX", "EX"),
    ("EOT", "ET"),
    ("ENQ", "EN"),
    ("ACK", "AK"),
    ("BEL", "BL"),
    ("BS", "BS"),
    ("HT", "HT"),
    ("LF", "LF"),
    ("VT", "VT"),
    ("FF", "FF"),
    ("CR", "CR"),
    ("SO", "SO"),
    ("SI", "SI"),
    ("DLE", "DE"),
    ("DC1", "D1"),
    ("DC2", "D2"),
    ("DC3", "D3"),
    ("DC4", "D4"),
    ("NAK", "NK"),
    ("SYN", "SY"),
    ("ETB", "EB"),
    ("CAN", "CA"),
    ("EM", "EM"),
    ("SUB", "SB"),
    ("ESC", "EC"),
    ("FS", "FS"),
    ("GS", "GS"),
    ("RS", "RS"),
    ("US", "US"),
    ("DEL", "DL"),
    ("PAD", "80"),
    ("HOP", "81"),
    ("BPH", "82"),
    ("NBH", "83"),
    ("IND", "IN"),
    ("NEL", "NE"),
    ("SSA", "SA"),
    ("ESA", "EA"),
    ("HTS", "HS"),
    ("HTJ", "HJ"),
    ("VTS", "VS"),
    ("PLD", "PD"),
    ("PLU", "PU"),
    ("RI", "RI"),
    ("SS2", "S2"),
    ("SS3", "S3"),
    ("DCS", "DC"),
    ("PU1", "P1"),
    ("PU2", "P2"),
    ("STS", "SS"),
    ("CCH", "CC"),
    ("MW", "MW"),
    ("SPA", "SP"),
    ("EPA", "EP"),
    ("SOS", "98"),
    ("SGC", "99"),
    ("SCI", "9A"),
    ("CSI", "CS"),
    ("ST", "ST"),
    ("OSC", "OS"),
    ("PM", "PM"),
    ("APC", "AP"),
    ("NS", "NS"),
];

/// The character each entry of [`NAMES`] is drawn for.
fn symbol_char(i: usize) -> char {
    let code = match i {
        0..=31 => 0x2400 + i as u32,
        32 => 0x2421,
        _ => C1_CONTROL_PICTURES + (i - 33) as u32,
    };
    char::from_u32(code).expect("valid code point")
}

/// A 3×5 dot letter, rows top to bottom, bit 2 = leftmost dot.
fn letter(ch: char) -> [u8; 5] {
    let rows: &str = match ch {
        'A' => ".#.|#.#|###|#.#|#.#",
        'B' => "##.|#.#|##.|#.#|##.",
        'C' => ".##|#..|#..|#..|.##",
        'D' => "##.|#.#|#.#|#.#|##.",
        'E' => "###|#..|##.|#..|###",
        'F' => "###|#..|##.|#..|#..",
        'G' => ".##|#..|#.#|#.#|.##",
        'H' => "#.#|#.#|###|#.#|#.#",
        'I' => "###|.#.|.#.|.#.|###",
        'J' => "..#|..#|..#|#.#|.#.",
        'K' => "#.#|#.#|##.|#.#|#.#",
        'L' => "#..|#..|#..|#..|###",
        'M' => "#.#|###|###|#.#|#.#",
        'N' => "##.|#.#|#.#|#.#|#.#",
        'O' => ".#.|#.#|#.#|#.#|.#.",
        'P' => "##.|#.#|##.|#..|#..",
        'Q' => ".#.|#.#|#.#|##.|.##",
        'R' => "##.|#.#|##.|#.#|#.#",
        'S' => ".##|#..|.#.|..#|##.",
        'T' => "###|.#.|.#.|.#.|.#.",
        'U' => "#.#|#.#|#.#|#.#|###",
        'V' => "#.#|#.#|#.#|.#.|.#.",
        'W' => "#.#|#.#|###|###|#.#",
        'X' => "#.#|#.#|.#.|#.#|#.#",
        'Y' => "#.#|#.#|.#.|.#.|.#.",
        'Z' => "###|..#|.#.|#..|###",
        '0' => "###|#.#|#.#|#.#|###",
        '1' => ".#.|##.|.#.|.#.|###",
        '2' => "##.|..#|.#.|#..|###",
        '3' => "##.|..#|.#.|..#|##.",
        '4' => "#.#|#.#|###|..#|..#",
        '8' => "###|#.#|###|#.#|###",
        '9' => "###|#.#|###|..#|###",
        _ => "...|...|...|...|...",
    };
    let mut out = [0; 5];
    for (row, text) in out.iter_mut().zip(rows.split('|')) {
        *row = text
            .bytes()
            .fold(0, |acc, b| (acc << 1) | u8::from(b == b'#'));
    }
    out
}

/// Draws `name` in a `width`×`height` cell: letters stepping down and to the
/// right, or side by side when the cell is too short to step. In a cell too
/// narrow to step right, the letters zigzag.
fn draw(name: &str, width: usize, height: usize) -> Vec<u16> {
    let mut rows = vec![0u16; height];
    let n = name.chars().count();
    let stepped = height >= 5 * n;
    let across = if !stepped && 3 + 4 * (n - 1) <= width {
        4
    } else {
        3
    };
    let span = 3 + across * (n - 1);
    let top = height.saturating_sub(if stepped { 5 * n } else { 5 }) / 2;
    for (i, ch) in name.chars().enumerate() {
        let x0 = if span <= width {
            (width - span) / 2 + i * across
        } else if i % 2 == 0 {
            0
        } else {
            width - 3
        };
        let y0 = if stepped { top + 5 * i } else { top };
        for (dy, bits) in letter(ch).iter().enumerate() {
            for dx in 0..3 {
                if bits & (4 >> dx) != 0 && y0 + dy < height {
                    rows[y0 + dy] |= 1 << (x0 + dx);
                }
            }
        }
    }
    rows
}

impl Font {
    /// Adds the display controls symbols the font lacks, drawn for its cell.
    pub fn add_control_pictures(&mut self) {
        let (width, height) = (usize::from(self.width), usize::from(self.height));
        for (i, (three, two)) in NAMES.iter().enumerate() {
            let name = if height >= 15 { three } else { two };
            let glyph = Glyph {
                ch: symbol_char(i),
                rows: draw(name, width, height),
            };
            // Hand-drawn symbols (DEC Special Graphics has HT, FF, CR, LF
            // and VT) are kept.
            if !self.index.contains_key(&glyph.ch) {
                self.index.insert(glyph.ch, self.glyphs.len() as u16);
                self.glyphs.push(glyph);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Family, FontSet};

    fn picture(font: &Font, ch: char) -> String {
        let g = &font.glyphs()[usize::from(font.index_of(ch))];
        (0..usize::from(font.height))
            .map(|y| {
                (0..usize::from(font.width))
                    .map(|x| if g.dot(x, y) { '#' } else { '.' })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_face_has_every_symbol() {
        for family in [Family::Vt220, Family::Vt420] {
            let set = FontSet::new(family);
            for face in set.faces() {
                for i in 0..NAMES.len() {
                    let ch = symbol_char(i);
                    let g = &face.glyphs()[usize::from(face.index_of(ch))];
                    assert_eq!(g.ch, ch, "{family:?} {}x{}", face.width, face.height);
                    let dots: u32 = g.rows.iter().map(|r| r.count_ones()).sum();
                    assert!(
                        dots > 8,
                        "{ch:?} is blank at {}x{}",
                        face.width,
                        face.height
                    );
                    assert!(g.rows.iter().all(|r| r >> face.width == 0));
                }
            }
        }
    }

    #[test]
    fn symbols_step_down_the_cell() {
        let set = FontSet::new(Family::Vt420);
        assert_eq!(
            picture(set.face(false, 24), '\u{241B}'),
            "###.......\n\
             #.........\n\
             ##........\n\
             #.........\n\
             ###.......\n\
             ....##....\n\
             ...#......\n\
             ....#.....\n\
             .....#....\n\
             ...##.....\n\
             .......##.\n\
             ......#...\n\
             ......#...\n\
             ......#...\n\
             .......##.\n\
             .........."
        );
        // Two letters side by side in a 132-column cell on a 48-line screen.
        assert_eq!(
            picture(
                set.face(true, 48),
                char::from_u32(C1_CONTROL_PICTURES + 0x1B).unwrap()
            ),
            "......\n\
             .##.##\n\
             #..#..\n\
             #...#.\n\
             #....#\n\
             .####.\n\
             ......\n\
             ......"
        );
    }
}
