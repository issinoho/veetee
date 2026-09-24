//! Text files cross the line in one form and are kept in another.
//!
//! On the line every line of a text file ends in a carriage return and a line
//! feed, whatever the two systems use themselves — that is what lets OpenVMS,
//! with its records, and Linux, with its line feeds, exchange text at all. So
//! a line feed that is not already after a carriage return gains one on the
//! way out, and on the way in the pair becomes whatever is local.
//!
//! Both directions keep a little state, because a packet can end between the
//! carriage return and the line feed.

/// How lines end in the files on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// A line feed alone: Linux and every other Unix.
    Lf,
    /// A carriage return and a line feed: Windows, and the line's own form.
    CrLf,
}

impl LineEnding {
    /// The convention of the machine veetee is running on.
    #[must_use]
    pub fn native() -> LineEnding {
        if cfg!(windows) {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        }
    }
}

/// Local text into the line's form.
#[derive(Debug, Clone, Default)]
pub struct ToLine {
    after_cr: bool,
}

impl ToLine {
    /// Adds a carriage return before every line feed that lacks one. A file
    /// that already has them, or has a mixture, comes out the same: every line
    /// ending once, as CR LF.
    pub fn convert(&mut self, data: &[u8], out: &mut Vec<u8>) {
        for &byte in data {
            if byte == b'\n' && !self.after_cr {
                out.push(b'\r');
            }
            out.push(byte);
            self.after_cr = byte == b'\r';
        }
    }
}

/// The line's form into local text.
#[derive(Debug, Clone)]
pub struct FromLine {
    local: LineEnding,
    /// A carriage return that ended the last packet, not yet known to be the
    /// first half of a line ending.
    held: bool,
}

impl FromLine {
    #[must_use]
    pub fn new(local: LineEnding) -> FromLine {
        FromLine { local, held: false }
    }

    /// Turns each CR LF into the local line ending. A carriage return on its
    /// own is kept: it means something (overprinting, on the systems that
    /// sent it), and dropping it would change the file.
    pub fn convert(&mut self, data: &[u8], out: &mut Vec<u8>) {
        if self.local == LineEnding::CrLf {
            out.extend_from_slice(data);
            return;
        }
        for &byte in data {
            if std::mem::take(&mut self.held) && byte != b'\n' {
                out.push(b'\r');
            }
            if byte == b'\r' {
                self.held = true;
            } else {
                out.push(byte);
            }
        }
    }

    /// The end of the file: a carriage return still held was not the start
    /// of a line ending after all.
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        if std::mem::take(&mut self.held) {
            out.push(b'\r');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_line(pieces: &[&[u8]]) -> Vec<u8> {
        let mut state = ToLine::default();
        let mut out = Vec::new();
        for piece in pieces {
            state.convert(piece, &mut out);
        }
        out
    }

    fn from_line(local: LineEnding, pieces: &[&[u8]]) -> Vec<u8> {
        let mut state = FromLine::new(local);
        let mut out = Vec::new();
        for piece in pieces {
            state.convert(piece, &mut out);
        }
        state.finish(&mut out);
        out
    }

    #[test]
    fn every_line_goes_out_ending_once_in_cr_lf() {
        assert_eq!(to_line(&[b"a\nb\n"]), b"a\r\nb\r\n");
        assert_eq!(to_line(&[b"a\r\nb\r\n"]), b"a\r\nb\r\n", "not twice");
        assert_eq!(to_line(&[b"a\nb\r\nc"]), b"a\r\nb\r\nc", "a mixture too");
        assert_eq!(
            to_line(&[b"a\r", b"\nb"]),
            b"a\r\nb",
            "a file read in pieces that split a CR LF"
        );
    }

    #[test]
    fn a_line_ending_arrives_as_the_local_one() {
        assert_eq!(from_line(LineEnding::Lf, &[b"a\r\nb\r\n"]), b"a\nb\n");
        assert_eq!(from_line(LineEnding::CrLf, &[b"a\r\nb\r\n"]), b"a\r\nb\r\n");
    }

    #[test]
    fn a_packet_that_ends_between_the_two_halves_is_waited_for() {
        assert_eq!(from_line(LineEnding::Lf, &[b"a\r", b"\nb"]), b"a\nb");
        assert_eq!(from_line(LineEnding::Lf, &[b"a\r", b"", b"\nb"]), b"a\nb");
    }

    #[test]
    fn a_carriage_return_on_its_own_is_kept() {
        assert_eq!(
            from_line(LineEnding::Lf, &[b"over\rstrike\r\n"]),
            b"over\rstrike\n"
        );
        assert_eq!(from_line(LineEnding::Lf, &[b"\r\r\n"]), b"\r\n");
        assert_eq!(
            from_line(LineEnding::Lf, &[b"ends\r"]),
            b"ends\r",
            "even at the very end of the file"
        );
    }
}
