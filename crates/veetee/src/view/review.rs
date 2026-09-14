//! Reviewing the session's history (scrollback) and searching it. The screen
//! can be moved back through lines that scrolled off the page, as with the
//! VT520's review of previous lines (EK-VT520-RM 2.8.3): new output from
//! the host returns it to the page.

use gtk::prelude::*;
use gtk::{gdk, glib};
use vt_core::Found;

use super::TerminalView;

/// The find bar over the top right of the terminal.
#[derive(Clone)]
pub(super) struct SearchBar {
    pub revealer: gtk::Revealer,
    entry: gtk::SearchEntry,
    status: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    close: gtk::Button,
}

impl SearchBar {
    pub fn new() -> SearchBar {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Find in history")
            .width_chars(24)
            .build();
        let status = gtk::Label::builder()
            .css_classes(["dim-label", "caption"])
            .width_chars(10)
            .build();
        let button = |icon: &str, tip: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tip)
                .css_classes(["flat"])
                .build()
        };
        let previous = button("go-up-symbolic", "Find Older (Enter)");
        let next = button("go-down-symbolic", "Find Newer (Shift+Enter)");
        let close = button("window-close-symbolic", "Close (Esc)");
        let bar = gtk::Box::builder()
            .spacing(4)
            .css_classes(["toolbar", "osd"])
            .build();
        bar.append(&entry);
        bar.append(&status);
        bar.append(&previous);
        bar.append(&next);
        bar.append(&close);
        let revealer = gtk::Revealer::builder()
            .child(&bar)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(8)
            .margin_end(8)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .build();
        SearchBar {
            revealer,
            entry,
            status,
            previous,
            next,
            close,
        }
    }
}

impl TerminalView {
    pub(super) fn connect_review(&self) {
        let bar = &self.search;
        bar.entry.connect_search_changed({
            let view = self.clone();
            move |_| view.find(true, true)
        });
        bar.entry.connect_activate({
            let view = self.clone();
            move |_| view.find(false, true)
        });
        bar.entry.connect_previous_match({
            let view = self.clone();
            move |_| view.find(false, true)
        });
        bar.entry.connect_next_match({
            let view = self.clone();
            move |_| view.find(false, false)
        });
        bar.entry.connect_stop_search({
            let view = self.clone();
            move |_| view.close_search()
        });
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let view = self.clone();
            move |_, key, _, mods| {
                if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                    && mods.contains(gdk::ModifierType::SHIFT_MASK)
                {
                    view.find(false, false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        });
        bar.entry.add_controller(keys);
        bar.previous.connect_clicked({
            let view = self.clone();
            move |_| view.find(false, true)
        });
        bar.next.connect_clicked({
            let view = self.clone();
            move |_| view.find(false, false)
        });
        bar.close.connect_clicked({
            let view = self.clone();
            move |_| view.close_search()
        });

        // The mouse wheel moves through the history, three lines a notch.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll({
            let view = self.clone();
            move |_, _, dy| {
                let lines = {
                    let mut st = view.state.borrow_mut();
                    st.wheel += -dy * 3.0;
                    let whole = st.wheel.trunc();
                    st.wheel -= whole;
                    whole as isize
                };
                if lines != 0 {
                    view.move_review(lines);
                }
                glib::Propagation::Stop
            }
        });
        self.area.add_controller(scroll);
    }

    /// Moves the screen back (positive) or forward through the history.
    pub(super) fn move_review(&self, lines: isize) {
        let mut st = self.state.borrow_mut();
        let (max, back) = {
            let term = st.session.terminal();
            (
                term.scrollback_len() + term.window().0,
                term.scrollback_len(),
            )
        };
        st.review_back = back;
        let review = st.review.saturating_add_signed(lines).min(max);
        if review != st.review {
            st.review = review;
            if review > 0 {
                st.selection = None;
            }
            self.area.queue_render();
        }
    }

    /// A screenful back or forward.
    pub(super) fn review_screen(&self, back: bool) {
        let lines = self.state.borrow().session.terminal().window().1.max(2) - 1;
        self.move_review(if back {
            lines as isize
        } else {
            -(lines as isize)
        });
    }

    /// Returns the screen to the page, unless the find bar is open.
    pub(super) fn leave_review(&self) {
        let mut st = self.state.borrow_mut();
        if st.review > 0 && !st.searching {
            st.review = 0;
            self.area.queue_render();
        }
    }

    pub fn open_search(&self) {
        {
            let mut st = self.state.borrow_mut();
            st.searching = true;
        }
        self.search.revealer.set_reveal_child(true);
        self.search.entry.grab_focus();
        self.search.entry.select_region(0, -1);
        if !self.search.entry.text().is_empty() {
            self.find(true, true);
        }
    }

    fn close_search(&self) {
        {
            let mut st = self.state.borrow_mut();
            st.searching = false;
            st.found = None;
            st.review = 0;
        }
        self.search.revealer.set_reveal_child(false);
        self.search.status.set_text("");
        self.area.grab_focus();
        self.area.queue_render();
    }

    /// Finds the search text: older (`backwards`) or newer than the last
    /// match, or from the newest line when `restart`.
    fn find(&self, restart: bool, backwards: bool) {
        let query = self.search.entry.text();
        let mut st = self.state.borrow_mut();
        let from = if restart {
            None
        } else {
            st.found.map(|f| (f.line, f.col))
        };
        let (found, review, back) = {
            let session = st.session.clone();
            let term = session.terminal();
            let found = term.find_text(&query, from, backwards);
            let review = found.map(|f| review_showing(&term, f, st.review));
            (found, review, term.scrollback_len())
        };
        st.review_back = back;
        st.found = found;
        if let Some(review) = review {
            st.review = review;
            st.selection = None;
        }
        drop(st);
        self.search.status.set_text(match found {
            None if !query.is_empty() => "Not found",
            _ => "",
        });
        if query.is_empty() {
            self.search.entry.remove_css_class("error");
        } else if found.is_none() {
            self.search.entry.add_css_class("error");
        } else {
            self.search.entry.remove_css_class("error");
        }
        self.area.queue_render();
    }
}

/// How far back the screen must be to show `found`, keeping the current
/// position when it is already on the screen.
fn review_showing(term: &vt_core::Terminal, found: Found, review: usize) -> usize {
    let back = term.scrollback_len();
    let (window_top, lines) = term.window();
    let newest_top = back + window_top;
    let top = newest_top - review.min(newest_top);
    if (top..top + lines).contains(&found.line) {
        return review;
    }
    let top = found.line.saturating_sub(lines / 2).min(newest_top);
    newest_top - top
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_core::{Config, Terminal};

    #[test]
    fn found_lines_are_brought_onto_the_screen() {
        let mut term = Terminal::new(Config {
            rows: 10,
            ..Config::default()
        });
        for i in 0..40 {
            term.advance(format!("line {i}\r\n").as_bytes());
        }
        // 31 lines scrolled off; the page shows lines 31–39 and a blank line.
        let back = term.scrollback_len();
        assert_eq!(back, 31);
        let found = |line| Found {
            line,
            col: 0,
            len: 4,
        };
        assert_eq!(
            review_showing(&term, found(35), 0),
            0,
            "already on the page"
        );
        let review = review_showing(&term, found(10), 0);
        let top = back - review;
        assert!(
            (top..top + 10).contains(&10) && top == 5,
            "centred: top {top}"
        );
        assert_eq!(
            review_showing(&term, found(12), review),
            review,
            "stays put"
        );
        assert_eq!(
            review_showing(&term, found(2), 0),
            back,
            "at most the oldest line"
        );
    }
}
