//! Downloadable soft character sets (DECDLD / DRCS).

use crate::charset::SOFT_BASE;

/// Number of font buffers (Pfn 1–2; Pfn 0 selects the first free one).
pub const BUFFERS: usize = 2;

/// Header parameters of a DECDLD string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DldParams {
    pub font_number: u16,
    pub start: u16,
    pub erase: u16,
    pub width: u16,
    pub font_set_size: u16,
    pub text_or_full_cell: u16,
    pub height: u16,
    pub is_96: bool,
}

/// A soft glyph: `height` rows of up to 16 dots, bit 0 leftmost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftGlyph {
    pub width: u8,
    pub height: u8,
    pub rows: Vec<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Rendition {
    glyphs: Vec<Option<SoftGlyph>>,
}

impl Rendition {
    fn new() -> Rendition {
        Rendition {
            glyphs: vec![None; 96],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Buffer {
    designation: Vec<u8>,
    is_96: bool,
    height: u8,
    /// Index 0: 80-column rendition; 1: 132-column rendition.
    renditions: [Rendition; 2],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SoftFonts {
    buffers: [Option<Buffer>; BUFFERS],
}

/// Default matrix size by conformance level: (80-column width, 132-column width, height).
// TODO(M4): confirm VT220/VT320 DRCS matrix sizes against EK-VT220-RM and EK-VT320-RM.
pub fn default_matrix(level: u8) -> (u8, u8, u8) {
    match level {
        0..=2 => (8, 5, 10),
        3 => (15, 9, 12),
        _ => (10, 6, 16),
    }
}

impl SoftFonts {
    pub fn clear(&mut self) {
        self.buffers = Default::default();
    }

    /// Finds the buffer designated by `designation` (intermediates + final).
    /// When two buffers share a designation the most recently loaded wins,
    /// which is always the higher-numbered slot we assign last.
    pub fn find(&self, designation: &[u8], is_96: bool) -> Option<u8> {
        self.buffers
            .iter()
            .enumerate()
            .rev()
            .find(|(_, b)| {
                b.as_ref()
                    .is_some_and(|b| b.designation == designation && b.is_96 == is_96)
            })
            .map(|(i, _)| i as u8)
    }

    pub fn designation(&self, slot: u8) -> Option<&[u8]> {
        self.buffers
            .get(usize::from(slot))?
            .as_ref()
            .map(|b| b.designation.as_slice())
    }

    /// The glyph for a soft character, in the rendition for the column mode.
    pub fn glyph(&self, ch: char, columns_132: bool) -> Option<&SoftGlyph> {
        let offset = u32::from(ch).checked_sub(SOFT_BASE)?;
        let (slot, code) = ((offset / 256) as usize, (offset % 256) as usize);
        let buffer = self.buffers.get(slot)?.as_ref()?;
        let index = code.checked_sub(0x20)?;
        buffer.renditions[usize::from(columns_132)]
            .glyphs
            .get(index)?
            .as_ref()
    }

    /// Loads a DECDLD string. `data` is everything after the final `{`.
    pub fn load(&mut self, params: DldParams, data: &[u8], level: u8) -> Result<(), &'static str> {
        let columns_132 = matches!(params.font_set_size, 2 | 12 | 22);
        if !matches!(params.font_set_size, 0 | 1 | 2 | 11 | 12 | 21 | 22) {
            return Err("unsupported font set size");
        }
        let (w80, w132, default_height) = default_matrix(level);
        let width = match params.width {
            0 => {
                if columns_132 {
                    w132
                } else {
                    w80
                }
            }
            w @ 2..=16 => w as u8,
            _ => return Err("bad matrix width"),
        };
        let height = match params.height {
            0 => default_height,
            h @ 1..=16 => h as u8,
            _ => return Err("bad matrix height"),
        };

        // Designator: up to two intermediates and a final.
        let inter = data
            .iter()
            .take_while(|b| (0x20..=0x2F).contains(*b))
            .count();
        if inter > 2 || data.len() <= inter || !(0x30..=0x7E).contains(&data[inter]) {
            return Err("bad designator");
        }
        let designation = data[..=inter].to_vec();
        let glyph_data = &data[inter + 1..];

        let slot = match params.font_number {
            0 => self.buffers.iter().position(Option::is_none).unwrap_or(0),
            n @ 1..=2 => usize::from(n - 1),
            _ => return Err("bad font number"),
        };
        let rendition = usize::from(columns_132);
        let buffer = &mut self.buffers[slot];
        let reshaped = buffer.as_ref().is_none_or(|b| {
            b.designation != designation || b.height != height || b.is_96 != params.is_96
        });
        if reshaped || params.erase == 2 {
            *buffer = Some(Buffer {
                designation,
                is_96: params.is_96,
                height,
                renditions: [Rendition::new(), Rendition::new()],
            });
        }
        let buffer = buffer.as_mut().expect("buffer initialised above");
        if params.erase == 0 {
            buffer.renditions[rendition] = Rendition::new();
        }

        for (n, glyph) in glyph_data.split(|&b| b == b';').enumerate() {
            let index = usize::from(params.start) + n;
            if index >= 96 {
                break;
            }
            let mut rows = vec![0u16; usize::from(height)];
            for (band, sixels) in glyph.split(|&b| b == b'/').enumerate() {
                for (x, &s) in sixels
                    .iter()
                    .filter(|b| (0x3F..=0x7E).contains(*b))
                    .enumerate()
                {
                    if x >= usize::from(width) {
                        break;
                    }
                    let bits = s - 0x3F;
                    for bit in 0..6 {
                        let y = band * 6 + bit;
                        if bits & (1 << bit) != 0 && y < rows.len() {
                            rows[y] |= 1 << x;
                        }
                    }
                }
            }
            buffer.renditions[rendition].glyphs[index] = Some(SoftGlyph {
                width,
                height,
                rows,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(start: u16) -> DldParams {
        DldParams {
            font_number: 1,
            start,
            erase: 0,
            width: 0,
            font_set_size: 0,
            text_or_full_cell: 0,
            height: 0,
            is_96: false,
        }
    }

    #[test]
    fn decodes_sixel_bands() {
        let mut fonts = SoftFonts::default();
        // One glyph: a vertical bar in column 0 spanning band 1 (rows 0-5) and
        // the top two rows of band 2; plus a dot at column 1 row 0.
        fonts.load(params(1), b" @~@/B", 4).unwrap();
        assert_eq!(fonts.find(b" @", false), Some(0));
        let ch = char::from_u32(SOFT_BASE + 0x21).unwrap();
        let g = fonts.glyph(ch, false).expect("loaded");
        assert_eq!((g.width, g.height), (10, 16));
        assert_eq!(&g.rows[..9], &[0b11, 1, 1, 1, 1, 1, 1, 1, 0]);
        assert!(
            fonts.glyph(ch, true).is_none(),
            "no 132-column rendition loaded"
        );
    }

    #[test]
    fn erase_modes() {
        let mut fonts = SoftFonts::default();
        fonts.load(params(1), b" @~;~", 4).unwrap();
        let second = char::from_u32(SOFT_BASE + 0x22).unwrap();
        fonts
            .load(
                DldParams {
                    erase: 1,
                    ..params(1)
                },
                b" @~",
                4,
            )
            .unwrap();
        assert!(
            fonts.glyph(second, false).is_some(),
            "Pe=1 keeps other glyphs"
        );
        fonts.load(params(1), b" @~", 4).unwrap();
        assert!(
            fonts.glyph(second, false).is_none(),
            "Pe=0 erases the rendition"
        );
    }

    #[test]
    fn rejects_bad_headers() {
        let mut fonts = SoftFonts::default();
        assert!(
            fonts
                .load(
                    DldParams {
                        font_set_size: 7,
                        ..params(1)
                    },
                    b" @~",
                    4
                )
                .is_err()
        );
        assert!(fonts.load(params(1), b"!!!@~", 4).is_err());
    }
}
