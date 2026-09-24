//! What a sender says about a file before its contents: whether it is text,
//! and how big it is.
//!
//! An attribute packet goes between the file's name and its data, where both
//! ends have said in their send-init that they can do them. It is a run of
//! attributes, each a tag, a length and a value:
//!
//! ```text
//! "#AMJ     type: text, records ending in CR LF   ("B8" is binary)
//! 1$4500    exact size in bytes
//! !!5       size in kilobytes, rounded up
//! @         no more attributes (a length of nought)
//! ```
//!
//! What matters most is the type. A Kermit receiving without one assumes
//! binary and keeps the line's CR LF in a text file, so without it a text
//! transfer needs text mode set at both ends. With it, the sender decides:
//! C-Kermit, left to itself, calls each file text or binary by looking at it.
//!
//! The data field is **not** prefix-encoded as file data is. C-Kermit's date
//! goes out as `#1` then the date, `#` being its tag and not a control
//! prefix: read as encoded data, that would lose the tag and everything after
//! it. Every value is printable in any case.
//!
//! The tags, and the forms of the values, are as C-Kermit 10.0 and G-Kermit
//! 2.01 send them, which is also what the protocol describes. Everything else
//! either sends — dates, protection, the sending system — is read past.

use crate::{tochar, unchar};

/// What kind of file this is, which decides how its contents travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Text,
    Binary,
}

/// The attributes veetee reads and sends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attributes {
    pub file_type: Option<FileType>,
    /// The size in bytes, as the sending system holds the file.
    pub size: Option<u64>,
}

/// Where to find the attribute capability in the first capability byte of a
/// send-init: long packets are 2, sliding windows 4, attributes 8, and 1 says
/// another capability byte follows. G-Kermit offers `*`, which is 10 — long
/// packets and attributes, and its manual says it does both and not windows.
pub(crate) const CAPABLE: u8 = 8;

impl Attributes {
    /// Reads the data field of an attribute packet, as it arrived.
    #[must_use]
    pub fn read(data: &[u8]) -> Attributes {
        let mut found = Attributes::default();
        let mut at = 0;
        while at + 1 < data.len() {
            let tag = data[at];
            let len = usize::from(unchar(data[at + 1]));
            let Some(value) = data.get(at + 2..at + 2 + len) else {
                // Cut short: what is here is all there is.
                break;
            };
            at += 2 + len;
            match tag {
                // Text is `A`, whatever follows it about records; binary is
                // `B`, and anything else is left for the user's setting.
                b'"' => {
                    found.file_type = match value.first() {
                        Some(b'A') => Some(FileType::Text),
                        Some(b'B') => Some(FileType::Binary),
                        _ => None,
                    }
                }
                b'1' => found.size = digits(value),
                // Kilobytes, when there is nothing exact.
                b'!' => {
                    if found.size.is_none() {
                        found.size = digits(value).map(|k| k * 1024);
                    }
                }
                b'@' => break,
                _ => {}
            }
        }
        found
    }

    /// The data field of an attribute packet saying what veetee knows.
    #[must_use]
    pub fn build(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut put = |tag: u8, value: &[u8]| {
            out.push(tag);
            out.push(tochar(value.len() as u8));
            out.extend_from_slice(value);
        };
        match self.file_type {
            Some(FileType::Text) => put(b'"', b"AMJ"),
            Some(FileType::Binary) => put(b'"', b"B8"),
            None => {}
        }
        if let Some(size) = self.size {
            put(b'1', size.to_string().as_bytes());
            put(b'!', size.div_ceil(1024).to_string().as_bytes());
        }
        out
    }
}

fn digits(value: &[u8]) -> Option<u64> {
    std::str::from_utf8(value).ok()?.trim().parse().ok()
}

/// Whether a send-init's capability field offers attribute packets.
#[must_use]
pub fn offered(capabilities: &[u8]) -> bool {
    capabilities
        .first()
        .is_some_and(|&c| unchar(c) & CAPABLE != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_g_kermit_says_about_a_text_file() {
        // Captured from G-Kermit 2.01, `gkermit -T -s lf.txt`.
        let a = Attributes::read(b"\"#AMJ*!A1$4500");
        assert_eq!(a.file_type, Some(FileType::Text));
        assert_eq!(a.size, Some(4500));
    }

    #[test]
    fn what_g_kermit_says_about_a_binary_one() {
        let a = Attributes::read(b"\"\"B81$1024");
        assert_eq!(a.file_type, Some(FileType::Binary));
        assert_eq!(a.size, Some(1024));
    }

    #[test]
    fn what_c_kermit_says_with_its_date_and_all() {
        // Captured from C-Kermit 10.0 Beta.12, `kermit -s lf.txt`, which
        // chose text itself. The date's tag is `#`, sent bare.
        let a = Attributes::read(b".\"U1\"#AMJ*!A#120260924 12:32:47!!51$4500,#664-!3@ ");
        assert_eq!(a.file_type, Some(FileType::Text));
        assert_eq!(a.size, Some(4500));
        let b = Attributes::read(b".\"U1\"\"B8#120260924 12:32:47!!11$1024,#664-!3@ ");
        assert_eq!(b.file_type, Some(FileType::Binary));
    }

    #[test]
    fn kilobytes_do_when_there_is_nothing_exact() {
        assert_eq!(Attributes::read(b"!!5").size, Some(5120));
        assert_eq!(Attributes::read(b"!!51$4500").size, Some(4500));
        assert_eq!(Attributes::read(b"1$4500!!5").size, Some(4500));
    }

    #[test]
    fn what_cannot_be_read_is_left_alone() {
        assert_eq!(Attributes::read(b""), Attributes::default());
        assert_eq!(
            Attributes::read(b"\"").file_type,
            None,
            "a tag and no length"
        );
        assert_eq!(Attributes::read(b"\"%AM").file_type, None, "cut short");
        assert_eq!(
            Attributes::read(b"\"!X").file_type,
            None,
            "a type it does not know"
        );
        assert_eq!(Attributes::read(b"1$45x0").size, None);
        assert_eq!(
            Attributes::read(b"@ \"#AMJ").file_type,
            None,
            "nothing after the end"
        );
    }

    #[test]
    fn what_veetee_sends_reads_back() {
        for a in [
            Attributes {
                file_type: Some(FileType::Text),
                size: Some(4500),
            },
            Attributes {
                file_type: Some(FileType::Binary),
                size: Some(0),
            },
            Attributes::default(),
        ] {
            assert_eq!(Attributes::read(&a.build()), a);
        }
        assert_eq!(
            Attributes {
                file_type: Some(FileType::Text),
                size: Some(4500),
            }
            .build(),
            b"\"#AMJ1$4500!!5",
            "in the forms the two Kermits use"
        );
    }

    #[test]
    fn the_capability_is_the_bit_both_kermits_set() {
        assert!(offered(b"*"), "G-Kermit: long packets and attributes");
        assert!(offered(b"^"), "C-Kermit: those and more");
        assert!(!offered(b"\""), "long packets only");
        assert!(!offered(b""), "no capabilities at all");
        assert!(offered(&[tochar(CAPABLE)]), "what veetee offers");
    }
}
