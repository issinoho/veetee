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

/// What MYI64 reports as its queue limit, and the distance past which it
/// stopped accepting anything veetee sent.
const QUEUE_LIMIT: u8 = 24;

/// The far end of a circuit: numbering, acknowledging and spending credit.
struct Host {
    /// The identifiers, each end naming the other's first.
    ours: u16,
    theirs: u16,
    sequence: u8,
    /// The newest number heard from veetee.
    heard: u8,
    /// The newest of this end's own numbers that veetee has acknowledged.
    /// A real node waits on this before sending anything further.
    acknowledged: u8,
    /// The newest of veetee's numbers this end has acknowledged, which is the
    /// newest that carried slots: an acknowledgement is not acknowledged.
    acked: u8,
    /// What veetee has granted and this end has not spent. A slot costs one.
    credit: i32,
    /// What this end has granted veetee and veetee has not spent. A node that
    /// sends past its allowance has its messages dropped by a real host, so
    /// this one refuses to let it happen quietly.
    granted: i32,
    /// What veetee has sent, to compare with what was typed.
    typed: Vec<u8>,
    /// Take veetee's messages only in order, as OpenVMS does: once one has
    /// gone missing, everything after it is discarded until it arrives.
    strict: bool,
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
            acknowledged: 0,
            acked: 0,
            credit: 0,
            granted: 0,
            typed: Vec::new(),
            strict: false,
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
                if watch::newer(run.acknowledged, self.acknowledged) {
                    self.acknowledged = run.acknowledged;
                }
                // Out of order, or already taken: its slots are not read and
                // its credit not counted. A sender that does not send the
                // missing message again is discarded from here on.
                //
                // Nor does it take an acknowledgement's number as a message
                // received: veetee's acknowledgements take no number of their
                // own, and a host that acknowledged whatever number they
                // carried would acknowledge messages it had discarded.
                if self.strict {
                    if run.slots.is_empty() {
                        return;
                    }
                    if run.sequence != self.acked.wrapping_add(1) {
                        return;
                    }
                }
                if watch::newer(run.sequence, self.heard) {
                    self.heard = run.sequence;
                }
                if watch::newer(run.acknowledged, self.acknowledged) {
                    self.acknowledged = run.acknowledged;
                }
                // Only a message with slots in it is acknowledged, and one
                // more than the queue limit ahead of the last acknowledged is
                // not accepted at all. A caller that takes a new number for
                // every keepalive runs away from this and the session dies.
                if !run.slots.is_empty() {
                    self.acked = run.sequence;
                }
                let ahead = run.sequence.wrapping_sub(self.acked);
                assert!(
                    ahead <= QUEUE_LIMIT,
                    "veetee is {ahead} ahead of the last acknowledged; the limit is {QUEUE_LIMIT}"
                );
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
    // The far end numbers every message; this end numbers only the ones
    // carrying slots, a bare acknowledgement taking no number of its own, so
    // its sequence crosses the wrap far more often than ours does.
    assert!(
        stats.wraps_in > 300 && stats.wraps_out > 10,
        "both ends should have crossed the wrap: {}",
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
fn a_far_end_waiting_to_be_acknowledged_is_not_left_waiting() {
    let (mut session, mut host, mut data) = opened();

    // The host says something with no slots in it and, like a real one, will
    // not send anything else until it hears that number back. A session that
    // takes its numbering only from the messages carrying slots never sends
    // it, and the two ends then acknowledge stale numbers at each other for
    // as long as anybody is willing to watch.
    for round in 0..50 {
        let ack = host.ack();
        session.receive(&ack, &mut data);
        session.keepalive();
        flush(&mut session, &mut host);
        assert_eq!(
            host.acknowledged, host.sequence,
            "round {round}: the host is still waiting to hear {} back",
            host.sequence
        );
    }
}

#[test]
fn a_long_idle_spell_does_not_run_away_from_the_far_end() {
    let (mut session, mut host, mut data) = opened();

    // Nobody says anything for a long while and both ends keep the circuit
    // up. A session that takes a new sequence number for every keepalive runs
    // away from a far end that acknowledges only what carries slots: the gap
    // grows by one every ten seconds whatever else happens, and past the
    // queue limit MYI64 stopped accepting anything at all -- typing went
    // unechoed, SET TERM/INQUIRE timed out into "unknown terminal type", and
    // the terminal was dead a few minutes into every session.
    for _ in 0..500 {
        session.keepalive();
        flush(&mut session, &mut host);
        let ack = host.ack();
        session.receive(&ack, &mut data);
        flush(&mut session, &mut host);
    }

    // And after all that, the far end still takes what is typed.
    let frame = host.speak(b"$ ", 15).expect("credit to prompt with");
    session.receive(&frame, &mut data);
    flush(&mut session, &mut host);
    session.write(b"SHOW TIME\r");
    flush(&mut session, &mut host);
    assert_eq!(
        host.typed, b"SHOW TIME\r",
        "the far end is still listening after a long idle spell"
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

    // Nothing retransmits here, so a dropped frame is simply gone, and the
    // session has to give each gap up rather than wait for it for ever. A loss
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
    // A gap is waited on until eight messages have come after it, this host
    // never sending the missing one again; the last loss or two are still
    // being waited on when the messages stop.
    let pending = dropped as u64 - stats.missed;
    assert!(
        pending <= 2,
        "every loss accounted for but the last few still awaited: {pending}; {}",
        stats.summary()
    );
    assert!(!session.is_closed(), "still open: {}", stats.summary());
    assert!(
        stats.rewinds == 0 && stats.duplicates == 0,
        "a loss is neither of those: {}",
        stats.summary()
    );
}

#[test]
fn a_message_of_ours_lost_on_the_way_is_sent_again() {
    let (mut session, mut host, mut data) = opened();
    host.strict = true;
    let mut typed = Vec::new();
    let mut frames = 0usize;
    let mut lost = 0usize;
    // One of veetee's frames in seven never arrives. Without sending again
    // the host would take nothing after the first of them, which is what
    // happened over Wi-Fi within minutes of a login.
    let mut deliver = |session: &mut Session, host: &mut Host| {
        for frame in session.take_outgoing() {
            frames += 1;
            if frames.is_multiple_of(7) {
                lost += 1;
                continue;
            }
            host.hear(&frame);
        }
    };
    for i in 0..20_000usize {
        if let Some(frame) = host.speak(b"x", 2) {
            session.receive(&frame, &mut data);
        }
        let key = [b'a' + u8::try_from(i % 26).unwrap()];
        session.write(&key);
        typed.push(key[0]);
        deliver(&mut session, &mut host);
        let ack = host.ack();
        session.receive(&ack, &mut data);
        // The transport's timer: a second without progress, in the real
        // thing. Here, every few messages.
        if i % 4 == 0 {
            session.retransmit();
        }
        deliver(&mut session, &mut host);
        assert!(
            !session.is_closed(),
            "the session gave up at {i}: {}",
            session.stats().summary()
        );
    }
    // Whatever is still in the air is sent again until it is taken.
    for _ in 0..100 {
        if !session.awaiting() && !session.holding() {
            break;
        }
        if let Some(frame) = host.speak(b"x", 2) {
            session.receive(&frame, &mut data);
        }
        session.retransmit();
        deliver(&mut session, &mut host);
        let ack = host.ack();
        session.receive(&ack, &mut data);
    }
    let stats = session.stats();
    assert!(lost > 1000, "a seventh of veetee's frames lost: {lost}");
    assert!(stats.retransmitted > 0, "{}", stats.summary());
    assert_eq!(
        host.typed.len(),
        typed.len(),
        "every keystroke arrived: {} of {}; {}",
        host.typed.len(),
        typed.len(),
        stats.summary()
    );
    if let Some(at) = host.typed.iter().zip(&typed).position(|(a, b)| a != b) {
        panic!(
            "first difference at {at}: arrived {:?}, typed {:?}",
            String::from_utf8_lossy(&host.typed[at.saturating_sub(5)..(at + 10).min(typed.len())]),
            String::from_utf8_lossy(&typed[at.saturating_sub(5)..(at + 10).min(typed.len())])
        );
    }
}

#[test]
fn a_host_that_takes_nothing_more_ends_the_session_rather_than_freezing_it() {
    let (mut session, mut host, mut data) = opened();
    host.strict = true;
    // The host prompts, so veetee will send what is typed.
    let prompt = host.speak(b"$ ", 2).expect("credit at the start");
    session.receive(&prompt, &mut data);
    session.take_outgoing();
    session.write(b"lost");
    // The message never arrives, and nothing sent again ever does either.
    session.take_outgoing();
    let mut tries = 0;
    while !session.is_closed() {
        session.retransmit();
        session.take_outgoing();
        let ack = host.ack();
        session.receive(&ack, &mut data);
        tries += 1;
        assert!(tries < 1000, "it never gave up");
    }
    assert_eq!(
        session.ending(),
        "the host stopped accepting what veetee sends"
    );
}

#[test]
fn a_message_of_the_hosts_that_goes_missing_is_waited_for_and_read_in_its_place() {
    let (mut session, mut host, mut data) = opened();
    let mut expected = Vec::new();
    let mut held_back: Option<Vec<u8>> = None;
    // Every tenth message of the host's is lost the first time, and arrives
    // three messages later, sent again, as a host sends what has not been
    // acknowledged. Each grants one credit, which is how OpenVMS answers.
    for i in 0..MESSAGES {
        let line = format!("line {i}\r\n");
        expected.extend_from_slice(line.as_bytes());
        let frame = host
            .speak(line.as_bytes(), 1)
            .unwrap_or_else(|| panic!("the host ran out of credit at message {i}"));
        if i % 10 == 5 {
            held_back = Some(frame);
        } else {
            session.receive(&frame, &mut data);
        }
        if i % 10 == 8
            && let Some(frame) = held_back.take()
        {
            session.receive(&frame, &mut data);
        }
        flush(&mut session, &mut host);
        // Typing spends what the host grants, as a transfer does.
        session.write(b"k");
        flush(&mut session, &mut host);
    }
    let stats = session.stats();
    assert_eq!(stats.missed, 0, "nothing given up: {}", stats.summary());
    assert_eq!(data.len(), expected.len(), "{}", stats.summary());
    assert!(data == expected, "every line, once, in order");
    assert!(
        stats.credit_ours >= 0 && !session.holding(),
        "the credit in the late messages was read, and typing never ran dry: {}",
        stats.summary()
    );
}
