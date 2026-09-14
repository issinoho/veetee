//! The session's history for review and search: lines that scrolled off the
//! top of the first page, followed by the lines of the displayed page.

use super::Terminal;
use crate::grid::Line;

/// Where search text was found: a history line, the first cell and the
/// number of cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub line: usize,
    pub col: usize,
    pub len: usize,
}

impl Terminal {
    /// Lines kept above the page.
    pub fn scrollback_len(&self) -> usize {
        self.emu.scrollback.len()
    }

    /// Scrollback lines and the displayed page's lines.
    pub fn history_len(&self) -> usize {
        self.emu.scrollback.len() + self.display_grid().rows()
    }

    /// Line `index` of the history: scrollback first (oldest at 0), then
    /// the displayed page.
    pub fn history_line(&self, index: usize) -> Option<&Line> {
        let back = self.emu.scrollback.len();
        if index < back {
            self.emu.scrollback.get(index)
        } else {
            let grid = self.display_grid();
            (index - back < grid.rows()).then(|| grid.line(index - back))
        }
    }

    /// Finds `text` (ignoring case) before or after `from`, a history line
    /// and column, wrapping around; from the end of the history backwards
    /// when `from` is `None`.
    pub fn find_text(
        &self,
        text: &str,
        from: Option<(usize, usize)>,
        backwards: bool,
    ) -> Option<Found> {
        let needle: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
        let total = self.history_len();
        if needle.is_empty() || total == 0 {
            return None;
        }
        let matches = |index: usize| -> Vec<usize> {
            let Some(line) = self.history_line(index) else {
                return Vec::new();
            };
            let hay: Vec<char> = line.cells()[..line.width()]
                .iter()
                .map(|c| c.ch.to_lowercase().next().unwrap_or(c.ch))
                .collect();
            (0..hay.len().saturating_sub(needle.len() - 1))
                .filter(|&i| hay[i..i + needle.len()] == needle[..])
                .collect()
        };
        let found = |line, col| Found {
            line,
            col,
            len: needle.len(),
        };
        let (start_line, start_col) = from.unwrap_or(if backwards {
            (total - 1, usize::MAX)
        } else {
            (0, 0)
        });
        let start_line = start_line.min(total - 1);
        // The starting line past `from`, every other line, then the starting
        // line again before `from`.
        for step in 0..=total {
            let index = if backwards {
                (start_line + total * 2 - step) % total
            } else {
                (start_line + step) % total
            };
            let cols = matches(index);
            let pick = match (step, backwards) {
                (0, true) if from.is_some() => cols.into_iter().rev().find(|&c| c < start_col),
                (0, true) => cols.into_iter().next_back(),
                (0, false) if from.is_some() => cols.into_iter().find(|&c| c > start_col),
                (0, false) => cols.into_iter().next(),
                (s, true) if s == total => cols.into_iter().rev().find(|&c| c >= start_col),
                (s, false) if s == total => cols.into_iter().find(|&c| c <= start_col),
                (_, true) => cols.into_iter().next_back(),
                (_, false) => cols.into_iter().next(),
            };
            if let Some(col) = pick {
                return Some(found(index, col));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn term() -> Terminal {
        let mut t = Terminal::new(Config {
            rows: 4,
            ..Config::default()
        });
        // Six lines: two scroll off the four-line page.
        t.advance(b"$ DIR\r\nLOGIN.COM\r\nlogin.old\r\n$ TYPE LOGIN.COM\r\n$ SET TERM\r\n$");
        t
    }

    #[test]
    fn history_is_scrollback_then_page() {
        let t = term();
        assert_eq!((t.scrollback_len(), t.history_len()), (2, 6));
        let text = |i| {
            t.history_line(i).map(|l| {
                l.cells()
                    .iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
        };
        assert_eq!(text(0).as_deref(), Some("$ DIR"));
        assert_eq!(text(2).as_deref(), Some("login.old"));
        assert_eq!(text(5).as_deref(), Some("$"));
        assert_eq!(text(6), None);
    }

    #[test]
    fn search_goes_back_through_history_and_wraps() {
        let t = term();
        let hit = |line, col| Some(Found { line, col, len: 5 });
        let first = t.find_text("login", None, true);
        assert_eq!(first, hit(3, 7), "the newest match first");
        assert_eq!(t.find_text("login", Some((3, 7)), true), hit(2, 0));
        assert_eq!(t.find_text("LOGIN", Some((2, 0)), true), hit(1, 0));
        assert_eq!(t.find_text("login", Some((1, 0)), true), hit(3, 7), "wraps");
        assert_eq!(t.find_text("login", Some((1, 0)), false), hit(2, 0));
        assert_eq!(
            t.find_text("login", Some((3, 7)), false),
            hit(1, 0),
            "wraps forwards"
        );
        assert_eq!(t.find_text("vms", None, true), None);
        assert_eq!(t.find_text("", None, true), None);
        let term = t.find_text("$ set term", None, false).unwrap();
        assert_eq!((term.line, term.col, term.len), (4, 0, 10));
    }

    #[test]
    fn a_single_match_is_found_again() {
        let t = term();
        let only = t.find_text("TYPE", None, true).unwrap();
        assert_eq!(
            t.find_text("TYPE", Some((only.line, only.col)), true),
            Some(only)
        );
    }
}
