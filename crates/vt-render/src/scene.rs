use vt_core::Terminal;
use vt_core::cell::{Color, Flags};
use vt_core::grid::LineSize;
use vt_fonts::Font;

use crate::theme::Theme;

/// Floats per instance: rect (4), glyph, flags, foreground (3), background (3).
pub const INSTANCE_LEN: usize = 12;

pub mod flag {
    pub const UNDERLINE: u32 = 1;
    pub const DOUBLE_TOP: u32 = 2;
    pub const DOUBLE_BOTTOM: u32 = 4;
    pub const HIDE_GLYPH: u32 = 8;
    pub const CURSOR: u32 = 16;
    pub const FILL: u32 = 32;
    pub const CURSOR_OUTLINE: u32 = 64;
    pub const SELECTED: u32 = 128;
}

/// Where the page sits in the window, in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rows: usize,
    pub cols: usize,
    pub cell_width: f32,
    pub cell_height: f32,
}

impl Layout {
    /// Left edge of a column, snapped to whole pixels so adjacent cells never leave gaps.
    pub fn col_x(&self, col: usize) -> f32 {
        (self.x + col as f32 * self.cell_width).round()
    }

    pub fn row_y(&self, row: usize) -> f32 {
        self.y + row as f32 * self.cell_height
    }

    /// The cell under a window position, if any.
    pub fn cell_at(&self, px: f32, py: f32) -> Option<(usize, usize)> {
        let (dx, dy) = (px - self.x, py - self.y);
        if dx < 0.0 || dy < 0.0 || dx >= self.width || dy >= self.height {
            return None;
        }
        Some((
            (dy / self.cell_height) as usize,
            (dx / self.cell_width) as usize,
        ))
    }
}

/// Fits the page into a window. A VT screen is 4:3 at 24 lines; other page
/// lengths keep the same width and line height. Line height is a whole
/// number of pixels so scan lines stay even.
pub fn layout(width: u32, height: u32, rows: usize, cols: usize) -> Layout {
    let (w, h) = (width as f32, height as f32);
    let rows_f = rows.max(1) as f32;
    // Page width is always 32 line heights (4:3 at 24 lines).
    let cell_height = (h / rows_f).min(w / 32.0).floor().max(1.0);
    let page_height = cell_height * rows_f;
    let page_width = cell_height * 32.0;
    Layout {
        x: ((w - page_width) / 2.0).floor().max(0.0),
        y: ((h - page_height) / 2.0).floor().max(0.0),
        width: page_width,
        height: page_height,
        rows,
        cols,
        cell_width: page_width / cols.max(1) as f32,
        cell_height,
    }
}

/// Per-frame display state not held by the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameState {
    /// Blinking characters are in their visible phase.
    pub blink_on: bool,
    /// The blinking cursor is in its visible phase.
    pub cursor_on: bool,
    /// The window has keyboard focus (an unfocused cursor is drawn hollow).
    pub focused: bool,
    /// Text selected with the mouse, drawn in reverse.
    pub selection: Option<vt_core::Selection>,
}

/// Builds the instance buffer for one frame: a page fill followed by every
/// cell that is not blank on the page background.
pub fn build_instances(
    term: &Terminal,
    layout: &Layout,
    frame: FrameState,
    theme: &Theme,
    font: &Font,
    out: &mut Vec<f32>,
) {
    out.clear();
    let reverse_screen = term.modes().reverse_screen;
    let normal = scale(theme.foreground, theme.normal_intensity);
    let (page_bg, text_normal, text_bold) = if reverse_screen {
        (normal, theme.background, theme.background)
    } else {
        (theme.background, normal, theme.foreground)
    };

    push(
        out,
        [layout.x, layout.y, layout.width, layout.height],
        0,
        flag::FILL,
        page_bg,
        page_bg,
    );

    let grid = term.grid();
    let cursor = term.cursor();
    let space = font.index_of(' ');
    for (row, line) in grid.lines().iter().enumerate().take(layout.rows) {
        let (mult, size_flag) = match line.size {
            LineSize::Single => (1, 0),
            LineSize::DoubleWidth => (2, 0),
            LineSize::DoubleHeightTop => (2, flag::DOUBLE_TOP),
            LineSize::DoubleHeightBottom => (2, flag::DOUBLE_BOTTOM),
        };
        let y = layout.row_y(row);
        for (col, cell) in line.cells().iter().enumerate().take(line.width()) {
            let a = cell.attrs;
            let bold = a.flags.contains(Flags::BOLD);
            let mut fg = match a.fg {
                Color::Default if bold => text_bold,
                Color::Default => text_normal,
                c => theme.color(c, text_normal),
            };
            let mut bg = theme.color(a.bg, page_bg);
            if a.flags.contains(Flags::REVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }

            let mut flags = size_flag;
            let hidden = a.flags.contains(Flags::INVISIBLE)
                || (a.flags.contains(Flags::BLINK) && !frame.blink_on);
            if hidden {
                flags |= flag::HIDE_GLYPH;
            } else if a.flags.contains(Flags::UNDERLINE) {
                flags |= flag::UNDERLINE;
            }
            let at_cursor = term.modes().cursor_visible && row == cursor.row && col == cursor.col;
            if at_cursor {
                if !frame.focused {
                    flags |= flag::CURSOR_OUTLINE;
                } else if frame.cursor_on {
                    flags |= flag::CURSOR;
                }
            }

            if frame.selection.is_some_and(|s| s.contains(row, col)) {
                flags |= flag::SELECTED;
            }

            let glyph = font.index_of(cell.ch);
            let decorated = flags
                & (flag::UNDERLINE | flag::CURSOR | flag::CURSOR_OUTLINE | flag::SELECTED)
                != 0;
            let blank = (glyph == space || hidden) && bg == page_bg && !decorated;
            if blank {
                continue;
            }
            let x0 = layout.col_x(col * mult);
            let x1 = layout.col_x((col + 1) * mult);
            push(
                out,
                [x0, y, x1 - x0, layout.cell_height],
                glyph,
                flags,
                fg,
                bg,
            );
        }
    }
}

fn scale(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

fn push(out: &mut Vec<f32>, rect: [f32; 4], glyph: u16, flags: u32, fg: [f32; 3], bg: [f32; 3]) {
    out.extend_from_slice(&rect);
    out.push(f32::from(glyph));
    out.push(flags as f32);
    out.extend_from_slice(&fg);
    out.extend_from_slice(&bg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_core::{Config, Model};

    fn font() -> Font {
        Font::parse(vt_fonts::VEETEE_10X10).unwrap()
    }

    const FOCUSED: FrameState = FrameState {
        selection: None,
        blink_on: true,
        cursor_on: true,
        focused: true,
    };

    fn instances(term: &Terminal, frame: FrameState) -> Vec<[f32; INSTANCE_LEN]> {
        let lay = layout(1280, 1024, term.grid().rows(), term.grid().cols());
        let mut out = Vec::new();
        build_instances(term, &lay, frame, &Theme::default(), &font(), &mut out);
        out.chunks(INSTANCE_LEN)
            .map(|c| c.try_into().unwrap())
            .collect()
    }

    #[test]
    fn page_is_four_by_three_and_centred() {
        let l = layout(1280, 1024, 24, 80);
        assert_eq!((l.cell_height, l.width, l.height), (40.0, 1280.0, 960.0));
        assert_eq!((l.x, l.y), (0.0, 32.0));
        assert_eq!(l.cell_width, 16.0);
        let wide = layout(2560, 1024, 24, 132);
        assert_eq!(wide.height, 1008.0);
        assert_eq!(wide.width, 42.0 * 32.0);
        assert!((wide.cell_width - wide.width / 132.0).abs() < 1e-4);
        assert_eq!(wide.cell_at(wide.x + 1.0, wide.y + 1.0), Some((0, 0)));
        assert_eq!(wide.cell_at(0.0, 0.0), None);
    }

    #[test]
    fn blank_page_draws_only_fill_and_cursor() {
        let term = Terminal::new(Config::default());
        let inst = instances(&term, FOCUSED);
        assert_eq!(inst.len(), 2);
        assert_eq!(inst[0][5] as u32, flag::FILL);
        assert_eq!(inst[1][5] as u32, flag::CURSOR);
    }

    #[test]
    fn double_width_cells_are_twice_as_wide() {
        let mut term = Terminal::new(Config {
            model: Model::Vt102,
            ..Config::default()
        });
        term.advance(b"\x1b#6AB\x1b[2;1H\x1b#3C");
        let inst = instances(
            &term,
            FrameState {
                cursor_on: false,
                ..FOCUSED
            },
        );
        let inst = &inst[1..];
        let a = inst.iter().find(|i| i[1] == 32.0 && i[0] == 0.0).unwrap();
        assert_eq!(a[2], 32.0);
        let b = inst.iter().find(|i| i[1] == 32.0 && i[0] == 32.0).unwrap();
        assert_eq!(b[2], 32.0);
        let c = inst.iter().find(|i| i[1] == 72.0).unwrap();
        assert_eq!(c[5] as u32, flag::DOUBLE_TOP);
    }

    #[test]
    fn blink_and_reverse() {
        let mut term = Terminal::new(Config::default());
        term.advance(b"\x1b[5mA\x1b[0;7m \x1b[H");
        let on = instances(
            &term,
            FrameState {
                cursor_on: false,
                ..FOCUSED
            },
        );
        let off = instances(
            &term,
            FrameState {
                cursor_on: false,
                blink_on: false,
                ..FOCUSED
            },
        );
        assert_eq!(on.len(), 3, "fill, blinking A, reverse space");
        assert_eq!(off.len(), 2, "blinking A disappears in its off phase");
        let theme = Theme::default();
        let reverse = on[2];
        assert_eq!(
            &reverse[9..12],
            &theme.foreground.map(|c| c * theme.normal_intensity)
        );
    }

    #[test]
    fn selected_cells_are_flagged_including_blanks() {
        use vt_core::{Point, Selection};
        let mut term = Terminal::new(Config::default());
        term.advance(b"ab");
        let selection = Some(Selection::new(
            Point { row: 0, col: 0 },
            Point { row: 0, col: 3 },
        ));
        let inst = instances(
            &term,
            FrameState {
                selection,
                cursor_on: false,
                ..FOCUSED
            },
        );
        let selected = inst
            .iter()
            .filter(|i| i[5] as u32 & flag::SELECTED != 0)
            .count();
        assert_eq!(selected, 4);
    }

    #[test]
    fn unfocused_cursor_is_hollow() {
        let term = Terminal::new(Config::default());
        let inst = instances(
            &term,
            FrameState {
                focused: false,
                ..FOCUSED
            },
        );
        assert_eq!(inst[1][5] as u32, flag::CURSOR_OUTLINE);
    }
}
