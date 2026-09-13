//! VT420 rectangular area operations (EK-VT420-RM chapter 9) and the
//! rectangle checksum (DECRQCRA).
//!
//! Coordinates are affected by origin mode but not limited by the margins;
//! values beyond the page are treated as the page size.

use vt_parser::Params;

use super::{Attrs, Cell, Color, Emulator, Flags};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    top: usize,
    left: usize,
    bottom: usize,
    right: usize,
}

impl Emulator {
    /// Reads `Pt;Pl;Pb;Pr` starting at parameter `i`.
    fn rect_at(&self, p: &Params, i: usize) -> Option<Rect> {
        let (rows, cols) = (self.rows(), self.cols());
        let (oy, ox) = if self.modes.origin {
            (self.top, self.left)
        } else {
            (0, 0)
        };
        let coord = |index: usize, offset: usize, default: usize, limit: usize| match p.get(index) {
            None | Some(0) => default,
            Some(v) => (usize::from(v) - 1 + offset).min(limit - 1),
        };
        let r = Rect {
            top: coord(i, oy, oy, rows),
            left: coord(i + 1, ox, ox, cols),
            bottom: coord(i + 2, oy, rows - 1, rows),
            right: coord(i + 3, ox, cols - 1, cols),
        };
        (r.top <= r.bottom && r.left <= r.right).then_some(r)
    }

    fn page_param(&self, p: &Params, i: usize) -> usize {
        usize::from(p.get_nonzero_or(i, 1)).min(self.page_count()) - 1
    }

    fn for_each_cell(&mut self, page: usize, r: Rect, mut f: impl FnMut(&mut Cell)) {
        let grid = self.page_grid_mut(page);
        for row in r.top..=r.bottom {
            for cell in &mut grid.line_mut(row).cells_mut()[r.left..=r.right] {
                f(cell);
            }
        }
    }

    /// DECCRA: copy a rectangle, possibly between pages. Cells keep their
    /// characters and attributes; lines keep their own line attributes.
    pub(super) fn copy_rectangle(&mut self, p: &Params) {
        let Some(src) = self.rect_at(p, 0) else {
            return;
        };
        let (from, to) = (self.page_param(p, 4), self.page_param(p, 7));
        let (oy, ox) = if self.modes.origin {
            (self.top, self.left)
        } else {
            (0, 0)
        };
        let dest_top = usize::from(p.get_nonzero_or(5, 1)) - 1 + oy;
        let dest_left = usize::from(p.get_nonzero_or(6, 1)) - 1 + ox;
        let copied: Vec<Vec<Cell>> = (src.top..=src.bottom)
            .map(|row| self.page_grid(from).line(row).cells()[src.left..=src.right].to_vec())
            .collect();
        let (rows, cols) = (self.rows(), self.cols());
        let grid = self.page_grid_mut(to);
        for (dy, line) in copied.iter().enumerate() {
            let row = dest_top + dy;
            if row >= rows {
                break;
            }
            let cells = grid.line_mut(row).cells_mut();
            for (dx, cell) in line.iter().enumerate() {
                let col = dest_left + dx;
                if col >= cols {
                    break;
                }
                cells[col] = *cell;
            }
        }
    }

    /// DECERA: erase characters and attributes.
    pub(super) fn erase_rectangle(&mut self, p: &Params) {
        if let Some(r) = self.rect_at(p, 0) {
            let page = self.page;
            self.for_each_cell(page, r, |c| *c = Cell::BLANK);
        }
    }

    /// DECSERA: erase unprotected characters, keeping visual attributes.
    pub(super) fn selective_erase_rectangle(&mut self, p: &Params) {
        if let Some(r) = self.rect_at(p, 0) {
            let page = self.page;
            self.for_each_cell(page, r, |c| {
                if !c.attrs.flags.contains(Flags::PROTECTED) {
                    c.ch = ' ';
                    c.code = 0;
                }
            });
        }
    }

    /// DECFRA: fill with a character from the in-use GL/GR table, using the
    /// current SGR rendition.
    pub(super) fn fill_rectangle(&mut self, p: &Params) {
        let Some(code) = p.get(0).and_then(|v| u8::try_from(v).ok()) else {
            return;
        };
        if !matches!(code, 32..=126 | 160..=255) {
            return;
        }
        let Some(r) = self.rect_at(p, 1) else { return };
        let mut sets = self.charsets;
        sets.single_shift = None;
        let Some(ch) = sets.translate(code) else {
            return;
        };
        let fill = Cell {
            ch,
            attrs: self.writing_attrs(),
            code,
        };
        let page = self.page;
        self.for_each_cell(page, r, |c| *c = fill);
    }

    /// DECCARA (`reverse` false) and DECRARA (`reverse` true).
    pub(super) fn change_rectangle_attributes(&mut self, p: &Params, reverse: bool) {
        let Some(r) = self.rect_at(p, 0) else { return };
        let mut ops: Vec<u16> = (4..p.len()).map(|i| p.get_or(i, 0)).collect();
        if ops.is_empty() {
            ops.push(0);
        }
        let colors = self.config.model.has_color() && !reverse;
        let apply = move |a: &mut Attrs| {
            for &op in &ops {
                let all = [Flags::BOLD, Flags::UNDERLINE, Flags::BLINK, Flags::REVERSE];
                let flag = match op {
                    1 | 22 => Some(Flags::BOLD),
                    4 | 24 => Some(Flags::UNDERLINE),
                    5 | 25 => Some(Flags::BLINK),
                    7 | 27 => Some(Flags::REVERSE),
                    _ => None,
                };
                match (reverse, op, flag) {
                    (false, 0, _) => all.iter().for_each(|f| a.flags.set(*f, false)),
                    (false, 1..=7, Some(f)) => a.flags.set(f, true),
                    (false, 22..=27, Some(f)) => a.flags.set(f, false),
                    (true, 0, _) => all
                        .iter()
                        .for_each(|f| a.flags.set(*f, !a.flags.contains(*f))),
                    (true, 1..=7, Some(f)) => a.flags.set(f, !a.flags.contains(f)),
                    (false, 30..=37, _) if colors => a.fg = Color::Indexed((op - 30) as u8),
                    (false, 39, _) if colors => a.fg = Color::Default,
                    (false, 40..=47, _) if colors => a.bg = Color::Indexed((op - 40) as u8),
                    (false, 49, _) if colors => a.bg = Color::Default,
                    _ => {}
                }
            }
        };
        let page = self.page;
        if self.sace_rectangle || r.top == r.bottom {
            self.for_each_cell(page, r, |c| apply(&mut c.attrs));
        } else {
            // Stream extent: from the first position to the second in reading order.
            let cols = self.cols();
            let grid = self.page_grid_mut(page);
            for row in r.top..=r.bottom {
                let from = if row == r.top { r.left } else { 0 };
                let to = if row == r.bottom { r.right } else { cols - 1 };
                for cell in &mut grid.line_mut(row).cells_mut()[from..=to] {
                    apply(&mut cell.attrs);
                }
            }
        }
    }

    /// DECSACE: 2 selects the rectangle, 0 and 1 the stream extent.
    pub(super) fn select_attribute_change_extent(&mut self, ps: u16) {
        self.sace_rectangle = ps == 2;
    }

    /// DECRQCRA, answered with DECCKSR.
    pub(super) fn request_checksum(&mut self, p: &Params) {
        let pid = p.get_or(0, 0);
        let pages: Vec<usize> = match p.get_or(1, 0) {
            0 if !(self.config.extensions.xterm_compat && p.len() > 2) => {
                (0..self.page_count()).collect()
            }
            0 => vec![self.page],
            n => vec![usize::from(n).min(self.page_count()) - 1],
        };
        let whole = Rect {
            top: 0,
            left: 0,
            bottom: self.rows() - 1,
            right: self.cols() - 1,
        };
        let rect = if p.len() > 2 {
            self.rect_at(p, 2)
        } else {
            Some(whole)
        };
        // xterm reports the plain sum of character codes.
        let xterm = self.config.extensions.xterm_compat;
        let mut sum: u32 = 0;
        if let Some(r) = rect {
            for page in pages {
                let grid = self.page_grid(page);
                for row in r.top..=r.bottom {
                    for cell in &grid.line(row).cells()[r.left..=r.right] {
                        let value = if xterm {
                            u32::from(cell.code)
                        } else {
                            cell.checksum()
                        };
                        sum = sum.wrapping_add(value);
                    }
                }
            }
        }
        let checksum = if xterm { sum } else { sum.wrapping_neg() } & 0xFFFF;
        self.reply_dcs(&format!("{pid}!~{checksum:04X}"));
    }
}
