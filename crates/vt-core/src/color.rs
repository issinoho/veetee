//! VT525 colour: the 16-entry colour map, normal text and window frame
//! colours (DECAC), alternate text colours (DECATC), the colour mode
//! (DECSTGLT), and how a cell's renditions select its colours
//! (EK-VT520-RM section 2.9 and chapter 5).

use crate::cell::{Attrs, Color, Flags};

/// DECSTGLT colour look-up table selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// A monochrome or grey-level map for applications not designed for colour.
    Mono,
    /// Renditions select colours from the alternate text colour table.
    Alternate,
    /// The second alternate colour selection, which DEC documents without detail.
    Alternate2,
    /// ANSI SGR colours (factory default).
    #[default]
    Sgr,
}

impl ColorMode {
    pub const fn code(self) -> u16 {
        match self {
            ColorMode::Mono => 0,
            ColorMode::Alternate => 1,
            ColorMode::Alternate2 => 2,
            ColorMode::Sgr => 3,
        }
    }

    pub const fn from_code(code: u16) -> Option<ColorMode> {
        Some(match code {
            0 => ColorMode::Mono,
            1 => ColorMode::Alternate,
            2 => ColorMode::Alternate2,
            3 => ColorMode::Sgr,
            _ => return None,
        })
    }
}

/// An RGB colour with components 0–100, as DECCTR reports them.
pub type Rgb100 = [u8; 3];

/// 🔎 The factory colour map is not tabulated in EK-VT520-RM. Entries 0–7 are
/// the ANSI colours in SGR order and 8–15 their bright forms ("bold adds 8").
pub const DEFAULT_MAP: [Rgb100; 16] = [
    [0, 0, 0],
    [67, 0, 0],
    [0, 67, 0],
    [67, 67, 0],
    [0, 0, 67],
    [67, 0, 67],
    [0, 67, 67],
    [67, 67, 67],
    [33, 33, 33],
    [100, 33, 33],
    [33, 100, 33],
    [100, 100, 33],
    [33, 33, 100],
    [100, 33, 100],
    [33, 100, 100],
    [100, 100, 100],
];

/// Attribute combinations in DECATC order: bold 1, reverse 2, underline 4, blink 8
/// map to Ps1 0–15.
const ATC_ORDER: [u8; 16] = [
    0b0000, 0b0001, 0b0010, 0b0100, 0b1000, 0b0011, 0b0101, 0b1001, 0b0110, 0b1010, 0b1100, 0b0111,
    0b1011, 0b1101, 0b1110, 0b1111,
];

/// 🔎 Factory alternate text colours are not tabulated either: underline
/// selects cyan, blink yellow, both magenta; bold brightens; reverse
/// exchanges foreground and background.
fn default_alternate() -> [(u8, u8); 16] {
    let mut table = [(7, 0); 16];
    for (ps1, bits) in ATC_ORDER.iter().enumerate() {
        let (bold, reverse, underline, blink) =
            (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);
        let mut color = match (underline, blink) {
            (false, false) => 7,
            (true, false) => 6,
            (false, true) => 3,
            (true, true) => 5,
        };
        if bold {
            color += 8;
        }
        table[ps1] = if reverse { (0, color) } else { (color, 0) };
    }
    table
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorTable {
    pub map: [Rgb100; 16],
    /// DECAC item 1: normal text foreground and background indexes.
    pub normal: (u8, u8),
    /// DECAC item 2: window frame foreground and background indexes.
    pub frame: (u8, u8),
    /// DECATC colours by Ps1.
    pub alternate: [(u8, u8); 16],
    pub mode: ColorMode,
}

impl Default for ColorTable {
    fn default() -> Self {
        ColorTable {
            map: DEFAULT_MAP,
            normal: (7, 0),
            frame: (7, 0),
            alternate: default_alternate(),
            mode: ColorMode::Sgr,
        }
    }
}

/// Mode settings that change how renditions are coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorOptions {
    /// DECBBSM: bold and blink affect the background too.
    pub bold_blink_background: bool,
    /// DECATCUM: underlined text is also underlined in alternate colour mode.
    pub alternate_underline: bool,
    /// DECATCBM: blinking text also blinks in alternate colour mode.
    pub alternate_blink: bool,
}

/// How a cell is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellColors {
    pub fg: Rgb100,
    pub bg: Rgb100,
    pub underline: bool,
    /// The glyph is not drawn (invisible, or the off phase of blinking).
    pub hidden: bool,
}

impl ColorTable {
    /// Position of an attribute combination in the DECATC table.
    pub fn alternate_index(flags: Flags) -> usize {
        let bits = u8::from(flags.contains(Flags::BOLD))
            | u8::from(flags.contains(Flags::REVERSE)) << 1
            | u8::from(flags.contains(Flags::UNDERLINE)) << 2
            | u8::from(flags.contains(Flags::BLINK)) << 3;
        ATC_ORDER.iter().position(|b| *b == bits).unwrap_or(0)
    }

    /// The attributes a character is written with. The colour mode in effect
    /// when a character is written decides its colours (vttest DECATC test):
    /// alternate mode stores the DECATC colours in the cell.
    pub fn attrs_for_writing(&self, attrs: Attrs) -> Attrs {
        let mut a = attrs;
        match self.mode {
            ColorMode::Sgr => {}
            ColorMode::Alternate | ColorMode::Alternate2 => {
                let (fg, bg) = self.alternate[Self::alternate_index(a.flags)];
                a.fg = Color::Indexed(fg);
                a.bg = Color::Indexed(bg);
                a.flags.set(Flags::COLOR_ALTERNATE, true);
            }
            ColorMode::Mono => a.flags.set(Flags::COLOR_MONO, true),
        }
        a
    }

    /// Colours for a cell. `blink_on` is the visible phase of the blink cycle.
    pub fn resolve(&self, attrs: Attrs, options: ColorOptions, blink_on: bool) -> CellColors {
        let f = attrs.flags;
        let (bold, blink) = (f.contains(Flags::BOLD), f.contains(Flags::BLINK));
        let index = |c: Color, default: u8| match c {
            Color::Indexed(i) if i < 16 => i,
            _ => default,
        };
        let mode = if f.contains(Flags::COLOR_ALTERNATE) {
            ColorMode::Alternate
        } else if f.contains(Flags::COLOR_MONO) {
            ColorMode::Mono
        } else {
            ColorMode::Sgr
        };
        let (mut fg, mut bg, underline, blinks) = match mode {
            ColorMode::Sgr => {
                let mut fg = index(attrs.fg, self.normal.0);
                let mut bg = index(attrs.bg, self.normal.1);
                if bold {
                    fg |= 8;
                    if options.bold_blink_background {
                        bg |= 8;
                    }
                }
                let (mut fg, mut bg) = (self.map[usize::from(fg)], self.map[usize::from(bg)]);
                if f.contains(Flags::REVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                (fg, bg, f.contains(Flags::UNDERLINE), blink)
            }
            ColorMode::Alternate | ColorMode::Alternate2 => {
                let (fg, bg) = (
                    index(attrs.fg, self.normal.0),
                    index(attrs.bg, self.normal.1),
                );
                (
                    self.map[usize::from(fg)],
                    self.map[usize::from(bg)],
                    f.contains(Flags::UNDERLINE) && options.alternate_underline,
                    blink && options.alternate_blink,
                )
            }
            ColorMode::Mono => {
                let (dim, bright) = ([67; 3], [100; 3]);
                let fg = if bold { bright } else { dim };
                let (fg, bg) = if f.contains(Flags::REVERSE) {
                    ([0; 3], fg)
                } else {
                    (fg, [0; 3])
                };
                (fg, bg, f.contains(Flags::UNDERLINE), blink)
            }
        };
        // SGR blink cycles to a dimmer, less saturated shade (EK-VT520-RM 2.9.6).
        if blinks && !blink_on {
            fg = dimmer(fg);
            if options.bold_blink_background {
                bg = dimmer(bg);
            }
        }
        let hidden = f.contains(Flags::INVISIBLE);
        if hidden {
            fg = bg;
        }
        CellColors {
            fg,
            bg,
            underline,
            hidden,
        }
    }
}

fn dimmer(c: Rgb100) -> Rgb100 {
    let grey = (u16::from(c[0]) + u16::from(c[1]) + u16::from(c[2])) / 3;
    c.map(|v| ((u16::from(v) + grey) * 3 / 10) as u8)
}

/// RGB (0–100) to DEC HLS: hue 0–360 with blue at 0°, red at 120° and green
/// at 240°; lightness and saturation 0–100.
pub fn rgb_to_hls(rgb: Rgb100) -> (u16, u8, u8) {
    let [r, g, b] = rgb.map(|v| f64::from(v) / 100.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0, (l * 100.0).round() as u8, 0);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    let dec_hue = (h.round() as u16 + 120) % 360;
    (
        dec_hue,
        (l * 100.0).round() as u8,
        (s * 100.0).round() as u8,
    )
}

/// DEC HLS back to RGB (0–100).
pub fn hls_to_rgb(hue: u16, lightness: u8, saturation: u8) -> Rgb100 {
    let h = f64::from((hue % 360 + 240) % 360) / 360.0;
    let (l, s) = (
        f64::from(lightness.min(100)) / 100.0,
        f64::from(saturation.min(100)) / 100.0,
    );
    if s == 0.0 {
        let v = (l * 100.0).round() as u8;
        return [v; 3];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 100.0).round() as u8
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dec_hls_puts_blue_at_zero_degrees() {
        assert_eq!(rgb_to_hls([0, 0, 100]).0, 0);
        assert_eq!(rgb_to_hls([100, 0, 0]).0, 120);
        assert_eq!(rgb_to_hls([0, 100, 0]).0, 240);
        for rgb in DEFAULT_MAP {
            let (h, l, s) = rgb_to_hls(rgb);
            let back = hls_to_rgb(h, l, s);
            for i in 0..3 {
                assert!(back[i].abs_diff(rgb[i]) <= 1, "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn default_alternate_table_order() {
        let t = default_alternate();
        assert_eq!(t[0], (7, 0));
        assert_eq!(t[1], (15, 0));
        assert_eq!(t[2], (0, 7));
        assert_eq!(t[3], (6, 0));
        assert_eq!(t[15], (0, 13));
        let bold_underline = Flags::BOLD | Flags::UNDERLINE;
        assert_eq!(ColorTable::alternate_index(bold_underline), 6);
    }

    #[test]
    fn sgr_mode_bold_brightens_and_reverse_swaps() {
        let table = ColorTable::default();
        let mut attrs = Attrs {
            fg: Color::Indexed(1),
            ..Attrs::default()
        };
        attrs.flags = Flags::BOLD | Flags::REVERSE;
        let c = table.resolve(attrs, ColorOptions::default(), true);
        assert_eq!(c.bg, DEFAULT_MAP[9]);
        assert_eq!(c.fg, DEFAULT_MAP[0]);
    }
}
