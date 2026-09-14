//! Text selection in the session's history and extraction for the clipboard.

use crate::terminal::Terminal;

/// A position in the history (see [`Terminal::history_line`]: scrollback
/// lines, then the displayed page), in addressable columns, so double-width
/// lines have half as many. With no scrollback the row is the page row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub row: usize,
    pub col: usize,
}

/// A linear (stream) selection between two points, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Selection {
    /// Where the selection started.
    pub anchor: Point,
    /// Where it currently ends.
    pub head: Point,
}

impl Selection {
    pub fn new(anchor: Point, head: Point) -> Selection {
        Selection { anchor, head }
    }

    /// Start and end in reading order.
    pub fn ordered(&self) -> (Point, Point) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn contains(&self, row: usize, col: usize) -> bool {
        let (start, end) = self.ordered();
        let p = Point { row, col };
        start <= p && p <= end
    }
}

/// Characters that end a double-click word. VMS file specifications such as
/// `DKA0:[SYS0.SYSCOMMON]LOGIN.COM;1` select as one word, so `:`, `[`, `]`,
/// `.`, `;` and `<>` are word characters.
fn is_word_char(ch: char) -> bool {
    !ch.is_whitespace()
        && ch != '\u{A0}'
        && !matches!(ch, '"' | '\'' | '(' | ')' | '{' | '}' | ',' | '`')
}

impl Terminal {
    /// The selected text. Trailing blanks are dropped from each line; lines
    /// joined by autowrap are not separated by a newline.
    pub fn selection_text(&self, selection: &Selection) -> String {
        let (start, end) = selection.ordered();
        let mut out = String::new();
        for row in start.row..=end.row.min(self.history_len().saturating_sub(1)) {
            let Some(line) = self.history_line(row) else {
                break;
            };
            let width = line.width();
            let from = if row == start.row {
                start.col.min(width)
            } else {
                0
            };
            let to_end = row != end.row || end.col + 1 >= width;
            let to = if row == end.row {
                (end.col + 1).min(width)
            } else {
                width
            };
            let mut text = String::new();
            for cell in &line.cells()[from..to.max(from)] {
                match crate::terminal::paste::copied_char(cell) {
                    crate::terminal::paste::CopiedChar::One(c) => text.push(c),
                    crate::terminal::paste::CopiedChar::Name(name) => {
                        text.push('<');
                        text.push_str(name);
                        text.push('>');
                    }
                }
            }
            if to_end {
                out.push_str(text.trim_end_matches(' '));
                if row != end.row && !line.wrapped {
                    out.push('\n');
                }
            } else {
                out.push_str(&text);
            }
        }
        out
    }

    /// The word under `p` (for double-click), or just `p` if it is blank.
    pub fn word_at(&self, p: Point) -> Selection {
        let line = self.history_line(p.row).unwrap_or_else(|| {
            self.history_line(self.history_len() - 1)
                .expect("the page has lines")
        });
        let cells = &line.cells()[..line.width()];
        let col = p.col.min(cells.len() - 1);
        if !is_word_char(cells[col].ch) {
            return Selection::new(Point { row: p.row, col }, Point { row: p.row, col });
        }
        let start = (0..col)
            .rev()
            .take_while(|&c| is_word_char(cells[c].ch))
            .last()
            .unwrap_or(col);
        let end = (col + 1..cells.len())
            .take_while(|&c| is_word_char(cells[c].ch))
            .last()
            .unwrap_or(col);
        Selection::new(
            Point {
                row: p.row,
                col: start,
            },
            Point {
                row: p.row,
                col: end,
            },
        )
    }

    /// The whole line containing `p` (for triple-click).
    pub fn line_at(&self, p: Point) -> Selection {
        let row = p.row.min(self.history_len() - 1);
        let last = self.history_line(row).map_or(1, |l| l.width()) - 1;
        Selection::new(Point { row, col: 0 }, Point { row, col: last })
    }
}
