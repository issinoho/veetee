//! DEC LAT (Local Area Transport) messages, as far as `docs/lat-protocol.md`
//! reads them.
//!
//! Frames in, frames out: no sockets, no privilege, so the whole of it is
//! testable with `cargo test` against captured frames. The datalink belongs to
//! `vt-transport`, which has the raw socket and the multicast group to join.
//!
//! Only the two message types seen on the wire are understood. Anything else
//! is kept whole as [`Message::Other`] rather than guessed at.

/// LAT rides directly on Ethernet under this type. There is no IP, so nothing
/// routes: both ends share a segment.
pub const ETHERTYPE: u16 = 0x6004;

/// Announcements and solicits go to this group. A receiver has to join it —
/// the card filters it out otherwise.
pub const GROUP: [u8; 6] = [0x09, 0x00, 0x2b, 0x00, 0x00, 0x0f];

/// The protocol version both OpenVMS nodes send, which LATCP calls 5.3.
const VERSION: [u8; 4] = [0x05, 0x05, 0x05, 0x03];

/// Ethernet will not carry less than this, so short messages are padded.
const MIN_PAYLOAD: usize = 46;

const ANNOUNCEMENT: u8 = 0x28;
const SOLICIT: u8 = 0x38;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message<'a> {
    Announcement(Announcement<'a>),
    Solicit(Solicit<'a>),
    /// A type we have never seen. Kept whole: guessing at it would be worse
    /// than admitting we cannot read it.
    Other {
        kind: u8,
        body: &'a [u8],
    },
}

/// A node saying what it offers, sent to the group every multicast timer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement<'a> {
    pub node: &'a str,
    /// Truncated to 64 characters by the sender.
    pub identification: &'a str,
    pub max_frame: u16,
    /// Seconds between announcements, as the sender has it set.
    pub multicast_timer: u8,
    pub services: Vec<Service<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service<'a> {
    /// Recalculated from load, so it is worth re-reading rather than caching:
    /// it is how a client picks between nodes offering the same service.
    pub rating: u8,
    pub name: &'a str,
    pub identification: &'a str,
}

/// A node asking who has a service, when it has heard no announcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solicit<'a> {
    /// The node asked about.
    pub node: &'a str,
    /// The node asking.
    pub from: &'a str,
    pub service: &'a str,
    pub request_id: u16,
    pub max_frame: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Shorter than the fields it claims to carry.
    Truncated,
    /// A name or identification that is not text.
    NotText,
}

/// Reads the fields of a message in order, refusing to run off the end.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, at: 0 }
    }

    fn skip(&mut self, n: usize) -> Result<(), Error> {
        self.at = self.at.checked_add(n).ok_or(Error::Truncated)?;
        (self.at <= self.data.len())
            .then_some(())
            .ok_or(Error::Truncated)
    }

    fn byte(&mut self) -> Result<u8, Error> {
        let b = *self.data.get(self.at).ok_or(Error::Truncated)?;
        self.at += 1;
        Ok(b)
    }

    /// Two bytes, least significant first, as every number in LAT is.
    fn word(&mut self) -> Result<u16, Error> {
        Ok(u16::from(self.byte()?) | u16::from(self.byte()?) << 8)
    }

    /// A LAT string: one length byte, then that many characters.
    fn text(&mut self) -> Result<&'a str, Error> {
        let len = usize::from(self.byte()?);
        let end = self.at.checked_add(len).ok_or(Error::Truncated)?;
        let raw = self.data.get(self.at..end).ok_or(Error::Truncated)?;
        self.at = end;
        std::str::from_utf8(raw).map_err(|_| Error::NotText)
    }
}

/// Reads a message from the payload of an Ethernet frame, the Ethernet header
/// already removed.
pub fn parse(payload: &[u8]) -> Result<Message<'_>, Error> {
    let kind = *payload.first().ok_or(Error::Truncated)?;
    match kind {
        ANNOUNCEMENT => parse_announcement(payload).map(Message::Announcement),
        SOLICIT => parse_solicit(payload).map(Message::Solicit),
        _ => Ok(Message::Other {
            kind,
            body: payload,
        }),
    }
}

fn parse_announcement(payload: &[u8]) -> Result<Announcement<'_>, Error> {
    let mut r = Reader::new(payload);
    r.skip(6)?; // type, an unread byte, and the version
    r.skip(2)?; // 🔎 changes between frames from one node
    let max_frame = r.word()?;
    let multicast_timer = r.byte()?;
    r.skip(3)?; // 🔎 unread
    let node = r.text()?;
    let identification = r.text()?;
    let count = usize::from(r.byte()?);
    let mut services = Vec::with_capacity(count.min(16));
    for _ in 0..count {
        services.push(Service {
            rating: r.byte()?,
            name: r.text()?,
            identification: r.text()?,
        });
    }
    // What follows the services is not understood; see docs/lat-protocol.md.
    Ok(Announcement {
        node,
        identification,
        max_frame,
        multicast_timer,
        services,
    })
}

fn parse_solicit(payload: &[u8]) -> Result<Solicit<'_>, Error> {
    let mut r = Reader::new(payload);
    r.skip(6)?; // type, an unread byte, and the version
    let max_frame = r.word()?;
    let request_id = r.word()?;
    r.skip(2)?; // 🔎 always 02 00
    let node = r.text()?;
    r.skip(2)?; // 🔎 a group mask: one byte, bit 0 set for group 0
    let from = r.text()?;
    let service = r.text()?;
    Ok(Solicit {
        node,
        from,
        service,
        request_id,
        max_frame,
    })
}

impl Solicit<'_> {
    /// Builds the message to send to [`GROUP`].
    ///
    /// 🔎 Shaped after a solicit OpenVMS itself sent, which went unanswered,
    /// as did this one. Something in it is still unread.
    pub fn build(&self) -> Vec<u8> {
        let mut out = vec![SOLICIT, 0x00];
        out.extend_from_slice(&VERSION);
        out.extend_from_slice(&self.max_frame.to_le_bytes());
        out.extend_from_slice(&self.request_id.to_le_bytes());
        out.extend_from_slice(&[0x02, 0x00]);
        text(&mut out, self.node);
        out.extend_from_slice(&[0x01, 0x01]);
        text(&mut out, self.from);
        text(&mut out, self.service);
        out.resize(out.len().max(MIN_PAYLOAD), 0);
        out
    }
}

/// Writes a LAT string, truncated to what its one length byte can count.
fn text(out: &mut Vec<u8>, s: &str) {
    let raw = s.as_bytes();
    let len = raw.len().min(255);
    out.push(len as u8);
    out.extend_from_slice(&raw[..len]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The announcement OpenVMS V8.4-2L3 sent on 16 September 2026.
    const MYI64: &[u8] = &[
        0x28, 0x08, 0x05, 0x05, 0x05, 0x03, 0x54, 0xff, 0xdc, 0x05, 0x3c, 0x02, 0x01, 0x01, 0x05,
        b'M', b'Y', b'I', b'6', b'4', 0x40, b' ', b'W', b'e', b'l', b'c', b'o', b'm', b'e', b' ',
        b't', b'o', b' ', b'V', b'M', b'S', b' ', b'S', b'o', b'f', b't', b'w', b'a', b'r', b'e',
        b',', b' ', b'I', b'n', b'c', b'.', b' ', b'O', b'p', b'e', b'n', b'V', b'M', b'S', b' ',
        b'(', b'T', b'M', b')', b' ', b'I', b'A', b'6', b'4', b' ', b'O', b'p', b'e', b'r', b'a',
        b't', b'i', b'n', b'g', b' ', b'S', b'y', b's', b't', b'e', 0x01, 0x52, 0x05, b'M', b'Y',
        b'I', b'6', b'4', 0x40, b' ', b'W', b'e', b'l', b'c', b'o', b'm', b'e', b' ', b't', b'o',
        b' ', b'V', b'M', b'S', b' ', b'S', b'o', b'f', b't', b'w', b'a', b'r', b'e', b',', b' ',
        b'I', b'n', b'c', b'.', b' ', b'O', b'p', b'e', b'n', b'V', b'M', b'S', b' ', b'(', b'T',
        b'M', b')', b' ', b'I', b'A', b'6', b'4', b' ', b'O', b'p', b'e', b'r', b'a', b't', b'i',
        b'n', b'g', b' ', b'S', b'y', b's', b't', b'e', 0x01, 0x01, 0x01, 0x08, 0x01, 0x10, 0x80,
        0x1a, 0xac, 0x93, 0x50, 0x27, 0xbc, 0x00, 0x00, 0x17, 0xa4, 0xab, 0x62, 0x51, 0x00, 0x00,
        0x00,
    ];

    /// The solicit OpenVMS V9.2-3 sent, asking MYI64 for its service.
    const X86VMS_SOLICIT: &[u8] = &[
        0x38, 0x00, 0x05, 0x05, 0x05, 0x03, 0xdc, 0x05, 0x26, 0x5a, 0x02, 0x00, 0x05, b'M', b'Y',
        b'I', b'6', b'4', 0x01, 0x01, 0x06, b'X', b'8', b'6', b'V', b'M', b'S', 0x05, b'M', b'Y',
        b'I', b'6', b'4', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];

    #[test]
    fn reads_a_real_announcement() {
        let Ok(Message::Announcement(a)) = parse(MYI64) else {
            panic!("not an announcement")
        };
        assert_eq!(a.node, "MYI64");
        assert_eq!(a.max_frame, 1500);
        assert_eq!(a.multicast_timer, 60);
        assert!(a.identification.starts_with(" Welcome to VMS Software"));
        // The sender truncates its identification.
        assert_eq!(a.identification.len(), 64);
        assert_eq!(a.services.len(), 1);
        assert_eq!(a.services[0].name, "MYI64");
        assert_eq!(a.services[0].rating, 82);
    }

    #[test]
    fn reads_a_real_solicit() {
        let Ok(Message::Solicit(s)) = parse(X86VMS_SOLICIT) else {
            panic!("not a solicit")
        };
        assert_eq!((s.node, s.from, s.service), ("MYI64", "X86VMS", "MYI64"));
        assert_eq!(s.max_frame, 1500);
        assert_eq!(s.request_id, 0x5a26);
    }

    #[test]
    fn builds_the_solicit_openvms_sends() {
        let built = Solicit {
            node: "MYI64",
            from: "X86VMS",
            service: "MYI64",
            request_id: 0x5a26,
            max_frame: 1500,
        }
        .build();
        assert_eq!(
            built, X86VMS_SOLICIT,
            "byte for byte, including the padding"
        );
    }

    #[test]
    fn an_unknown_type_is_kept_whole() {
        let body = &[0x01, 0x02, 0x03][..];
        assert_eq!(parse(body), Ok(Message::Other { kind: 1, body }));
    }

    #[test]
    fn a_short_message_is_refused_rather_than_read_past() {
        // The fields of that solicit end here; the rest is Ethernet padding.
        const FIELDS: usize = 33;
        assert!(matches!(
            parse(&X86VMS_SOLICIT[..FIELDS]),
            Ok(Message::Solicit(_))
        ));
        for n in 0..FIELDS {
            if let Ok(Message::Solicit(_)) = parse(&X86VMS_SOLICIT[..n]) {
                panic!("read a solicit from {n} of {FIELDS} bytes of fields");
            }
        }
        assert_eq!(parse(&MYI64[..40]), Err(Error::Truncated));
    }

    #[test]
    fn a_name_that_is_not_text_is_refused() {
        let mut broken = X86VMS_SOLICIT.to_vec();
        broken[13] = 0xff; // the first letter of the node name
        assert_eq!(parse(&broken), Err(Error::NotText));
    }
}
