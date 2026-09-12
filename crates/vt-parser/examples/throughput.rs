//! Rough parser throughput check: `cargo run --release -p vt-parser --example throughput [FILE]`.
//! Without FILE, a synthetic mix of text and SGR/CUP sequences is used.

use std::time::Instant;
use vt_parser::{InputMode, Parser, Perform, Sequence};

#[derive(Default)]
struct Count {
    printed: usize,
    sequences: usize,
}

impl Perform for Count {
    fn print_run(&mut self, bytes: &[u8]) {
        self.printed += bytes.len();
    }
    fn print_char(&mut self, _: char) {
        self.printed += 1;
    }
    fn csi_dispatch(&mut self, _: Sequence<'_>) {
        self.sequences += 1;
    }
}

fn main() -> std::io::Result<()> {
    let data = match std::env::args().nth(1) {
        Some(path) => std::fs::read(path)?,
        None => {
            let line = b"\x1b[1;31mERROR\x1b[0m  %SYSTEM-F-NOPRIV, insufficient privilege\r\n\x1b[12;40H\x1b(0lqqqk\x1b(B";
            line.repeat(64 * 1024 * 1024 / line.len())
        }
    };
    for mode in [InputMode::EightBit, InputMode::Utf8] {
        let mut parser = Parser::new();
        parser.set_input_mode(mode);
        let mut count = Count::default();
        let start = Instant::now();
        for chunk in data.chunks(4096) {
            parser.advance(&mut count, chunk);
        }
        let secs = start.elapsed().as_secs_f64();
        println!(
            "{mode:?}: {:.0} MB/s ({} printed, {} CSI)",
            data.len() as f64 / 1e6 / secs,
            count.printed,
            count.sequences
        );
    }
    Ok(())
}
