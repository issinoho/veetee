//! Kermit packets, built and read without any I/O.
//!
//! A terminal on a serial console or a LAT line has one channel and no other:
//! no SCP, no FTP, nothing to fall back on. Kermit is how a file leaves such a
//! machine, and it is the DEC answer as much as anything is — OpenVMS ships
//! KERMIT-32, and the protocol was designed for exactly the lines DEC
//! terminals live on: seven data bits, parity in the eighth, XON/XOFF holding
//! the flow, and a host that will not pass a control character unescaped.
//! Everything travels as printable ASCII for that reason.
//!
//! This is the protocol and nothing else — no files, no sockets, no timers, on
//! the same footing as [`vt_lat`](../vt_lat/index.html). What arrives is
//! handed to [`read`] and what should go out comes from [`Packet::build`], so
//! the whole of it is testable against known bytes on any platform. A whole
//! transfer is a [`Sender`] or a [`Receiver`], which are driven the same way:
//! bytes and the time in, bytes to send out, and files through [`Source`] and
//! [`Store`].
//!
//! Written from the protocol specification (Frank da Cruz, *Kermit: A File
//! Transfer Protocol*, Digital Press 1987, and the Kermit Protocol Manual).
//! No Kermit implementation's source has been read: gkermit and C-Kermit serve
//! as counterparties to test against over a pipe, never as a reference to
//! copy, gkermit being GPL.

pub mod attributes;
mod data;
pub mod files;
mod init;
pub mod names;
pub mod text;
mod transfer;

pub use attributes::{Attributes, FileType};
pub use data::{decode, encode};
pub use init::{Params, ours};
pub use names::{Names, local_name};
pub use text::LineEnding;
pub use transfer::{Mode, Progress, Receiver, Sender, Settings, Source, Status, Store};

/// A value carried as a printable character, which is how Kermit passes
/// numbers through a line that may not be eight bits clean.
///
/// Nought becomes a space and ninety-four becomes a tilde, so every count,
/// length and sequence number in a packet is something a host will hand on
/// unaltered.
#[must_use]
pub fn tochar(n: u8) -> u8 {
    n.wrapping_add(32)
}

/// The reverse of [`tochar`].
#[must_use]
pub fn unchar(c: u8) -> u8 {
    c.wrapping_sub(32)
}

/// Turns a control character into a printable one and back again, the pair
/// that makes control quoting work: it is its own inverse.
#[must_use]
pub fn ctl(c: u8) -> u8 {
    c ^ 64
}

/// The largest value [`tochar`] can carry, and so the longest a short
/// packet's contents may be.
pub const MAX_COUNT: u8 = 94;

/// What guards a packet, agreed between the two ends at the start.
///
/// One character is the default and what every implementation supports; the
/// other two buy more certainty on a noisy line at the cost of a character or
/// two of every packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Check {
    /// A six-bit arithmetic checksum folded into one character.
    #[default]
    One,
    /// A twelve-bit arithmetic checksum in two characters.
    Two,
    /// A sixteen-bit CRC in three characters.
    ///
    /// What veetee asks for, proved against C-Kermit and G-Kermit in both
    /// directions (`crates/vt-headless/tests/kermit_interop.rs`).
    Three,
}

impl Check {
    /// How many characters the check takes up.
    #[must_use]
    pub fn chars(self) -> usize {
        match self {
            Check::One => 1,
            Check::Two => 2,
            Check::Three => 3,
        }
    }

    /// The character the far end names it by in a send-init.
    #[must_use]
    pub fn as_byte(self) -> u8 {
        match self {
            Check::One => b'1',
            Check::Two => b'2',
            Check::Three => b'3',
        }
    }

    /// The check a send-init asked for, if it is one that can be honoured.
    #[must_use]
    pub fn from_byte(c: u8) -> Option<Check> {
        match c {
            b'1' => Some(Check::One),
            b'2' => Some(Check::Two),
            b'3' => Some(Check::Three),
            _ => None,
        }
    }

    /// Works the check out over the characters between the mark and the check
    /// itself, which is everything the far end also counted.
    #[must_use]
    pub fn of(self, covered: &[u8]) -> Vec<u8> {
        match self {
            Check::One => {
                let sum = sum(covered);
                // The two top bits are folded back in, so that all eight bits
                // of every character reach the six that are sent.
                vec![tochar(((sum + ((sum & 0xc0) >> 6)) & 0x3f) as u8)]
            }
            Check::Two => {
                let sum = sum(covered);
                vec![
                    tochar(((sum >> 6) & 0x3f) as u8),
                    tochar((sum & 0x3f) as u8),
                ]
            }
            Check::Three => {
                let crc = crc16(covered);
                vec![
                    tochar(((crc >> 12) & 0x0f) as u8),
                    tochar_six(crc >> 6),
                    tochar_six(crc),
                ]
            }
        }
    }
}

fn sum(covered: &[u8]) -> u32 {
    covered.iter().map(|&b| u32::from(b)).sum()
}

fn tochar_six(v: u16) -> u8 {
    tochar((v & 0x3f) as u8)
}

/// Kermit's CRC-16, a nibble at a time so there is no table to carry.
fn crc16(covered: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in covered {
        let mut q = (crc ^ u16::from(byte)) & 0x0f;
        crc = (crc >> 4) ^ q.wrapping_mul(0x1081);
        q = (crc ^ (u16::from(byte) >> 4)) & 0x0f;
        crc = (crc >> 4) ^ q.wrapping_mul(0x1081);
    }
    crc
}

/// What a packet is for, which is its single-letter type on the wire.
///
/// Only the types a file transfer needs are named. Anything else is kept as
/// it arrived rather than guessed at: a server command veetee cannot honour
/// is answered with an error, not misread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Send-init: the parameters each end proposes, answered by an ack
    /// carrying the other end's.
    SendInit,
    /// A file is coming, and this is its name.
    File,
    /// The file's attributes — size, type, and so on. Optional both ways.
    Attributes,
    /// A file's contents, encoded.
    Data,
    /// End of this file.
    EndOfFile,
    /// End of transmission: there are no more files.
    Break,
    /// Acknowledged. A send-init's ack carries the answering parameters.
    Ack,
    /// Not acknowledged: send it again.
    Nak,
    /// Something went wrong, and the text says what.
    Error,
    /// A type veetee does not act on, kept whole.
    Other(u8),
}

impl Kind {
    #[must_use]
    pub fn as_byte(self) -> u8 {
        match self {
            Kind::SendInit => b'S',
            Kind::File => b'F',
            Kind::Attributes => b'A',
            Kind::Data => b'D',
            Kind::EndOfFile => b'Z',
            Kind::Break => b'B',
            Kind::Ack => b'Y',
            Kind::Nak => b'N',
            Kind::Error => b'E',
            Kind::Other(c) => c,
        }
    }

    #[must_use]
    pub fn from_byte(c: u8) -> Kind {
        match c {
            b'S' => Kind::SendInit,
            b'F' => Kind::File,
            b'A' => Kind::Attributes,
            b'D' => Kind::Data,
            b'Z' => Kind::EndOfFile,
            b'B' => Kind::Break,
            b'Y' => Kind::Ack,
            b'N' => Kind::Nak,
            b'E' => Kind::Error,
            other => Kind::Other(other),
        }
    }
}

/// One packet: what it is, which in the sequence, and what it carries.
///
/// The data is the packet's own field, still encoded — control quoting and
/// the rest are the business of whatever is being carried, not of the packet
/// that carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet<'a> {
    /// Counts 0 to 63 and begins again, so a packet arriving twice can be
    /// told from the next one.
    pub sequence: u8,
    pub kind: Kind,
    pub data: &'a [u8],
}

/// Why a run of bytes was not a packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// No mark yet, or not all of the packet has arrived. Keep the bytes and
    /// try again when more turn up.
    Incomplete,
    /// The length said more than a packet can hold: past 94 in a short
    /// packet's length (95 is allowed, see [`read`]), or the extra-long form,
    /// which is not read.
    TooLong,
    /// The check did not match, so the packet is not to be trusted. The far
    /// end will send it again when it hears nothing, or a nak.
    BadCheck,
}

/// The character that starts a packet, unless the far end asks for another.
pub const MARK: u8 = 0x01;

/// The most an extended packet's two length characters can count: data and
/// check together, 94 × 95 + 94.
pub const MAX_LONG: usize = 9024;

impl Packet<'_> {
    /// The packet as it goes on the wire: mark, length, sequence, type, data,
    /// check, and whatever the far end wants at the end of a line.
    ///
    /// The length counts everything after itself including the check, which
    /// is why it depends on which check was agreed.
    ///
    /// Data too long for a short packet goes in the extended form: a length of
    /// nought, then after the type two characters counting the data and the
    /// check, and a one-character check on the header alone, so that a
    /// damaged length is caught before anything waits for thousands of bytes
    /// that are not coming. Only a far end that has offered long packets is
    /// sent one; see [`Params::long`].
    #[must_use]
    pub fn build(&self, check: Check, mark: u8, eol: Option<u8>) -> Vec<u8> {
        let count = 3 + self.data.len() + check.chars() - 1;
        if count > MAX_COUNT as usize {
            let n = self.data.len() + check.chars();
            debug_assert!(n <= MAX_LONG, "longer than the extended form counts");
            let mut out = vec![
                mark,
                tochar(0),
                tochar(self.sequence),
                self.kind.as_byte(),
                tochar((n / 95) as u8),
                tochar((n % 95) as u8),
            ];
            out.extend(Check::One.of(&out[1..]));
            out.extend_from_slice(self.data);
            // The block check covers the extended header, its check
            // included, as well as the data.
            out.extend_from_slice(&check.of(&out[1..]));
            out.extend(eol);
            return out;
        }
        let mut out = vec![mark, tochar(count as u8), tochar(self.sequence)];
        out.push(self.kind.as_byte());
        out.extend_from_slice(self.data);
        // Everything from the length to the end of the data is covered; the
        // mark is not, being a marker rather than part of the packet.
        out.extend_from_slice(&check.of(&out[1..]));
        out.extend(eol);
        out
    }

    /// How long a packet's data may be, for a given check and the longest
    /// count a short packet can carry.
    #[must_use]
    pub fn max_data(check: Check) -> usize {
        MAX_COUNT as usize - 2 - check.chars()
    }
}

/// Reads the first packet in `bytes`, and says how much of them it used.
///
/// Anything before the mark is skipped: a host echoes, and a line picks up
/// noise, so the bytes before a packet are not the packet's concern. What is
/// left over is not consumed, so a caller can keep it and read again.
pub fn read(bytes: &[u8], check: Check, mark: u8) -> Result<(Packet<'_>, usize), Error> {
    let start = bytes
        .iter()
        .position(|&b| b == mark)
        .ok_or(Error::Incomplete)?;
    let rest = &bytes[start..];
    // Mark, length, sequence and type, before any data at all.
    if rest.len() < 4 {
        return Err(Error::Incomplete);
    }
    let count = unchar(rest[1]) as usize;
    if count == 0 {
        return read_long(rest, check, start);
    }
    // Ninety-four is the most a short packet may count, and what veetee asks
    // for. C-Kermit, asked for 94 and sending with the CRC, sends 95 — the
    // length travelling as DEL — one character over (seen against C-Kermit
    // 10.0 Beta.12, `vt-headless kermit receive --check 3`). The length is
    // still unambiguous, and naking a packet the far end will only send the
    // same way again would end the transfer, so 95 is read. Nothing past it
    // is: that is not a seven-bit character at all.
    if count > MAX_COUNT as usize + 1 {
        return Err(Error::TooLong);
    }
    // The count covers everything after the length, so the packet is that
    // much again plus the mark and the length themselves.
    let total = count + 2;
    if rest.len() < total {
        return Err(Error::Incomplete);
    }
    let packet = &rest[..total];
    // Too short to hold a sequence number, a type and its check: whatever
    // this is, it is all here and it is not a packet. Waiting for more would
    // wait for ever, the length having already said there is no more.
    if total < 4 + check.chars() {
        return Err(Error::BadCheck);
    }
    let data_end = total - check.chars();
    let expected = check.of(&packet[1..data_end]);
    if packet[data_end..] != expected[..] {
        return Err(Error::BadCheck);
    }
    Ok((
        Packet {
            sequence: unchar(packet[2]),
            kind: Kind::from_byte(packet[3]),
            data: &packet[4..data_end],
        },
        start + total,
    ))
}

/// The extended form, from its mark on.
fn read_long(rest: &[u8], check: Check, start: usize) -> Result<(Packet<'_>, usize), Error> {
    // Mark, length, sequence, type, two length characters, header check.
    const HEADER: usize = 7;
    if rest.len() < HEADER {
        return Err(Error::Incomplete);
    }
    if Check::One.of(&rest[1..6])[0] != rest[6] {
        return Err(Error::BadCheck);
    }
    let n = usize::from(unchar(rest[4])) * 95 + usize::from(unchar(rest[5]));
    // C-Kermit goes one over here as it does with short packets: offered
    // 9024 and sending with the CRC, it sends 9025, the high digit travelling
    // as DEL (C-Kermit 10.0 Beta.12, a 200 KB file, its tenth data packet).
    // One over is read, as 95 is; nothing more.
    if n > MAX_LONG + 1 {
        return Err(Error::TooLong);
    }
    if n < check.chars() {
        return Err(Error::BadCheck);
    }
    let total = HEADER + n;
    if rest.len() < total {
        return Err(Error::Incomplete);
    }
    let packet = &rest[..total];
    let data_end = total - check.chars();
    if packet[data_end..] != check.of(&packet[1..data_end])[..] {
        return Err(Error::BadCheck);
    }
    Ok((
        Packet {
            sequence: unchar(packet[2]),
            kind: Kind::from_byte(packet[3]),
            data: &packet[HEADER..data_end],
        },
        start + total,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_travels_as_a_printable_character() {
        assert_eq!(tochar(0), b' ', "nought is a space");
        assert_eq!(tochar(94), b'~', "and ninety-four a tilde");
        for n in 0..=MAX_COUNT {
            assert_eq!(unchar(tochar(n)), n);
            assert!(
                tochar(n).is_ascii_graphic() || tochar(n) == b' ',
                "{n} becomes {:?}, which a seven-bit line may not carry",
                tochar(n) as char
            );
        }
    }

    #[test]
    fn control_quoting_is_its_own_inverse() {
        for c in 0..=255u8 {
            assert_eq!(ctl(ctl(c)), c);
        }
        assert_eq!(ctl(0x0d), b'M', "a carriage return travels as M");
        assert_eq!(ctl(0x00), b'@', "and a null as @");
    }

    /// An acknowledgement of the first packet, worked out by hand from the
    /// specification: the length counts three (sequence, type and check), so
    /// the characters covered are `#`, a space and `Y` — 35, 32 and 89, which
    /// sum to 156. Folding the top two bits back in gives 158, and the low
    /// six bits of that are 30, which travels as `>`.
    #[test]
    fn an_acknowledgement_is_built_as_the_specification_says() {
        let ack = Packet {
            sequence: 0,
            kind: Kind::Ack,
            data: &[],
        };
        assert_eq!(
            ack.build(Check::One, MARK, Some(b'\r')),
            b"\x01# Y>\r",
            "mark, length, sequence, type, check, and the end of the line"
        );
    }

    #[test]
    fn a_packet_reads_back_as_it_was_built() {
        for (kind, data) in [
            (Kind::Ack, &b""[..]),
            (Kind::File, &b"LOGIN.COM"[..]),
            (Kind::Data, &b"#M#J"[..]),
            (Kind::Error, &b"no such file"[..]),
            (Kind::Other(b'G'), &b"F"[..]),
        ] {
            for check in [Check::One, Check::Two, Check::Three] {
                let sent = Packet {
                    sequence: 27,
                    kind,
                    data,
                };
                let wire = sent.build(check, MARK, Some(b'\r'));
                let (read_back, used) = read(&wire, check, MARK)
                    .unwrap_or_else(|e| panic!("{kind:?} {check:?}: {e:?}"));
                assert_eq!(read_back, sent, "{kind:?} with {check:?}");
                assert_eq!(
                    used,
                    wire.len() - 1,
                    "the end of the line is not part of the packet"
                );
            }
        }
    }

    #[test]
    fn whatever_comes_before_the_mark_is_not_the_packets_concern() {
        let ack = Packet {
            sequence: 3,
            kind: Kind::Ack,
            data: &[],
        };
        let mut wire = b"login echoed this\r\n".to_vec();
        let packet = ack.build(Check::One, MARK, Some(b'\r'));
        wire.extend_from_slice(&packet);
        let (read_back, used) = read(&wire, Check::One, MARK).expect("a packet is in there");
        assert_eq!(read_back, ack);
        assert_eq!(used, wire.len() - 1, "the noise is counted as consumed");
    }

    #[test]
    fn a_packet_that_has_not_all_arrived_is_waited_for() {
        let whole = Packet {
            sequence: 1,
            kind: Kind::Data,
            data: b"half a packet",
        }
        .build(Check::One, MARK, None);
        for upto in 0..whole.len() {
            assert_eq!(
                read(&whole[..upto], Check::One, MARK).map(|(_, used)| used),
                Err(Error::Incomplete),
                "{upto} of {} bytes should not read as a packet",
                whole.len()
            );
        }
        assert!(read(&whole, Check::One, MARK).is_ok(), "all of it does");
    }

    #[test]
    fn a_packet_the_line_damaged_is_refused_rather_than_read() {
        let whole = Packet {
            sequence: 9,
            kind: Kind::Data,
            data: b"SYS$SYSTEM",
        }
        .build(Check::One, MARK, None);
        for at in 1..whole.len() {
            let mut damaged = whole.clone();
            // A bit picked up on the line, in a character the check covers.
            damaged[at] ^= 0x01;
            match read(&damaged, Check::One, MARK) {
                Err(Error::BadCheck) | Err(Error::Incomplete) => {}
                other => panic!("a bit flipped at {at} was not noticed: {other:?}"),
            }
        }
    }

    #[test]
    fn nothing_at_all_is_waited_for_rather_than_refused() {
        assert_eq!(read(b"", Check::One, MARK), Err(Error::Incomplete));
        assert_eq!(
            read(b"no mark here", Check::One, MARK),
            Err(Error::Incomplete)
        );
    }

    #[test]
    fn a_length_longer_than_a_short_packet_holds_is_refused() {
        // Past DEL, a length is not a seven-bit character at all.
        let claimed = [MARK, tochar(96), tochar(0), b'D', b'x'];
        assert_eq!(read(&claimed, Check::One, MARK), Err(Error::TooLong));
    }

    #[test]
    fn a_packet_one_over_as_c_kermit_sends_it_is_read() {
        // Ninety-five, the length travelling as DEL: C-Kermit's size for a
        // full packet with the CRC, when it was asked for no more than 94.
        let data = vec![b'x'; 95 - 2 - Check::Three.chars()];
        let mut wire = vec![MARK, tochar(95), tochar(2), b'D'];
        wire.extend_from_slice(&data);
        wire.extend(Check::Three.of(&wire[1..]));
        assert_eq!(wire[1], 0x7f);
        let (read_back, used) = read(&wire, Check::Three, MARK).expect("read, not refused");
        assert_eq!((read_back.data, used), (&data[..], wire.len()));
    }

    #[test]
    fn the_longest_data_a_short_packet_carries() {
        assert_eq!(Packet::max_data(Check::One), 91);
        assert_eq!(Packet::max_data(Check::Two), 90);
        assert_eq!(Packet::max_data(Check::Three), 89);
        let full = vec![b'x'; Packet::max_data(Check::One)];
        let wire = Packet {
            sequence: 0,
            kind: Kind::Data,
            data: &full,
        }
        .build(Check::One, MARK, None);
        assert_eq!(unchar(wire[1]), MAX_COUNT, "the length is at its limit");
        let (read_back, _) = read(&wire, Check::One, MARK).expect("still a packet");
        assert_eq!(read_back.data, &full[..]);
    }

    #[test]
    fn a_long_packet_reads_back_as_it_was_built() {
        for len in [92, 200, 1000, 3999, MAX_LONG - 3] {
            let data: Vec<u8> = (0..len).map(|i| b' ' + (i % 90) as u8).collect();
            for check in [Check::One, Check::Two, Check::Three] {
                let sent = Packet {
                    sequence: 5,
                    kind: Kind::Data,
                    data: &data,
                };
                let wire = sent.build(check, MARK, Some(b'\r'));
                if len > Packet::max_data(check) {
                    assert_eq!(wire[1], b' ', "the extended form's length is nought");
                }
                let (read_back, used) =
                    read(&wire, check, MARK).unwrap_or_else(|e| panic!("{len} {check:?}: {e:?}"));
                assert_eq!(read_back, sent, "{len} {check:?}");
                assert_eq!(used, wire.len() - 1);
            }
        }
    }

    /// Worked out by hand from the specification: 100 bytes of `x` in packet
    /// 1 with the one-character check. The count is the data and the check,
    /// 101, which is 1 × 95 + 6 and travels as `!&`; the header check covers
    /// the space, `!`, `D`, `!` and `&` — 32, 33, 68, 33 and 38, summing to
    /// 204, which folds to 207 and leaves 15, travelling as `/`.
    #[test]
    fn a_long_packet_is_built_as_the_specification_says() {
        let data = [b'x'; 100];
        let wire = Packet {
            sequence: 1,
            kind: Kind::Data,
            data: &data,
        }
        .build(Check::One, MARK, None);
        assert_eq!(&wire[..7], b"\x01 !D!&/");
        assert_eq!(wire.len(), 7 + 100 + 1);
    }

    #[test]
    fn a_long_packet_is_waited_for_and_a_damaged_one_refused() {
        let data = [b'x'; 500];
        let whole = Packet {
            sequence: 3,
            kind: Kind::Data,
            data: &data,
        }
        .build(Check::Three, MARK, None);
        for upto in [0, 3, 6, 7, 100, whole.len() - 1] {
            assert_eq!(
                read(&whole[..upto], Check::Three, MARK).map(|(_, u)| u),
                Err(Error::Incomplete),
                "{upto} bytes"
            );
        }
        for at in 1..whole.len() {
            let mut damaged = whole.clone();
            damaged[at] ^= 0x01;
            match read(&damaged, Check::Three, MARK) {
                Err(Error::BadCheck | Error::Incomplete | Error::TooLong) => {}
                other => panic!("a bit flipped at {at} was not noticed: {other:?}"),
            }
        }
        // One over the most that can be offered, as C-Kermit sends it, is
        // read; two over is refused.
        for (n, readable) in [(MAX_LONG + 1, true), (MAX_LONG + 2, false)] {
            let mut wire = vec![MARK, tochar(0), tochar(11), b'D'];
            wire.extend([tochar((n / 95) as u8), tochar((n % 95) as u8)]);
            wire.extend(Check::One.of(&wire[1..]));
            wire.extend(std::iter::repeat_n(b'x', n - Check::Three.chars()));
            wire.extend(Check::Three.of(&wire[1..]));
            assert_eq!(read(&wire, Check::Three, MARK).is_ok(), readable, "{n}");
        }
        // A damaged length is caught by the header check at once, rather
        // than waited on.
        let mut long_length = whole.clone();
        long_length[4] ^= 0x04;
        assert_eq!(
            read(&long_length[..20], Check::Three, MARK).map(|(_, u)| u),
            Err(Error::BadCheck)
        );
    }

    #[test]
    fn a_check_is_named_by_a_character_and_read_back_from_one() {
        for check in [Check::One, Check::Two, Check::Three] {
            assert_eq!(Check::from_byte(check.as_byte()), Some(check));
        }
        assert_eq!(Check::from_byte(b'4'), None, "there is no fourth");
        assert_eq!(Check::from_byte(b'B'), None);
    }
}
