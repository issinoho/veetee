//! The streams of a program run with pipes for its standard handles and a
//! pseudo console only for its prompts: SSH on Windows (see `windows.rs`).
//!
//! A pseudo console does not pass the host's output through: the console
//! host interprets it and sends its own rendering of the screen, so DECCOLM
//! never arrived, DEC line drawing came back as UTF-8 box drawing, and
//! keypad sequences such as `ESC O M` were parsed into Windows key events
//! that OpenSSH re-encoded as CR. With pipes the session's bytes go both
//! ways untouched. OpenSSH still reads passwords, passphrases and host key
//! answers from the console (`readpassphrase` uses `_getwch`), so typing
//! goes there until the session has begun, and the console's output, plain
//! prompt text, is shown with the escape sequences of its rendering removed.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The most typed characters remembered as expected echo.
const ECHO_LIMIT: usize = 256;

/// Where typed input goes: the console while OpenSSH prompts, the
/// program's standard input once the session has begun.
#[derive(Debug, Default)]
pub struct Routing {
    started: AtomicBool,
    /// Printable characters typed at a prompt, not yet seen echoed.
    echo: Mutex<VecDeque<u8>>,
}

impl Routing {
    /// Whether the session has begun, so input goes to standard input.
    pub fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    /// Input typed at a prompt, as the console should receive it.
    /// `readpassphrase` erases with BS, and veetee's Delete key sends DEL
    /// (the LK401 backarrow key), so DEL becomes BS.
    pub fn to_console(&self, bytes: &[u8]) -> Vec<u8> {
        let mut echo = self.echo.lock().unwrap_or_else(|e| e.into_inner());
        bytes
            .iter()
            .map(|&b| {
                if (0x20..0x7F).contains(&b) && echo.len() < ECHO_LIMIT {
                    echo.push_back(b);
                }
                if b == 0x7F { 0x08 } else { b }
            })
            .collect()
    }

    /// Notes output on standard output. A prompt that echoes (the host key
    /// question) echoes there, with `"\b \b"` for an erase; anything more
    /// means the host is talking and the session has begun.
    pub fn standard_output(&self, bytes: &[u8]) {
        if self.started() {
            return;
        }
        let mut echo = self.echo.lock().unwrap_or_else(|e| e.into_inner());
        for &b in bytes {
            if echo.front() == Some(&b) {
                echo.pop_front();
            } else if b != 0x08 && b != b' ' {
                echo.clear();
                self.started.store(true, Ordering::Release);
                return;
            }
        }
    }
}

/// The console's output reduced to its text: escape sequences (cursor
/// movement, clearing, titles, modes such as win32-input-mode) are dropped,
/// so they neither disturb the screen nor set modes that would send reports
/// to the host. Characters beyond ASCII become `?`; their UTF-8 bytes would
/// otherwise be taken as DEC Supplemental characters or C1 controls.
#[derive(Debug, Default)]
pub struct ConsoleText {
    state: Sequence,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Sequence {
    #[default]
    Ground,
    Escape,
    Csi,
    /// OSC, DCS, SOS, PM or APC, ended by BEL or ST.
    String,
    StringEscape,
}

impl ConsoleText {
    pub fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        for &b in bytes {
            self.state = match (self.state, b) {
                (_, 0x1B) if self.state != Sequence::String => Sequence::Escape,
                (Sequence::Ground, b) => {
                    match b {
                        b'\r' | b'\n' | 0x08 | b'\t' | 0x07 | 0x20..=0x7E => out.push(b),
                        0xC0.. => out.push(b'?'),
                        _ => {}
                    }
                    Sequence::Ground
                }
                (Sequence::Escape, b'[') => Sequence::Csi,
                (Sequence::Escape, b']' | b'P' | b'X' | b'^' | b'_') => Sequence::String,
                (Sequence::Escape, 0x20..=0x2F) => Sequence::Escape,
                (Sequence::Escape, _) => Sequence::Ground,
                (Sequence::Csi, 0x40..=0x7E) => Sequence::Ground,
                (Sequence::Csi, _) => Sequence::Csi,
                (Sequence::String, 0x07) => Sequence::Ground,
                (Sequence::String, 0x1B) => Sequence::StringEscape,
                (Sequence::String, _) => Sequence::String,
                (Sequence::StringEscape, b'\\') => Sequence::Ground,
                (Sequence::StringEscape, _) => Sequence::String,
            };
        }
        out
    }
}

/// Standard error with each bare LF made CR LF: OpenSSH's messages end in
/// LF alone, which a terminal with new line mode reset takes as a line feed
/// without a return.
#[derive(Debug, Default)]
pub struct Newlines {
    after_cr: bool,
}

impl Newlines {
    pub fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        for &b in bytes {
            if b == b'\n' && !self.after_cr {
                out.push(b'\r');
            }
            out.push(b);
            self.after_cr = b == b'\r';
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_goes_to_the_console_until_the_host_speaks() {
        let routing = Routing::default();
        assert_eq!(routing.to_console(b"yes\x7f\x7fes\r"), b"yes\x08\x08es\r");
        // The host key question's echo, with its erases, is not the host.
        routing.standard_output(b"yes\x08 \x08\x08 \x08");
        routing.standard_output(b"es");
        assert!(!routing.started());
        routing.to_console(b"secret\r");
        routing.standard_output(b"\r\n Welcome to OpenVMS");
        assert!(routing.started());
    }

    #[test]
    fn a_password_is_never_taken_for_echo() {
        // A password is not echoed, so the host's first output cannot match it.
        let routing = Routing::default();
        routing.to_console(b"Welcome\r");
        routing.standard_output(b"$ ");
        assert!(routing.started());
    }

    #[test]
    fn console_rendering_is_reduced_to_text() {
        let mut text = ConsoleText::default();
        let rendered = b"\x1b[?9001h\x1b[?1004h\x1b[?25l\x1b[2J\x1b[m\x1b[H\
            \x1b]0;C:\\Windows\\System32\\OpenSSH\\ssh.exe\x07\x1b[?25h\
            system@vms's password: \x1b[K";
        assert_eq!(text.filter(rendered), b"system@vms's password: ");
        // A sequence split between reads, ST-terminated strings and UTF-8.
        assert_eq!(text.filter(b"a\x1b["), b"a");
        assert_eq!(text.filter(b"1;1Hb\x1b]0;t\x1b\\c"), b"bc");
        assert_eq!(text.filter("é\r\n".as_bytes()), b"?\r\n");
    }

    #[test]
    fn bare_line_feeds_become_newlines() {
        let mut nl = Newlines::default();
        assert_eq!(
            nl.filter(b"Warning: added\nok\r"),
            b"Warning: added\r\nok\r"
        );
        assert_eq!(nl.filter(b"\n"), b"\n", "CR LF split between reads");
    }
}
