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

use crate::watch::{self, Stats};
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

/// The type of slot that says the session is over.
///
/// It comes from slot zero with nothing in it, as the last slot the host
/// sends: seen when OpenVMS timed a login out for want of a password, and
/// again on `LOGOUT`, after which MYI64 said nothing but what it had heard.
/// 🔎 Type 11 arrives beside it carrying a single `@`, which is unread.
const SLOT_END: u8 = 0xd0;

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

/// Messages read before the far end's allowance is worked out from scratch
/// rather than adjusted.
///
/// Small enough that a session recovers in seconds, and large enough that in
/// ordinary running it never fires: granting already happens about every eight
/// messages, so this only ever catches a reckoning that has gone wrong.
const REGRANT: u32 = 32;

/// The identifier this end falls back on for its side of a circuit.
const FIRST_CIRCUIT: u16 = 0x1001;

/// Where this run of veetee starts numbering its circuits.
///
/// Not a constant, and that matters: a host remembers a circuit until it
/// times out, so one veetee that was killed rather than closed leaves MYI64
/// to take its circuit down on its own, and the stop that follows names an
/// identifier. Were every run to begin at the same number, the next veetee
/// would take that stop for its own and lose a session it had just opened.
fn first_circuit() -> u16 {
    let pid = u16::try_from(std::process::id() & 0xffff).unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos() as u16);
    // Zero is what a start message sends for an end it cannot name yet, so
    // the top bit keeps these clear of it however the mixing turns out.
    0x1000 | ((pid ^ now) & 0x0fff)
}

/// The identifier for the next session, so that two sessions from one node
/// cannot be taken for each other: a window holds two, and the far end tells
/// circuits apart by nothing else.
fn next_circuit() -> u16 {
    static BASE: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
    static COUNT: AtomicU16 = AtomicU16::new(0);
    let id = BASE
        .get_or_init(first_circuit)
        .wrapping_add(COUNT.fetch_add(1, Ordering::Relaxed));
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
    /// Whether there is still a circuit to take down. The far end can end the
    /// session and leave the circuit standing, and does: it acknowledges for
    /// as long as it is asked to.
    circuit: bool,
    /// Frames waiting to be sent.
    outgoing: Vec<Vec<u8>>,
    /// Typing not yet sent: because the far end has not prompted and would
    /// ignore it, or because there is no allowance left to send it with.
    /// Held back rather than dropped.
    typed: Vec<u8>,
    /// Slots this end may still send, by the far end's grant.
    ///
    /// Signed because it has been known to go negative, which is the fault
    /// worth seeing: a node that sends past its allowance has its messages
    /// dropped, and the far end then stops acknowledging and takes the
    /// circuit down without a word.
    allowance: i32,
    /// How the session ended, for the terminal to say. The two are worth
    /// telling apart: a logout ends a session and leaves the circuit up.
    ended: Option<&'static str>,
    /// Whether anything has been heard yet, which decides what the first
    /// message's number is compared against: nought is a number the far end
    /// may legitimately send, so it cannot stand for "nothing heard".
    heard_any: bool,
    /// Messages read since the far end's allowance was last worked out from
    /// scratch rather than adjusted.
    since_grant: u32,
    /// What has been seen, for a trace to write out. Counted and never acted
    /// on; see [`crate::watch`].
    stats: Stats,
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
            circuit: false,
            outgoing: Vec::new(),
            typed: Vec::new(),
            allowance: 0,
            ended: None,
            heard_any: false,
            since_grant: 0,
            stats: Stats::default(),
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
        self.stats.frames_out += 1;
        self.state = State::Calling;
    }

    /// Reads a frame's payload, appending any session data to `data` and
    /// queueing whatever it calls for in reply.
    pub fn receive(&mut self, payload: &[u8], data: &mut Vec<u8>) -> Event {
        // Every LAT frame on the wire is offered here, this circuit's or not,
        // so the two counts are worth telling apart: what arrived, and what
        // turned out to be ours.
        self.stats.frames_in += 1;
        let Ok(message) = crate::parse(payload) else {
            self.stats.ignored += 1;
            return Event::Ignored;
        };
        let event = match message {
            Message::Start(reply) => self.agreed(&reply),
            Message::Run(run) => self.run(&run, data),
            Message::Stop(stop) => self.stopped(stop),
            // Announcements, solicits and the types nobody has read are no
            // part of a session. The far end solicits whoever calls it, which
            // is one of these arriving mid-session and is safely nothing.
            Message::Announcement(_) | Message::Solicit(_) | Message::Other { .. } => {
                Event::Ignored
            }
        };
        if event == Event::Ignored {
            self.stats.ignored += 1;
        }
        event
    }

    /// Queues what is typed, in slots of as much as a slot will carry.
    ///
    /// Held back until the far end has said something: it will not read before
    /// it has prompted, and a slot sent earlier is ignored rather than queued.
    pub fn write(&mut self, data: &[u8]) {
        if self.state == State::Closed {
            return;
        }
        self.typed.extend_from_slice(data);
        self.send_typed(false);
    }

    /// Sends what has been typed, as far as the allowance goes.
    ///
    /// Everything waiting goes in as few slots as it takes rather than one
    /// slot for every time [`Session::write`] was called: a slot costs a
    /// credit whether it carries one byte or two hundred and fifty-five, and
    /// a keystroke is one byte.
    ///
    /// `force` sends whatever the allowance says, for a caller that has
    /// waited long enough. Holding strictly would be the correct reading of
    /// the protocol and the wrong thing to do: a far end that stops granting
    /// would silently take the keyboard with it.
    fn send_typed(&mut self, force: bool) {
        if self.state != State::Open {
            return;
        }
        while !self.typed.is_empty() {
            if self.allowance < 1 && !force {
                return;
            }
            let take = self.typed.len().min(MAX_SLOT);
            let chunk: Vec<u8> = self.typed.drain(..take).collect();
            let frame = self.data_frame(&chunk);
            self.outgoing.push(frame);
        }
    }

    /// Sends what is held whatever the allowance says.
    pub fn release_typing(&mut self) {
        self.send_typed(true);
    }

    /// Whether anything typed is waiting on the far end's allowance.
    #[must_use]
    pub fn holding(&self) -> bool {
        !self.typed.is_empty() && self.state == State::Open
    }

    /// Nothing has been heard from the far end for long enough to call it
    /// gone.
    ///
    /// LAT gives each end a keepalive timer and a retransmit limit so that
    /// either can decide the other has stopped; veetee sent the keepalives
    /// from the first and did none of the deciding, so a host that dropped
    /// the circuit left a terminal that went quiet and never said why. No
    /// stop is sent: there is nothing at the far end to take down.
    pub fn peer_gone(&mut self) {
        if self.is_open() {
            self.ended = Some("the host stopped answering");
            self.state = State::Closed;
            self.circuit = false;
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
        if self.circuit {
            self.outgoing.push(
                Stop {
                    theirs: self.theirs,
                    ours: self.ours,
                }
                .build(),
            );
            self.stats.frames_out += 1;
            self.stats.stops += 1;
            self.circuit = false;
        }
        self.state = State::Closed;
    }

    /// What this session has seen, for a trace to write out.
    ///
    /// The far end's remaining credit is this end's own reckoning of it
    /// rather than anything the far end has said.
    #[must_use]
    pub fn stats(&self) -> Stats {
        Stats {
            credit_theirs: self.credit,
            ..self.stats
        }
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

    /// How the session ended, in a few words a terminal can show.
    pub fn ending(&self) -> &'static str {
        self.ended.unwrap_or("the session is over")
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
        self.circuit = true;
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
        // Every message names the highest number its sender has heard,
        // acknowledgements included, so this is the one number that says
        // whether the far end is still listening. On a healthy circuit it
        // sits at nought or one.
        self.stats.unacked = self.sequence.wrapping_sub(run.acknowledged);
        self.stats.max_unacked = self.stats.max_unacked.max(self.stats.unacked);
        self.stats.slots_in += run.slots.len() as u64;

        // Follow the far end's numbering across *everything* it sends, its
        // own acknowledgements included: they carry a sequence number like
        // any other message, and two quite different things hang on it.
        //
        // Counting only the messages with slots reads every acknowledgement
        // as a message lost, and a message believed lost is credit believed
        // spent, so the far end gets granted more to make up for traffic that
        // never existed — 165 phantom losses and 1361 credits granted against
        // 107 received, on the session that showed it.
        //
        // Worse, *acknowledging* only those numbers deadlocks the circuit.
        // A far end whose last message carried no slots waits to hear that
        // number back before it sends anything else, and waits for ever:
        // MYI64 repeated `seq=86` every ten seconds for eleven minutes while
        // veetee answered `ack=85` every ten seconds, both ends healthy, full
        // credit either way, and the terminal frozen. Which number is
        // acknowledged and whether a frame is sent back are separate
        // questions, and only the second one is answered below.
        //
        // 🔎 Advancing past a gap tells the far end a message arrived when it
        // did not, so anything lost stays lost. The alternative — holding the
        // number until the missing one is repeated — is the stricter reading
        // and risks the same deadlock when what went missing is never
        // repeated. Deadlock being the worse failure, this is the way round
        // veetee takes it.
        let fresh = !self.heard_any || watch::newer(run.sequence, self.heard);
        let missed = if self.heard_any && fresh {
            usize::from(run.sequence.wrapping_sub(self.heard)) - 1
        } else {
            0
        };
        if fresh {
            if self.heard_any && run.sequence < self.heard {
                self.stats.wraps_in += 1;
            }
            self.stats.missed += missed as u64;
            self.heard = run.sequence;
            self.heard_any = true;
        }

        // Answer only what carries slots. Answering an acknowledgement draws
        // another back, and the two ends then answer each other for ever. The
        // number just taken from it goes out with the next keepalive, which
        // is what lets the far end move on.
        if run.slots.is_empty() {
            self.stats.acks_in += 1;
            return Event::Housekeeping;
        }
        // Anything not newer is the far end repeating itself, which it does
        // whenever an acknowledgement of ours goes missing. It wants
        // acknowledging again and it has spent its credit again, but its
        // slots have been read once already: reading them twice paints them
        // on the terminal twice.
        if !fresh {
            if run.sequence == self.heard {
                self.stats.duplicates += 1;
            } else {
                self.stats.rewinds += 1;
            }
            self.spend(run.slots.len());
            let frame = self.acknowledge();
            self.outgoing.push(frame);
            return Event::Housekeeping;
        }
        let frame = self.acknowledge();
        self.outgoing.push(frame);
        let before = data.len();
        let mut ended = false;
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
            // The low nibble is credit granted to this end. Nothing spends
            // it — what is typed goes out when it is typed, without asking
            // what allowance there is for it — so this count measures a
            // fault that has been reasoned about and never yet seen.
            // The low nibble is credit granted to this end, and it is now
            // spent against rather than ignored.
            let granted = u32::from(slot.control & 0x0f);
            self.stats.granted_in += u64::from(granted);
            self.allowance += i32::try_from(granted).unwrap_or(0);
            self.stats.credit_ours = self.allowance;
            match slot.control & SLOT_KIND {
                SLOT_DATA => {
                    self.stats.bytes_in += slot.data.len() as u64;
                    data.extend_from_slice(slot.data);
                }
                SLOT_END => ended = true,
                // Every other type is the circuit's own business: the block
                // of terminal parameters, the LTA device it names at the
                // start, and the single `@` that comes with the end.
                _ => {}
            }
        }
        self.spend(run.slots.len() + missed);
        if ended {
            self.ended = Some("the host ended the session");
            // Whatever came with it is still the terminal's: OpenVMS says
            // who logged out in the message before this one, and sometimes
            // in the same one. The caller reads what is waiting before it
            // reads the end.
            self.state = State::Closed;
            return Event::Closed;
        }
        if data.len() == before {
            return Event::Housekeeping;
        }
        // It has spoken, so it is reading: anything typed early can go now.
        if self.state == State::Asked {
            self.state = State::Open;
        }
        self.send_typed(false);
        Event::Data
    }

    /// The far end taking the circuit down.
    fn stopped(&mut self, stop: Stop) -> Event {
        // Naming this end is the least a stop of ours can do. 🔎 The one
        // ever captured sent zero for its own end, so that much is allowed,
        // but nothing looser: a stop for somebody else's circuit must not
        // take this one down.
        if stop.theirs != self.ours || (stop.ours != 0 && stop.ours != self.theirs) {
            return Event::Ignored;
        }
        self.stats.stops += 1;
        self.state = State::Closed;
        self.circuit = false;
        self.ended = Some("the host closed the circuit");
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
        self.since_grant += 1;
        // Counting what the far end has spent can only ever be an estimate: a
        // gap in the numbering says how many messages were missed but not how
        // many slots each of them carried. So every so often the estimate is
        // thrown away rather than corrected — the far end taken to hold
        // nothing and granted a full allowance — which makes every way of
        // losing count right itself within REGRANT messages.
        //
        // The bias is deliberate. Granting more than the far end has spent
        // means reading more than was expected, which is nothing at all;
        // granting less means a session that stops dead with no error at
        // either end. 🔎 Whether a node lets its allowance run past what the
        // nibble holds is unread, and this assumes it clamps.
        if self.since_grant >= REGRANT {
            self.since_grant = 0;
            self.credit = 0;
        }
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
        self.stats.frames_out += 1;
        if self.sequence == 0 {
            self.stats.wraps_out += 1;
        }
        if slots.is_empty() {
            self.stats.acks_out += 1;
        }
        self.stats.slots_out += slots.len() as u64;
        // Only session data with something in it spends the allowance, and
        // both halves of that matter. The slot asking for a service goes
        // before the far end has granted anything, so a session could never
        // be opened otherwise; and an empty slot is how credit is granted, so
        // if those spent too, two ends that had both run out could never
        // grant each other any and the circuit would stand there deadlocked.
        //
        // 🔎 How a real node reckons it is unread. This is the reading under
        // which a session can start and cannot wedge itself, which is the
        // most that can be said for it.
        let spending = slots
            .iter()
            .filter(|slot| slot.control & SLOT_KIND == SLOT_DATA && !slot.data.is_empty())
            .count();
        self.allowance -= i32::try_from(spending).unwrap_or(0);
        self.stats.credit_ours = self.allowance;
        for slot in slots {
            self.stats.bytes_out += slot.data.len() as u64;
            self.stats.granted_out += u64::from(slot.control & 0x0f);
        }
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
    fn the_same_message_twice_is_read_once_and_acknowledged_again() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        assert_eq!(session.take_outgoing().len(), 1);
        let once = data.len();

        // The far end repeats whatever it has not heard acknowledged. Reading
        // it again would paint the same text on the terminal a second time, so
        // its slots are dropped — but it is acknowledged again, a lost
        // acknowledgement being the reason it was repeated at all. Answering
        // with nothing, as this once did, leaves it repeating for ever.
        assert_eq!(
            session.receive(PROMPT, &mut data),
            Event::Housekeeping,
            "there is nothing new in it"
        );
        assert_eq!(data.len(), once, "and nothing more for the terminal");

        let out = session.take_outgoing();
        assert_eq!(out.len(), 1, "acknowledged again");
        let Ok(Message::Run(ack)) = crate::parse(&out[0]) else {
            panic!("not a run")
        };
        assert!(ack.slots.is_empty(), "an acknowledgement carries nothing");
        assert_eq!(ack.acknowledged, 3, "the same number as the first time");
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
        // The prompt grants one slot's worth, so the first slot goes and the
        // rest waits on an allowance; releasing is what the caller does once
        // it has waited long enough for a grant that is not coming.
        assert!(session.holding(), "the rest of it is waiting on credit");
        session.release_typing();
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
    fn typing_waits_on_the_allowance_and_is_let_go_when_it_has_to_be() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        session.take_outgoing();

        // The prompt granted one slot, and the first keystroke spends it.
        session.write(b"a");
        assert_eq!(session.take_outgoing().len(), 1, "the allowance covers it");
        assert!(!session.holding());

        session.write(b"b");
        assert!(
            session.take_outgoing().is_empty(),
            "nothing left to send it with"
        );
        assert!(session.holding(), "held rather than dropped");

        // A far end that never grants again would otherwise take the keyboard
        // with it, so waiting has a limit and the caller decides when.
        session.release_typing();
        let out = session.take_outgoing();
        assert_eq!(out.len(), 1);
        let Ok(Message::Run(sent)) = crate::parse(&out[0]) else {
            panic!("not a run")
        };
        assert_eq!(sent.slots[0].data, b"b");
        assert!(!session.holding());
    }

    #[test]
    fn everything_waiting_goes_in_one_slot() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        session.take_outgoing();

        // Three keystrokes with one slot's worth of credit between them. A
        // slot costs a credit whether it carries one byte or two hundred, so
        // what is waiting goes together rather than one slot each.
        session.write(b"S");
        session.take_outgoing();
        session.write(b"Y");
        session.write(b"S");
        session.release_typing();
        let out = session.take_outgoing();
        assert_eq!(out.len(), 1, "one slot, not two");
        let Ok(Message::Run(sent)) = crate::parse(&out[0]) else {
            panic!("not a run")
        };
        assert_eq!(sent.slots[0].data, b"YS");
    }

    #[test]
    fn a_far_end_that_stops_answering_ends_the_session() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        session.take_outgoing();
        assert!(!session.is_closed());

        session.peer_gone();
        assert!(session.is_closed());
        assert_eq!(session.ending(), "the host stopped answering");
        assert!(
            session.take_outgoing().is_empty(),
            "no stop: there is nothing at the far end to take down"
        );
        let _ = data;
    }

    #[test]
    fn an_acknowledgement_is_counted_even_though_it_is_not_answered() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        session.take_outgoing();

        // A far end whose last message carried no slots waits to hear that
        // number back before it sends anything else. Answering it directly
        // would draw another answer and go on for ever — but taking the
        // number from it must still happen, or the wait never ends. MYI64
        // repeated seq=86 every ten seconds for eleven minutes against a
        // veetee answering ack=85, both ends healthy and the terminal frozen.
        let ack = Run {
            flags: 0,
            theirs: 0x7001,
            ours: 0xe001,
            sequence: 4,
            acknowledged: 2,
            slots: Vec::new(),
        }
        .build();
        assert_eq!(session.receive(&ack, &mut data), Event::Housekeeping);
        assert!(
            session.take_outgoing().is_empty(),
            "an acknowledgement is not answered"
        );

        session.keepalive();
        let out = session.take_outgoing();
        assert_eq!(out.len(), 1);
        let Ok(Message::Run(keepalive)) = crate::parse(&out[0]) else {
            panic!("not a run")
        };
        assert_eq!(
            keepalive.acknowledged, 4,
            "the number it carried goes out with the next thing sent"
        );
    }

    #[test]
    fn the_far_ends_own_acknowledgements_are_not_read_as_losses() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        session.take_outgoing();
        let numbered = session.stats().missed;

        // An acknowledgement carries a sequence number like anything else.
        // Counting only the messages with slots in them reads each of these
        // as a message lost, and then grants credit to make up for it.
        for sequence in 4..=8 {
            let ack = Run {
                flags: 0,
                theirs: 0x7001,
                ours: 0xe001,
                sequence,
                acknowledged: 2,
                slots: Vec::new(),
            }
            .build();
            assert_eq!(
                session.receive(&ack, &mut data),
                Event::Housekeeping,
                "an acknowledgement is not answered"
            );
        }
        assert!(
            session.take_outgoing().is_empty(),
            "and draws nothing back, or the two ends answer each other for ever"
        );
        assert_eq!(
            session.stats().missed,
            numbered,
            "five acknowledgements are not five lost messages"
        );
    }

    #[test]
    fn the_host_ending_the_session_ends_it_here() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        session.take_outgoing();
        data.clear();

        // What MYI64 sends on LOGOUT, and on timing a login out: a slot from
        // slot zero with nothing in it, and a single `@` beside it.
        let goodbye = Run {
            flags: 0,
            theirs: 0x7001,
            ours: 0xe001,
            sequence: 9,
            acknowledged: 8,
            slots: vec![
                Slot {
                    to: 1,
                    from: 1,
                    control: 0xb0,
                    data: b"@",
                },
                Slot {
                    to: 1,
                    from: 0,
                    control: 0xd1,
                    data: &[],
                },
            ],
        }
        .build();
        assert_eq!(session.receive(&goodbye, &mut data), Event::Closed);
        assert!(session.is_closed());
        assert!(data.is_empty(), "neither slot is the terminal's");

        // The session is over but the circuit is not: MYI64 goes on
        // acknowledging until it is taken down.
        session.take_outgoing();
        session.close();
        let out = session.take_outgoing();
        assert_eq!(out.len(), 1, "the circuit is still ours to take down");
        assert!(matches!(crate::parse(&out[0]), Ok(Message::Stop(_))));
    }

    #[test]
    fn a_stop_for_another_circuit_is_left_alone() {
        let (mut session, mut data) = agreed();
        // A veetee that was killed rather than closed leaves a circuit for
        // the host to time out on its own, and the stop that follows names
        // that one. Taking it would lose a session just opened.
        let earlier = Stop {
            theirs: 0x1001,
            ours: 0xe001,
        }
        .build();
        assert_eq!(session.receive(&earlier, &mut data), Event::Ignored);
        assert!(!session.is_closed(), "that circuit is not this one");

        // Naming this end but some other far end is no better.
        let confused = Stop {
            theirs: 0x7001,
            ours: 0x9999,
        }
        .build();
        assert_eq!(session.receive(&confused, &mut data), Event::Ignored);
        assert!(!session.is_closed());
    }

    #[test]
    fn an_identifier_is_never_zero_whatever_the_run() {
        let base = first_circuit();
        assert_ne!(base, 0, "zero is an end that has not been named");
        assert_eq!(base & 0xf000, 0x1000, "and it is kept well clear of it");
    }

    #[test]
    fn how_a_session_ended_is_worth_saying() {
        let (mut session, mut data) = agreed();
        session.receive(PROMPT, &mut data);
        let goodbye = Run {
            flags: 0,
            theirs: 0x7001,
            ours: 0xe001,
            sequence: 9,
            acknowledged: 8,
            slots: vec![Slot {
                to: 1,
                from: 0,
                control: 0xd1,
                data: &[],
            }],
        }
        .build();
        session.receive(&goodbye, &mut data);
        assert_eq!(session.ending(), "the host ended the session");

        let (mut session, mut data) = agreed();
        let stop = Stop {
            theirs: 0x7001,
            ours: 0xe001,
        }
        .build();
        session.receive(&stop, &mut data);
        assert_eq!(
            session.ending(),
            "the host closed the circuit",
            "a logout is not a circuit going down, and a reader should be told which"
        );
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
