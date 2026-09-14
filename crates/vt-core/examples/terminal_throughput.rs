//! Measures how fast the terminal model processes typical host output.
//!
//! `cargo run --release -p vt-core --example terminal_throughput [MB]`

use std::time::Instant;

use vt_core::{Config, Extensions, Model, Terminal};

fn workloads(size: usize) -> Vec<(&'static str, Config, Vec<u8>)> {
    // THROUGHPUT_ONLY=text skips building the other workloads.
    let only = std::env::var("THROUGHPUT_ONLY").unwrap_or_default();
    let size = |name: &str| if name.starts_with(&only) { size } else { 0 };
    let fill = |pattern: &[u8], size: usize| {
        let mut out = Vec::with_capacity(size + pattern.len());
        while out.len() < size {
            out.extend_from_slice(pattern);
        }
        out
    };
    // TYPE of a text file: 72-character lines.
    let text = fill(
        b"$ TYPE SYS$MANAGER:SYSTARTUP_VMS.COM  The quick brown fox jumps over it.\r\n",
        size("text"),
    );
    // Renditions on every word, as a directory listing or report in colour.
    let sgr = fill(b"\x1b[1mDKA0:\x1b[m[\x1b[4mSYS0\x1b[m]\x1b[7mLOGIN.COM\x1b[m;1  \x1b[1;31m12/14\x1b[m  \x1b[32m14-SEP-2026 13:05\x1b[m\r\n", size("renditions"));
    // A forms application: cursor addressing and short fields over a full screen.
    let mut forms = Vec::new();
    let mut row = 0;
    while forms.len() < size("forms") {
        row = row % 24 + 1;
        forms.extend_from_slice(
            format!(
                "\x1b[{row};5H\x1b[7mName:\x1b[m\x1b[{row};12H{:<20}\x1b[{row};40H\x1b[1mQty\x1b[m {:>6}",
                "ACME WIDGET", row * 17
            )
            .as_bytes(),
        );
    }
    // Line drawing and national characters through G-sets.
    let graphics = fill(
        b"\x1b(0lqqqqqqqqqqqqqqqqqqqqqqqqwqqqqqqqqqqqk\x1b(B Gr\xfc\xdfe \x1b(0x\x1b(B caf\xe9\r\n",
        size("line"),
    );
    // UTF-8 text with the xterm extension.
    let utf8 = fill(
        "Grüße aus Zürich — «OpenVMS» ✓ ┌──────┐ λ→∞\r\n".as_bytes(),
        size("UTF"),
    );
    let vt525 = Config {
        model: Model::Vt525,
        ..Config::default()
    };
    let utf8_config = Config {
        extensions: Extensions {
            utf8: true,
            ..Extensions::default()
        },
        ..Config::default()
    };
    vec![
        ("text lines", Config::default(), text),
        ("renditions (VT525)", vt525, sgr),
        ("forms screen", Config::default(), forms),
        ("line drawing", Config::default(), graphics),
        ("UTF-8", utf8_config, utf8),
    ]
}

struct Count(usize);

impl vt_parser::Perform for Count {
    fn print(&mut self, _: u8) {
        self.0 += 1;
    }
    fn print_run(&mut self, bytes: &[u8]) {
        self.0 += bytes.len();
    }
}

fn main() {
    let mb: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(32);
    let experiments = std::env::var_os("THROUGHPUT_EXPERIMENTS").is_some();
    let only = std::env::var("THROUGHPUT_ONLY").ok();
    for (name, config, data) in workloads(mb * 1024 * 1024) {
        if only.as_deref().is_some_and(|o| !name.starts_with(o)) {
            continue;
        }
        if experiments {
            let mut parser = vt_parser::Parser::new();
            let mut count = Count(0);
            let start = Instant::now();
            for chunk in data.chunks(64 * 1024) {
                parser.advance(&mut count, chunk);
            }
            let secs = start.elapsed().as_secs_f64();
            println!(
                "{name:<20} parser only {:>8.1} MB/s",
                data.len() as f64 / 1048576.0 / secs
            );
            let mut term = Terminal::new(Config {
                scrollback_lines: 0,
                ..config.clone()
            });
            let start = Instant::now();
            for chunk in data.chunks(64 * 1024) {
                term.advance(chunk);
            }
            let secs = start.elapsed().as_secs_f64();
            println!(
                "{name:<20} no scrollback {:>8.1} MB/s",
                data.len() as f64 / 1048576.0 / secs
            );
        }
        let mut term = Terminal::new(config);
        let start = Instant::now();
        // Host output arrives in reads of up to 64 KB.
        for chunk in data.chunks(64 * 1024) {
            term.advance(chunk);
            let _ = term.take_output();
            let _ = term.take_events();
        }
        let secs = start.elapsed().as_secs_f64();
        println!(
            "{name:<20} {:>8.1} MB/s",
            data.len() as f64 / (1024.0 * 1024.0) / secs
        );
    }
}
