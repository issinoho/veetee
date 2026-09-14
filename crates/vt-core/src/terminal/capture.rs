//! Capturing the text written to the page, for session logs. The log is the
//! characters as the terminal shows them (after character set translation),
//! with a line break for each line feed and when the cursor moves to another
//! line; other control functions and the status line are left out.

use super::{Emulator, Terminal};

impl Terminal {
    /// Starts or stops capturing text for [`Terminal::take_captured_text`].
    pub fn set_capture(&mut self, on: bool) {
        self.emu.capture = on.then(String::new);
    }

    /// The text written since the last call.
    pub fn take_captured_text(&mut self) -> String {
        match &mut self.emu.capture {
            Some(text) => std::mem::take(text),
            None => String::new(),
        }
    }
}

impl Emulator {
    pub(super) fn capture_text(&mut self, ch: char) {
        if let Some(text) = &mut self.capture {
            text.push(ch);
        }
    }

    pub(super) fn capture_newline(&mut self) {
        self.capture_text('\n');
    }

    /// A line break unless the current line is empty.
    pub(super) fn capture_line_break(&mut self) {
        if let Some(text) = &mut self.capture {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
        }
    }

    /// Backspace takes back the last character of the line.
    pub(super) fn capture_backspace(&mut self) {
        if let Some(text) = &mut self.capture {
            if !text.is_empty() && !text.ends_with('\n') {
                text.pop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Config, Terminal};

    #[test]
    fn captures_text_as_shown() {
        let mut term = Terminal::new(Config::default());
        term.advance(b"not captured\r\n");
        term.set_capture(true);
        term.advance(b"$ DIR\x08\x08IR\r\n\x1b(0lqk\x1b(B \x1b[1mbold\x1b[m\r\n\r\n");
        term.advance(b"\x1b[10;1Hform field\x1b[12;5Hnext\x1b[12;20Hsame line");
        term.advance(b"\x1b[2$~\x1b[1$}status\x1b[0$}");
        assert_eq!(
            term.take_captured_text(),
            "$ DIR\n┌─┐ bold\n\nform field\nnextsame line"
        );
        assert_eq!(term.take_captured_text(), "");
        term.advance(b"\x1bcafter reset");
        assert_eq!(term.take_captured_text(), "after reset");
        term.set_capture(false);
        term.advance(b"x");
        assert_eq!(term.take_captured_text(), "");
    }
}
