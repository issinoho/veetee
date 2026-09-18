//! Watching a session without changing it: running counts, and a line to
//! describe a frame by.
//!
//! A LAT session that fails after an hour leaves nothing behind to say why.
//! The frames are long gone, the terminal shows one line about a circuit going
//! down, and the interesting part — whether the far end had stopped
//! acknowledging, whether credit had run out, whether a message arrived out of
//! order — happened minutes earlier. This module is what remembers.
//!
//! Nothing here alters what a [`Session`](crate::Session) does. The counts are
//! read and never acted on, deliberately: several of them measure faults that
//! have been reasoned about but never actually seen on a wire, and changing
//! how a session behaves on a suspicion is how a working protocol gets broken.
//! Counting first says whether the fault is real.

use crate::{Message, Run, Slot};

/// Whether `a` is a later sequence number than `b`, counting the wrap.
///
/// Sequence numbers are one byte and run out every 256 messages, which on an
/// idle circuit — a keepalive every ten seconds, and an acknowledgement for
/// every message that carries slots — is something like three quarters of an
/// hour. So `a > b` is wrong for a fortyish-minute session and right for no
/// other, and the comparison has to be modular: `a` is newer if the distance
/// from `b` up to it is less than half the range.
///
/// Used only for counting here. The session itself still asks whether a
/// number differs from the last one heard, which is a different question.
#[must_use]
pub fn newer(a: u8, b: u8) -> bool {
    a != b && a.wrapping_sub(b) < 128
}

/// What one session has seen since it opened.
///
/// Cheap to keep and cheap to copy, so the transport can take a snapshot
/// whenever it wants to write a summary out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub frames_in: u64,
    pub frames_out: u64,
    /// Messages carrying no slots: the keepalives and bare acknowledgements
    /// that keep a quiet circuit up. On an idle session these are all of it.
    pub acks_in: u64,
    pub acks_out: u64,
    pub slots_in: u64,
    pub slots_out: u64,
    /// Session data, which is what reached the terminal and what was typed.
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// A message numbered the same as the one before it, which is the far end
    /// repeating itself because it has not heard an acknowledgement.
    pub duplicates: u64,
    /// A message numbered *older* than the newest heard — reordered, or a
    /// retransmission of something older still.
    ///
    /// Worth counting on its own because the session takes any number that
    /// differs as the newest, so one of these drags the acknowledgement
    /// backwards and asks the far end for everything since all over again.
    pub rewinds: u64,
    /// Messages the far end sent that never arrived, counted from the gaps
    /// in its numbering.
    ///
    /// Each one spent credit at its end, so each has to be spent at this one
    /// too; a loss that is not counted leaves the reckoning permanently one
    /// too high, and enough of them stop the far end sending at all.
    pub missed: u64,
    /// Times each end's sequence number ran out and began again.
    pub wraps_out: u64,
    pub wraps_in: u64,
    /// Credit granted away, and credit granted to us.
    ///
    /// The second has never been read by anything else: the session grants
    /// the far end an allowance and spends against it, but sends what is
    /// typed whenever it is typed, without asking what allowance it has been
    /// given. If that is the fault, this is the count that shows it.
    pub granted_out: u64,
    pub granted_in: u64,
    /// Credit the far end has left to spend, as this end last reckoned it.
    pub credit_theirs: u8,
    /// Credit this end has been granted and not spent, by the same reckoning
    /// — which is a reckoning nothing acts on yet.
    pub credit_ours: i32,
    /// Messages sent and not yet acknowledged, now and at its worst.
    ///
    /// The far end names the highest number it has heard in every message, so
    /// this is the one number that says whether it is still listening. On a
    /// healthy circuit it sits at nought or one.
    pub unacked: u8,
    pub max_unacked: u8,
    /// Frames belonging to no session of ours, which on a packet socket is
    /// most of what arrives and is not a fault.
    pub ignored: u64,
    pub stops: u64,
}

impl Stats {
    /// A summary line, for a trace to write every so often.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "in={} out={} acks={}/{} slots={}/{} bytes={}/{} \
             dup={} rewind={} missed={} wrap={}/{} credit={}/{} granted={}/{} \
             unacked={} max_unacked={} ignored={} stops={}",
            self.frames_in,
            self.frames_out,
            self.acks_in,
            self.acks_out,
            self.slots_in,
            self.slots_out,
            self.bytes_in,
            self.bytes_out,
            self.duplicates,
            self.rewinds,
            self.missed,
            self.wraps_in,
            self.wraps_out,
            self.credit_ours,
            self.credit_theirs,
            self.granted_in,
            self.granted_out,
            self.unacked,
            self.max_unacked,
            self.ignored,
            self.stops,
        )
    }
}

/// One line describing a frame, for a trace file.
///
/// `data` decides whether session data is spelled out. It is off by default
/// everywhere, and for a reason worth stating plainly: a LAT session carries
/// the password typed into it in clear, so a trace with data in it is a trace
/// holding a password.
#[must_use]
pub fn describe(payload: &[u8], data: bool) -> String {
    let Ok(message) = crate::parse(payload) else {
        return format!("unreadable {} bytes", payload.len());
    };
    match message {
        Message::Start(start) => format!(
            "{} ours={:04x} theirs={:04x} to={} from={} keepalive={}s frame={}",
            if start.calling { "call" } else { "agree" },
            start.ours,
            start.theirs,
            start.to,
            start.from,
            start.keepalive,
            start.max_frame,
        ),
        Message::Run(run) => describe_run(&run, data),
        Message::Stop(stop) => {
            format!("stop ours={:04x} theirs={:04x}", stop.ours, stop.theirs)
        }
        Message::Announcement(announcement) => format!(
            "announce {} services={}",
            announcement.node,
            announcement.services.len()
        ),
        Message::Solicit(solicit) => {
            format!("solicit {} for {}", solicit.node, solicit.service)
        }
        Message::Other { kind, body } => {
            format!("type {kind:#04x}, {} bytes", body.len())
        }
    }
}

fn describe_run(run: &Run<'_>, data: bool) -> String {
    let mut line = format!(
        "{} seq={} ack={} ours={:04x} theirs={:04x}",
        if run.slots.is_empty() { "ack" } else { "run" },
        run.sequence,
        run.acknowledged,
        run.ours,
        run.theirs,
    );
    for slot in &run.slots {
        line.push(' ');
        line.push('[');
        line.push_str(&describe_slot(slot, data));
        line.push(']');
    }
    line
}

fn describe_slot(slot: &Slot<'_>, data: bool) -> String {
    let kind = match slot.control & 0xf0 {
        0x00 => "data",
        0x90 => "start",
        0xa0 => "params",
        0xb0 => "at",
        0xd0 => "end",
        other => return format!("{}->{} type={other:#04x}", slot.from, slot.to),
    };
    let mut described = format!(
        "{}->{} {kind} credit={} len={}",
        slot.from,
        slot.to,
        slot.control & 0x0f,
        slot.data.len(),
    );
    if data && !slot.data.is_empty() {
        described.push(' ');
        described.push('"');
        described.push_str(&escape(slot.data));
        described.push('"');
    }
    described
}

/// Bytes as something that can sit on one line of a log.
fn escape(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len());
    for &byte in data {
        match byte {
            b'"' => out.push_str("''"),
            0x20..=0x7e => out.push(byte as char),
            b'\r' => out.push('\u{240d}'),
            b'\n' => out.push('\u{240a}'),
            0x1b => out.push('\u{241b}'),
            _ => out.push('\u{fffd}'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_counts_the_wrap() {
        assert!(newer(2, 1), "the ordinary case");
        assert!(!newer(1, 2), "and its opposite");
        assert!(!newer(1, 1), "the same number is not newer than itself");
        assert!(newer(0, 255), "0 follows 255, which is the whole point");
        assert!(!newer(255, 0), "and 255 does not follow 0");
        assert!(newer(127, 0), "just inside half the range");
        assert!(!newer(129, 0), "and just outside it, so read as older");
    }

    #[test]
    fn a_run_message_describes_itself() {
        let line = describe(crate::tests::PROMPT, false);
        assert!(line.starts_with("run seq="), "{line}");
        assert!(line.contains("data credit="), "{line}");
        assert!(line.contains("len="), "{line}");
    }

    #[test]
    fn a_keepalive_is_an_ack_with_no_slots() {
        let line = describe(crate::tests::KEEPALIVE, false);
        assert!(line.starts_with("ack seq="), "{line}");
        assert!(!line.contains('['), "nothing to describe: {line}");
    }

    #[test]
    fn data_is_left_out_unless_it_is_asked_for() {
        let with = describe(crate::tests::PROMPT, true);
        let without = describe(crate::tests::PROMPT, false);
        assert!(with.contains('"'), "asked for: {with}");
        assert!(
            !without.contains('"'),
            "a password could be in there: {without}"
        );
        assert!(with.len() > without.len());
    }

    #[test]
    fn what_cannot_be_read_says_so_rather_than_guessing() {
        assert_eq!(
            describe(&[0xff], false),
            "type 0xff, 1 bytes",
            "a type nobody has read is still a type, and says which"
        );
        assert_eq!(
            describe(&[], false),
            "unreadable 0 bytes",
            "and something too short to hold a type is not"
        );
    }

    #[test]
    fn escaping_keeps_a_line_on_one_line() {
        let escaped = escape(b"ab\r\ncd\x1b[0m\x00");
        assert!(!escaped.contains('\n'), "{escaped}");
        assert!(!escaped.contains('\r'), "{escaped}");
        assert!(escaped.starts_with("ab"), "{escaped}");
    }
}
