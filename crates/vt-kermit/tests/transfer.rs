//! Whole transfers, with both ends of them here.
//!
//! Both ends being veetee's own is the weakness of this file: a peer written
//! alongside the thing it tests shares its misreadings and is too obliging to
//! find them. What it can do is hold the two ends to each other over a line
//! that loses, damages and repeats packets, with time that passes only when
//! the test says so. The exchanges written out by hand below follow the
//! protocol rather than this implementation, and interop with a real Kermit
//! is the next step (`docs/kermit.md`, K2).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use vt_kermit::{
    Attributes, Check, FileType, Kind, LineEnding, MARK, Mode, Packet, Params, Receiver, Sender,
    Settings, Source, Status, Store, encode, ours, read,
};

/// Files received, and whether each was finished as complete.
#[derive(Default)]
struct Received {
    files: Vec<(String, Vec<u8>, Option<bool>)>,
    refuse: bool,
}

impl Store for Received {
    fn create(&mut self, name: &str) -> Result<(), String> {
        if self.refuse {
            return Err(format!("{name} already exists"));
        }
        self.files.push((name.to_string(), Vec::new(), None));
        Ok(())
    }
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.files
            .last_mut()
            .expect("a file is open")
            .1
            .extend_from_slice(data);
        Ok(())
    }
    fn finish(&mut self, complete: bool) -> Result<(), String> {
        self.files.last_mut().expect("a file is open").2 = Some(complete);
        Ok(())
    }
}

/// Files to send, read in whatever size of piece the sender asks for.
struct Files {
    files: VecDeque<(String, Vec<u8>)>,
    current: Vec<u8>,
    at: usize,
}

impl Files {
    fn new(files: &[(&str, Vec<u8>)]) -> Files {
        Files {
            files: files
                .iter()
                .map(|(name, data)| (name.to_string(), data.clone()))
                .collect(),
            current: Vec::new(),
            at: 0,
        }
    }
}

impl Source for Files {
    fn next_file(&mut self) -> Result<Option<String>, String> {
        Ok(self.files.pop_front().map(|(name, data)| {
            self.current = data;
            self.at = 0;
            name
        }))
    }
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let n = buf.len().min(self.current.len() - self.at);
        buf[..n].copy_from_slice(&self.current[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
    fn size(&self) -> Option<u64> {
        Some(self.current.len() as u64)
    }
}

/// A small, repeatable source of chance.
struct Chance(u64);

impl Chance {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn one_in(&mut self, n: u64) -> bool {
        n > 0 && self.next().is_multiple_of(n)
    }
}

/// A line between the two ends, and how badly it behaves: each of these is
/// the chance, one in so many, of it happening to anything sent.
#[derive(Clone, Copy)]
struct Line {
    lose: u64,
    damage: u64,
    repeat: u64,
}

const CLEAN: Line = Line {
    lose: 0,
    damage: 0,
    repeat: 0,
};

impl Line {
    fn carry(&self, chance: &mut Chance, bytes: Vec<u8>, to: &mut VecDeque<Vec<u8>>) {
        if bytes.is_empty() || chance.one_in(self.lose) {
            return;
        }
        let mut bytes = bytes;
        if chance.one_in(self.damage) {
            let at = (chance.next() as usize) % bytes.len();
            bytes[at] ^= 1 << (chance.next() % 7);
        }
        if chance.one_in(self.repeat) {
            to.push_back(bytes.clone());
        }
        to.push_back(bytes);
    }
}

struct Outcome {
    sender: Status,
    receiver: Status,
    received: Received,
    /// Seconds that passed with nothing arriving.
    waited: u64,
}

/// Runs a transfer to its end, cancelling from either end once `cancel_after`
/// bytes have arrived, where asked.
fn transfer(
    files: &[(&str, Vec<u8>)],
    send: Settings,
    receive: Settings,
    line: Line,
    seed: u64,
    cancel: Option<(Side, u64)>,
) -> Outcome {
    let start = Instant::now();
    let mut now = start;
    let mut chance = Chance(seed | 1);
    let mut source = Files::new(files);
    let mut received = Received::default();
    let mut sender = Sender::new(send);
    let mut receiver = Receiver::new(receive, now);
    let (mut to_receiver, mut to_sender) = (VecDeque::new(), VecDeque::new());
    let mut cancelled = false;

    line.carry(&mut chance, sender.start(now), &mut to_receiver);
    for _ in 0..1_000_000 {
        let running = |s: &Status| *s == Status::Running;
        if !running(sender.status()) && !running(receiver.status()) {
            break;
        }
        if let Some((side, after)) = cancel
            && !cancelled
            && receiver.progress().bytes >= after
        {
            cancelled = true;
            match side {
                Side::Sender => line.carry(&mut chance, sender.cancel(), &mut to_receiver),
                Side::Receiver => {
                    let out = receiver.cancel(&mut received);
                    line.carry(&mut chance, out, &mut to_sender);
                }
            }
        }
        let mut moved = false;
        if let Some(bytes) = to_receiver.pop_front() {
            moved = true;
            let out = receiver.feed(&bytes, now, &mut received);
            line.carry(&mut chance, out, &mut to_sender);
        }
        if let Some(bytes) = to_sender.pop_front() {
            moved = true;
            let out = sender.feed(&bytes, now, &mut source);
            line.carry(&mut chance, out, &mut to_receiver);
        }
        if !moved {
            now += Duration::from_secs(1);
            let out = receiver.tick(now, &mut received);
            line.carry(&mut chance, out, &mut to_sender);
            let out = sender.tick(now);
            line.carry(&mut chance, out, &mut to_receiver);
        }
    }
    Outcome {
        sender: sender.status().clone(),
        receiver: receiver.status().clone(),
        received,
        waited: (now - start).as_secs(),
    }
}

#[derive(Clone, Copy)]
enum Side {
    Sender,
    Receiver,
}

fn binary() -> Settings {
    Settings {
        mode: Mode::Binary,
        ..Settings::default()
    }
}

/// Files of every awkward shape.
fn awkward() -> Vec<(&'static str, Vec<u8>)> {
    let mut noise = Chance(0x5eed);
    vec![
        ("EMPTY.DAT", Vec::new()),
        ("ONE.DAT", vec![b'x']),
        ("EVERY.DAT", (0..=255).collect()),
        ("RUNS.DAT", {
            let mut v = vec![0u8; 500];
            v.extend([b'~'; 200]);
            v.extend([b'#'; 3]);
            v.extend([0xff; 97]);
            v
        }),
        (
            "NOISE.DAT",
            (0..20_000).map(|_| noise.next() as u8).collect(),
        ),
    ]
}

fn assert_arrived(files: &[(&str, Vec<u8>)], outcome: &Outcome) {
    assert_eq!(outcome.sender, Status::Done);
    assert_eq!(outcome.receiver, Status::Done);
    assert_eq!(outcome.received.files.len(), files.len());
    for ((name, data), (got_name, got, complete)) in files.iter().zip(&outcome.received.files) {
        assert_eq!(got_name, &name.to_lowercase());
        assert_eq!(*complete, Some(true), "{name}");
        assert!(
            got == data,
            "{name}: {} bytes sent, {} arrived",
            data.len(),
            got.len()
        );
    }
}

#[test]
fn every_kind_of_file_arrives_as_it_was_sent() {
    let files = awkward();
    let outcome = transfer(&files, binary(), binary(), CLEAN, 1, None);
    assert_arrived(&files, &outcome);
    assert_eq!(outcome.waited, 0, "a clean line never waits");
}

#[test]
fn a_line_that_loses_and_repeats_still_delivers() {
    // No damage here: the one-character check misses about one damaged
    // packet in sixty-four where the damage is to the length (it moves where
    // the check is read from), which a soak of a thousand transfers with one
    // packet in five damaged turned into about two transfers in a hundred
    // arriving wrong and reporting success. That is the check's weakness,
    // not this code's, and the CRC soak below is the one that damages.
    let files = awkward();
    let bad = Line {
        lose: 7,
        damage: 0,
        repeat: 7,
    };
    let one = Settings {
        params: Params {
            check: Check::One,
            ..ours()
        },
        ..binary()
    };
    for seed in 1..=40 {
        let outcome = transfer(&files, one.clone(), one.clone(), bad, seed, None);
        assert_arrived(&files, &outcome);
    }
}

#[test]
fn without_repeat_or_eighth_bit_prefixing_it_all_still_arrives() {
    // What a minimal Kermit offers: neither prefix, so a byte with its top
    // bit set goes as it is and the line has to carry it.
    let plain = Settings {
        params: Params {
            quote_eighth: None,
            repeat: None,
            ..ours()
        },
        ..binary()
    };
    let files = awkward();
    let outcome = transfer(&files, plain.clone(), binary(), CLEAN, 1, None);
    assert_arrived(&files, &outcome);
    let outcome = transfer(&files, binary(), plain, CLEAN, 1, None);
    assert_arrived(&files, &outcome);
}

#[test]
fn with_the_crc_a_line_that_also_damages_still_delivers() {
    let crc = Settings {
        params: Params {
            check: Check::Three,
            ..ours()
        },
        ..binary()
    };
    let files = awkward();
    let bad = Line {
        lose: 9,
        damage: 5,
        repeat: 9,
    };
    for seed in 1..=40 {
        assert_arrived(
            &files,
            &transfer(&files, crc.clone(), crc.clone(), bad, seed, None),
        );
    }
}

#[test]
fn text_arrives_with_the_local_line_endings() {
    let text = |local| Settings {
        local,
        ..Settings::default()
    };
    let files = [("NOTES.TXT", b"one\ntwo\r\nthree\n\nlast".to_vec())];
    let outcome = transfer(
        &files,
        text(LineEnding::Lf),
        text(LineEnding::Lf),
        CLEAN,
        1,
        None,
    );
    assert_eq!(outcome.received.files[0].1, b"one\ntwo\nthree\n\nlast");
    let outcome = transfer(
        &files,
        text(LineEnding::Lf),
        text(LineEnding::CrLf),
        CLEAN,
        1,
        None,
    );
    assert_eq!(
        outcome.received.files[0].1,
        b"one\r\ntwo\r\nthree\r\n\r\nlast"
    );
}

#[test]
fn a_refused_file_stops_both_ends_and_says_why() {
    let files = [("LOGIN.COM", b"$ exit".to_vec())];
    let mut settings = binary();
    settings.retries = 3;
    let start = Instant::now();
    let mut sender = Sender::new(settings.clone());
    let mut receiver = Receiver::new(settings, start);
    let mut source = Files::new(&files);
    let mut store = Received {
        refuse: true,
        ..Received::default()
    };
    let mut out = sender.start(start);
    for _ in 0..10 {
        let back = receiver.feed(&out, start, &mut store);
        out = sender.feed(&back, start, &mut source);
    }
    let Status::Failed(why) = sender.status() else {
        panic!("{:?}", sender.status())
    };
    assert!(why.contains("login.com already exists"), "{why}");
    assert!(matches!(receiver.status(), Status::Failed(_)));
}

#[test]
fn cancelling_from_either_end_stops_tidily_and_keeps_nothing_half_done() {
    let files = awkward();
    for side in [Side::Receiver, Side::Sender] {
        let outcome = transfer(&files, binary(), binary(), CLEAN, 1, Some((side, 5_000)));
        assert_eq!(outcome.sender, Status::Cancelled);
        assert_eq!(outcome.receiver, Status::Cancelled);
        let (name, _, complete) = outcome.received.files.last().unwrap();
        assert_eq!(name, "noise.dat", "the file under way when it stopped");
        assert_eq!(*complete, Some(false), "and it is not kept");
        assert_eq!(
            outcome.received.files.len(),
            files.len(),
            "and none after it"
        );
    }
}

#[test]
fn a_sender_that_hears_nothing_gives_up() {
    let mut sender = Sender::new(Settings {
        retries: 3,
        ..binary()
    });
    let start = Instant::now();
    let first = sender.start(start);
    let mut now = start;
    let mut sent = vec![first.clone()];
    while *sender.status() == Status::Running {
        now += Duration::from_secs(1);
        let out = sender.tick(now);
        if !out.is_empty() {
            sent.push(out);
        }
    }
    assert!(matches!(sender.status(), Status::Failed(_)));
    assert_eq!(
        sent.len(),
        5,
        "the send-init, three more tries and an error"
    );
    assert!(
        sent[1..4].iter().all(|s| *s == first),
        "the same packet each time"
    );
    assert_eq!(sent[4][3], b'E');
}

#[test]
fn a_receiver_waiting_asks_for_the_send_init() {
    let start = Instant::now();
    let mut receiver = Receiver::new(binary(), start);
    let mut store = Received::default();
    assert!(receiver.tick(start, &mut store).is_empty(), "not yet");
    let nak = receiver.tick(start + Duration::from_secs(10), &mut store);
    let (packet, _) = read(&nak, Check::One, MARK).expect("a nak");
    assert_eq!((packet.kind, packet.sequence), (Kind::Nak, 0));
}

/// The far end of the exchanges written by hand: veetee's own parameters,
/// but with the one-character check the hand-built packets carry. So each of
/// these is also a Kermit that cannot do the CRC, answered by veetee falling
/// back to type 1.
fn far() -> Vec<u8> {
    Params {
        check: Check::One,
        ..ours()
    }
    .build()
}

/// The same, offering no attribute packets either, for exchanges about
/// something else.
fn plain() -> Vec<u8> {
    Params {
        check: Check::One,
        capabilities: Vec::new(),
        ..ours()
    }
    .build()
}

/// Builds a packet as a far end would, for the exchanges written by hand.
fn packet(seq: u8, kind: Kind, data: &[u8]) -> Vec<u8> {
    Packet {
        sequence: seq,
        kind,
        data,
    }
    .build(Check::One, MARK, Some(b'\r'))
}

fn only(bytes: &[u8]) -> (u8, Kind, Vec<u8>) {
    let (p, used) = read(bytes, Check::One, MARK).expect("a packet");
    assert_eq!(used + 1, bytes.len(), "one packet and its end of line");
    (p.sequence, p.kind, p.data.to_vec())
}

/// G-Kermit 2.01's own send-init, as `init.rs` has it.
const GKERMIT_SEND_INIT: &[u8] = &[
    0x01, 0x39, 0x20, 0x53, 0x7e, 0x27, 0x20, 0x40, 0x2d, 0x23, 0x59, 0x33, 0x7e, 0x2a, 0x21, 0x4a,
    0x2a, 0x30, 0x2b, 0x2b, 0x2b, 0x42, 0x22, 0x55, 0x31, 0x40, 0x47, 0x0d,
];

#[test]
fn a_real_kermits_send_init_is_answered_and_the_check_agreed_is_kept_to() {
    // Both ends asking for the CRC agree on it; one asking for the CRC and
    // answered 1 falls back to 1. Either way the next packet has to be read
    // with the check agreed, and a nak would mean it was not.
    for (asking, agreed_check) in [(Check::Three, Check::Three), (Check::One, Check::One)] {
        let start = Instant::now();
        let settings = Settings {
            params: Params {
                check: asking,
                ..ours()
            },
            ..binary()
        };
        let mut receiver = Receiver::new(settings, start);
        let mut store = Received::default();
        let answer = receiver.feed(GKERMIT_SEND_INIT, start, &mut store);
        let (seq, kind, data) = only(&answer);
        assert_eq!((seq, kind), (0, Kind::Ack));
        let ours_read = Params::read(&data);
        assert_eq!(ours_read.check, asking);
        assert_eq!(ours_read.quote_eighth, Some(b'&'));

        let agreed = Params::agreed(&Params::read(&GKERMIT_SEND_INIT[4..26]), &ours_read);
        assert_eq!(agreed.check, agreed_check);
        let (name, _) = encode(b"notes.txt", &agreed, 80);
        let file = Packet {
            sequence: 1,
            kind: Kind::File,
            data: &name,
        }
        .build(agreed_check, MARK, Some(b'\r'));
        let answer = receiver.feed(&file, start, &mut store);
        let (p, _) = read(&answer, agreed_check, MARK).expect("an answer");
        assert_eq!(
            p.kind,
            Kind::Ack,
            "{asking:?}: not a nak, the check was right"
        );
        assert_eq!(store.files[0].0, "notes.txt");
    }
}

#[test]
fn a_repeated_packet_is_acknowledged_again_and_written_once() {
    let start = Instant::now();
    let mut receiver = Receiver::new(binary(), start);
    let mut store = Received::default();
    let init = far();
    receiver.feed(&packet(0, Kind::SendInit, &init), start, &mut store);
    receiver.feed(&packet(1, Kind::File, b"A.DAT"), start, &mut store);
    let first = receiver.feed(&packet(2, Kind::Data, b"hello"), start, &mut store);
    let again = receiver.feed(&packet(2, Kind::Data, b"hello"), start, &mut store);
    assert_eq!(first, again, "the same answer");
    assert_eq!(store.files[0].1, b"hello", "and the data once");

    // The send-init repeated after the file has begun is too old to answer:
    // it is neither the packet expected nor the one before it.
    let stale = receiver.feed(&packet(0, Kind::SendInit, &init), start, &mut store);
    assert!(stale.is_empty());
}

#[test]
fn a_nak_for_the_next_packet_is_an_acknowledgement_of_this_one() {
    let start = Instant::now();
    let mut sender = Sender::new(binary());
    let mut source = Files::new(&[("A.DAT", b"abc".to_vec())]);
    sender.start(start);
    sender.feed(&packet(0, Kind::Ack, &plain()), start, &mut source);
    // The answer to the file's name was lost, and all the sender hears is
    // the receiver asking for packet 2.
    let out = sender.feed(&packet(2, Kind::Nak, b""), start, &mut source);
    assert_eq!(only(&out), (2, Kind::Data, b"abc".to_vec()));
    // A nak for the packet it has just sent is a request to send it again.
    let again = sender.feed(&packet(2, Kind::Nak, b""), start, &mut source);
    assert_eq!(again, out);
}

#[test]
fn but_not_of_the_send_init_whose_answer_carries_the_parameters() {
    let start = Instant::now();
    let mut sender = Sender::new(binary());
    let mut source = Files::new(&[("A.DAT", b"abc".to_vec())]);
    let init = sender.start(start);
    // The receiver answered the send-init, the answer was lost, and the
    // receiver timed out first and asked for packet 1. Going on would mean
    // going on without its parameters, so the send-init goes again.
    let out = sender.feed(&packet(1, Kind::Nak, b""), start, &mut source);
    assert_eq!(out, init);
}

#[test]
fn a_receiver_that_says_x_skips_that_file_and_not_the_rest() {
    let start = Instant::now();
    let mut sender = Sender::new(binary());
    let mut source = Files::new(&[
        ("SKIP.DAT", vec![b'a'; 1000]),
        ("KEEP.DAT", b"kept".to_vec()),
    ]);
    sender.start(start);
    sender.feed(&packet(0, Kind::Ack, &plain()), start, &mut source);
    let data = sender.feed(&packet(1, Kind::Ack, b""), start, &mut source);
    assert_eq!(only(&data).1, Kind::Data);
    let end = sender.feed(&packet(2, Kind::Ack, b"X"), start, &mut source);
    assert_eq!(
        only(&end),
        (3, Kind::EndOfFile, b"D".to_vec()),
        "discard it"
    );
    let next = sender.feed(&packet(3, Kind::Ack, b""), start, &mut source);
    let (_, kind, name) = only(&next);
    assert_eq!((kind, name.as_slice()), (Kind::File, &b"KEEP.DAT"[..]));
    assert_eq!(
        sender.progress().files,
        0,
        "the skipped file does not count"
    );
}

#[test]
fn an_error_from_the_far_end_is_reported_in_its_own_words() {
    let start = Instant::now();
    let mut receiver = Receiver::new(binary(), start);
    let mut store = Received::default();
    receiver.feed(&packet(0, Kind::SendInit, &far()), start, &mut store);
    receiver.feed(&packet(1, Kind::File, b"A.DAT"), start, &mut store);
    receiver.feed(
        &packet(2, Kind::Error, b"%RMS-E-FNF, file not found"),
        start,
        &mut store,
    );
    assert_eq!(
        receiver.status(),
        &Status::Failed("the other end stopped: %RMS-E-FNF, file not found".into())
    );
    assert_eq!(
        store.files[0].2,
        Some(false),
        "the half-written file is not kept"
    );
}

#[test]
fn a_sender_whose_last_answer_is_lost_still_finishes() {
    // Everything arrived; only the answer to "no more files" is missing, and
    // the receiver may well have gone. The files are all acknowledged, so
    // that is not a failure.
    let start = Instant::now();
    let mut sender = Sender::new(Settings {
        retries: 2,
        ..binary()
    });
    let mut source = Files::new(&[]);
    sender.start(start);
    let out = sender.feed(&packet(0, Kind::Ack, &far()), start, &mut source);
    assert_eq!(only(&out).1, Kind::Break);
    let mut now = start;
    while *sender.status() == Status::Running {
        now += Duration::from_secs(10);
        sender.tick(now);
    }
    assert_eq!(sender.status(), &Status::Done);
}

#[test]
fn a_hostile_file_name_is_refused_or_made_safe() {
    let start = Instant::now();
    let mut receiver = Receiver::new(binary(), start);
    let mut store = Received::default();
    receiver.feed(&packet(0, Kind::SendInit, &far()), start, &mut store);
    receiver.feed(&packet(1, Kind::File, b"../../.PROFILE"), start, &mut store);
    assert_eq!(store.files[0].0, ".profile");

    let mut receiver = Receiver::new(binary(), start);
    receiver.feed(&packet(0, Kind::SendInit, &far()), start, &mut store);
    let out = receiver.feed(&packet(1, Kind::File, b"[SYSMGR]"), start, &mut store);
    assert_eq!(only(&out).1, Kind::Error);
    assert!(matches!(receiver.status(), Status::Failed(_)));
}

#[test]
fn where_both_offer_them_the_sender_says_what_the_file_is() {
    let start = Instant::now();
    let mut source = Files::new(&[("NOTES.TXT", b"one\ntwo\n".to_vec())]);
    let mut sender = Sender::new(Settings::default());
    sender.start(start);
    sender.feed(&packet(0, Kind::Ack, &far()), start, &mut source);
    let out = sender.feed(&packet(1, Kind::Ack, b""), start, &mut source);
    let (seq, kind, data) = only(&out);
    assert_eq!((seq, kind), (2, Kind::Attributes));
    let said = Attributes::read(&data);
    assert_eq!(said.file_type, Some(FileType::Text));
    assert_eq!(said.size, Some(8), "the file's own size, not the line's");
    let out = sender.feed(&packet(2, Kind::Ack, b"Y"), start, &mut source);
    assert_eq!(only(&out).1, Kind::Data, "and then its contents");

    // A far end that does not offer them gets none.
    let mut source = Files::new(&[("NOTES.TXT", b"one\n".to_vec())]);
    let mut sender = Sender::new(Settings::default());
    sender.start(start);
    sender.feed(&packet(0, Kind::Ack, &plain()), start, &mut source);
    let out = sender.feed(&packet(1, Kind::Ack, b""), start, &mut source);
    assert_eq!(only(&out).1, Kind::Data);
}

#[test]
fn a_file_refused_on_its_attributes_is_skipped_and_the_rest_sent() {
    let start = Instant::now();
    let mut source = Files::new(&[("BIG.DAT", vec![0; 5000]), ("SMALL.DAT", b"x".to_vec())]);
    let mut sender = Sender::new(binary());
    sender.start(start);
    sender.feed(&packet(0, Kind::Ack, &far()), start, &mut source);
    sender.feed(&packet(1, Kind::Ack, b""), start, &mut source);
    // N, and the tag of what it objects to: the size.
    let out = sender.feed(&packet(2, Kind::Ack, b"N1"), start, &mut source);
    assert_eq!(only(&out), (3, Kind::EndOfFile, b"D".to_vec()));
    let out = sender.feed(&packet(3, Kind::Ack, b""), start, &mut source);
    let (_, kind, name) = only(&out);
    assert_eq!((kind, name.as_slice()), (Kind::File, &b"SMALL.DAT"[..]));
    assert_eq!(sender.progress().files, 0);
}

#[test]
fn the_senders_word_on_text_or_binary_beats_the_receivers_setting() {
    // What C-Kermit does left to itself: it looks at each file and says.
    let files = [
        ("NOTES.TXT", b"one\ntwo\n".to_vec()),
        ("DATA.BIN", b"one\r\ntwo".to_vec()),
    ];
    let start = Instant::now();
    for (receiving, said, sent, arrives) in [
        (
            Mode::Binary,
            &b"\"#AMJ"[..],
            &files[0].1,
            &b"one\ntwo\n"[..],
        ),
        (Mode::Text, &b"\"\"B8"[..], &files[1].1, &b"one\r\ntwo"[..]),
    ] {
        let mut receiver = Receiver::new(
            Settings {
                mode: receiving,
                local: LineEnding::Lf,
                ..Settings::default()
            },
            start,
        );
        let mut store = Received::default();
        receiver.feed(&packet(0, Kind::SendInit, &far()), start, &mut store);
        receiver.feed(&packet(1, Kind::File, b"F"), start, &mut store);
        let out = receiver.feed(&packet(2, Kind::Attributes, said), start, &mut store);
        assert_eq!(only(&out), (2, Kind::Ack, b"Y".to_vec()));
        // As text, the line form: CR LF, sent as #M#J.
        let data: Vec<u8> = if said.starts_with(b"\"#A") {
            b"one#M#Jtwo#M#J".to_vec()
        } else {
            sent.iter()
                .flat_map(|&b| match b {
                    b'\r' => b"#M".to_vec(),
                    b'\n' => b"#J".to_vec(),
                    b => vec![b],
                })
                .collect()
        };
        receiver.feed(&packet(3, Kind::Data, &data), start, &mut store);
        receiver.feed(&packet(4, Kind::EndOfFile, b""), start, &mut store);
        assert_eq!(
            store.files[0].1,
            arrives,
            "{:?}",
            String::from_utf8_lossy(said)
        );
    }
}

#[test]
fn over_a_bad_line_attributes_and_all_still_arrive() {
    // Both ends are veetee, which offers attributes, so every file here goes
    // with an attribute packet; the receiver is set to text and the sender
    // says binary, which has to win for these files to arrive intact.
    let files = awkward();
    let text_receiver = Settings {
        mode: Mode::Text,
        ..Settings::default()
    };
    let bad = Line {
        lose: 7,
        damage: 0,
        repeat: 7,
    };
    for seed in 1..=10 {
        assert_arrived(
            &files,
            &transfer(&files, binary(), text_receiver.clone(), bad, seed, None),
        );
    }
}
