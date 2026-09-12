use crate::cell::Cell;

/// DEC line attribute (DECSWL, DECDWL, DECDHL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum LineSize {
    #[default]
    Single,
    DoubleWidth,
    DoubleHeightTop,
    DoubleHeightBottom,
}

impl LineSize {
    pub const fn is_double_width(self) -> bool {
        !matches!(self, LineSize::Single)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    cells: Vec<Cell>,
    pub size: LineSize,
    /// The line was ended by autowrap rather than an explicit line break.
    pub wrapped: bool,
}

impl Line {
    pub fn new(cols: usize, fill: Cell) -> Line {
        Line {
            cells: vec![fill; cols],
            size: LineSize::Single,
            wrapped: false,
        }
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    pub fn cells_mut(&mut self) -> &mut [Cell] {
        &mut self.cells
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Number of addressable columns, halved for double-width lines.
    pub fn width(&self) -> usize {
        if self.size.is_double_width() {
            self.cells.len() / 2
        } else {
            self.cells.len()
        }
    }

    pub fn clear(&mut self, fill: Cell) {
        self.cells.fill(fill);
        self.size = LineSize::Single;
        self.wrapped = false;
    }

    /// Fills `cols` (clamped to the line) with `fill`.
    pub fn erase(&mut self, cols: core::ops::Range<usize>, fill: Cell) {
        let end = cols.end.min(self.cells.len());
        if cols.start < end {
            self.cells[cols.start..end].fill(fill);
        }
    }

    /// Inserts `n` copies of `fill` at `at`, shifting cells up to and
    /// including `right` rightwards; cells pushed past `right` are lost.
    pub fn insert(&mut self, at: usize, right: usize, n: usize, fill: Cell) {
        let right = right.min(self.cells.len() - 1);
        if at > right {
            return;
        }
        let span = &mut self.cells[at..=right];
        let n = n.min(span.len());
        span.rotate_right(n);
        span[..n].fill(fill);
    }

    /// Deletes `n` cells at `at`, shifting cells up to and including `right`
    /// leftwards and filling the vacated cells at `right` with `fill`.
    pub fn delete(&mut self, at: usize, right: usize, n: usize, fill: Cell) {
        let right = right.min(self.cells.len() - 1);
        if at > right {
            return;
        }
        let span = &mut self.cells[at..=right];
        let n = n.min(span.len());
        span.rotate_left(n);
        let len = span.len();
        span[len - n..].fill(fill);
    }

    pub fn resize(&mut self, cols: usize, fill: Cell) {
        self.cells.resize(cols, fill);
    }
}

/// The visible page: a fixed number of lines of equal length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    lines: Vec<Line>,
    cols: usize,
}

impl Grid {
    pub fn new(rows: usize, cols: usize) -> Grid {
        Grid {
            lines: (0..rows).map(|_| Line::new(cols, Cell::BLANK)).collect(),
            cols,
        }
    }

    pub fn rows(&self) -> usize {
        self.lines.len()
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn line(&self, row: usize) -> &Line {
        &self.lines[row]
    }

    pub fn line_mut(&mut self, row: usize) -> &mut Line {
        &mut self.lines[row]
    }

    pub fn clear(&mut self, fill: Cell) {
        for line in &mut self.lines {
            line.clear(fill);
        }
    }

    /// Scrolls rows `top..=bottom`, columns `left..=right` up by `n`.
    /// Full-width scrolls move whole lines (with their attributes) and return
    /// the lines that left the top, oldest first; partial-width scrolls move
    /// cells only and return nothing.
    pub fn scroll_up(&mut self, rows: Region, n: usize, fill: Cell) -> Vec<Line> {
        let height = rows.bottom - rows.top + 1;
        let n = n.min(height);
        if n == 0 {
            return Vec::new();
        }
        if rows.left == 0 && rows.right + 1 >= self.cols {
            let span = &mut self.lines[rows.top..=rows.bottom];
            span.rotate_left(n);
            let fresh = Line::new(self.cols, fill);
            span[height - n..]
                .iter_mut()
                .map(|line| core::mem::replace(line, fresh.clone()))
                .collect()
        } else {
            for row in rows.top..=rows.bottom {
                let (dst, src) = (row, row + n);
                if src <= rows.bottom {
                    let cells: Vec<Cell> = self.lines[src].cells[rows.left..=rows.right].to_vec();
                    self.lines[dst].cells[rows.left..=rows.right].copy_from_slice(&cells);
                } else {
                    self.lines[dst].erase(rows.left..rows.right + 1, fill);
                }
            }
            Vec::new()
        }
    }

    /// Scrolls rows `top..=bottom`, columns `left..=right` down by `n`.
    pub fn scroll_down(&mut self, rows: Region, n: usize, fill: Cell) {
        let height = rows.bottom - rows.top + 1;
        let n = n.min(height);
        if n == 0 {
            return;
        }
        if rows.left == 0 && rows.right + 1 >= self.cols {
            let span = &mut self.lines[rows.top..=rows.bottom];
            span.rotate_right(n);
            for line in &mut span[..n] {
                line.clear(fill);
            }
        } else {
            for row in (rows.top..=rows.bottom).rev() {
                if row >= rows.top + n {
                    let cells: Vec<Cell> =
                        self.lines[row - n].cells[rows.left..=rows.right].to_vec();
                    self.lines[row].cells[rows.left..=rows.right].copy_from_slice(&cells);
                } else {
                    self.lines[row].erase(rows.left..rows.right + 1, fill);
                }
            }
        }
    }

    /// Changes the page size. Lines are kept from the top; new cells are blank.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.lines
            .resize_with(rows, || Line::new(cols, Cell::BLANK));
        for line in &mut self.lines {
            line.resize(cols, Cell::BLANK);
        }
        self.cols = cols;
    }

    /// Removes `n` lines from the top (e.g. when the page shrinks below the cursor).
    pub fn drain_top(&mut self, n: usize) -> Vec<Line> {
        let n = n.min(self.lines.len());
        let rows = self.lines.len();
        let taken: Vec<Line> = self.lines.drain(..n).collect();
        self.lines
            .resize_with(rows, || Line::new(self.cols, Cell::BLANK));
        taken
    }
}

/// An inclusive rectangle of rows and columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub top: usize,
    pub bottom: usize,
    pub left: usize,
    pub right: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Line {
        let mut l = Line::new(text.len(), Cell::BLANK);
        for (cell, ch) in l.cells_mut().iter_mut().zip(text.chars()) {
            cell.ch = ch;
        }
        l
    }

    fn text(l: &Line) -> String {
        l.cells().iter().map(|c| c.ch).collect()
    }

    #[test]
    fn insert_and_delete_within_margin() {
        let mut l = line("abcdefgh");
        l.insert(2, 5, 2, Cell::BLANK);
        assert_eq!(text(&l), "ab  cdgh");
        let mut l = line("abcdefgh");
        l.delete(2, 5, 1, Cell::BLANK);
        assert_eq!(text(&l), "abdef gh");
        let mut l = line("abcdefgh");
        l.delete(2, 7, 99, Cell::BLANK);
        assert_eq!(text(&l), "ab      ");
    }

    #[test]
    fn partial_width_scroll() {
        let mut g = Grid::new(3, 4);
        for (r, t) in ["abcd", "efgh", "ijkl"].iter().enumerate() {
            *g.line_mut(r) = line(t);
        }
        let region = Region {
            top: 0,
            bottom: 2,
            left: 1,
            right: 2,
        };
        assert!(g.scroll_up(region, 1, Cell::BLANK).is_empty());
        let rows: Vec<String> = g.lines().iter().map(text).collect();
        assert_eq!(rows, ["afgd", "ejkh", "i  l"]);
        g.scroll_down(region, 1, Cell::BLANK);
        let rows: Vec<String> = g.lines().iter().map(text).collect();
        assert_eq!(rows, ["a  d", "efgh", "ijkl"]);
    }
}
