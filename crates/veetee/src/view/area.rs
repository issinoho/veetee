//! The terminal's drawing area, and what it tells assistive technologies:
//! it has the terminal role, its text is the screen's lines, the caret is
//! the cursor, and changes are reported as the text that was removed and
//! inserted, so a screen reader speaks new output rather than the whole
//! screen.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct TerminalArea {
        pub text: RefCell<Vec<char>>,
        pub caret: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TerminalArea {
        const NAME: &'static str = "VeeteeTerminalArea";
        type Type = super::TerminalArea;
        type ParentType = gtk::GLArea;
        type Interfaces = (gtk::AccessibleText,);

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Terminal);
        }
    }

    impl ObjectImpl for TerminalArea {}
    impl WidgetImpl for TerminalArea {}
    impl GLAreaImpl for TerminalArea {}

    impl AccessibleTextImpl for TerminalArea {
        fn caret_position(&self) -> u32 {
            self.caret.get()
        }

        fn contents(&self, start: u32, end: u32) -> Option<glib::Bytes> {
            let text = self.text.borrow();
            let (start, end) = clamp(&text, start, end);
            let s: String = text[start..end].iter().collect();
            Some(glib::Bytes::from_owned(s.into_bytes()))
        }

        fn contents_at(
            &self,
            offset: u32,
            granularity: gtk::AccessibleTextGranularity,
        ) -> Option<(u32, u32, glib::Bytes)> {
            let text = self.text.borrow();
            let (start, end) = unit_at(&text, offset as usize, granularity);
            let s: String = text[start..end].iter().collect();
            Some((
                start as u32,
                end as u32,
                glib::Bytes::from_owned(s.into_bytes()),
            ))
        }
    }
}

glib::wrapper! {
    pub struct TerminalArea(ObjectSubclass<imp::TerminalArea>)
        @extends gtk::GLArea, gtk::Widget,
        @implements gtk::Accessible, gtk::AccessibleText, gtk::Buildable, gtk::ConstraintTarget;
}

impl TerminalArea {
    pub fn new() -> TerminalArea {
        glib::Object::builder::<TerminalArea>()
            .property("hexpand", true)
            .property("vexpand", true)
            .property("focusable", true)
            .property("has-depth-buffer", false)
            .build()
            .named("Terminal")
    }

    fn named(self, name: &str) -> TerminalArea {
        self.update_property(&[gtk::accessible::Property::Label(name)]);
        self
    }

    /// Updates the accessible text to the screen's lines and the caret to
    /// the cursor, reporting what changed.
    pub fn set_screen_text(&self, lines: &[String], cursor: (usize, usize)) {
        let new: Vec<char> = lines.join("\n").chars().collect();
        let caret = caret_offset(lines, cursor);
        let imp = self.imp();
        let old = imp.text.borrow().clone();
        if old != new {
            let changes = changes(&old, &new);
            // Assistive technologies read removed text from the old text and
            // inserted text from the new, as each change is reported.
            for &(change, start, end) in &changes {
                if change == gtk::AccessibleTextContentChange::Remove {
                    self.update_contents(change, start as u32, end as u32);
                }
            }
            imp.text.replace(new.clone());
            for &(change, start, end) in &changes {
                if change == gtk::AccessibleTextContentChange::Insert {
                    self.update_contents(change, start as u32, end as u32);
                }
            }
        }
        if imp.caret.replace(caret) != caret || old != new {
            self.update_caret_position();
        }
    }
}

fn clamp(text: &[char], start: u32, end: u32) -> (usize, usize) {
    let len = text.len();
    let start = (start as usize).min(len);
    let end = if end == u32::MAX {
        len
    } else {
        (end as usize).min(len)
    };
    (start, end.max(start))
}

/// The character offset of the cursor in the joined lines.
fn caret_offset(lines: &[String], (row, col): (usize, usize)) -> u32 {
    let before: usize = lines.iter().take(row).map(|l| l.chars().count() + 1).sum();
    let here = lines.get(row).map_or(0, |l| l.chars().count());
    (before + col.min(here)) as u32
}

/// The character, word or line around `offset`.
fn unit_at(
    text: &[char],
    offset: usize,
    granularity: gtk::AccessibleTextGranularity,
) -> (usize, usize) {
    let len = text.len();
    let offset = offset.min(len);
    match granularity {
        gtk::AccessibleTextGranularity::Character => (offset, (offset + 1).min(len)),
        gtk::AccessibleTextGranularity::Word => {
            let is_word = |c: char| !c.is_whitespace();
            let mut start = offset;
            while start > 0 && is_word(text[start - 1]) {
                start -= 1;
            }
            let mut end = offset;
            while end < len && is_word(text[end]) {
                end += 1;
            }
            // Include the spaces after the word, as GTK's own widgets do.
            while end < len && text[end] == ' ' {
                end += 1;
            }
            (start, end)
        }
        // Lines, sentences and paragraphs are all screen lines.
        _ => {
            let start = text[..offset]
                .iter()
                .rposition(|&c| c == '\n')
                .map_or(0, |i| i + 1);
            let end = text[offset..]
                .iter()
                .position(|&c| c == '\n')
                .map_or(len, |i| offset + i + 1);
            (start, end)
        }
    }
}

/// What changed between two screens: removals (against the old text) then
/// insertions (against the new). A screen that scrolled is the top lines
/// removed and the new bottom lines inserted.
fn changes(old: &[char], new: &[char]) -> Vec<(gtk::AccessibleTextContentChange, usize, usize)> {
    use gtk::AccessibleTextContentChange::{Insert, Remove};
    let split =
        |t: &[char]| -> Vec<Vec<char>> { t.split(|&c| c == '\n').map(<[char]>::to_vec).collect() };
    let (old_lines, new_lines) = (split(old), split(new));
    let n = old_lines.len();
    if n == new_lines.len() && n > 1 {
        for k in 1..n {
            if old_lines[k..] == new_lines[..n - k] && old_lines[..k] != new_lines[n - k..] {
                let removed: usize = old_lines[..k].iter().map(|l| l.len() + 1).sum();
                let kept: usize = new_lines[..n - k].iter().map(|l| l.len() + 1).sum();
                return vec![
                    (Remove, 0, removed),
                    (Insert, kept.min(new.len()), new.len()),
                ];
            }
        }
    }
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = Vec::new();
    if old.len() - suffix > prefix {
        out.push((Remove, prefix, old.len() - suffix));
    }
    if new.len() - suffix > prefix {
        out.push((Insert, prefix, new.len() - suffix));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::AccessibleTextContentChange::{Insert, Remove};

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn typing_is_an_insertion() {
        assert_eq!(
            changes(&chars("$ DI\n"), &chars("$ DIR\n")),
            [(Insert, 4, 5)]
        );
        assert_eq!(changes(&chars("$ DIR"), &chars("$ DI")), [(Remove, 4, 5)]);
    }

    #[test]
    fn scrolling_removes_the_top_and_inserts_the_bottom() {
        let old = chars("one\ntwo\nthree");
        let new = chars("two\nthree\nfour");
        assert_eq!(changes(&old, &new), [(Remove, 0, 4), (Insert, 10, 14)]);
    }

    #[test]
    fn a_changed_line_is_replaced() {
        let old = chars("a\nPrinter: None\nb");
        let new = chars("a\nPrinter: Busy\nb");
        assert_eq!(changes(&old, &new), [(Remove, 11, 15), (Insert, 11, 15)]);
    }

    #[test]
    fn units_around_an_offset() {
        let text = chars("$ SHOW USERS\nOpenVMS");
        use gtk::AccessibleTextGranularity as G;
        assert_eq!(unit_at(&text, 3, G::Word), (2, 7));
        assert_eq!(unit_at(&text, 3, G::Line), (0, 13));
        assert_eq!(unit_at(&text, 15, G::Line), (13, 20));
        assert_eq!(unit_at(&text, 15, G::Character), (15, 16));
        assert_eq!(caret_offset(&["$ DIR".into(), "x".into()], (1, 9)), 7);
    }
}
