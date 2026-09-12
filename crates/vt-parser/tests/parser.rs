//! Parser behaviour tests. Section references are to the DEC ANSI parser
//! description (vt100.net/emu/dec_ansi_parser) and DEC STD 070.

use proptest::prelude::*;
use vt_parser::{InputMode, MAX_PARAMS, Parser, Perform, Sequence, StringEnd, StringKind};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ev {
    Print(u8),
    Char(char),
    Exec(u8),
    Esc(Vec<u8>, u8),
    Csi(Option<u8>, Vec<Option<u16>>, Vec<u8>, u8),
    Hook(Option<u8>, Vec<Option<u16>>, Vec<u8>, u8),
    Put(u8),
    Unhook(StringEnd),
    OscStart,
    OscPut(u8),
    OscEnd(StringEnd),
    StrStart(StringKind),
    StrPut(u8),
    StrEnd(StringEnd),
    Vt52Cup(u8, u8),
}

#[derive(Default)]
struct Rec {
    ev: Vec<Ev>,
    subparams: Vec<bool>,
    truncated: bool,
}

fn header(seq: &Sequence<'_>) -> (Option<u8>, Vec<Option<u16>>, Vec<u8>, u8) {
    (
        seq.private,
        seq.params.iter().map(|p| p.value).collect(),
        seq.intermediates.to_vec(),
        seq.final_byte,
    )
}

impl Perform for Rec {
    fn print(&mut self, b: u8) {
        self.ev.push(Ev::Print(b));
    }
    fn print_char(&mut self, c: char) {
        self.ev.push(Ev::Char(c));
    }
    fn execute(&mut self, b: u8) {
        self.ev.push(Ev::Exec(b));
    }
    fn esc_dispatch(&mut self, i: &[u8], f: u8) {
        self.ev.push(Ev::Esc(i.to_vec(), f));
    }
    fn csi_dispatch(&mut self, seq: Sequence<'_>) {
        self.subparams = (0..seq.params.len())
            .map(|i| seq.params.is_subparam(i))
            .collect();
        self.truncated = seq.params.truncated();
        let (p, v, i, f) = header(&seq);
        self.ev.push(Ev::Csi(p, v, i, f));
    }
    fn dcs_hook(&mut self, seq: Sequence<'_>) {
        let (p, v, i, f) = header(&seq);
        self.ev.push(Ev::Hook(p, v, i, f));
    }
    fn dcs_put(&mut self, b: u8) {
        self.ev.push(Ev::Put(b));
    }
    fn dcs_unhook(&mut self, e: StringEnd) {
        self.ev.push(Ev::Unhook(e));
    }
    fn osc_start(&mut self) {
        self.ev.push(Ev::OscStart);
    }
    fn osc_put(&mut self, b: u8) {
        self.ev.push(Ev::OscPut(b));
    }
    fn osc_end(&mut self, e: StringEnd) {
        self.ev.push(Ev::OscEnd(e));
    }
    fn string_start(&mut self, k: StringKind) {
        self.ev.push(Ev::StrStart(k));
    }
    fn string_put(&mut self, b: u8) {
        self.ev.push(Ev::StrPut(b));
    }
    fn string_end(&mut self, e: StringEnd) {
        self.ev.push(Ev::StrEnd(e));
    }
    fn vt52_cursor(&mut self, l: u8, c: u8) {
        self.ev.push(Ev::Vt52Cup(l, c));
    }
}

fn run_with(parser: &mut Parser, input: &[u8]) -> Rec {
    let mut rec = Rec::default();
    parser.advance(&mut rec, input);
    rec
}

fn run_mode(mode: InputMode, input: &[u8]) -> Vec<Ev> {
    let mut parser = Parser::new();
    parser.set_input_mode(mode);
    run_with(&mut parser, input).ev
}

fn run(input: &[u8]) -> Vec<Ev> {
    run_mode(InputMode::EightBit, input)
}

fn prints(s: &str) -> Vec<Ev> {
    s.bytes().map(Ev::Print).collect()
}

fn csi(private: Option<u8>, params: &[Option<u16>], inter: &[u8], f: u8) -> Ev {
    Ev::Csi(private, params.to_vec(), inter.to_vec(), f)
}

use Ev::*;
use StringEnd::{Aborted, Terminated};

// ---------------------------------------------------------------- ground / C0

#[test]
fn plain_text_and_c0() {
    let mut expected = prints("AB");
    expected.extend([Exec(b'\r'), Exec(b'\n'), Print(b'C')]);
    assert_eq!(run(b"AB\r\nC"), expected);
}

#[test]
fn del_in_ground_is_passed_to_print() {
    // Whether DEL is displayable depends on the charset in GL; the emulator decides.
    assert_eq!(run(b"\x7f"), vec![Print(0x7F)]);
}

#[test]
fn gr_bytes_print_in_eight_bit_mode() {
    assert_eq!(
        run(b"\xa0\xe9\xff"),
        vec![Print(0xA0), Print(0xE9), Print(0xFF)]
    );
}

// ------------------------------------------------------------------------ ESC

#[test]
fn esc_final_dispatch() {
    assert_eq!(
        run(b"\x1b7\x1b8"),
        vec![Esc(vec![], b'7'), Esc(vec![], b'8')]
    );
}

#[test]
fn esc_with_intermediates() {
    // SCS G0 = DEC Special Graphics; DECALN; SCS 96-set G1 = Latin-1.
    assert_eq!(
        run(b"\x1b(0\x1b#8\x1b-A"),
        vec![
            Esc(vec![b'('], b'0'),
            Esc(vec![b'#'], b'8'),
            Esc(vec![b'-'], b'A')
        ]
    );
}

#[test]
fn esc_nrcs_with_two_intermediates() {
    // SCS G0 = Portuguese NRCS: ESC ( % 6
    assert_eq!(run(b"\x1b(%6"), vec![Esc(vec![b'(', b'%'], b'6')]);
}

#[test]
fn esc_too_many_intermediates_is_ignored() {
    assert_eq!(run(b"\x1b(%%6X"), vec![Print(b'X')]);
}

#[test]
fn c0_inside_escape_is_executed() {
    assert_eq!(run(b"\x1b(\n0"), vec![Exec(b'\n'), Esc(vec![b'('], b'0')]);
}

#[test]
fn esc_restarts_escape() {
    assert_eq!(run(b"\x1b(\x1bc"), vec![Esc(vec![], b'c')]);
}

#[test]
fn st_outside_string_dispatches_as_esc_backslash() {
    assert_eq!(run(b"\x1b\\"), vec![Esc(vec![], b'\\')]);
}

// ------------------------------------------------------------------------ CSI

#[test]
fn csi_basic() {
    assert_eq!(
        run(b"\x1b[12;34H"),
        vec![csi(None, &[Some(12), Some(34)], &[], b'H')]
    );
}

#[test]
fn csi_no_params() {
    assert_eq!(run(b"\x1b[H"), vec![csi(None, &[], &[], b'H')]);
}

#[test]
fn csi_omitted_params() {
    assert_eq!(
        run(b"\x1b[;5H"),
        vec![csi(None, &[None, Some(5)], &[], b'H')]
    );
    assert_eq!(
        run(b"\x1b[5;H"),
        vec![csi(None, &[Some(5), None], &[], b'H')]
    );
    assert_eq!(run(b"\x1b[;H"), vec![csi(None, &[None, None], &[], b'H')]);
}

#[test]
fn csi_explicit_zero_is_distinct_from_omitted() {
    assert_eq!(run(b"\x1b[0A"), vec![csi(None, &[Some(0)], &[], b'A')]);
}

#[test]
fn csi_private_marker() {
    assert_eq!(
        run(b"\x1b[?25;7h"),
        vec![csi(Some(b'?'), &[Some(25), Some(7)], &[], b'h')]
    );
    assert_eq!(run(b"\x1b[>c"), vec![csi(Some(b'>'), &[], &[], b'c')]);
    assert_eq!(run(b"\x1b[=c"), vec![csi(Some(b'='), &[], &[], b'c')]);
}

#[test]
fn csi_private_marker_after_param_is_ignored() {
    assert_eq!(run(b"\x1b[1?2hZ"), vec![Print(b'Z')]);
}

#[test]
fn csi_intermediates() {
    // DECSCL, DECSTR, DECSCUSR, DECCARA
    assert_eq!(
        run(b"\x1b[64;1\"p"),
        vec![csi(None, &[Some(64), Some(1)], b"\"", b'p')]
    );
    assert_eq!(run(b"\x1b[!p"), vec![csi(None, &[], b"!", b'p')]);
    assert_eq!(run(b"\x1b[2 q"), vec![csi(None, &[Some(2)], b" ", b'q')]);
    assert_eq!(
        run(b"\x1b[1;1;24;80;1$r"),
        vec![csi(
            None,
            &[Some(1), Some(1), Some(24), Some(80), Some(1)],
            b"$",
            b'r'
        )]
    );
}

#[test]
fn csi_private_and_intermediate() {
    // DECRQM for a DEC private mode.
    assert_eq!(
        run(b"\x1b[?3$p"),
        vec![csi(Some(b'?'), &[Some(3)], b"$", b'p')]
    );
}

#[test]
fn csi_param_after_intermediate_is_ignored() {
    assert_eq!(run(b"\x1b[ 1qZ"), vec![Print(b'Z')]);
}

#[test]
fn csi_too_many_intermediates_is_ignored() {
    assert_eq!(run(b"\x1b[1!!!pZ"), vec![Print(b'Z')]);
}

#[test]
fn c0_inside_csi_is_executed_without_aborting() {
    assert_eq!(
        run(b"\x1b[1\n2\x08H"),
        vec![Exec(b'\n'), Exec(0x08), csi(None, &[Some(12)], &[], b'H')]
    );
}

#[test]
fn del_inside_csi_is_ignored() {
    assert_eq!(
        run(b"\x1b[1\x7f2H"),
        vec![csi(None, &[Some(12)], &[], b'H')]
    );
}

#[test]
fn can_and_sub_abort_csi() {
    assert_eq!(run(b"\x1b[12\x18A"), vec![Exec(0x18), Print(b'A')]);
    assert_eq!(run(b"\x1b[12\x1aA"), vec![Exec(0x1A), Print(b'A')]);
}

#[test]
fn esc_inside_csi_restarts() {
    assert_eq!(
        run(b"\x1b[12\x1b[3H"),
        vec![csi(None, &[Some(3)], &[], b'H')]
    );
}

#[test]
fn csi_ignore_executes_c0_and_consumes_final() {
    assert_eq!(run(b"\x1b[1<\r2hZ"), vec![Exec(b'\r'), Print(b'Z')]);
}

#[test]
fn param_value_saturates() {
    assert_eq!(
        run(b"\x1b[99999999A"),
        vec![csi(None, &[Some(u16::MAX)], &[], b'A')]
    );
}

#[test]
fn param_count_is_truncated_but_dispatched() {
    let mut input = b"\x1b[".to_vec();
    for i in 0..40 {
        if i > 0 {
            input.push(b';');
        }
        input.extend(format!("{i}").bytes());
    }
    input.push(b'm');
    let mut parser = Parser::new();
    let rec = run_with(&mut parser, &input);
    let expected: Vec<Option<u16>> = (0..MAX_PARAMS as u16).map(Some).collect();
    assert_eq!(rec.ev, vec![csi(None, &expected, &[], b'm')]);
    assert!(rec.truncated);
}

#[test]
fn colon_subparameters() {
    let mut parser = Parser::new();
    let rec = run_with(&mut parser, b"\x1b[1;38:2::255:0:0m");
    assert_eq!(
        rec.ev,
        vec![csi(
            None,
            &[
                Some(1),
                Some(38),
                Some(2),
                None,
                Some(255),
                Some(0),
                Some(0)
            ],
            &[],
            b'm'
        )]
    );
    assert_eq!(
        rec.subparams,
        vec![false, false, true, true, true, true, true]
    );
}

#[test]
fn params_helpers() {
    struct P(Option<(u16, u16, u16, bool)>);
    impl Perform for P {
        fn csi_dispatch(&mut self, seq: Sequence<'_>) {
            let p = seq.params;
            self.0 = Some((
                p.get_or(0, 7),
                p.get_nonzero_or(1, 1),
                p.get_or(9, 3),
                p.is_empty(),
            ));
        }
    }
    let mut p = P(None);
    Parser::new().advance(&mut p, b"\x1b[;0H");
    assert_eq!(p.0, Some((7, 1, 3, false)));
}

// ------------------------------------------------------------------ 8-bit C1

#[test]
fn eight_bit_csi_and_c1_execute() {
    assert_eq!(
        run(b"\x9b5A\x84\x85"),
        vec![csi(None, &[Some(5)], &[], b'A'), Exec(0x84), Exec(0x85)]
    );
}

#[test]
fn c1_aborts_sequence_in_progress() {
    assert_eq!(run(b"\x1b[12\x84A"), vec![Exec(0x84), Print(b'A')]);
}

#[test]
fn eight_bit_st_in_ground_does_nothing() {
    assert_eq!(run(b"\x9cA"), vec![Print(b'A')]);
}

#[test]
fn gr_bytes_in_csi_are_treated_as_gl() {
    // 0xB5 = '5' | 0x80, 0xC1 = 'A' | 0x80
    assert_eq!(run(b"\x9b\xb5\xc1"), vec![csi(None, &[Some(5)], &[], b'A')]);
}

#[test]
fn seven_bit_mode_strips_bit_eight() {
    assert_eq!(run_mode(InputMode::SevenBit, b"\xc1"), vec![Print(b'A')]);
    // 0x9B masks to ESC.
    assert_eq!(
        run_mode(InputMode::SevenBit, b"\x9b[2J"),
        vec![csi(None, &[Some(2)], &[], b'J')]
    );
}

#[test]
fn no_c1_mode_discards_c1_bytes() {
    assert_eq!(run_mode(InputMode::EightBitNoC1, b"\x9b2J\xe9"), {
        let mut v = prints("2J");
        v.push(Print(0xE9));
        v
    });
}

// ------------------------------------------------------------------------ DCS

#[test]
fn dcs_decudk() {
    let mut expected = vec![Ev::Hook(None, vec![Some(1), Some(1)], vec![], b'|')];
    expected.extend(b"17/414243".iter().map(|&b| Put(b)));
    expected.extend([Unhook(Terminated), Esc(vec![], b'\\')]);
    assert_eq!(run(b"\x1bP1;1|17/414243\x1b\\"), expected);
}

#[test]
fn dcs_decrqss_with_intermediate() {
    assert_eq!(
        run(b"\x1bP$qm\x1b\\"),
        vec![
            Ev::Hook(None, vec![], b"$".to_vec(), b'q'),
            Put(b'm'),
            Unhook(Terminated),
            Esc(vec![], b'\\')
        ]
    );
}

#[test]
fn dcs_decdld_header() {
    let rec = run(b"\x1bP1;1;1;10;0;1;20;0{ @???\x9c");
    assert_eq!(
        rec[0],
        Ev::Hook(
            None,
            [1, 1, 1, 10, 0, 1, 20, 0]
                .iter()
                .map(|&v| Some(v))
                .collect(),
            vec![],
            b'{'
        )
    );
    assert_eq!(rec.last(), Some(&Unhook(Terminated)));
    assert_eq!(rec.len(), 1 + 5 + 1);
}

#[test]
fn dcs_passes_c0_and_drops_del() {
    assert_eq!(
        run(b"\x1bPq\r\x7f#\x9c"),
        vec![
            Ev::Hook(None, vec![], vec![], b'q'),
            Put(b'\r'),
            Put(b'#'),
            Unhook(Terminated)
        ]
    );
}

#[test]
fn dcs_aborted_by_can() {
    assert_eq!(
        run(b"\x1bP1|ab\x18c"),
        vec![
            Ev::Hook(None, vec![Some(1)], vec![], b'|'),
            Put(b'a'),
            Put(b'b'),
            Unhook(Aborted),
            Exec(0x18),
            Print(b'c')
        ]
    );
}

#[test]
fn dcs_aborted_by_c1() {
    assert_eq!(
        run(b"\x90|a\x85"),
        vec![
            Ev::Hook(None, vec![], vec![], b'|'),
            Put(b'a'),
            Unhook(Aborted),
            Exec(0x85)
        ]
    );
}

#[test]
fn dcs_header_ignores_c0() {
    assert_eq!(
        run(b"\x1bP1\r;2|\x9c"),
        vec![
            Ev::Hook(None, vec![Some(1), Some(2)], vec![], b'|'),
            Unhook(Terminated)
        ]
    );
}

#[test]
fn dcs_invalid_header_is_ignored_until_st() {
    assert_eq!(
        run(b"\x1bP1?2|data\x1b\\Z"),
        vec![Esc(vec![], b'\\'), Print(b'Z')]
    );
    assert_eq!(run(b"\x1bP!!!|data\x9cZ"), vec![Print(b'Z')]);
}

// ------------------------------------------------------------------------ OSC

#[test]
fn osc_terminated_by_bel_and_st() {
    let mut e = vec![OscStart];
    e.extend(b"0;hi".iter().map(|&b| OscPut(b)));
    e.push(OscEnd(Terminated));
    assert_eq!(run(b"\x1b]0;hi\x07"), e);

    let mut e2 = e.clone();
    e2.push(Esc(vec![], b'\\'));
    assert_eq!(run(b"\x1b]0;hi\x1b\\"), e2);
    assert_eq!(run(b"\x9d0;hi\x9c"), e);
}

#[test]
fn osc_bel_can_be_disabled() {
    let mut parser = Parser::new();
    parser.set_osc_bel_terminates(false);
    let rec = run_with(&mut parser, b"\x1b]x\x07y\x9c");
    assert_eq!(
        rec.ev,
        vec![OscStart, OscPut(b'x'), OscPut(b'y'), OscEnd(Terminated)]
    );
}

#[test]
fn osc_keeps_raw_high_bytes() {
    assert_eq!(
        run(b"\x1b]\xe9\x9c"),
        vec![OscStart, OscPut(0xE9), OscEnd(Terminated)]
    );
}

// ------------------------------------------------------------- SOS / PM / APC

#[test]
fn sos_pm_apc_strings() {
    assert_eq!(
        run(b"\x1b_ab\x1b\\\x1b^c\x9c\x98d\x18"),
        vec![
            StrStart(StringKind::Apc),
            StrPut(b'a'),
            StrPut(b'b'),
            StrEnd(Terminated),
            Esc(vec![], b'\\'),
            StrStart(StringKind::Pm),
            StrPut(b'c'),
            StrEnd(Terminated),
            StrStart(StringKind::Sos),
            StrPut(b'd'),
            StrEnd(Aborted),
            Exec(0x18),
        ]
    );
}

// ----------------------------------------------------------------------- VT52

fn run_vt52(input: &[u8]) -> Vec<Ev> {
    let mut parser = Parser::new();
    parser.set_vt52(true);
    run_with(&mut parser, input).ev
}

#[test]
fn vt52_simple_escapes() {
    assert_eq!(
        run_vt52(b"\x1bA\x1bH\x1b<\x1b["),
        vec![
            Esc(vec![], b'A'),
            Esc(vec![], b'H'),
            Esc(vec![], b'<'),
            Esc(vec![], b'[')
        ]
    );
}

#[test]
fn vt52_direct_cursor_address() {
    assert_eq!(
        run_vt52(b"\x1bY !\x1bY7o"),
        vec![Vt52Cup(0, 1), Vt52Cup(23, 79)]
    );
}

#[test]
fn vt52_cursor_address_executes_c0_and_honours_can() {
    assert_eq!(run_vt52(b"\x1bY\r #"), vec![Exec(b'\r'), Vt52Cup(0, 3)]);
    assert_eq!(run_vt52(b"\x1bY \x18A"), vec![Exec(0x18), Print(b'A')]);
}

#[test]
fn leaving_vt52_restores_ansi_parsing() {
    let mut parser = Parser::new();
    parser.set_vt52(true);
    assert_eq!(run_with(&mut parser, b"\x1b<").ev, vec![Esc(vec![], b'<')]);
    parser.set_vt52(false);
    assert_eq!(
        run_with(&mut parser, b"\x1b[H").ev,
        vec![csi(None, &[], &[], b'H')]
    );
}

// ----------------------------------------------------------------------- UTF-8

fn utf8(input: &[u8]) -> Vec<Ev> {
    run_mode(InputMode::Utf8, input)
}

const FFFD: char = char::REPLACEMENT_CHARACTER;

#[test]
fn utf8_decodes_multibyte() {
    assert_eq!(
        utf8("aé─😀".as_bytes()),
        vec![Print(b'a'), Char('é'), Char('─'), Char('😀')]
    );
}

#[test]
fn utf8_invalid_sequences() {
    // Unexpected continuation, bad lead, overlong, surrogate, beyond U+10FFFF.
    assert_eq!(utf8(b"\x80"), vec![Char(FFFD)]);
    assert_eq!(utf8(b"\xff"), vec![Char(FFFD)]);
    assert_eq!(utf8(b"\xc0\xaf"), vec![Char(FFFD), Char(FFFD)]);
    assert_eq!(
        utf8(b"\xed\xa0\x80"),
        vec![Char(FFFD), Char(FFFD), Char(FFFD)]
    );
    assert_eq!(utf8(b"\xf4\x90\x80\x80"), vec![Char(FFFD); 4]);
}

#[test]
fn utf8_truncated_sequence_then_ascii() {
    assert_eq!(
        utf8(b"\xc3(\xe2\x94A"),
        vec![Char(FFFD), Print(b'('), Char(FFFD), Print(b'A')]
    );
}

#[test]
fn utf8_truncated_sequence_then_escape() {
    assert_eq!(
        utf8(b"\xe2\x1b[H"),
        vec![Char(FFFD), csi(None, &[], &[], b'H')]
    );
}

#[test]
fn utf8_truncated_by_new_lead_byte() {
    assert_eq!(utf8(b"\xc3\xc3\xa9"), vec![Char(FFFD), Char('é')]);
}

#[test]
fn utf8_encoded_c1_acts_as_control() {
    assert_eq!(
        utf8(b"\xc2\x9b5A\xc2\x85"),
        vec![csi(None, &[Some(5)], &[], b'A'), Exec(0x85)]
    );
}

#[test]
fn utf8_raw_c1_bytes_are_not_controls_in_strings() {
    assert_eq!(
        utf8("\x1b]é\x07".as_bytes()),
        vec![OscStart, OscPut(0xC3), OscPut(0xA9), OscEnd(Terminated)]
    );
}

#[test]
fn utf8_high_bytes_ignored_inside_csi() {
    assert_eq!(
        utf8(b"\x1b[1\xc3\xa92H"),
        vec![csi(None, &[Some(12)], &[], b'H')]
    );
}

// ------------------------------------------------------------------ properties

#[test]
fn reset_returns_to_ground() {
    let mut parser = Parser::new();
    run_with(&mut parser, b"\x1bP1|abc");
    assert!(!parser.is_ground());
    parser.reset();
    assert!(parser.is_ground());
    assert_eq!(run_with(&mut parser, b"x").ev, vec![Print(b'x')]);
}

fn mode_strategy() -> impl Strategy<Value = InputMode> {
    prop_oneof![
        Just(InputMode::SevenBit),
        Just(InputMode::EightBit),
        Just(InputMode::EightBitNoC1),
        Just(InputMode::Utf8),
    ]
}

/// Biased towards bytes that drive state transitions.
fn stream_strategy() -> impl Strategy<Value = Vec<u8>> {
    let byte = prop_oneof![
        4 => any::<u8>(),
        2 => prop::sample::select(b"\x1b[]P\\;:?$\" 019Hm|q\x07\x18\x1a\x90\x9b\x9c\x9d\x9f\xc2\xe2\x94\x80".to_vec()),
    ];
    prop::collection::vec(byte, 0..300)
}

proptest! {
    #[test]
    fn chunking_does_not_change_output(
        mode in mode_strategy(),
        vt52 in any::<bool>(),
        data in stream_strategy(),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..8),
    ) {
        let mut whole = Parser::new();
        whole.set_input_mode(mode);
        whole.set_vt52(vt52);
        let expected = run_with(&mut whole, &data).ev;

        let mut points: Vec<usize> = cuts.iter().map(|c| c.index(data.len() + 1)).collect();
        points.sort_unstable();
        let mut split = Parser::new();
        split.set_input_mode(mode);
        split.set_vt52(vt52);
        let mut rec = Rec::default();
        let mut start = 0;
        for p in points.into_iter().chain([data.len()]) {
            split.advance(&mut rec, &data[start..p]);
            start = p;
        }
        prop_assert_eq!(rec.ev, expected);
    }

    #[test]
    fn byte_at_a_time_matches_bulk(mode in mode_strategy(), data in stream_strategy()) {
        let mut bulk = Parser::new();
        bulk.set_input_mode(mode);
        let expected = run_with(&mut bulk, &data).ev;

        let mut single = Parser::new();
        single.set_input_mode(mode);
        let mut rec = Rec::default();
        for &b in &data {
            single.advance_byte(&mut rec, b);
        }
        prop_assert_eq!(rec.ev, expected);
    }

    #[test]
    fn string_hooks_are_balanced(mode in mode_strategy(), data in stream_strategy()) {
        let mut parser = Parser::new();
        parser.set_input_mode(mode);
        let ev = run_with(&mut parser, &data).ev;
        let mut open: Option<&Ev> = None;
        for e in &ev {
            match e {
                Hook(..) | OscStart | StrStart(_) => {
                    prop_assert!(open.is_none(), "nested string start {:?}", e);
                    open = Some(e);
                }
                Unhook(_) => { prop_assert!(matches!(open, Some(Hook(..)))); open = None; }
                OscEnd(_) => { prop_assert!(matches!(open, Some(OscStart))); open = None; }
                StrEnd(_) => { prop_assert!(matches!(open, Some(StrStart(_)))); open = None; }
                Put(_) => prop_assert!(matches!(open, Some(Hook(..)))),
                OscPut(_) => prop_assert!(matches!(open, Some(OscStart))),
                StrPut(_) => prop_assert!(matches!(open, Some(StrStart(_)))),
                _ => prop_assert!(open.is_none(), "{:?} while string open", e),
            }
        }
    }
}
