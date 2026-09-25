//! A whole transfer, a packet at a time, still without any I/O.
//!
//! Kermit is stop-and-wait: every packet is answered before the next one
//! goes, so at any moment each end is waiting for exactly one thing. The
//! [`Sender`] waits for the acknowledgement of what it last sent and sends it
//! again when none comes; the [`Receiver`] waits for the next packet in the
//! sequence and asks for it with a nak when none comes. Everything else —
//! duplicates, damage, a far end that gives up — is one of those two waits
//! going wrong.
//!
//! The caller owns the line, the clock and the files. It hands over what
//! arrives with `feed`, calls `tick` now and then when nothing has, and writes
//! whatever either returns to the line. Files go through [`Store`] and
//! [`Source`], so the whole exchange can be run in a test against a line that
//! loses and damages packets, with time that passes only when told to.
//!
//! The order of a transfer, from the sending end:
//!
//! ```text
//! S  send-init: the sender's parameters      Y  the receiver's
//! F  a file's name                           Y
//! D  its contents, as many as it takes       Y  (or Y with X or Z: stop)
//! Z  end of file (with D: throw it away)     Y
//!    ... F, D, Z again for each file ...
//! B  no more files                           Y
//! ```

use std::time::{Duration, Instant};

use crate::attributes::{self, Attributes, FileType};
use crate::names::{self, Names};
use crate::text::{FromLine, LineEnding, ToLine};
use crate::{Check, Error, Kind, MARK, Packet, Params, decode, encode, ours};

/// How a file's contents travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Lines, which end in CR LF on the line and in the local convention at
    /// each end. Kermit's own default, and what OpenVMS text files need.
    #[default]
    Text,
    /// The bytes exactly. A seven-bit line needs the eighth-bit prefix agreed
    /// to carry them.
    Binary,
}

/// What a transfer is to do, beyond the parameters it offers the far end.
#[derive(Debug, Clone)]
pub struct Settings {
    /// What veetee asks of the far end in its send-init or its answer to one.
    pub params: Params,
    pub mode: Mode,
    /// How lines end in text files here.
    pub local: LineEnding,
    /// How a received file's name is written down.
    pub names: Names,
    /// How many times to send a packet again, or ask for one again, before
    /// giving up.
    pub retries: u32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            params: ours(),
            mode: Mode::default(),
            local: LineEnding::native(),
            names: Names::default(),
            retries: 10,
        }
    }
}

/// Where received files go.
pub trait Store {
    /// A file is arriving. `name` has already been made safe (see
    /// [`names::local_name`]); where it goes, and what to do if the name is
    /// taken, is the store's decision. An error refuses the file, and its
    /// text is sent to the far end.
    fn create(&mut self, name: &str) -> Result<(), String>;
    /// More of the file.
    fn write(&mut self, data: &[u8]) -> Result<(), String>;
    /// The end of the file. `complete` is false when it did not all arrive —
    /// the transfer was cancelled or failed — and what was written should
    /// not be mistaken for the file.
    fn finish(&mut self, complete: bool) -> Result<(), String>;
}

/// Where files to send come from.
pub trait Source {
    /// The next file to send, by the name the far end should give it, or
    /// `None` when there are no more.
    fn next_file(&mut self) -> Result<Option<String>, String>;
    /// More of the current file; nought at its end.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String>;
    /// The current file's size in bytes, where it is known, to tell the far
    /// end in an attribute packet.
    fn size(&self) -> Option<u64> {
        None
    }
    /// Whether the current file goes as text or binary, where the source has
    /// decided; otherwise the transfer's setting.
    fn mode(&self) -> Option<Mode> {
        None
    }
}

/// Where a transfer has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Running,
    /// Every file went, and the far end knows it.
    Done,
    /// Stopped by the user.
    Cancelled,
    /// Stopped, and why.
    Failed(String),
}

/// How far a transfer has got, for showing to the user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// The file now going, by its name at this end.
    pub file: Option<String>,
    /// Bytes of it so far, as the file has them.
    pub bytes: u64,
    /// Files finished and kept.
    pub files: u32,
    /// The size of the file now going, where the sender said or the source
    /// knows, so that progress can be a proportion.
    pub size: Option<u64>,
}

/// Said when both ends were told to receive, or both to send: the user
/// needs to know which to change, not that a packet was unexpected.
pub const BOTH_RECEIVING: &str =
    "the other end is waiting to receive too: to send it a file, choose Send File";
pub const BOTH_SENDING: &str =
    "the other end is sending too: to take its files, choose Receive File";

/// Sequence numbers count to 63 and begin again.
fn next(seq: u8) -> u8 {
    (seq + 1) % 64
}

fn previous(seq: u8) -> u8 {
    (seq + 63) % 64
}

/// A packet as it arrived, kept once the bytes it came in are gone.
struct Got {
    seq: u8,
    kind: Kind,
    data: Vec<u8>,
}

enum Arrival {
    Packet(Got),
    /// Something with a mark that did not read as a packet.
    Damaged,
    /// Nothing whole yet.
    Nothing,
}

/// The half of a transfer that both directions share: what was agreed, what
/// has arrived, what was last sent, and when to stop waiting.
#[derive(Debug)]
struct Link {
    ours: Params,
    /// What veetee sends with and what it reads with, once agreed. The two
    /// differ only in the control prefix, which each end names for itself.
    agreed: Option<(Params, Params)>,
    /// What to read and build with before anything is agreed.
    before: Params,
    /// Both ends offered attribute packets, so a sender sends them.
    attributes: bool,
    /// Both ends offered long packets, and this is the longest the far end
    /// will read.
    long: Option<usize>,
    buffer: Vec<u8>,
    /// The last packet that might need sending again.
    last: Vec<u8>,
    retries: u32,
    limit: u32,
    deadline: Option<Instant>,
}

/// Past this, bytes that have not made a packet are noise: a packet is never
/// more than a long packet's 9024 and its header.
const MOST_BUFFERED: usize = 2 * crate::MAX_LONG + 64;

impl Link {
    fn new(settings: &Settings) -> Link {
        Link {
            ours: settings.params.clone(),
            agreed: None,
            before: Params::default(),
            attributes: false,
            long: None,
            buffer: Vec::new(),
            last: Vec::new(),
            retries: 0,
            limit: settings.retries,
            deadline: None,
        }
    }

    fn agree(&mut self, theirs: &Params) {
        self.attributes = attributes::offered(&self.ours.capabilities)
            && attributes::offered(&theirs.capabilities);
        self.long = self.ours.long().and(theirs.long());
        let reading = Params::agreed(&self.ours, theirs);
        let mut sending = reading.clone();
        // Each end prefixes control characters with the character it named,
        // so veetee sends with its own and reads with theirs.
        sending.quote_control = self.ours.quote_control;
        self.agreed = Some((sending, reading));
    }

    fn sending(&self) -> &Params {
        self.agreed.as_ref().map_or(&self.before, |(s, _)| s)
    }

    fn reading(&self) -> &Params {
        self.agreed.as_ref().map_or(&self.before, |(_, r)| r)
    }

    fn check(&self) -> Check {
        self.agreed.as_ref().map_or(Check::One, |(s, _)| s.check)
    }

    /// Room for data in a packet the far end can read.
    ///
    /// With long packets that is the far end's limit less the header and the
    /// check, whichever way that limit is counted: a few characters short of
    /// what it could take costs nothing, and one over would be refused.
    fn room(&self) -> usize {
        if let Some(long) = self.long {
            return long.saturating_sub(7 + self.check().chars());
        }
        usize::from(self.sending().max_length).saturating_sub(2 + self.check().chars())
    }

    /// How long to wait for the far end: what it asked for, once it has said,
    /// and what veetee asked of it until then. Nought is no time limit at all.
    fn wait(&self) -> Option<Duration> {
        let secs = self
            .agreed
            .as_ref()
            .map_or(self.ours.timeout, |(s, _)| s.timeout);
        (secs > 0).then(|| Duration::from_secs(secs.into()))
    }

    fn arm(&mut self, now: Instant) {
        self.deadline = self.wait().map(|wait| now + wait);
    }

    fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }

    /// A packet on the wire, with the padding and end of line the far end
    /// asked for.
    fn build(&self, seq: u8, kind: Kind, data: &[u8], check: Check) -> Vec<u8> {
        let p = self.sending();
        let mut out = vec![p.pad_with; usize::from(p.padding)];
        out.extend(
            Packet {
                sequence: seq,
                kind,
                data,
            }
            .build(check, MARK, Some(p.end_of_line)),
        );
        out
    }

    /// Sends a packet that moves the transfer on, and so may need sending
    /// again: the count of retries starts over with it.
    fn send(&mut self, seq: u8, kind: Kind, data: &[u8], check: Check, now: Instant) -> Vec<u8> {
        self.last = self.build(seq, kind, data, check);
        self.retries = 0;
        self.arm(now);
        self.last.clone()
    }

    /// Counts one more try, and says whether that is still within the limit.
    fn retry(&mut self, now: Instant) -> bool {
        self.retries += 1;
        self.arm(now);
        self.retries <= self.limit
    }

    /// An error packet carrying `why`, cut to what one packet holds.
    fn error(&self, seq: u8, why: &str) -> Vec<u8> {
        let (data, _) = encode(why.as_bytes(), self.sending(), self.room());
        self.build(seq, Kind::Error, &data, self.check())
    }

    /// What an error packet from the far end said.
    fn said(&self, data: &[u8]) -> String {
        String::from_utf8_lossy(&decode(data, self.reading()))
            .trim()
            .to_string()
    }

    /// The next packet in what has arrived, read with the first of `checks`
    /// that it passes.
    fn take(&mut self, checks: &[Check]) -> Arrival {
        let Some(start) = self.buffer.iter().position(|&b| b == MARK) else {
            self.buffer.clear();
            return Arrival::Nothing;
        };
        self.buffer.drain(..start);
        let mut damaged = false;
        for &check in checks {
            match crate::read(&self.buffer, check, MARK) {
                Ok((packet, used)) => {
                    let got = Got {
                        seq: packet.sequence,
                        kind: packet.kind,
                        data: packet.data.to_vec(),
                    };
                    self.buffer.drain(..used);
                    return Arrival::Packet(got);
                }
                Err(Error::Incomplete) => {}
                Err(Error::TooLong | Error::BadCheck) => damaged = true,
            }
        }
        if damaged || self.buffer.len() > MOST_BUFFERED {
            // Step past this mark, so the next one gets its chance.
            self.buffer.drain(..1);
            return Arrival::Damaged;
        }
        Arrival::Nothing
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Receiving {
    SendInit,
    File,
    Data,
    Finished,
}

/// The receiving end of a transfer.
#[derive(Debug)]
pub struct Receiver {
    link: Link,
    settings: Settings,
    state: Receiving,
    expected: u8,
    lines: FromLine,
    /// How this file's contents travel: the user's setting, unless the
    /// sender said otherwise in an attribute packet.
    mode: Mode,
    /// A file is open in the store.
    open: bool,
    cancelling: bool,
    status: Status,
    progress: Progress,
}

impl Receiver {
    /// A receiver waiting for a send-init.
    #[must_use]
    pub fn new(settings: Settings, now: Instant) -> Receiver {
        let mut link = Link::new(&settings);
        link.arm(now);
        let lines = FromLine::new(settings.local);
        let mode = settings.mode;
        Receiver {
            link,
            settings,
            state: Receiving::SendInit,
            expected: 0,
            lines,
            mode,
            open: false,
            cancelling: false,
            status: Status::Running,
            progress: Progress::default(),
        }
    }

    #[must_use]
    pub fn status(&self) -> &Status {
        &self.status
    }

    #[must_use]
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// Bytes from the line. Returns what to send back.
    pub fn feed(&mut self, bytes: &[u8], now: Instant, store: &mut dyn Store) -> Vec<u8> {
        let mut out = Vec::new();
        if self.status == Status::Done {
            // The acknowledgement of the Break may be the packet that was
            // lost, in which case the sender is still asking. Answering costs
            // nothing and lets it finish too.
            self.link.buffer.extend_from_slice(bytes);
            loop {
                match self.link.take(&[self.link.check()]) {
                    Arrival::Nothing => return out,
                    Arrival::Damaged => {}
                    Arrival::Packet(p) => {
                        if p.kind == Kind::Break && p.seq == previous(self.expected) {
                            out.extend_from_slice(&self.link.last);
                        }
                    }
                }
            }
        }
        if self.status != Status::Running {
            return out;
        }
        self.link.buffer.extend_from_slice(bytes);
        while self.status == Status::Running {
            let checks = self.checks();
            match self.link.take(&checks) {
                Arrival::Nothing => break,
                Arrival::Damaged => out.extend(self.nak()),
                Arrival::Packet(p) => out.extend(self.handle(p, now, store)),
            }
        }
        out
    }

    /// Time has passed with nothing arriving. Returns what to send.
    pub fn tick(&mut self, now: Instant, store: &mut dyn Store) -> Vec<u8> {
        if self.status != Status::Running || !self.link.expired(now) {
            return Vec::new();
        }
        // Whatever part of a packet is waiting is not going to be finished.
        self.link.buffer.clear();
        if !self.link.retry(now) {
            // Between files, whatever came before is complete and kept; only
            // whether more were coming is unknown. Say so, so a user does not
            // take the files already here for broken ones.
            let why = match (self.state, self.progress.files) {
                (Receiving::File, 1) => "nothing more arrived after 1 complete file".to_string(),
                (Receiving::File, n) if n > 1 => {
                    format!("nothing more arrived after {n} complete files")
                }
                _ => "nothing arrived from the other end".to_string(),
            };
            return self.fail(store, &why);
        }
        self.nak()
    }

    /// The user wants to stop. The first time, the sender is asked to stop at
    /// the next packet, which ends the transfer tidily; if it will not, or
    /// the user asks again, the transfer ends at once.
    pub fn cancel(&mut self, store: &mut dyn Store) -> Vec<u8> {
        if self.status != Status::Running {
            return Vec::new();
        }
        if self.state == Receiving::Data && !self.cancelling {
            self.cancelling = true;
            return Vec::new();
        }
        let out = self.link.error(self.expected, "cancelled");
        self.stop(store, Status::Cancelled);
        out
    }

    fn checks(&self) -> Vec<Check> {
        let agreed = self.link.check();
        // The sender goes on using the one-character check until it has
        // heard the answer to its send-init. If that answer was lost, what
        // arrives next is the send-init again, so until a file has begun it
        // is read either way.
        if self.state == Receiving::File && self.expected == 1 && agreed != Check::One {
            vec![agreed, Check::One]
        } else {
            vec![agreed]
        }
    }

    fn nak(&self) -> Vec<u8> {
        // Not kept as the last packet: a duplicate of the previous packet
        // still wants the previous answer.
        self.link
            .build(self.expected, Kind::Nak, &[], self.link.check())
    }

    fn handle(&mut self, p: Got, now: Instant, store: &mut dyn Store) -> Vec<u8> {
        if p.kind == Kind::Error {
            let why = format!("the other end stopped: {}", self.link.said(&p.data));
            self.stop(store, Status::Failed(why));
            return Vec::new();
        }
        if self.state != Receiving::SendInit && p.seq == previous(self.expected) {
            // The answer to it went missing. Send it again rather than act
            // on the packet twice.
            self.link.arm(now);
            return self.link.last.clone();
        }
        if p.seq != self.expected {
            return Vec::new();
        }
        let seq = p.seq;
        match (self.state, p.kind) {
            (Receiving::SendInit, Kind::SendInit) => {
                let answer = self.link.ours.build();
                self.link.agree(&Params::read(&p.data));
                self.state = Receiving::File;
                self.expected = next(seq);
                // The answer to a send-init goes with the one-character
                // check, as the send-init came: nothing else is agreed yet.
                self.link.send(seq, Kind::Ack, &answer, Check::One, now)
            }
            (Receiving::File, Kind::File) => {
                let sent =
                    String::from_utf8_lossy(&decode(&p.data, self.link.reading())).into_owned();
                let Some(name) = names::local_name(&sent, self.settings.names) else {
                    return self.fail(store, &format!("refusing the file name {sent:?}"));
                };
                if let Err(why) = store.create(&name) {
                    return self.fail(store, &why);
                }
                self.open = true;
                self.lines = FromLine::new(self.settings.local);
                self.mode = self.settings.mode;
                self.progress.file = Some(name);
                self.progress.bytes = 0;
                self.progress.size = None;
                self.state = Receiving::Data;
                self.ack(seq, b"", now)
            }
            (Receiving::File, Kind::Break) => {
                let out = self.ack(seq, b"", now);
                self.state = Receiving::Finished;
                self.link.deadline = None;
                self.status = if self.cancelling {
                    Status::Cancelled
                } else {
                    Status::Done
                };
                out
            }
            (Receiving::Data, Kind::Attributes) => {
                // Read whether or not they were agreed: a sender that sends
                // them knows best what its file is.
                let said = Attributes::read(&p.data);
                match said.file_type {
                    Some(FileType::Text) => self.mode = Mode::Text,
                    Some(FileType::Binary) => self.mode = Mode::Binary,
                    None => {}
                }
                self.progress.size = said.size;
                self.ack(seq, b"Y", now)
            }
            (Receiving::Data, Kind::Data) => {
                if !self.cancelling {
                    let bytes = decode(&p.data, self.link.reading());
                    let written = match self.mode {
                        Mode::Binary => bytes,
                        Mode::Text => {
                            let mut text = Vec::with_capacity(bytes.len());
                            self.lines.convert(&bytes, &mut text);
                            text
                        }
                    };
                    if let Err(why) = store.write(&written) {
                        return self.fail(store, &why);
                    }
                    self.progress.bytes += written.len() as u64;
                }
                // Z in the acknowledgement asks the sender to stop the whole
                // batch; it answers with the end of the file, marked discard.
                let answer: &[u8] = if self.cancelling { b"Z" } else { b"" };
                self.ack(seq, answer, now)
            }
            (Receiving::Data, Kind::EndOfFile) => {
                // D in the end of a file: the sender stopped part way, and what
                // arrived is not the file. It means the batch is ending too.
                if p.data.first() == Some(&b'D') {
                    self.cancelling = true;
                }
                let keep = !self.cancelling;
                if keep && self.mode == Mode::Text {
                    let mut tail = Vec::new();
                    self.lines.finish(&mut tail);
                    if !tail.is_empty() {
                        if let Err(why) = store.write(&tail) {
                            return self.fail(store, &why);
                        }
                        self.progress.bytes += tail.len() as u64;
                    }
                }
                self.open = false;
                if let Err(why) = store.finish(keep) {
                    return self.fail(store, &why);
                }
                if keep {
                    self.progress.files += 1;
                }
                self.state = Receiving::File;
                self.ack(seq, b"", now)
            }
            // A Kermit told RECEIVE naks packet 0 while it waits. Hearing
            // that while waiting to receive means both ends are receiving,
            // which is easily done from two menu items side by side.
            (Receiving::SendInit, Kind::Nak) => self.fail(store, BOTH_RECEIVING),
            (_, kind) => self.fail(
                store,
                &format!(
                    "a {} packet was not expected here",
                    char::from(kind.as_byte())
                ),
            ),
        }
    }

    fn ack(&mut self, seq: u8, data: &[u8], now: Instant) -> Vec<u8> {
        self.expected = next(seq);
        let check = self.link.check();
        self.link.send(seq, Kind::Ack, data, check, now)
    }

    /// Stops, telling the far end why.
    fn fail(&mut self, store: &mut dyn Store, why: &str) -> Vec<u8> {
        let out = self.link.error(self.expected, why);
        self.stop(store, Status::Failed(why.to_string()));
        out
    }

    fn stop(&mut self, store: &mut dyn Store, status: Status) {
        if std::mem::take(&mut self.open) {
            let _ = store.finish(false);
        }
        self.state = Receiving::Finished;
        self.link.deadline = None;
        self.status = status;
    }
}

/// Where a sender's packets start when long packets are agreed, and the
/// least it halves them to.
const FIRST_SIZE: usize = 250;
const LEAST_SIZE: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sending {
    SendInit,
    File,
    Attributes,
    Data,
    EndOfFile,
    Break,
    Finished,
}

/// The sending end of a transfer.
#[derive(Debug)]
pub struct Sender {
    link: Link,
    settings: Settings,
    state: Sending,
    /// The sequence number of the packet waiting to be acknowledged.
    seq: u8,
    /// File contents read and converted but not yet sent.
    pending: Vec<u8>,
    lines: ToLine,
    /// How this file goes: the source's choice, or the setting.
    mode: Mode,
    eof: bool,
    /// The end of this file was sent marked discard.
    discarding: bool,
    /// How much data to put in the next packet, where long packets were
    /// agreed: it starts small, doubles with every packet that gets through
    /// and halves with every one sent again, so a slow or noisy line settles
    /// on what it can carry. A packet of 9 KB takes nine seconds on a line at
    /// 9600 baud, which is most of a host's timeout, and at 2400 could never
    /// arrive in time. C-Kermit does the same, from 253 up.
    size: usize,
    /// The most a packet may carry for the rest of the transfer: half of any
    /// packet that had to be sent again. Growing straight back to a size
    /// that has just failed finds the same limit again — an OpenVMS process
    /// out of quota for reads of 4000 bytes did, until C-Kermit gave up.
    ceiling: usize,
    /// How much data the packet waiting for an answer carries.
    in_flight: usize,
    cancelling: bool,
    status: Status,
    progress: Progress,
}

impl Sender {
    #[must_use]
    pub fn new(settings: Settings) -> Sender {
        let mode = settings.mode;
        Sender {
            link: Link::new(&settings),
            settings,
            state: Sending::SendInit,
            seq: 0,
            pending: Vec::new(),
            lines: ToLine::default(),
            mode,
            eof: false,
            discarding: false,
            size: FIRST_SIZE,
            ceiling: usize::MAX,
            in_flight: 0,
            cancelling: false,
            status: Status::Running,
            progress: Progress::default(),
        }
    }

    #[must_use]
    pub fn status(&self) -> &Status {
        &self.status
    }

    #[must_use]
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// The send-init that begins the transfer.
    pub fn start(&mut self, now: Instant) -> Vec<u8> {
        let init = self.link.ours.build();
        self.link.send(0, Kind::SendInit, &init, Check::One, now)
    }

    /// Bytes from the line. Returns what to send.
    pub fn feed(&mut self, bytes: &[u8], now: Instant, source: &mut dyn Source) -> Vec<u8> {
        let mut out = Vec::new();
        if self.status != Status::Running {
            return out;
        }
        self.link.buffer.extend_from_slice(bytes);
        while self.status == Status::Running {
            match self.link.take(&[self.link.check()]) {
                Arrival::Nothing => break,
                // A damaged answer is no answer: the packet goes again when
                // the wait for one runs out.
                Arrival::Damaged => {}
                Arrival::Packet(p) => out.extend(self.handle(p, now, source)),
            }
        }
        out
    }

    /// Time has passed with nothing arriving. Returns what to send.
    pub fn tick(&mut self, now: Instant) -> Vec<u8> {
        if self.status != Status::Running || !self.link.expired(now) {
            return Vec::new();
        }
        self.link.buffer.clear();
        self.again(now)
    }

    /// The user wants to stop. The first time, the file is ended marked
    /// discard at the next packet and the batch ended after it; the second,
    /// or where there is nothing to end tidily, the transfer ends at once.
    pub fn cancel(&mut self) -> Vec<u8> {
        if self.status != Status::Running {
            return Vec::new();
        }
        if matches!(self.state, Sending::Data | Sending::EndOfFile) && !self.cancelling {
            self.cancelling = true;
            return Vec::new();
        }
        let out = self.link.error(self.seq, "cancelled");
        self.stop(Status::Cancelled);
        out
    }

    fn handle(&mut self, p: Got, now: Instant, source: &mut dyn Source) -> Vec<u8> {
        match p.kind {
            Kind::Error => {
                let why = format!("the other end stopped: {}", self.link.said(&p.data));
                self.stop(Status::Failed(why));
                Vec::new()
            }
            Kind::Ack if p.seq == self.seq => self.acknowledged(&p.data, now, source),
            // A send-init answering a send-init: both ends are sending.
            Kind::SendInit if self.state == Sending::SendInit => self.fail(BOTH_SENDING),
            // A nak for the packet after this one means this one arrived:
            // the receiver is asking for what comes next. Except for the
            // send-init, whose answer carries the receiver's parameters: a
            // nak has none, and going on without them would leave the two
            // ends disagreeing about the prefixes. Sent again, the send-init
            // is a duplicate, which the receiver answers with them.
            Kind::Nak if p.seq == next(self.seq) && self.state != Sending::SendInit => {
                self.acknowledged(&[], now, source)
            }
            Kind::Nak if p.seq == self.seq || p.seq == next(self.seq) => self.again(now),
            // Anything else is an old answer arriving late.
            _ => Vec::new(),
        }
    }

    fn again(&mut self, now: Instant) -> Vec<u8> {
        if self.state == Sending::Data && self.in_flight > LEAST_SIZE {
            self.ceiling = self.ceiling.min((self.in_flight / 2).max(LEAST_SIZE));
        }
        self.size = (self.size / 2).max(LEAST_SIZE).min(self.ceiling);
        if self.link.retry(now) {
            return self.link.last.clone();
        }
        if self.state == Sending::Break {
            // Every file was acknowledged on its own; only the answer to
            // "no more files" is missing, and that is most likely because
            // the receiver has already finished.
            self.stop(Status::Done);
            return Vec::new();
        }
        self.fail("no answer from the other end")
    }

    fn acknowledged(&mut self, data: &[u8], now: Instant, source: &mut dyn Source) -> Vec<u8> {
        self.seq = next(self.seq);
        match self.state {
            Sending::SendInit => {
                self.link.agree(&Params::read(data));
                self.next_file(now, source)
            }
            Sending::File => {
                self.pending.clear();
                self.lines = ToLine::default();
                self.eof = false;
                self.discarding = false;
                if !self.link.attributes {
                    self.state = Sending::Data;
                    return self.send_data(now, source);
                }
                self.state = Sending::Attributes;
                let said = Attributes {
                    file_type: Some(match self.mode {
                        Mode::Text => FileType::Text,
                        Mode::Binary => FileType::Binary,
                    }),
                    size: self.progress.size,
                };
                self.packet(Kind::Attributes, &said.build(), now)
            }
            Sending::Attributes => {
                // N refuses the file — too big, say — and whatever follows
                // it names the attributes it objected to. The file is ended
                // unsent, marked discard, and the next one offered.
                if data.first() == Some(&b'N') {
                    self.state = Sending::EndOfFile;
                    self.discarding = true;
                    return self.packet(Kind::EndOfFile, b"D", now);
                }
                self.state = Sending::Data;
                self.send_data(now, source)
            }
            Sending::Data => {
                // X stops this file, Z the whole batch.
                let stop = data.first().copied();
                if stop == Some(b'Z') {
                    self.cancelling = true;
                }
                if stop == Some(b'X') || self.cancelling {
                    self.state = Sending::EndOfFile;
                    self.discarding = true;
                    return self.packet(Kind::EndOfFile, b"D", now);
                }
                self.send_data(now, source)
            }
            Sending::EndOfFile => {
                if !self.discarding {
                    self.progress.files += 1;
                }
                if self.cancelling {
                    self.state = Sending::Break;
                    return self.packet(Kind::Break, b"", now);
                }
                self.next_file(now, source)
            }
            Sending::Break => {
                let status = if self.cancelling {
                    Status::Cancelled
                } else {
                    Status::Done
                };
                self.stop(status);
                Vec::new()
            }
            Sending::Finished => Vec::new(),
        }
    }

    fn next_file(&mut self, now: Instant, source: &mut dyn Source) -> Vec<u8> {
        match source.next_file() {
            Err(why) => self.fail(&why),
            Ok(None) => {
                self.state = Sending::Break;
                self.packet(Kind::Break, b"", now)
            }
            Ok(Some(name)) => {
                // A name too long for one packet is cut short: there is no
                // way to send the rest of it.
                let (encoded, _) = encode(name.as_bytes(), self.link.sending(), self.link.room());
                self.progress.file = Some(name);
                self.progress.bytes = 0;
                self.progress.size = source.size();
                self.mode = source.mode().unwrap_or(self.settings.mode);
                self.state = Sending::File;
                self.packet(Kind::File, &encoded, now)
            }
        }
    }

    fn send_data(&mut self, now: Instant, source: &mut dyn Source) -> Vec<u8> {
        let room = if self.link.long.is_some() {
            let room = self.link.room().min(self.size).min(self.ceiling);
            self.size = (self.size * 2).min(self.link.room()).min(self.ceiling);
            room
        } else {
            self.link.room()
        };
        // Read a packet's worth at a time, so the count of bytes sent is
        // never more than a packet ahead of what the far end has.
        let mut buf = vec![0u8; room.max(1)];
        while !self.eof && self.pending.len() < room {
            match source.read(&mut buf) {
                Ok(0) => self.eof = true,
                Ok(n) => {
                    self.progress.bytes += n as u64;
                    match self.mode {
                        Mode::Binary => self.pending.extend_from_slice(&buf[..n]),
                        Mode::Text => self.lines.convert(&buf[..n], &mut self.pending),
                    }
                }
                Err(why) => return self.fail(&why),
            }
        }
        if self.pending.is_empty() {
            self.state = Sending::EndOfFile;
            return self.packet(Kind::EndOfFile, b"", now);
        }
        let (encoded, used) = encode(&self.pending, self.link.sending(), room);
        if used == 0 {
            return self.fail("the other end's packets are too short to carry anything");
        }
        self.pending.drain(..used);
        self.in_flight = encoded.len();
        self.packet(Kind::Data, &encoded, now)
    }

    fn packet(&mut self, kind: Kind, data: &[u8], now: Instant) -> Vec<u8> {
        let check = self.link.check();
        self.link.send(self.seq, kind, data, check, now)
    }

    fn fail(&mut self, why: &str) -> Vec<u8> {
        let out = self.link.error(self.seq, why);
        self.stop(Status::Failed(why.to_string()));
        out
    }

    fn stop(&mut self, status: Status) {
        self.state = Sending::Finished;
        self.link.deadline = None;
        self.status = status;
    }
}
