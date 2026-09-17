//! Driving one LAT session: frames in, frames out, and no sockets.
//!
//! [`Session`] is the client half of a circuit — asking a node for one, asking
//! it for a service, acknowledging what arrives, carrying what is typed — with
//! the datalink left to `vt-transport`, which has the raw socket. Keeping the
//! two apart is what lets the whole of it be tested against captured frames on
//! any platform, LAT being Linux only in practice.
//!
//! The sequence rules are in `docs/lat-protocol.md`: this end's number counts
//! up with every message sent, the acknowledgement is the highest number heard
//! from the far end, and a message with no slots carries nothing to
//! acknowledge and so is answered with nothing.

use std::sync::atomic::{AtomicU16, Ordering};

use crate::{Message, Run, Slot, Start, Stop, session_start};

/// A slot's control byte: the type in the high nibble, credit in the low.
///
/// Read from a login to OpenVMS. Type 9 starts a session — `0x9f` named the
/// `LTA` device the host had created, and is what veetee sends to ask for a
/// service. Type 10 carries a block of terminal parameters, seen as `0xa0`,
/// `0xa1` and `0xaf`: the same thirty-five bytes each time, so what varies is
/// the low nibble and not the meaning. Type 0 is session data, seen as `0x00`,
/// `0x01`, `0x03` and `0x0f` with the text coming through regardless.
///
/// A number that changes while the meaning does not is credit, which is the
/// way round DEC documents it and the opposite of what was first read here.
/// Only type zero reaches the terminal.
const SLOT_KIND: u8 = 0xf0;
const SLOT_DATA: u8 = 0x00;

/// One length byte counts a slot's data, so this is as much as one carries.
const MAX_SLOT: usize = 255;

/// As much credit as the low nibble of a control byte will hold.
///
/// A slot spends one of the credits the far end has been given, and a node
/// with none left stops sending: MYI64 broke off mid-word in the middle of a
/// system description, then acknowledged politely for as long as it was
/// asked to, having been granted fifteen at the start of the session and
/// nothing since.
const MAX_CREDIT: u8 = 15;

/// Grant more once the far end is down to this much, so that it is never
/// left waiting between one grant and the next.
///
/// 🔎 Granting is read as adding to what the far end has rather than
/// replacing it, because OpenVMS grants zero on most slots and a session
/// carries on through them. How much a node ought to grant, and what it
/// spends credit on besides a slot, are not read.
const LOW_CREDIT: u8 = MAX_CREDIT / 2;

/// The first identifier this end gives its own side of a circuit. Any number
/// will do — the far end simply quotes it back — and this one is veetee's.
const FIRST_CIRCUIT: u16 = 0x1001;

/// The identifier for the next session, so that two sessions from one node
/// cannot be taken for each other: a window holds two, and the far end tells
/// circuits apart by nothing else.
fn next_circuit() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(FIRST_CIRCUIT);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    // A start message sends zero for an end it cannot name yet, so zero is
    // never an identifier of ours. Wrapping round to the first one again
    // takes 65,535 sessions, by which time the first is long gone.
    if id == 0 { FIRST_CIRCUIT } else { id }
}

/// The flag bits veetee sends on every run message.
///
/// 🔎 The calling node sent `0x02` throughout the captured session and the
/// answering node `0x00` and `0x01`, so this is the caller's value; which bit
/// means what is unread.
const CALLER_FLAGS: u8 = 2;

/// Who to call, what to ask for, and what to say this end is.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// The node offering the service, as it announces itself.
    pub node: String,
    /// The service wanted. A node's own name is usually one of its services.
    pub service: String,
    /// The name this end goes by on the circuit.
    pub from: String,
    /// The page size, which the slot asking for a service carries.
    pub rows: u16,
    pub cols: u16,
    /// This interface's own Ethernet address, which a start message carries.
    pub address: [u8; 6],
}

/// What a received frame turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// No part of this session: another node's circuit, an announcement, a
    /// solicit, or something unreadable.
    Ignored,
    /// The far end agreed the circuit, and the service has been asked for.
    Opened,
    /// Session data, appended to the caller's buffer.
    Data,
    /// The circuit's own business: an acknowledgement, a keepalive, or a slot
    /// that is not session data.
    Housekeeping,
    /// The circuit is down and the session is finished.
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing sent yet.
    New,
    /// A start message has gone; waiting for the far end to agree.
    Calling,
    /// The circuit is open and a service has been asked for, but the far end
    /// has said nothing yet — and it will not read before it has prompted.
    Asked,
    /// The far end has spoken, so it is listening too.
    Open,
    /// Finished, whichever end ended it.
    Closed,
}

/// One LAT session: a circuit to a node, and a terminal on a service of it.
///
/// Frames arrive through [`Session::receive`] and leave through
/// [`Session::take_outgoing`]. Nothing here touches a socket or a clock, so
/// when to send and when a keepalive is due are the caller's business.
#[derive(Debug)]
pub struct Session {
    config: SessionConfig,
    state: State,
    /// This end's circuit identifier, and the far end's once it names it.
    ours: u16,
    theirs: u16,
    /// The number on the last message sent, and the highest one heard.
    sequence: u8,
    heard: u8,
    /// The slot numbers the two ends use. Both were 1 in the captured
    /// session; the far end's is relearned from the first slot it addresses
    /// to us.
    local_slot: u8,
    remote_slot: u8,
    /// How much credit the far end has left: it spends one on every slot it
    /// sends, and stops sending when it has none.
    credit: u8,
    /// Frames waiting to be sent.
    outgoing: Vec<Vec<u8>>,
    /// Typing that arrived before the far end had prompted, which it would
    /// have ignored. Held back rather than dropped, and sent once it speaks.
    early: Vec<u8>,
}

impl Session {
    /// A session that has not called yet.
    pub fn new(config: SessionConfig) -> Session {
        Session {
            config,
            state: State::New,
            ours: next_circuit(),
            theirs: 0,
            sequence: 0,
            heard: 0,
            local_slot: 1,
            remote_slot: 1,
            credit: 0,
            outgoing: Vec::new(),
            early: Vec::new(),
        }
    }

    /// Asks the node for a circuit, which is the first thing on the wire.
    ///
    /// Sent again if nothing answers: a lost start is the caller's to notice,
    /// there being no timer here.
    pub fn call(&mut self) {
        let start = Start {
            calling: true,
            // The far end cannot be named until it names itself.
            theirs: 0,
            ours: self.ours,
            max_frame: 1500,
            version: (5, 3),
            keepalive: 20,
            to: &self.config.node,
            from: &self.config.from,
        };
        self.outgoing.push(start.build(self.config.address));
        self.state = State::Calling;
    }

    /// Reads a frame's payload, appending any session data to `data` and
    /// queueing whatever it calls for in reply.
    pub fn receive(&mut self, payload: &[u8], data: &mut Vec<u8>) -> Event {
        let Ok(message) = crate::parse(payload) else {
            return Event::Ignored;
        };
        match message {
            Message::Start(reply) => self.agreed(&reply),
            Message::Run(run) => self.run(&run, data),
            Message::Stop(stop) => self.stopped(stop),
            // Announcements, solicits and the types nobody has read are no
            // part of a session. The far end solicits whoever calls it, which
            // is one of these arriving mid-session and is safely nothing.
            Message::Announcement(_) | Message::Solicit(_) | Message::Other { .. } => {
                Event::Ignored
            }
        }
    }

    /// Queues what is typed, in slots of as much as a slot will carry.
    ///
    /// Held back until the far end has said something: it will not read before
    /// it has prompted, and a slot sent earlier is ignored rather than queued.
    pub fn write(&mut self, data: &[u8]) {
        match self.state {
            State::Open => {
                for chunk in data.chunks(MAX_SLOT) {
                    let frame = self.data_frame(chunk);
                    self.outgoing.push(frame);
                }
            }
            State::New | State::Calling | State::Asked => self.early.extend_from_slice(data),
            State::Closed => {}
        }
    }

    /// Queues the acknowledgement an idle circuit sends, which is what keeps
    /// it open. Due after each keepalive interval of silence; the interval is
    /// the caller's business, as the clock is.
    pub fn keepalive(&mut self) {
        if self.is_open() {
            let frame = self.acknowledge();
            self.outgoing.push(frame);
        }
    }

    /// Queues the message that takes the circuit down, so the far end releases
    /// the terminal it created rather than waiting out its own timer.
    ///
    /// Nothing is sent for a circuit the far end never agreed to: there is
    /// nothing at its end to take down, and no identifier to name it by.
    pub fn close(&mut self) {
        if self.is_open() {
            self.outgoing.push(
                Stop {
                    theirs: self.theirs,
                    ours: self.ours,
                }
                .build(),
            );
        }
        self.state = State::Closed;
    }

    /// The frames waiting to be sent, taken away.
    pub fn take_outgoing(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.outgoing)
    }

    /// True once the far end has agreed the circuit and been asked for a
    /// service, whether or not it has said anything yet.
    pub fn is_open(&self) -> bool {
        matches!(self.state, State::Asked | State::Open)
    }

    /// True once the circuit is down, whichever end ended it.
    pub fn is_closed(&self) -> bool {
        self.state == State::Closed
    }

    /// The service this session asked for.
    pub fn service(&self) -> &str {
        &self.config.service
    }

    /// The far end agreeing to a circuit. Its reply fills in both identifiers,
    /// so this is where the session learns what to call the far end.
    ///
    /// The first message back does two things at once, as OpenVMS does at this
    /// point: acknowledges the agreement and asks for a service.
    fn agreed(&mut self, reply: &Start<'_>) -> Event {
        // A node calling us is not ours to answer, veetee offering no
        // services; nor is an agreement to a circuit we never asked for,
        // which is how another veetee on the same wire is told from this one.
        if reply.calling || self.state != State::Calling || reply.theirs != self.ours {
            return Event::Ignored;
        }
        self.theirs = reply.ours;
        let data = session_start(&self.config.service, self.config.rows, self.config.cols);
        let frame = self.slot_frame(&[Slot {
            // It has not named a slot of its own yet.
            to: 0,
            from: self.local_slot,
            control: crate::SLOT_START,
            data: &data,
        }]);
        self.outgoing.push(frame);
        // SLOT_START carries fifteen in its low nibble, which is the whole of
        // the far end's allowance until this end grants more.
        self.credit = MAX_CREDIT;
        self.state = State::Asked;
        Event::Opened
    }

    /// A message on the circuit. Everything after the start arrives in these.
    fn run(&mut self, run: &Run<'_>, data: &mut Vec<u8>) -> Event {
        // Each end names the other's identifier first and its own second, so
        // a message of ours has them the other way round. That is what tells
        // another node's circuit on the same wire from the traffic this
        // session wants.
        if !self.is_open() || run.ours != self.theirs || run.theirs != self.ours {
            return Event::Ignored;
        }
        // Answer only what carries slots. Acknowledging an acknowledgement
        // draws another back, and the two ends then answer each other for
        // ever.
        if run.slots.is_empty() {
            return Event::Housekeeping;
        }
        if run.sequence != self.heard {
            self.heard = run.sequence;
            let frame = self.acknowledge();
            self.outgoing.push(frame);
        }
        let before = data.len();
        for slot in &run.slots {
            if slot.to != self.local_slot {
                continue;
            }
            // Whatever it addresses to us comes from a slot of its own —
            // except when it comes from slot zero, which is the circuit
            // talking rather than the session, as the slot that asks for a
            // service is addressed to zero before either end has named one.
            // MYI64 sends one from zero as it ends a login, and taking that
            // for the far end's slot would address everything typed after it
            // to nothing.
            if slot.from != 0 {
                self.remote_slot = slot.from;
            }
            if slot.control & SLOT_KIND == SLOT_DATA {
                data.extend_from_slice(slot.data);
            }
        }
        self.spend(run.slots.len());
        if data.len() == before {
            return Event::Housekeeping;
        }
        // It has spoken, so it is reading: anything typed early can go now.
        if self.state == State::Asked {
            self.state = State::Open;
            let early = std::mem::take(&mut self.early);
            self.write(&early);
        }
        Event::Data
    }

    /// The far end taking the circuit down.
    fn stopped(&mut self, stop: Stop) -> Event {
        // 🔎 The one stop message ever captured named the receiving end and
        // sent zero for its own, so either identifier matching is taken as
        // enough; the rest of the message is unread.
        if stop.theirs != self.ours && (stop.ours == 0 || stop.ours != self.theirs) {
            return Event::Ignored;
        }
        self.state = State::Closed;
        Event::Closed
    }

    /// A run message carrying one slot of session data, and any credit that
    /// has fallen due with it.
    fn data_frame(&mut self, data: &[u8]) -> Vec<u8> {
        // Type zero is session data, against SLOT_START for the slot that
        // asks for a service; the low nibble is what the far end may spend.
        let control = SLOT_DATA | self.grant();
        self.slot_frame(&[Slot {
            to: self.remote_slot,
            from: self.local_slot,
            control,
            data,
        }])
    }

    /// Counts what the far end has spent, and grants more before it runs out.
    ///
    /// A terminal has nothing to say for as long as the user is reading, so
    /// the grant cannot wait for something to carry it: OpenVMS sends empty
    /// slots for this, and so does veetee.
    fn spend(&mut self, slots: usize) {
        self.credit = self
            .credit
            .saturating_sub(u8::try_from(slots).unwrap_or(u8::MAX));
        if self.credit <= LOW_CREDIT {
            let control = SLOT_DATA | self.grant();
            let frame = self.slot_frame(&[Slot {
                to: self.remote_slot,
                from: self.local_slot,
                control,
                data: &[],
            }]);
            self.outgoing.push(frame);
        }
    }

    /// What to put in the low nibble of the next slot sent: enough to bring
    /// the far end back to a full allowance.
    fn grant(&mut self) -> u8 {
        let grant = MAX_CREDIT - self.credit;
        self.credit = MAX_CREDIT;
        grant
    }

    /// A run message with nothing in it, which says only what has been heard.
    fn acknowledge(&mut self) -> Vec<u8> {
        self.slot_frame(&[])
    }

    /// Builds a run message, counting this end's sequence number up as every
    /// message sent does, acknowledgements included.
    fn slot_frame(&mut self, slots: &[Slot<'_>]) -> Vec<u8> {
        self.sequence = self.sequence.wrapping_add(1);
        Run {
            flags: CALLER_FLAGS,
            theirs: self.theirs,
            ours: self.ours,
            sequence: self.sequence,
            acknowledged: self.heard,
            slots: slots.to_vec(),
        }
        .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{KEEPALIVE, PROMPT, START_REPLY_FRAME, X86VMS_SOLICIT};

    /// A session about to call MYI64, named as the captured one was.
    ///
    /// The frames below come from a real session between two OpenVMS nodes, in
    /// which the caller named its end `0x7001` and MYI64 quotes that back.
    /// veetee picks an identifier of its own for every session, so the test
    /// borrows the captured one to be talking about the same circuit.
    fn calling() -> Session {
        let mut session = Session::new(SessionConfig {
            node: "MYI64".into(),
            service: "MYI64".into(),
            from: "VEETEE".into(),
            rows: 24,
            cols: 80,
            address: [0xaa, 0x00, 0x04, 0x00, 0x01, 0x04],
        });
        session.ours = 0x7001;
        session
    }

    /// A session as far as the login prompt, which is the state most of these
    /// tests want, and the buffer the terminal reads from.
    fn agreed() -> (Session, Vec<u8>) {
        let mut session = calling();
        session.call();
        session.take_outgoing();
        let mut data = Vec::new();
        session.receive(START_REPLY_FRAME, &mut data);
        session.take_outgoing();
        (session, data)
    }

    #[test]
    fn two_sessions_do_not_share_a_circuit_identifier() {
        let config = SessionConfig {
            node: "MYI64".into(),
            service: "MYI64".into(),
            from: "VEETEE".into(),
            rows: 24,
            cols: 80,
            address: [0; 6],
        };
        let first = Session::new(config.clone()).ours;
        let second = Session::new(config).ours;
        assert_ne!(first, second, "a window holds two sessions");
        assert!(first != 0 && second != 0, "zero means an end not yet named");
    }

    #[test]
    fn asks_for_a_circuit() {
        let mut session = calling();
        session.call();
        let out = session.take_outgoing();
        assert_eq!(out.len(), 1);
        let Ok(Message::Start(call)) = crate::parse(&out[0]) else {
            panic!("not a start")
        };
        assert!(call.calling);
        assert_eq!((call.to, call.from), ("MYI64", "VEETEE"));
        assert_eq!(call.ours, 0x7001);
        assert_eq!(call.theirs, 0, "the far end has not named itself yet");
        assert!(!session.is_open());
    }

    #[test]
    fn asks_for_a_service_once_the_circuit_is_agreed() {
        let mut session = calling();
        session.call();
        session.take_outgoing();

        let mut data = Vec::new();
        assert_eq!(
            session.receive(START_REPLY_FRAME, &mut data),
            Event::Opened,
            "the answer MYI64 itself sent"
        );
        assert!(data.is_empty(), "a start carries no session data");
        assert!(session.is_open());

        let out = session.take_outgoing();
        assert_eq!(
            out.len(),
            1,
            "the agreement is acknowledged and the service asked for together"
        );
        let Ok(Message::Run(open)) = crate::parse(&out[0]) else {
            panic!("not a run")
        };
        assert_eq!(
            (open.theirs, open.ours),
            (0xe001, 0x7001),
            "the far end named first, as every message on a circuit does"
        );
        assert_eq!(open.sequence, 1, "the first message on the circuit");
        assert_eq!(open.slots.len(), 1);
        assert_eq!(open.slots[0].control, crate::SLOT_START);
        assert_eq!(open.slots[0].data, crate::session_start("MYI64", 24, 80));
    }

    #[test]
    fn a_circuit_someone_else_asked_for_is_not_ours() {
        let mut session = calling();
        session.ours = 0x1234; // not the circuit the captured reply answers
        session.call();
        session.take_outgoing();
        let mut data = Vec::new();
        assert_eq!(
            session.receive(START_REPLY_FRAME, &mut data),
            Event::Ignored,
            "another veetee on the same wire is told apart by this"
        );
        assert!(session.take_outgoing().is_empty());
        assert!(!session.is_open());
    }

    #[test]
    fn the_prompt_is_read_and_acknowledged() {
        let (mut session, mut data) = agreed();
        assert_eq!(session.receive(PROMPT, &mut data), Event::Data);
        assert_eq!(
            data, b"\n\r\n\rUsername: ",
            "the slot beside it is the circuit business, not the terminal data"
        );

        let out = session.take_outgoing();
        assert_eq!(out.len(), 1);
        let Ok(Message::Run(ack)) = crate::parse(&out[0]) else {
            panic!("not a run")
        };
        assert!(ack.slots.is_empty(), "an acknowledgement carries nothing");
        assert_eq!(ack.acknowledged, 3, "the highest sequence heard");
        assert_eq!(ack.sequence, 2, "ours counts up with every message sent");
    }

    #[test]
    fn an_acknowledgement_is_not_acknowledged() {
        let (mut session, mut data) = agreed();
        assert_eq!(
            session.receive(KEEPALIVE, &mut data),
            Event::Housekeeping,
            "the keepalive of an idle circuit"
        );
        assert!(
            session.take_outgoing().is_empty(),
            "answering one draws another back, for ever"
        );
        assert!(data.is_empty());
    }

    #[test]
    fn the_same_message_twice_is_acknowledged_once() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        assert_eq!(session.take_outgoing().len(), 1);
        // The far end repeats whatever it has not heard acknowledged.
        assert_eq!(session.receive(PROMPT, &mut data), Event::Data);
        assert!(
            session.take_outgoing().is_empty(),
            "that sequence number has been heard already"
        );
    }

    #[test]
    fn another_circuit_on_the_wire_is_not_ours() {
        let (mut session, mut data) = agreed();
        let mut elsewhere = PROMPT.to_vec();
        elsewhere[4..6].copy_from_slice(&0x2222u16.to_le_bytes()); // their end
        assert_eq!(session.receive(&elsewhere, &mut data), Event::Ignored);
        assert!(data.is_empty());
        assert!(session.take_outgoing().is_empty());
    }

    #[test]
    fn typing_waits_until_the_far_end_has_prompted() {
        let (mut session, mut data) = agreed();
        session.write(b"SYSTEM\r");
        assert!(
            session.take_outgoing().is_empty(),
            "a slot sent before it prompts is ignored, so it is held back"
        );

        session.receive(PROMPT, &mut data);
        let out = session.take_outgoing();
        assert_eq!(out.len(), 2, "the acknowledgement, and then the typing");
        let Ok(Message::Run(typed)) = crate::parse(&out[1]) else {
            panic!("not a run")
        };
        assert_eq!(typed.slots.len(), 1);
        assert_eq!(typed.slots[0].data, b"SYSTEM\r");
        assert_eq!(
            typed.slots[0].control,
            SLOT_DATA | 2,
            "session data, granting back the two slots the prompt spent"
        );
        assert_eq!(
            (typed.slots[0].to, typed.slots[0].from),
            (1, 1),
            "the slot numbers the far end used"
        );
        assert_eq!(
            typed.sequence, 3,
            "after the start and the prompt were answered"
        );
        assert_eq!(typed.acknowledged, 3);
    }

    #[test]
    fn the_far_end_is_granted_more_credit_before_it_runs_out() {
        let (mut session, mut data) = agreed();
        assert_eq!(
            session.credit, MAX_CREDIT,
            "the slot asking for a service grants a full allowance"
        );

        // Eight slots of the circuit talking, which carry no terminal data
        // but are spent all the same.
        let spent = Run {
            flags: 0,
            theirs: 0x7001,
            ours: 0xe001,
            sequence: 4,
            acknowledged: 3,
            slots: vec![
                Slot {
                    to: 1,
                    from: 1,
                    control: 0xa0,
                    data: &[0xff; 4],
                };
                8
            ],
        }
        .build();
        assert_eq!(session.receive(&spent, &mut data), Event::Housekeeping);
        assert_eq!(session.credit, MAX_CREDIT, "topped back up");

        let out = session.take_outgoing();
        assert_eq!(out.len(), 2, "the acknowledgement, and then the credit");
        let Ok(Message::Run(granted)) = crate::parse(&out[1]) else {
            panic!("not a run")
        };
        assert_eq!(granted.slots.len(), 1);
        assert!(
            granted.slots[0].data.is_empty(),
            "a terminal has nothing to say while the user reads, so the grant
             goes on its own"
        );
        assert_eq!(
            granted.slots[0].control,
            SLOT_DATA | 8,
            "session data, granting back the eight that were spent"
        );
    }

    #[test]
    fn typing_carries_the_credit_that_is_due() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        session.take_outgoing();
        session.credit = MAX_CREDIT - 2;

        session.write(b"x");
        let out = session.take_outgoing();
        let Ok(Message::Run(typed)) = crate::parse(&out[0]) else {
            panic!("not a run")
        };
        assert_eq!(
            typed.slots[0].control,
            SLOT_DATA | 2,
            "a slot on its way carries the grant rather than waiting"
        );
        assert_eq!(typed.slots[0].data, b"x");
    }

    #[test]
    fn data_is_read_whatever_credit_it_carries() {
        let (mut session, mut data) = agreed();
        // A login to MYI64, as it arrives: the echo and the banner carry
        // credit in the low nibble, and the block of terminal parameters
        // beside them carries none but is not data at all.
        let login = Run {
            flags: 0,
            theirs: 0x7001,
            ours: 0xe001,
            sequence: 8,
            acknowledged: 7,
            slots: vec![
                Slot {
                    to: 1,
                    from: 1,
                    control: 0xa0,
                    data: &[0xff; 35],
                },
                Slot {
                    to: 1,
                    from: 1,
                    control: 0x01,
                    data: b"on node MYI64",
                },
                Slot {
                    to: 1,
                    from: 1,
                    control: 0x0f,
                    data: b"!",
                },
                Slot {
                    to: 1,
                    from: 1,
                    control: 0x9f,
                    data: b"LTA5046",
                },
            ],
        }
        .build();
        assert_eq!(session.receive(&login, &mut data), Event::Data);
        assert_eq!(
            data, b"on node MYI64!",
            "type zero is the terminal data, whatever credit goes with it"
        );
    }

    #[test]
    fn the_circuit_slot_is_not_taken_for_the_far_end() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        session.take_outgoing();

        // MYI64 sends a slot from zero as it ends a login: the circuit
        // talking rather than the session, and no slot to answer.
        let circuit = Run {
            flags: 0,
            theirs: 0x7001,
            ours: 0xe001,
            sequence: 9,
            acknowledged: 7,
            slots: vec![Slot {
                to: 1,
                from: 0,
                control: 1,
                data: &[],
            }],
        }
        .build();
        session.receive(&circuit, &mut data);
        session.take_outgoing();

        session.write(b"x");
        let out = session.take_outgoing();
        let Ok(Message::Run(typed)) = crate::parse(&out[0]) else {
            panic!("not a run")
        };
        assert_eq!(
            typed.slots[0].to, 1,
            "typing still goes to the slot the far end speaks from"
        );
    }

    #[test]
    fn more_than_a_slot_will_carry_is_split() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        session.take_outgoing();

        session.write(&[b'x'; MAX_SLOT + 10]);
        let out = session.take_outgoing();
        assert_eq!(out.len(), 2, "one length byte counts a slot of data");
        let lengths: Vec<usize> = out
            .iter()
            .map(|frame| match crate::parse(frame) {
                Ok(Message::Run(run)) => run.slots[0].data.len(),
                _ => panic!("not a run"),
            })
            .collect();
        assert_eq!(lengths, vec![MAX_SLOT, 10]);
    }

    #[test]
    fn a_stop_from_the_far_end_ends_the_session() {
        let (mut session, mut data) = agreed();
        let stop = Stop {
            theirs: 0x7001,
            ours: 0xe001,
        }
        .build();
        assert_eq!(session.receive(&stop, &mut data), Event::Closed);
        assert!(session.is_closed());
        assert!(!session.is_open());

        // Nothing more goes out on a circuit that is down.
        session.write(b"anyone there?");
        session.keepalive();
        assert!(session.take_outgoing().is_empty());
    }

    #[test]
    fn a_circuit_never_agreed_is_not_taken_down() {
        let mut session = calling();
        session.call();
        session.take_outgoing();
        session.close();
        assert!(
            session.take_outgoing().is_empty(),
            "nothing at the far end to take down, and no identifier for it"
        );
        assert!(session.is_closed());
    }

    #[test]
    fn closing_takes_the_circuit_down() {
        let (mut session, _) = agreed();
        session.close();
        let out = session.take_outgoing();
        assert_eq!(out.len(), 1);
        assert_eq!(
            crate::parse(&out[0]),
            Ok(Message::Stop(Stop {
                theirs: 0xe001,
                ours: 0x7001,
            })),
            "so the far end releases the terminal it created"
        );
        assert!(session.is_closed());
        session.close();
        assert!(
            session.take_outgoing().is_empty(),
            "closing twice sends one stop"
        );
    }

    #[test]
    fn an_idle_circuit_is_kept_open() {
        let (mut session, _) = agreed();
        session.keepalive();
        let out = session.take_outgoing();
        assert_eq!(out.len(), 1);
        let Ok(Message::Run(idle)) = crate::parse(&out[0]) else {
            panic!("not a run")
        };
        assert!(idle.slots.is_empty());
        assert_eq!(idle.sequence, 2, "counted up from the service request");
    }

    #[test]
    fn a_solicit_mid_session_is_nothing_to_answer() {
        let (mut session, mut data) = agreed();
        // A LAT node solicits whoever calls it, repeatedly.
        assert_eq!(session.receive(X86VMS_SOLICIT, &mut data), Event::Ignored);
        assert!(session.take_outgoing().is_empty());
    }
}
