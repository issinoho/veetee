//! The parameters the two ends agree before a file moves.
//!
//! A send-init carries them and its acknowledgement answers with the other
//! end's, after which each side sends what the *other* asked for: its packet
//! length, its timeout, the padding it needs, the character it wants at the
//! end of a packet, and the prefixes it will use for the characters a line
//! cannot carry plainly. There is no argument and no second round — whatever
//! comes back in the acknowledgement is what is used.
//!
//! Every field is optional from the right. A far end that stops after four of
//! them means the defaults for the rest, so a short block is not a short
//! measure and must not be read as one.

use crate::{Check, ctl, tochar, unchar};

/// What each end needs of the other.
///
/// The defaults are the ones the specification gives for a field that is not
/// sent at all, which is what makes a truncated block safe to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Params {
    /// The longest packet this end can receive, counting as the length field
    /// does. Never more than [`crate::MAX_COUNT`] until long packets are
    /// read.
    pub max_length: u8,
    /// Seconds the far end should wait before assuming a packet went
    /// missing. Nought asks it not to time out at all.
    pub timeout: u8,
    /// Padding this end needs before a packet, and what to pad with. A host
    /// that needs a moment after a line ending asks for it here; nothing
    /// modern does.
    pub padding: u8,
    pub pad_with: u8,
    /// The character this end wants at the end of every packet. Carriage
    /// return for almost everything, because a host in line mode will not
    /// hand a packet over until it sees one.
    pub end_of_line: u8,
    /// The prefix for a control character, which is then flipped into
    /// printable range.
    pub quote_control: u8,
    /// The prefix for a character with its top bit set, where the two ends
    /// have agreed on one. A seven-bit line needs it to carry a binary file
    /// at all.
    pub quote_eighth: Option<u8>,
    /// The prefix for a run of the same character, where agreed.
    pub repeat: Option<u8>,
    pub check: Check,
    /// The capability field and anything after it, kept exactly as it
    /// arrived.
    ///
    /// Only the attribute bit is read (see [`crate::attributes::offered`]).
    /// The others cover long packets and sliding windows, which veetee does
    /// not do, so a far end offering them gets short packets one at a time
    /// and is right to.
    pub capabilities: Vec<u8>,
}

/// What a field means when the far end did not send it.
impl Default for Params {
    fn default() -> Params {
        Params {
            max_length: 80,
            timeout: 5,
            padding: 0,
            pad_with: 0,
            end_of_line: b'\r',
            quote_control: b'#',
            // Absent means none: a line that needs eight-bit prefixing has to
            // ask for it.
            quote_eighth: None,
            repeat: None,
            check: Check::One,
            capabilities: Vec::new(),
        }
    }
}

/// What veetee asks for: the longest short packet there is, and every prefix
/// it knows how to use.
///
/// Asking for eighth-bit and repeat prefixing costs nothing where the far end
/// cannot do it — it answers with what it can, and that is what gets used.
#[must_use]
pub fn ours() -> Params {
    Params {
        max_length: crate::MAX_COUNT,
        timeout: 10,
        quote_eighth: Some(b'&'),
        repeat: Some(b'~'),
        // The CRC: proved both ways against C-Kermit and G-Kermit, and the
        // one-character check lets a damaged length through one time in
        // sixty-four. Asking costs nothing where the far end cannot do it:
        // it answers with another check, and both ends then use type 1.
        check: Check::Three,
        // Attribute packets and long packets, and no sliding windows; then
        // the window size, which without windows is one, and the longest
        // extended packet veetee will read, as two base-95 digits.
        capabilities: vec![
            tochar(crate::attributes::CAPABLE | LONG_PACKETS),
            tochar(1),
            tochar((crate::MAX_LONG / 95) as u8),
            tochar((crate::MAX_LONG % 95) as u8),
        ],
        ..Params::default()
    }
}

/// The capability bit for long packets. G-Kermit offers `*`, which is 10:
/// this and attributes, and its manual says it does both.
pub(crate) const LONG_PACKETS: u8 = 2;

/// What an end offering long packets but naming no length can receive.
const DEFAULT_LONG: usize = 500;

/// `Y` in the eighth-bit field: "whatever you propose".
const AGREED: u8 = b'Y';
/// `N` in the eighth-bit or repeat field: "not at all".
const REFUSED: u8 = b'N';

impl Params {
    /// The longest extended packet this end can receive, if it offers long
    /// packets at all.
    ///
    /// The capability bytes run on for as long as their lowest bit says
    /// another follows; after them come the window size, and then the
    /// length as two base-95 digits. G-Kermit's `*!J*` is long packets and
    /// attributes, a window of one, and 42 × 95 + 10 = 4000, which its manual
    /// gives as its default; C-Kermit on OpenVMS sends `^>J)`, for 3999.
    #[must_use]
    pub fn long(&self) -> Option<usize> {
        let caps = &self.capabilities;
        if unchar(*caps.first()?) & LONG_PACKETS == 0 {
            return None;
        }
        let last = caps.iter().position(|&c| unchar(c) & 1 == 0)?;
        let digit = |i: usize| caps.get(last + i).map(|&c| usize::from(unchar(c)));
        let length = match (digit(2), digit(3)) {
            (Some(high), Some(low)) if high * 95 + low > 0 => high * 95 + low,
            _ => DEFAULT_LONG,
        };
        Some(length.min(crate::MAX_LONG))
    }

    /// The data field of a send-init or of its acknowledgement.
    #[must_use]
    pub fn build(&self) -> Vec<u8> {
        let mut out = vec![
            tochar(self.max_length),
            tochar(self.timeout),
            tochar(self.padding),
            // The pad character is flipped rather than offset, being a
            // control character and not a count.
            ctl(self.pad_with),
            tochar(self.end_of_line),
            self.quote_control,
            self.quote_eighth.unwrap_or(REFUSED),
            self.check.as_byte(),
            self.repeat.unwrap_or(REFUSED),
        ];
        out.extend_from_slice(&self.capabilities);
        out
    }

    /// Reads a send-init or its acknowledgement, filling in the default for
    /// every field the far end did not send.
    #[must_use]
    pub fn read(data: &[u8]) -> Params {
        let mut params = Params::default();
        let at = |i: usize| data.get(i).copied();
        if let Some(c) = at(0) {
            params.max_length = unchar(c);
        }
        if let Some(c) = at(1) {
            params.timeout = unchar(c);
        }
        if let Some(c) = at(2) {
            params.padding = unchar(c);
        }
        if let Some(c) = at(3) {
            params.pad_with = ctl(c);
        }
        if let Some(c) = at(4) {
            params.end_of_line = unchar(c);
        }
        if let Some(c) = at(5) {
            params.quote_control = c;
        }
        if let Some(c) = at(6) {
            params.quote_eighth = prefix(c);
        }
        if let Some(c) = at(7) {
            // A far end asking for a check veetee cannot do gets the one
            // everything can do, rather than a refusal.
            params.check = Check::from_byte(c).unwrap_or(Check::One);
        }
        if let Some(c) = at(8) {
            params.repeat = prefix(c);
        }
        if data.len() > 9 {
            params.capabilities = data[9..].to_vec();
        }
        params
    }

    /// What the two ends will actually use, given what veetee asked for and
    /// what came back.
    ///
    /// Most of it is simply the far end's: it is the one that has to receive
    /// what veetee sends, so its length, its timeout, its padding and its end
    /// of line are the ones that matter. The prefixes are the exception,
    /// because both ends have to want them.
    #[must_use]
    pub fn agreed(ours: &Params, theirs: &Params) -> Params {
        Params {
            // Never send more than a short packet holds, whatever is offered.
            max_length: theirs.max_length.min(crate::MAX_COUNT),
            timeout: theirs.timeout,
            padding: theirs.padding,
            pad_with: theirs.pad_with,
            end_of_line: theirs.end_of_line,
            quote_control: theirs.quote_control,
            quote_eighth: settle(ours.quote_eighth, theirs.quote_eighth),
            repeat: settle(ours.repeat, theirs.repeat),
            // The check named in the send-init is used only if the answer
            // names the same one; otherwise both ends fall back to the
            // one-character check, which every Kermit has (Kermit Protocol
            // Manual, the CHKT field). Taking the far end's regardless would
            // have veetee checking one way while it checked another.
            check: if ours.check == theirs.check {
                theirs.check
            } else {
                Check::One
            },
            capabilities: theirs.capabilities.clone(),
        }
    }
}

/// A prefix field: a character to use, or nothing.
fn prefix(c: u8) -> Option<u8> {
    match c {
        REFUSED => None,
        // `Y` on its own agrees to whatever was proposed but names nothing,
        // so the proposer's character stands; [`settle`] sorts that out.
        AGREED => Some(AGREED),
        // A space is how some ends say nothing rather than no.
        b' ' => None,
        other => Some(other),
    }
}

/// Which prefix to use, if either.
///
/// Both ends have to want one. `Y` means the other end's choice, so an end
/// that answered `Y` gets what was proposed; two ends naming characters use
/// the answering end's, it being the one that will have to read them.
fn settle(ours: Option<u8>, theirs: Option<u8>) -> Option<u8> {
    match (ours, theirs) {
        (Some(mine), Some(AGREED)) => Some(mine),
        (Some(_), Some(theirs)) => Some(theirs),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The send-init G-Kermit 2.01 actually sends, captured from it over a
    /// pty. Written as numbers because that is what was on the wire; as
    /// characters it reads
    ///
    /// ```text
    /// ^A9 S~' @-#Y3~*!J*0+++B"U1@G<CR>
    /// ```
    ///
    /// Length 25, sequence 0, type S. The check is `G`, and the characters it
    /// covers sum to 1445 — past eight bits, which is the case the folding
    /// rule exists for and the one worth having a real implementation agree
    /// on.
    const GKERMIT_SEND_INIT: &[u8] = &[
        0x01, 0x39, 0x20, 0x53, 0x7e, 0x27, 0x20, 0x40, 0x2d, 0x23, 0x59, 0x33, 0x7e, 0x2a, 0x21,
        0x4a, 0x2a, 0x30, 0x2b, 0x2b, 0x2b, 0x42, 0x22, 0x55, 0x31, 0x40, 0x47, 0x0d,
    ];

    #[test]
    fn a_real_kermits_send_init_reads_as_it_meant_it() {
        let (packet, used) = crate::read(GKERMIT_SEND_INIT, Check::One, crate::MARK)
            .expect("G-Kermit's own send-init");
        assert_eq!(packet.kind, crate::Kind::SendInit);
        assert_eq!(packet.sequence, 0);
        assert_eq!(
            used,
            GKERMIT_SEND_INIT.len() - 1,
            "everything but the carriage return"
        );

        let theirs = Params::read(packet.data);
        assert_eq!(theirs.max_length, 94, "the longest a short packet holds");
        assert_eq!(theirs.timeout, 7);
        assert_eq!(theirs.padding, 0);
        assert_eq!(theirs.pad_with, 0, "a null, which travelled as @");
        assert_eq!(theirs.end_of_line, b'\r');
        assert_eq!(theirs.quote_control, b'#');
        assert_eq!(
            theirs.quote_eighth,
            Some(b'Y'),
            "whatever veetee proposes, which settles to veetee's own"
        );
        assert_eq!(theirs.check, Check::Three, "it asks for the CRC");
        assert_eq!(theirs.repeat, Some(b'~'));
        assert_eq!(theirs.capabilities.len(), 13, "carried, not read");
    }

    #[test]
    fn what_is_agreed_with_a_real_kermit() {
        let (packet, _) =
            crate::read(GKERMIT_SEND_INIT, Check::One, crate::MARK).expect("a packet");
        let agreed = Params::agreed(&ours(), &Params::read(packet.data));
        assert_eq!(agreed.max_length, 94);
        assert_eq!(
            agreed.quote_eighth,
            Some(b'&'),
            "it answered Y, so veetee's own prefix stands"
        );
        assert_eq!(agreed.repeat, Some(b'~'), "both want one, and it named it");
        assert_eq!(
            agreed.check,
            Check::Three,
            "both ask for the CRC, so it is used"
        );
        let answering_one = Params {
            check: Check::One,
            ..ours()
        };
        assert_eq!(
            Params::agreed(&answering_one, &Params::read(packet.data)).check,
            Check::One,
            "an end answering 1 to it has both fall back to 1"
        );
    }

    #[test]
    fn a_check_is_used_only_where_both_ends_name_it() {
        let asking = |check| Params { check, ..ours() };
        for (mine, theirs, used) in [
            (Check::One, Check::One, Check::One),
            (Check::One, Check::Three, Check::One),
            (Check::Three, Check::One, Check::One),
            (Check::Two, Check::Three, Check::One),
            (Check::Three, Check::Three, Check::Three),
            (Check::Two, Check::Two, Check::Two),
        ] {
            assert_eq!(
                Params::agreed(&asking(mine), &asking(theirs)).check,
                used,
                "{mine:?} and {theirs:?}"
            );
        }
    }

    #[test]
    fn how_long_a_packet_each_end_can_take() {
        let (packet, _) =
            crate::read(GKERMIT_SEND_INIT, Check::One, crate::MARK).expect("a packet");
        assert_eq!(Params::read(packet.data).long(), Some(4000), "G-Kermit");
        let vms = Params {
            capabilities: b"^>J)0___N\"D7".to_vec(),
            ..Params::default()
        };
        assert_eq!(vms.long(), Some(3999), "C-Kermit 9.0.300 on OpenVMS");
        assert_eq!(ours().long(), Some(crate::MAX_LONG), "veetee");
        let unnamed = Params {
            capabilities: vec![tochar(LONG_PACKETS)],
            ..Params::default()
        };
        assert_eq!(unnamed.long(), Some(500), "offered, with no length named");
        let attributes_only = Params {
            capabilities: vec![tochar(crate::attributes::CAPABLE)],
            ..Params::default()
        };
        assert_eq!(attributes_only.long(), None);
        assert_eq!(Params::default().long(), None, "no capabilities at all");
        let continued = Params {
            // Two capability bytes, the first saying another follows.
            capabilities: vec![
                tochar(LONG_PACKETS | 1),
                tochar(0),
                tochar(1),
                tochar(10),
                tochar(50),
            ],
            ..Params::default()
        };
        assert_eq!(continued.long(), Some(10 * 95 + 50));
    }

    #[test]
    fn what_veetee_asks_for_reads_back_as_it_was_asked() {
        let asked = ours();
        let read = Params::read(&asked.build());
        assert_eq!(read, asked);
    }

    #[test]
    fn a_field_the_far_end_did_not_send_is_the_default_and_not_nothing() {
        // Four fields and no more, which is legal and means the rest are the
        // defaults. Reading a missing field as nought would ask a host for no
        // timeout, no quoting and no end of line, and nothing would move.
        let short = Params::read(&[tochar(94), tochar(5), tochar(0), ctl(0)]);
        assert_eq!(short.max_length, 94);
        assert_eq!(short.timeout, 5);
        assert_eq!(short.end_of_line, b'\r', "a carriage return, not a null");
        assert_eq!(short.quote_control, b'#', "not nothing");
        assert_eq!(short.check, Check::One);
        assert_eq!(short.quote_eighth, None);
        assert_eq!(short.repeat, None);
    }

    #[test]
    fn nothing_at_all_is_every_default() {
        assert_eq!(Params::read(&[]), Params::default());
    }

    #[test]
    fn the_pad_character_is_flipped_rather_than_offset() {
        // It is a control character, so it travels the way control characters
        // do and not the way counts do.
        let built = Params {
            pad_with: 0,
            ..Params::default()
        }
        .build();
        assert_eq!(built[3], b'@', "a null pads as @");
        assert_eq!(Params::read(&built).pad_with, 0);
    }

    #[test]
    fn a_prefix_is_used_only_where_both_ends_want_it() {
        let mine = ours();
        let refusing = Params {
            quote_eighth: None,
            repeat: None,
            ..Params::default()
        };
        let agreed = Params::agreed(&mine, &refusing);
        assert_eq!(agreed.quote_eighth, None, "asked for, and refused");
        assert_eq!(agreed.repeat, None);

        let naming = Params {
            quote_eighth: Some(b'%'),
            repeat: Some(b'*'),
            ..Params::default()
        };
        let agreed = Params::agreed(&mine, &naming);
        assert_eq!(
            (agreed.quote_eighth, agreed.repeat),
            (Some(b'%'), Some(b'*')),
            "the answering end's characters, it being the one that reads them"
        );
    }

    #[test]
    fn an_end_that_answers_yes_gets_what_was_proposed() {
        let mine = ours();
        let agreeable = Params {
            quote_eighth: Some(AGREED),
            repeat: Some(AGREED),
            ..Params::default()
        };
        let agreed = Params::agreed(&mine, &agreeable);
        assert_eq!(agreed.quote_eighth, Some(b'&'), "what veetee asked for");
        assert_eq!(agreed.repeat, Some(b'~'));
    }

    #[test]
    fn a_far_end_that_wants_a_prefix_veetee_did_not_offer_does_not_get_one() {
        let quiet = Params {
            quote_eighth: None,
            repeat: None,
            ..Params::default()
        };
        let keen = Params {
            quote_eighth: Some(b'&'),
            repeat: Some(b'~'),
            ..Params::default()
        };
        let agreed = Params::agreed(&quiet, &keen);
        assert_eq!(agreed.quote_eighth, None);
        assert_eq!(agreed.repeat, None);
    }

    #[test]
    fn whatever_is_offered_no_packet_goes_out_longer_than_a_short_one() {
        let boastful = Params {
            // A far end offering the extended form, which is not read yet.
            max_length: 200,
            ..Params::default()
        };
        assert_eq!(
            Params::agreed(&ours(), &boastful).max_length,
            crate::MAX_COUNT
        );
    }

    #[test]
    fn a_check_veetee_cannot_do_falls_back_to_the_one_everything_can() {
        let mut block = ours().build();
        block[7] = b'9';
        assert_eq!(Params::read(&block).check, Check::One);
    }

    #[test]
    fn the_capability_field_is_carried_whole() {
        let mut block = ours().build();
        block.extend_from_slice(&[tochar(0x1a), tochar(0x02)]);
        let read = Params::read(&block);
        let mut expected = ours().capabilities;
        expected.extend_from_slice(&[tochar(0x1a), tochar(0x02)]);
        assert_eq!(
            read.capabilities, expected,
            "what follows is kept as it came"
        );
        assert_eq!(read.build(), block, "and goes back out untouched");
    }

    #[test]
    fn a_send_init_is_a_packet_like_any_other() {
        let packet = crate::Packet {
            sequence: 0,
            kind: crate::Kind::SendInit,
            data: &ours().build(),
        };
        let wire = packet.build(Check::One, crate::MARK, Some(b'\r'));
        let (read, _) = crate::read(&wire, Check::One, crate::MARK).expect("a send-init");
        assert_eq!(read.kind, crate::Kind::SendInit);
        assert_eq!(Params::read(read.data), ours());
    }
}
