#![no_main]

use libfuzzer_sys::fuzz_target;
use vt_parser::{InputMode, Parser, Perform, Sequence};

/// Touches every parameter accessor so out-of-range bugs surface.
struct Sink;

impl Perform for Sink {
    fn csi_dispatch(&mut self, seq: Sequence<'_>) {
        for (i, p) in seq.params.iter().enumerate() {
            assert_eq!(p.value, seq.params.get(i));
        }
        let _ = seq.params.get_nonzero_or(seq.params.len(), 1);
    }
    fn dcs_hook(&mut self, seq: Sequence<'_>) {
        self.csi_dispatch(seq);
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&control, rest)) = data.split_first() else {
        return;
    };
    let mut parser = Parser::new();
    parser.set_input_mode(match control & 3 {
        0 => InputMode::SevenBit,
        1 => InputMode::EightBit,
        2 => InputMode::EightBitNoC1,
        _ => InputMode::Utf8,
    });
    parser.set_vt52(control & 4 != 0);
    // Feed in chunks whose size comes from the control byte to exercise boundaries.
    let chunk = usize::from(control >> 3).max(1);
    for piece in rest.chunks(chunk) {
        parser.advance(&mut Sink, piece);
    }
});
