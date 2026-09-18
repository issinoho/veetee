//! A long session against a host that is not there.
//!
//! Sessions are suspected of failing after an hour or so, and an hour is a
//! number of messages rather than a length of time: a sequence number is one
//! byte, and an idle circuit spends the whole byte in about three quarters of
//! an hour. So the interesting states are all reachable without waiting, and
//! without an OpenVMS node — which is the point of this file, the captured
//! frames in `session.rs` being a handful of messages from one healthy login.
//!
//! The host below is only as much of a host as a soak needs, but it is honest
//! about the two things that matter: it numbers its messages, and it spends
//! the credit it has been granted and stops when it has none. A session that
//! grants credit wrongly deadlocks it, which is a failure the assertions here
//! will show.

use vt_lat::{Event, Message, Run, Session, SessionConfig, Slot, Start, watch};

/// As much credit as the low nibble of a control byte holds, which is as much
/// as either end can be granted at once.
const MAX_CREDIT: u8 = 15;

/// The far end of a circuit: numbering, acknowledging and spending credit.
struct Host {
    /// The identifiers, each end naming the other's first.
    ours: u16,
    theirs: u16,
    sequence: u8,
    /// The newest number heard from veetee.
    heard: u8,
    /// What veetee has granted and this end has not spent. A slot costs one.
    credit: i32,
    /// What this end has granted veetee and veetee has not spent. A node that
    /// sends past its allowance has its messages dropped by a real host, so
    /// this one refuses to let it happen quietly.
    granted: i32,
    /// What veetee has sent, to compare with what was typed.
    typed: Vec<u8>,
}

impl Host {
    /// Answers a call, learning the caller's identifier from it.
    fn agree(call: &[u8]) -> (Host, Vec<u8>) {
        let Ok(Message::Start(start)) = vt_lat::parse(call) else {
            panic!("a session's first frame is a call")
        };
        assert!(start.calling, "and the caller says so");
        let host = Host {
            ours: 0xe001,
            theirs: start.ours,
            sequence: 0,
            heard: 0,
            credit: 0,
            granted: 0,
            typed: Vec::new(),
        };
        let reply = Start {
            calling: false,
            theirs: host.theirs,
            ours: host.ours,
            max_frame: 1500,
            version: (5, 3),
            keepalive: 20,
            to: "VEETEE",
            from: "MYI64",
        }
        .build([0xaa, 0x00, 0x04, 0x00, 0x02, 0x04]);
        (host, reply)
    }

    /// Reads a frame from veetee: what it has numbered, and what it grants.
    fn hear(&mut self, frame: &[u8]) {
        match vt_lat::parse(frame) {
            Ok(Message::Run(run)) => {
                assert_eq!(run.ours, self.theirs, "veetee names its own end");
                assert_eq!(run.theirs, self.ours, "and this one second");
                if watch::newer(run.sequence, self.heard) {
                    self.heard = run.sequence;
                }
                for slot in &run.slots {
                    self.credit += i32::from(slot.control & 0x0f);
                    // Only session data with something in it spends an
                    // allowance: the slot asking for a service goes before
                    // there is one, and an empty slot is how credit is
                    // granted, which must not itself need credit.
                    if slot.control & 0xf0 == 0x00 && !slot.data.is_empty() {
                        self.granted -= 1;
                        assert!(
                            self.granted >= 0,
                            "veetee sent past the allowance it was granted"
                        );
                        self.typed.extend_from_slice(slot.data);
                    }
                }
                // A nibble limits what one grant carries, not what an end can
                // be holding: granting is read as adding to the allowance
                // rather than replacing it. 🔎 Whether a real node lets the
                // total run past fifteen is unread, so this one clamps, which
                // is the unforgiving reading of the two.
                self.credit = self.credit.min(i32::from(MAX_CREDIT));
            }
            Ok(Message::Start(_) | Message::Stop(_)) => {}
            other => panic!("veetee sent something unreadable: {other:?}"),
        }
    }

    /// An acknowledgement: no slots, and a sequence number like any other
    /// message. A session that does not follow the numbering of these reads
    /// every one as a message lost.
    fn ack(&mut self) -> Vec<u8> {
        self.sequence = self.sequence.wrapping_add(1);
        Run {
            flags: 0,
            theirs: self.theirs,
            ours: self.ours,
            sequence: self.sequence,
            acknowledged: self.heard,
            slots: Vec::new(),
        }
        .build()
    }

    /// Says something, if there is credit left to say it with, granting
    /// `grant` slots back to veetee as it goes.
    fn speak(&mut self, text: &[u8], grant: u8) -> Option<Vec<u8>> {
        if self.credit < 1 {
            return None;
        }
        self.credit -= 1;
        self.granted += i32::from(grant);
        self.sequence = self.sequence.wrapping_add(1);
        Some(
            Run {
                flags: 0,
                theirs: self.theirs,
                ours: self.ours,
                sequence: self.sequence,
                acknowledged: self.heard,
                // Type zero is session data, granting nothing back: OpenVMS
                // grants nothing on most slots and a session carries on.
                slots: vec![Slot {
                    to: 1,
                    from: 1,
                    control: grant,
                    data: text,
                }],
            }
            .build(),
        )
    }
}

/// A session opened against the host above, and the buffer it reads into.
fn opened() -> (Session, Host, Vec<u8>) {
    let mut session = Session::new(SessionConfig {
        node: "MYI64".into(),
        service: "MYI64".into(),
        from: "VEETEE".into(),
        rows: 24,
        cols: 80,
        address: [0xaa, 0x00, 0x04, 0x00, 0x01, 0x04],
    });
    session.call();
    let call = session.take_outgoing();
    let (mut host, reply) = Host::agree(&call[0]);
    let mut data = Vec::new();
    assert_eq!(session.receive(&reply, &mut data), Event::Opened);
    for frame in session.take_outgoing() {
        host.hear(&frame);
    }
    assert!(
        host.credit > 0,
        "the slot asking for a service grants the first allowance"
    );
    (session, host, data)
}

/// Hands everything veetee has queued to the host.
fn flush(session: &mut Session, host: &mut Host) {
    for frame in session.take_outgoing() {
        host.hear(&frame);
    }
}

/// How many messages to run. Enough to cross the one-byte sequence number
/// hundreds of times, which is the state an hour-long session reaches.
const MESSAGES: usize = 100_000;

#[test]
fn a_long_session_in_order() {
    let (mut session, mut host, mut data) = opened();
    let mut expected = Vec::new();

    for i in 0..MESSAGES {
        let line = format!("line {i}\r\n");
        let frame = host
            .speak(line.as_bytes(), 0)
            .unwrap_or_else(|| panic!("the host ran out of credit at message {i}"));
        assert_eq!(session.receive(&frame, &mut data), Event::Data);
        flush(&mut session, &mut host);
        expected.extend_from_slice(line.as_bytes());
        assert!(!session.is_closed(), "the session ended itself at {i}");
    }

    let stats = session.stats();
    assert_eq!(
        data.len(),
        expected.len(),
        "read {} bytes of {}",
        data.len(),
        expected.len()
    );
    assert!(
        data == expected,
        "every byte the host said, once and in order"
    );
    assert_eq!(stats.rewinds, 0, "nothing arrived out of order");
    assert_eq!(stats.duplicates, 0, "and nothing arrived twice");
    assert!(
        stats.wraps_in > 300 && stats.wraps_out > 300,
        "both ends should have crossed the wrap hundreds of times: {}",
        stats.summary()
    );
    assert!(
        stats.max_unacked <= 2,
        "the host acknowledged as it went: {}",
        stats.summary()
    );
}

#[test]
fn the_far_ends_acknowledgements_are_not_read_as_losses() {
    let (mut session, mut host, mut data) = opened();

    // A real host acknowledges as it goes, and every acknowledgement carries
    // a sequence number like any other message. A session that follows the
    // numbering of only the messages with slots in them reads each of these
    // as a message lost -- and a loss believed is credit granted to make up
    // for it, which is an empty slot sent for every phantom. On a real
    // session that came to 165 phantom losses and 1361 credits granted
    // against 107 received.
    for i in 0..MESSAGES {
        let line = format!("line {i}");
        if let Some(frame) = host.speak(line.as_bytes(), 0) {
            session.receive(&frame, &mut data);
            flush(&mut session, &mut host);
        }
        let ack = host.ack();
        assert_eq!(
            session.receive(&ack, &mut data),
            Event::Housekeeping,
            "an acknowledgement carries nothing for the terminal"
        );
        flush(&mut session, &mut host);
    }

    let stats = session.stats();
    assert_eq!(
        stats.missed,
        0,
        "nothing was lost here at all: {}",
        stats.summary()
    );
    assert!(
        stats.acks_in > MESSAGES as u64 / 2,
        "the acknowledgements were seen: {}",
        stats.summary()
    );
}

#[test]
fn typing_stays_within_the_allowance() {
    let (mut session, mut host, mut data) = opened();
    let mut typed = Vec::new();

    // The host asserts, on every slot it receives, that veetee has not sent
    // past what it was granted: a node that does has its messages dropped,
    // and the far end then stops acknowledging and takes the circuit down
    // without a word. This is what that looked like in the field.
    for i in 0..20_000usize {
        let line = format!("line {i}");
        if let Some(frame) = host.speak(line.as_bytes(), 2) {
            session.receive(&frame, &mut data);
            flush(&mut session, &mut host);
        }
        let key = [b'a' + u8::try_from(i % 26).unwrap()];
        session.write(&key);
        typed.extend_from_slice(&key);
        flush(&mut session, &mut host);
    }

    assert!(
        !session.holding(),
        "the allowance kept up: {}",
        session.stats().summary()
    );
    assert_eq!(
        host.typed.len(),
        typed.len(),
        "every keystroke arrived: {} of {}",
        host.typed.len(),
        typed.len()
    );
    assert!(host.typed == typed, "and in the order they were typed");
}

#[test]
fn a_repeated_message_is_not_read_twice() {
    let (mut session, mut host, mut data) = opened();
    let mut expected = Vec::new();

    // A host that has not heard an acknowledgement says it again. Every
    // hundredth message here is delivered twice, which is what a lost
    // acknowledgement looks like from this end.
    for i in 0..MESSAGES {
        let line = format!("line {i}\r\n");
        let frame = host
            .speak(line.as_bytes(), 0)
            .unwrap_or_else(|| panic!("the host ran out of credit at message {i}"));
        session.receive(&frame, &mut data);
        if i % 100 == 0 {
            session.receive(&frame, &mut data);
        }
        flush(&mut session, &mut host);
        expected.extend_from_slice(line.as_bytes());
    }

    let stats = session.stats();
    assert_eq!(stats.duplicates, (MESSAGES / 100) as u64, "counted");
    // Compared by length first: two hundred thousand bytes of difference
    // printed in full says less than one number does.
    assert_eq!(
        data.len(),
        expected.len(),
        "a repeat is the same message, not more output: read {} bytes of {}",
        data.len(),
        expected.len()
    );
    assert!(data == expected, "and the same bytes in the same order");
}

#[test]
fn a_message_arriving_late_does_not_rewind_the_acknowledgement() {
    let (mut session, mut host, mut data) = opened();

    // Two messages swapped on the wire. The second is read first, so the
    // first arrives older than what has already been heard.
    let first = host.speak(b"first ", 0).expect("credit at the start");
    let second = host.speak(b"second ", 0).expect("credit at the start");
    session.receive(&second, &mut data);
    flush(&mut session, &mut host);
    let acknowledged = host.heard;
    session.receive(&first, &mut data);

    let out = session.take_outgoing();
    for frame in &out {
        let Ok(Message::Run(run)) = vt_lat::parse(frame) else {
            continue;
        };
        assert!(
            !watch::newer(acknowledged, run.acknowledged),
            "acknowledged {} after having acknowledged {acknowledged}",
            run.acknowledged
        );
    }
    assert_eq!(session.stats().rewinds, 1, "counted");
}

#[test]
fn a_lost_message_does_not_wedge_the_session() {
    let (mut session, mut host, mut data) = opened();
    let mut dropped = 0;

    // Nothing retransmits here, so a dropped frame is simply gone. A loss
    // still spends the far end's credit, though, and the only sign of it is
    // the gap left in the numbering: a session that does not read that gap
    // grants one too little for every loss, and stops dead once the shortfall
    // passes what it holds back before granting. At a thousandth of the loss
    // rate below, this used to run out of credit at message 7503.
    for i in 0..MESSAGES {
        let line = format!("line {i}\r\n");
        let frame = host
            .speak(line.as_bytes(), 0)
            .unwrap_or_else(|| panic!("the host ran out of credit at message {i}"));
        if i % 7 == 3 {
            dropped += 1;
            continue;
        }
        session.receive(&frame, &mut data);
        flush(&mut session, &mut host);
        assert!(!session.is_closed(), "the session ended itself at {i}");
    }

    let stats = session.stats();
    assert!(dropped > MESSAGES / 8, "a seventh of them: {dropped}");
    assert_eq!(
        stats.missed,
        dropped as u64,
        "every loss accounted for: {}",
        stats.summary()
    );
    assert!(!session.is_closed(), "still open: {}", stats.summary());
    assert!(
        stats.rewinds == 0 && stats.duplicates == 0,
        "a loss is neither of those: {}",
        stats.summary()
    );
}
