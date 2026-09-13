//! The fonts a terminal family draws its screen sizes with.

use crate::{Font, VEETEE_10X10, VEETEE_VT420_6X16, VEETEE_VT420_10X16};

/// Terminal families with different character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// VT52, VT100, VT102 and VT220: 10×10 cells, each dot row drawn as two
    /// scan lines, on a 4:3 screen at 24 lines.
    Vt220,
    /// VT320, VT420 and the VT500 series: 10×16 and 6×16 cells at 24
    /// lines, 10 and 8 dots high at 36 and 48 lines, on an 800×400 raster
    /// of 1:1.4 pixels (EK-VT420-RM table 5-5, EK-VT520-RM table 7-1).
    // 🔎 The VT320's own 15×12 cell is not modelled; it uses the VT420 fonts.
    Vt420,
}

impl Family {
    /// Scan lines per dot row.
    pub const fn scan_lines_per_dot(self) -> u8 {
        match self {
            Family::Vt220 => 2,
            Family::Vt420 => 1,
        }
    }

    /// Height class of a screen showing `rows` lines (status line included):
    /// 0 for 24–26 lines, 1 for 36–42, 2 for 48 and more.
    pub fn size_class(self, rows: usize) -> usize {
        match self {
            Family::Vt220 => 0,
            Family::Vt420 => match rows {
                0..=27 => 0,
                28..=44 => 1,
                _ => 2,
            },
        }
    }

    /// Page width in line heights, which fixes the screen's proportions.
    /// A VT220 screen is 4:3 at 24 lines; the VT420 raster is 800 pixels
    /// wide with lines `cell height × 1.4` pixels tall.
    pub fn line_aspect(self, rows: usize) -> f32 {
        match self {
            Family::Vt220 => 32.0,
            Family::Vt420 => {
                let cell = [16.0, 10.0, 8.0][self.size_class(rows)];
                800.0 / (cell * 1.4)
            }
        }
    }
}

/// Every face of a family, indexed by [`FontSet::face_index`].
#[derive(Debug, Clone)]
pub struct FontSet {
    family: Family,
    faces: Vec<Font>,
}

impl FontSet {
    /// Parses and derives the faces of `family`.
    pub fn new(family: Family) -> FontSet {
        let parse = |source| Font::parse(source).expect("built-in font parses");
        let faces = match family {
            Family::Vt220 => {
                let wide = parse(VEETEE_10X10);
                let narrow = wide.derive(6, 10);
                vec![wide, narrow]
            }
            Family::Vt420 => {
                let wide_24 = parse(VEETEE_VT420_10X16);
                let mut narrow_24 = parse(VEETEE_VT420_6X16);
                narrow_24.fill_from(&wide_24);
                let mut wide_36 = parse(VEETEE_10X10);
                wide_36.fill_from(&wide_24);
                let mut narrow_36 = narrow_24.derive(6, 10);
                narrow_36.fill_from(&wide_36);
                let wide_48 = wide_36.derive(10, 8);
                let narrow_48 = narrow_36.derive(6, 8);
                vec![wide_24, narrow_24, wide_36, narrow_36, wide_48, narrow_48]
            }
        };
        FontSet { family, faces }
    }

    pub fn family(&self) -> Family {
        self.family
    }

    pub fn faces(&self) -> &[Font] {
        &self.faces
    }

    /// The face for a screen of `rows` lines in 80 or 132 columns.
    pub fn face_index(&self, columns_132: bool, rows: usize) -> usize {
        self.family.size_class(rows) * 2 + usize::from(columns_132)
    }

    pub fn face(&self, columns_132: bool, rows: usize) -> &Font {
        &self.faces[self.face_index(columns_132, rows)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_follow_dec_cell_sizes() {
        let set = FontSet::new(Family::Vt420);
        let size = |c, r| {
            let f = set.face(c, r);
            (f.width, f.height)
        };
        assert_eq!(size(false, 25), (10, 16));
        assert_eq!(size(true, 24), (6, 16));
        assert_eq!(size(false, 37), (10, 10));
        assert_eq!(size(true, 42), (6, 10));
        assert_eq!(size(false, 49), (10, 8));
        assert_eq!(size(true, 53), (6, 8));
        let vt220 = FontSet::new(Family::Vt220);
        assert_eq!(vt220.face(true, 24).width, 6);
        assert_eq!(vt220.face(false, 24).height, 10);
    }

    #[test]
    fn vt420_screen_proportions() {
        // 800 pixels across; a 16-dot line is 16 × 1.4 = 22.4 pixel widths tall.
        let aspect = Family::Vt420.line_aspect(25);
        assert!((aspect * 22.4 - 800.0).abs() < 0.01);
        assert_eq!(Family::Vt220.line_aspect(24), 32.0);
    }
}
