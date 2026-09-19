//! Turning a file's bytes into something a terminal line will carry, and back.
//!
//! Three prefixes do the work, and they come in a fixed order. A run of the
//! same character can be replaced by the repeat prefix and a count; a
//! character with its top bit set is announced by the eighth-bit prefix and
//! then sent as seven; a control character is announced by the control prefix
//! and flipped into printable range. So a repeated high-bit control character
//! goes out as all three — repeat, count, eighth-bit, control, character.
//!
//! What the far end asked for decides which of them are in use, and a line
//! that is eight bits clean needs none of the second sort. Nothing here knows
//! about files or line endings: whether a text file's endings are rewritten on
//! the way out is the business of whatever is sending it.

use crate::{Params, ctl, tochar, unchar};

/// The most a repeat count can carry, [`tochar`] being what carries it.
const MAX_RUN: usize = crate::MAX_COUNT as usize;

/// A run shorter than this is cheaper sent as itself: the prefix and its
/// count cost two characters, so three is where it stops losing.
const WORTH_REPEATING: usize = 3;

/// Characters the control prefix turns into control characters.
///
/// `ctl` maps 0x00–0x1f onto 0x40–0x5f and 0x7f onto 0x3f, so a character in
/// this range after the prefix means a control character, and anything else
/// means itself — which is how a prefix character is sent literally. It is
/// also why the prefixes have to be chosen from outside this range, and every
/// implementation chooses `#`, `&` and `~`, which are.
const CONTROL_RANGE: std::ops::RangeInclusive<u8> = 0x3f..=0x5f;

/// How many characters this byte needs, as a single character.
fn cost(byte: u8, params: &Params) -> usize {
    let mut cost = 0;
    let seven = if byte & 0x80 == 0 {
        byte
    } else if params.quote_eighth.is_some() {
        cost += 1;
        byte & 0x7f
    } else {
        // Nothing agreed to announce it with, so it goes as it is and the
        // line had better be eight bits clean. 🔎 A stricter reading would
        // refuse the file; this sends it, which is what works on the lines
        // that do not need prefixing in the first place.
        byte
    };
    cost + if needs_quoting(seven, params) { 2 } else { 1 }
}

/// Whether a seven-bit value has to be announced by the control prefix:
/// because it is a control character, or because it is one of the prefixes
/// and would otherwise be read as one.
fn needs_quoting(seven: u8, params: &Params) -> bool {
    if seven < 0x20 || seven == 0x7f {
        return true;
    }
    seven == params.quote_control
        || Some(seven) == params.quote_eighth
        || Some(seven) == params.repeat
}

/// Writes one byte out, without the repeat prefix.
fn put(byte: u8, params: &Params, out: &mut Vec<u8>) {
    let seven = if byte & 0x80 == 0 {
        byte
    } else if let Some(eighth) = params.quote_eighth {
        out.push(eighth);
        byte & 0x7f
    } else {
        byte
    };
    if needs_quoting(seven, params) {
        out.push(params.quote_control);
        out.push(if seven < 0x20 || seven == 0x7f {
            ctl(seven)
        } else {
            // A prefix character sent as itself, which the far end reads as
            // itself because it is not in the control range.
            seven
        });
    } else {
        out.push(seven);
    }
}

/// Encodes as much of `data` as will fit in `room` characters, and says how
/// much of it went in.
///
/// Stops rather than overflowing, so a caller fills one packet, sends it, and
/// calls again with what is left. `room` too small for even one character
/// gives nothing back and consumes nothing — a caller that loops on this must
/// check, or it will loop for ever.
#[must_use]
pub fn encode(data: &[u8], params: &Params, room: usize) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let byte = data[at];
        let left = room - out.len();
        let single = cost(byte, params);

        // A run is worth the prefix and a count regardless of how long it is,
        // the count being one character either way, so the only question is
        // whether the three parts fit at all.
        if let Some(repeat) = params.repeat {
            let run = data[at..]
                .iter()
                .take(MAX_RUN)
                .take_while(|&&b| b == byte)
                .count();
            if run >= WORTH_REPEATING && 2 + single <= left {
                out.push(repeat);
                out.push(tochar(run as u8));
                put(byte, params, &mut out);
                at += run;
                continue;
            }
        }
        if single > left {
            break;
        }
        put(byte, params, &mut out);
        at += 1;
    }
    (out, at)
}

/// Reads an encoded packet's data back into the bytes it was made from.
///
/// A prefix with nothing behind it is dropped rather than guessed at: a
/// packet that ends mid-sequence failed its check and should never have got
/// this far, and inventing a byte for it would be worse than losing one.
#[must_use]
pub fn decode(data: &[u8], params: &Params) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let mut run = 1;
        if Some(data[at]) == params.repeat {
            let Some(&count) = data.get(at + 1) else {
                break;
            };
            run = usize::from(unchar(count));
            at += 2;
        }
        let Some((byte, used)) = take(&data[at..], params) else {
            break;
        };
        at += used;
        out.extend(std::iter::repeat_n(byte, run));
    }
    out
}

/// One character and its prefixes, and how many characters it took.
fn take(data: &[u8], params: &Params) -> Option<(u8, usize)> {
    let mut at = 0;
    let mut eighth = false;
    if params.quote_eighth.is_some() && Some(*data.first()?) == params.quote_eighth {
        eighth = true;
        at += 1;
    }
    let mut byte = *data.get(at)?;
    at += 1;
    if byte == params.quote_control {
        let quoted = *data.get(at)?;
        at += 1;
        byte = if CONTROL_RANGE.contains(&quoted) {
            ctl(quoted)
        } else {
            quoted
        };
    }
    Some((if eighth { byte | 0x80 } else { byte }, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Packet, ours};

    /// Room enough that nothing is held back, for tests about encoding
    /// rather than about fitting.
    const PLENTY: usize = 4096;

    fn everything() -> Params {
        ours()
    }

    fn eight_bit_clean() -> Params {
        Params {
            quote_eighth: None,
            repeat: None,
            ..Params::default()
        }
    }

    #[test]
    fn every_byte_there_is_survives_the_journey() {
        for params in [everything(), eight_bit_clean()] {
            let all: Vec<u8> = (0..=255u8).collect();
            let (encoded, used) = encode(&all, &params, PLENTY);
            assert_eq!(used, all.len());
            assert_eq!(decode(&encoded, &params), all, "{params:?}");
        }
    }

    #[test]
    fn what_goes_out_is_something_a_terminal_line_will_carry() {
        let params = everything();
        let all: Vec<u8> = (0..=255u8).collect();
        let (encoded, _) = encode(&all, &params, PLENTY);
        for byte in encoded {
            assert!(
                (0x20..0x7f).contains(&byte),
                "{byte:#04x} is not printable, and a host may eat it"
            );
        }
    }

    #[test]
    fn a_control_character_is_announced_and_flipped() {
        let params = everything();
        let (encoded, _) = encode(b"\r\n", &params, PLENTY);
        assert_eq!(encoded, b"#M#J", "carriage return and line feed");
        assert_eq!(decode(&encoded, &params), b"\r\n");
    }

    #[test]
    fn a_prefix_character_is_sent_as_itself_and_read_as_itself() {
        let params = everything();
        // The three prefixes, which would otherwise be read as prefixes.
        for &byte in b"#&~" {
            let (encoded, _) = encode(&[byte], &params, PLENTY);
            assert_eq!(
                encoded,
                vec![b'#', byte],
                "{} needs announcing",
                byte as char
            );
            assert_eq!(decode(&encoded, &params), vec![byte]);
        }
    }

    #[test]
    fn a_high_bit_is_announced_where_the_two_ends_agreed_on_how() {
        let params = everything();
        let (encoded, _) = encode(&[0xc9], &params, PLENTY);
        assert_eq!(encoded, b"&I", "the eighth-bit prefix, then the seven");
        assert_eq!(decode(&encoded, &params), vec![0xc9]);

        // A high-bit control character needs both prefixes.
        let (encoded, _) = encode(&[0x9b], &params, PLENTY);
        assert_eq!(encoded, b"&#[", "eighth bit, control prefix, flipped");
        assert_eq!(decode(&encoded, &params), vec![0x9b]);
    }

    #[test]
    fn a_high_bit_goes_as_it_is_where_nothing_was_agreed() {
        let params = eight_bit_clean();
        let (encoded, _) = encode(&[0xc9], &params, PLENTY);
        assert_eq!(encoded, vec![0xc9], "nothing to announce it with");
        assert_eq!(decode(&encoded, &params), vec![0xc9]);
    }

    #[test]
    fn a_run_is_sent_as_a_count() {
        let params = everything();
        let (encoded, used) = encode(&[b'x'; 40], &params, PLENTY);
        assert_eq!(used, 40);
        assert_eq!(encoded, vec![b'~', tochar(40), b'x'], "three characters");
        assert_eq!(decode(&encoded, &params), vec![b'x'; 40]);
    }

    #[test]
    fn a_run_of_control_characters_is_a_count_and_two_more() {
        let params = everything();
        let (encoded, _) = encode(&[0x00; 94], &params, PLENTY);
        assert_eq!(encoded, vec![b'~', tochar(94), b'#', b'@']);
        assert_eq!(decode(&encoded, &params), vec![0x00; 94]);
    }

    #[test]
    fn a_run_longer_than_a_count_holds_is_split() {
        let params = everything();
        let (encoded, used) = encode(&[b'z'; 200], &params, PLENTY);
        assert_eq!(used, 200);
        assert_eq!(decode(&encoded, &params), vec![b'z'; 200]);
        assert_eq!(encoded.len(), 9, "three runs: 94, 94 and 12");
    }

    #[test]
    fn a_run_too_short_to_pay_for_itself_is_sent_plainly() {
        let params = everything();
        for run in 1..WORTH_REPEATING {
            let run_of = vec![b'q'; run];
            let (encoded, _) = encode(&run_of, &params, PLENTY);
            assert_eq!(encoded, run_of, "a run of {run}");
        }
        let (encoded, _) = encode(&[b'q'; WORTH_REPEATING], &params, PLENTY);
        assert_eq!(encoded[0], b'~', "and three is where it starts paying");
    }

    #[test]
    fn a_run_is_not_used_where_the_far_end_did_not_agree_to_it() {
        let params = eight_bit_clean();
        let (encoded, _) = encode(&[b'x'; 40], &params, PLENTY);
        assert_eq!(encoded, vec![b'x'; 40]);
    }

    #[test]
    fn nothing_longer_than_the_room_given_is_ever_built() {
        let params = everything();
        // Control characters, so every one costs two, and a length that is
        // not a multiple of the cost.
        let awkward = vec![0x01; 100];
        for room in 0..40 {
            let (encoded, used) = encode(&awkward, &params, room);
            assert!(
                encoded.len() <= room,
                "room for {room} and {} built",
                encoded.len()
            );
            assert_eq!(
                decode(&encoded, &params).len(),
                used,
                "what was built is what was consumed"
            );
        }
    }

    #[test]
    fn room_for_nothing_consumes_nothing_rather_than_looping_for_ever() {
        let params = everything();
        let (encoded, used) = encode(b"\x01", &params, 1);
        assert!(encoded.is_empty(), "a control character needs two");
        assert_eq!(used, 0);
    }

    #[test]
    fn a_file_goes_in_packets_and_comes_back_whole() {
        let params = everything();
        // Something with runs, control characters, high bits and prefixes in
        // it, which is most files.
        let mut file = Vec::new();
        let mut seed = 0x1234_5678u32;
        for _ in 0..5000 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let byte = (seed >> 16) as u8;
            let run = 1 + usize::from(byte % 7);
            file.extend(std::iter::repeat_n(byte, run));
        }
        file.extend_from_slice(b"#&~\r\n\x00\x7f\xff");

        let room = Packet::max_data(params.check);
        let mut packets = 0;
        let mut at = 0;
        let mut back = Vec::new();
        while at < file.len() {
            let (encoded, used) = encode(&file[at..], &params, room);
            assert!(used > 0, "no progress at {at} of {}", file.len());
            assert!(encoded.len() <= room);
            back.extend(decode(&encoded, &params));
            at += used;
            packets += 1;
        }
        assert_eq!(back, file, "every byte, in order, across {packets} packets");
        assert!(packets > 10, "and it really did take several");
    }

    #[test]
    fn a_packet_that_ends_mid_sequence_loses_a_byte_rather_than_inventing_one() {
        let params = everything();
        let (encoded, _) = encode(&[0x01, 0x02], &params, PLENTY);
        // Cut the last character off, leaving a prefix with nothing behind.
        let cut = &encoded[..encoded.len() - 1];
        assert_eq!(decode(cut, &params), vec![0x01], "the whole one only");
    }
}
