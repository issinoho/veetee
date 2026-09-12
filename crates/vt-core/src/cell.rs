use core::ops::{BitOr, BitOrAssign};

/// Character rendition flags (SGR and DECSCA).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Flags(u16);

impl Flags {
    pub const NONE: Flags = Flags(0);
    pub const BOLD: Flags = Flags(1 << 0);
    pub const UNDERLINE: Flags = Flags(1 << 1);
    pub const BLINK: Flags = Flags(1 << 2);
    pub const REVERSE: Flags = Flags(1 << 3);
    /// SGR 8 (VT220 and later).
    pub const INVISIBLE: Flags = Flags(1 << 4);
    /// DECSCA: not erasable by DECSED/DECSEL/DECSERA.
    pub const PROTECTED: Flags = Flags(1 << 5);

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn set(&mut self, flag: Flags, on: bool) {
        if on {
            self.0 |= flag.0;
        } else {
            self.0 &= !flag.0;
        }
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for Flags {
    type Output = Flags;
    fn bitor(self, rhs: Flags) -> Flags {
        Flags(self.0 | rhs.0)
    }
}

impl BitOrAssign for Flags {
    fn bitor_assign(&mut self, rhs: Flags) {
        self.0 |= rhs.0;
    }
}

/// Foreground or background colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Color {
    /// The terminal's default (phosphor) colour.
    #[default]
    Default,
    /// Index into the colour table (VT525 / ANSI 0–15, xterm 16–255).
    Indexed(u8),
    /// Direct colour (xterm extension; never produced in DEC-strict mode).
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Attrs {
    pub flags: Flags,
    pub fg: Color,
    pub bg: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cell {
    /// Displayed character. DEC graphic sets are stored as their Unicode
    /// equivalents; soft-font (DRCS) glyphs use the Private Use Area.
    pub ch: char,
    pub attrs: Attrs,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        attrs: Attrs {
            flags: Flags::NONE,
            fg: Color::Default,
            bg: Color::Default,
        },
    };

    /// A cell cleared by an erase function. DEC erases to spaces with no
    /// rendition; on colour terminals the current background is kept.
    pub const fn erased(bg: Color) -> Cell {
        Cell {
            ch: ' ',
            attrs: Attrs {
                flags: Flags::NONE,
                fg: Color::Default,
                bg,
            },
        }
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}
