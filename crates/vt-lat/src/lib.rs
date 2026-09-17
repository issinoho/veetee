//! DEC LAT (Local Area Transport) messages, as far as `docs/lat-protocol.md`
//! reads them.
//!
//! Frames in, frames out: no sockets, no privilege, so the whole of it is
//! testable with `cargo test` against captured frames. The datalink belongs to
//! `vt-transport`, which has the raw socket and the multicast group to join.
//!
//! Service announcements, solicits, circuit start and stop, and the run
//! messages that carry a session are understood. Anything else is kept whole
//! as [`Message::Other`] rather than guessed at: several message types have
//! never been seen, and parts of the ones here are still unread and marked.
//!
//! [`Session`] drives a circuit made of them, which is what a terminal needs.

mod session;

pub use session::{Event, Session, SessionConfig};

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
/// A circuit being asked for, and the answer to one.
const START: u8 = 0x06;
const START_REPLY: u8 = 0x04;
const STOP: u8 = 0x0a;
/// Everything in a circuit rides in these. The low bits are flags: the calling
/// node used `0x02` throughout and the answering node `0x00` and `0x01`.
/// 🔎 Which bit means what is unread.
const RUN: u8 = 0x00;
const RUN_FLAGS: u8 = 0x03;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message<'a> {
    Announcement(Announcement<'a>),
    Solicit(Solicit<'a>),
    /// A circuit being asked for, or the answer to one.
    Start(Start<'a>),
    /// Session data and acknowledgements.
    Run(Run<'a>),
    /// A circuit being taken down. 🔎 Read no further than its shape.
    Stop(Stop),
    /// A type we have never seen. Kept whole: guessing at it would be worse
    /// than admitting we cannot read it.
    Other {
        kind: u8,
        body: &'a [u8],
    },
}

/// Asking for a circuit, or answering. Each end names the circuit with an
/// identifier of its own choosing; the caller has none for the far end yet and
/// sends zero, which the reply fills in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start<'a> {
    /// True for the node asking, false for the node answering.
    pub calling: bool,
    pub theirs: u16,
    pub ours: u16,
    pub max_frame: u16,
    /// Protocol version, 5.3 on the OpenVMS nodes seen.
    pub version: (u8, u8),
    /// Seconds an idle circuit waits before a keepalive.
    pub keepalive: u8,
    pub to: &'a str,
    pub from: &'a str,
}

/// A message on an open circuit. With no slots it is an acknowledgement, and
/// that is also the keepalive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run<'a> {
    /// 🔎 The low bits of the message type.
    pub flags: u8,
    pub theirs: u16,
    pub ours: u16,
    pub sequence: u8,
    pub acknowledged: u8,
    pub slots: Vec<Slot<'a>>,
}

/// One session's worth of a run message. Data is padded to an even length on
/// the wire; the padding is not part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot<'a> {
    pub to: u8,
    pub from: u8,
    /// The type in the high nibble and credit in the low: type 0 is session
    /// data, 9 starts a session and 10 carries terminal parameters. Read from
    /// a login to OpenVMS, where the same parameter block arrived as `0xa0`,
    /// `0xa1` and `0xaf` — so the low nibble is what varies while the meaning
    /// does not.
    pub control: u8,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stop {
    pub theirs: u16,
    pub ours: u16,
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

    fn bytes(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(len).ok_or(Error::Truncated)?;
        let raw = self.data.get(self.at..end).ok_or(Error::Truncated)?;
        self.at = end;
        Ok(raw)
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
        START | START_REPLY => parse_start(payload).map(Message::Start),
        STOP => parse_stop(payload).map(Message::Stop),
        k if k & !RUN_FLAGS == RUN => parse_run(payload).map(Message::Run),
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

fn parse_start(payload: &[u8]) -> Result<Start<'_>, Error> {
    let mut r = Reader::new(payload);
    let kind = r.byte()?;
    r.skip(1)?;
    let theirs = r.word()?;
    let ours = r.word()?;
    r.skip(2)?; // 🔎 a sequence and an acknowledgement, zero or 0xff here
    let max_frame = r.word()?;
    let version = (r.byte()?, r.byte()?);
    r.skip(3)?; // 🔎 unread
    let keepalive = r.byte()?;
    r.skip(4)?; // 🔎 unread
    let to = r.text()?;
    let from = r.text()?;
    Ok(Start {
        calling: kind == START,
        theirs,
        ours,
        max_frame,
        version,
        keepalive,
        to,
        from,
    })
}

fn parse_run(payload: &[u8]) -> Result<Run<'_>, Error> {
    let mut r = Reader::new(payload);
    let flags = r.byte()? & RUN_FLAGS;
    let count = usize::from(r.byte()?);
    let theirs = r.word()?;
    let ours = r.word()?;
    let sequence = r.byte()?;
    let acknowledged = r.byte()?;
    let mut slots = Vec::with_capacity(count.min(16));
    for _ in 0..count {
        let to = r.byte()?;
        let from = r.byte()?;
        let len = usize::from(r.byte()?);
        let control = r.byte()?;
        let data = r.bytes(len)?;
        // Data is padded to an even length; the pad is not part of it.
        r.skip(len & 1)?;
        slots.push(Slot {
            to,
            from,
            control,
            data,
        });
    }
    // Whatever follows is padding to the Ethernet minimum.
    Ok(Run {
        flags,
        theirs,
        ours,
        sequence,
        acknowledged,
        slots,
    })
}

fn parse_stop(payload: &[u8]) -> Result<Stop, Error> {
    let mut r = Reader::new(payload);
    r.skip(2)?;
    Ok(Stop {
        theirs: r.word()?,
        ours: r.word()?,
    })
}

/// 🔎 Fourteen bytes of a start message that have never been read. These
/// are what one OpenVMS node sent; five of them differ between the two nodes
/// seen, so they are not simply constant, and sending them may or may not be
/// what a host wants.
pub const START_UNREAD: [u8; 14] = [
    0x01, 0x02, 0x64, 0x00, 0x02, 0x10, 0x00, 0x73, 0x9b, 0x3f, 0xa8, 0x29, 0xbc, 0x00,
];

impl Start<'_> {
    /// Builds the message that asks for a circuit, or agrees to one.
    ///
    /// `address` is this node's own Ethernet address, which the message
    /// carries near its end.
    ///
    /// 🔎 Several fields here are copied from what OpenVMS sends rather
    /// than understood, [`START_UNREAD`] among them.
    pub fn build(&self, address: [u8; 6]) -> Vec<u8> {
        let mut out = vec![if self.calling { START } else { START_REPLY }, 0x00];
        out.extend_from_slice(&self.theirs.to_le_bytes());
        out.extend_from_slice(&self.ours.to_le_bytes());
        out.extend_from_slice(&[0x00, 0xff]); // 🔎 a sequence and an acknowledgement
        out.extend_from_slice(&self.max_frame.to_le_bytes());
        out.extend_from_slice(&[self.version.0, self.version.1]);
        out.extend_from_slice(&[0x10, 0x09, 0x08]); // 🔎 unread
        out.push(self.keepalive);
        out.extend_from_slice(&[0x00, 0x00, 0x03, 0x03]); // 🔎 unread
        text(&mut out, self.to);
        text(&mut out, self.from);
        out.push(0); // 🔎 an empty string
        out.extend_from_slice(&START_UNREAD);
        out.extend_from_slice(&address);
        out.extend_from_slice(&[0, 0, 0]);
        out.resize(out.len().max(MIN_PAYLOAD), 0);
        out
    }
}

/// 🔎 Five bytes at the end of a stop message that have never been read.
/// These are what the one stop message ever captured carried; whether any of
/// it is a reason for the circuit ending is unknown.
pub const STOP_UNREAD: [u8; 5] = [0x27, 0x29, 0x01, 0x00, 0x01];

impl Stop {
    /// Builds the message that takes a circuit down.
    ///
    /// 🔎 Shaped after the one that was captured, which named the end it was
    /// sent to and carried zero for its own. This names both, as every message
    /// on an open circuit does, and ends with [`STOP_UNREAD`].
    pub fn build(&self) -> Vec<u8> {
        let mut out = vec![STOP, 0x00];
        out.extend_from_slice(&self.theirs.to_le_bytes());
        out.extend_from_slice(&self.ours.to_le_bytes());
        out.extend_from_slice(&STOP_UNREAD);
        out.resize(out.len().max(MIN_PAYLOAD), 0);
        out
    }
}

/// The control byte of the slot that asks for a service: type 9, a session
/// being started, with fifteen credits granted to the far end.
///
/// OpenVMS answers with a slot of the same type — `0x9f` again, naming the
/// `LTA` device it has created — which is what says the type is the session
/// start rather than anything peculiar to a request.
pub const SLOT_START: u8 = 0x9f;

/// Builds the data of the slot that asks for a service, on a page of `rows`
/// by `cols`.
///
/// 🔎 Copied from the one slot of its kind ever captured, with the service
/// name replaced. The bytes around the name are unread: they end in what look
/// like coded values, a tag and a length before each, of which `07 02 18 00`
/// is twenty-four lines and `08 02 50 00` is eighty columns — so the terminal
/// describes itself here, and the page size is sent rather than assumed.
pub fn session_start(service: &str, rows: u16, cols: u16) -> Vec<u8> {
    let mut out = vec![0x01, 0x01, 0xfe];
    text(&mut out, service);
    out.push(0); // 🔎 an empty name, a port perhaps
    out.extend_from_slice(&[0x01, 0x02, 0x04, 0x00]); // 🔎
    out.extend_from_slice(&[0x07, 0x02]); // lines
    out.extend_from_slice(&rows.to_le_bytes());
    out.extend_from_slice(&[0x08, 0x02]); // columns
    out.extend_from_slice(&cols.to_le_bytes());
    out.push(0x00); // the end of them
    out
}

impl Run<'_> {
    /// Builds the message to send on an open circuit.
    ///
    /// Every byte of a run message is accounted for, so this is the exact
    /// inverse of parsing one: the fixtures round-trip byte for byte.
    pub fn build(&self) -> Vec<u8> {
        let mut out = vec![RUN | (self.flags & RUN_FLAGS), self.slots.len() as u8];
        out.extend_from_slice(&self.theirs.to_le_bytes());
        out.extend_from_slice(&self.ours.to_le_bytes());
        out.push(self.sequence);
        out.push(self.acknowledged);
        for slot in &self.slots {
            let len = slot.data.len().min(255);
            out.extend_from_slice(&[slot.to, slot.from, len as u8, slot.control]);
            out.extend_from_slice(&slot.data[..len]);
            if len & 1 == 1 {
                out.push(0); // data is padded to an even length
            }
        }
        out.resize(out.len().max(MIN_PAYLOAD), 0);
        out
    }

    /// The message that acknowledges what has been heard and carries nothing,
    /// which is also what an idle circuit sends as a keepalive.
    pub fn acknowledgement(theirs: u16, ours: u16, sequence: u8, acknowledged: u8) -> Vec<u8> {
        Run {
            flags: 0,
            theirs,
            ours,
            sequence,
            acknowledged,
            slots: Vec::new(),
        }
        .build()
    }
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
    pub(crate) const X86VMS_SOLICIT: &[u8] = &[
        0x38, 0x00, 0x05, 0x05, 0x05, 0x03, 0xdc, 0x05, 0x26, 0x5a, 0x02, 0x00, 0x05, b'M', b'Y',
        b'I', b'6', b'4', 0x01, 0x01, 0x06, b'X', b'8', b'6', b'V', b'M', b'S', 0x05, b'M', b'Y',
        b'I', b'6', b'4', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];

    /// The circuit start X86VMS sent to MYI64. These four come from a real
    /// session, chosen because none of them carries anything typed: a login
    /// is in the clear on the wire (see docs/lat-protocol.md).
    const START_FRAME: &[u8] = &[
        0x06, 0x00, 0x00, 0x00, 0x01, 0x70, 0x00, 0xff, 0xdc, 0x05, 0x05, 0x03, 0x10, 0x09, 0x08,
        0x14, 0x00, 0x00, 0x03, 0x03, 0x05, b'M', b'Y', b'I', b'6', b'4', 0x06, b'X', b'8', b'6',
        b'V', b'M', b'S', 0x00, 0x01, 0x02, 0x64, 0x00, 0x02, 0x10, 0x00, 0x73, 0x9b, 0x3f, 0xa8,
        0x29, 0xbc, 0x00, 0xaa, 0x00, 0x04, 0x00, 0x01, 0x04, 0x00, 0x00, 0x00,
    ];

    /// MYI64 answering, naming its own end of the circuit.
    pub(crate) const START_REPLY_FRAME: &[u8] = &[
        0x04, 0x00, 0x01, 0x70, 0x01, 0xe0, 0x00, 0x00, 0xdc, 0x05, 0x05, 0x03, 0x10, 0x09, 0x08,
        0x14, 0x00, 0x00, 0x03, 0x03, 0x05, b'M', b'Y', b'I', b'6', b'4', 0x06, b'X', b'8', b'6',
        b'V', b'M', b'S', 0x00, 0x01, 0x02, 0x0a, 0x00, 0x02, 0x10, 0x80, 0xf2, 0xb4, 0x78, 0x6a,
        0x29, 0xbc, 0x00, 0x00, 0x17, 0xa4, 0xab, 0x62, 0x50, 0x00, 0x00, 0x00,
    ];

    /// Two slots, the second carrying MYI64's username prompt.
    pub(crate) const PROMPT: &[u8] = &[
        0x00, 0x02, 0x01, 0x70, 0x01, 0xe0, 0x03, 0x02, 0x01, 0x01, 0x23, 0xa1, 0x46, 0x13, 0x11,
        0x13, 0x11, 0x01, 0x01, 0x48, 0x02, 0x04, 0x80, 0x25, 0x00, 0x00, 0x03, 0x04, 0x80, 0x25,
        0x00, 0x00, 0x04, 0x01, 0x01, 0x05, 0x01, 0x00, 0x07, 0x02, 0x18, 0x00, 0x08, 0x02, 0x50,
        0x00, 0x00, 0x25, 0x01, 0x01, 0x0e, 0x00, 0x0a, 0x0d, 0x0a, 0x0d, b'U', b's', b'e', b'r',
        b'n', b'a', b'm', b'e', b':', b' ',
    ];

    /// An idle circuit: no slots, padded to the Ethernet minimum.
    pub(crate) const KEEPALIVE: &[u8] = &[
        0x00, 0x00, 0x01, 0x70, 0x01, 0xe0, 0x04, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];

    #[test]
    fn reads_a_circuit_being_asked_for_and_answered() {
        let Ok(Message::Start(call)) = parse(START_FRAME) else {
            panic!("not a start")
        };
        assert!(call.calling);
        assert_eq!((call.to, call.from), ("MYI64", "X86VMS"));
        assert_eq!(call.max_frame, 1500);
        assert_eq!(call.version, (5, 3));
        assert_eq!(call.keepalive, 20);
        // The caller cannot name the far end yet.
        assert_eq!(call.theirs, 0);
        assert_eq!(call.ours, 0x7001);

        let Ok(Message::Start(answer)) = parse(START_REPLY_FRAME) else {
            panic!("not a start")
        };
        assert!(!answer.calling);
        // The answer fills in both ends.
        assert_eq!((answer.theirs, answer.ours), (call.ours, 0xe001));
    }

    #[test]
    fn reads_the_slots_of_a_run_message() {
        let Ok(Message::Run(run)) = parse(PROMPT) else {
            panic!("not a run")
        };
        assert_eq!((run.sequence, run.acknowledged), (3, 2));
        assert_eq!((run.theirs, run.ours), (0x7001, 0xe001));
        assert_eq!(run.slots.len(), 2);
        // An odd-length slot is padded, and the pad is not part of its data.
        assert_eq!(run.slots[0].data.len(), 0x23);
        assert_eq!(run.slots[1].data, b"\n\r\n\rUsername: ");
        assert_eq!((run.slots[1].to, run.slots[1].from), (1, 1));
    }

    #[test]
    fn a_run_with_no_slots_is_an_acknowledgement() {
        let Ok(Message::Run(run)) = parse(KEEPALIVE) else {
            panic!("not a run")
        };
        assert!(run.slots.is_empty(), "the padding is not a slot");
        assert_eq!((run.sequence, run.acknowledged), (4, 3));
    }

    #[test]
    fn reads_a_circuit_being_taken_down() {
        let stop = &[
            0x0a, 0x00, 0x01, 0xc0, 0x00, 0x00, 0x27, 0x29, 0x01, 0x00, 0x01,
        ][..];
        assert_eq!(
            parse(stop),
            Ok(Message::Stop(Stop {
                theirs: 0xc001,
                ours: 0,
            }))
        );
    }

    #[test]
    fn a_start_rebuilds_as_openvms_sent_it() {
        let Ok(Message::Start(call)) = parse(START_FRAME) else {
            panic!("not a start")
        };
        assert_eq!(
            call.build([0xaa, 0x00, 0x04, 0x00, 0x01, 0x04]),
            START_FRAME,
            "the unread fields are copied, so this is exact"
        );
    }

    #[test]
    fn a_run_message_rebuilds_as_it_arrived() {
        // With no odd-length slot there is nothing to pad, so this is exact.
        let Ok(Message::Run(idle)) = parse(KEEPALIVE) else {
            panic!("not a run")
        };
        assert_eq!(idle.build(), KEEPALIVE);

        // The prompt has an odd slot, and OpenVMS does not zero the byte it
        // pads with -- it sent 0x25, which looks like whatever was in its
        // buffer. So the test is that building is the inverse of parsing,
        // rather than that the bytes match a value nobody specified.
        let Ok(Message::Run(prompt)) = parse(PROMPT) else {
            panic!("not a run")
        };
        let built = prompt.build();
        assert_ne!(built, PROMPT, "only the pad byte should differ");
        assert_eq!(parse(&built), parse(PROMPT));
    }

    #[test]
    fn an_acknowledgement_is_what_an_idle_circuit_sends() {
        assert_eq!(
            Run::acknowledgement(0x7001, 0xe001, 4, 3),
            KEEPALIVE,
            "the keepalive captured from OpenVMS"
        );
    }

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
        // 0x01 is a run message now, so pick a type we really have not seen.
        let body = &[0x99, 0x02, 0x03][..];
        assert_eq!(parse(body), Ok(Message::Other { kind: 0x99, body }));
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

    #[test]
    fn the_service_request_carries_the_page_size() {
        // The coded values at the end of the one slot of its kind captured:
        // twenty-four lines and eighty columns, a tag and a length before each.
        let page = session_start("MYI64", 24, 80);
        assert!(page.ends_with(&[0x07, 0x02, 0x18, 0x00, 0x08, 0x02, 0x50, 0x00, 0x00]));
        let wide = session_start("MYI64", 48, 132);
        assert!(wide.ends_with(&[0x07, 0x02, 0x30, 0x00, 0x08, 0x02, 0x84, 0x00, 0x00]));
        assert_eq!(page.len(), wide.len(), "the size is coded, not written out");
    }

    #[test]
    fn a_stop_rebuilds_as_it_is_read() {
        let stop = Stop {
            theirs: 0xc001,
            ours: 0x1001,
        };
        // The captured stop was eleven bytes: this pads to the Ethernet
        // minimum as the other messages do, which the card would do anyway.
        assert_eq!(parse(&stop.build()), Ok(Message::Stop(stop)));
    }
}
