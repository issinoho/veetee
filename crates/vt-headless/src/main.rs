//! `vt-headless` — drives the emulator without a GUI.
//!
//! Currently provides `trace`, which prints every parser action for a byte
//! stream. Replay, scripted PTY sessions and screen dumps arrive with vt-core.

use std::fmt::Write as _;
use std::io::{self, BufWriter, Read, Write};
use std::process::ExitCode;

use vt_parser::{InputMode, Parser, Perform, Sequence, StringEnd, StringKind};

const USAGE: &str = "\
usage: vt-headless trace [--7bit | --8bit | --no-c1 | --utf8] [--vt52] [FILE]

Prints each parser action for FILE (or stdin), one per line.";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("trace") => match trace(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("vt-headless: {e}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn trace(args: impl Iterator<Item = String>) -> io::Result<()> {
    let mut parser = Parser::new();
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "--7bit" => parser.set_input_mode(InputMode::SevenBit),
            "--8bit" => parser.set_input_mode(InputMode::EightBit),
            "--no-c1" => parser.set_input_mode(InputMode::EightBitNoC1),
            "--utf8" => parser.set_input_mode(InputMode::Utf8),
            "--vt52" => parser.set_vt52(true),
            _ if path.is_none() && !arg.starts_with("--") => path = Some(arg),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unexpected argument {arg:?}\n{USAGE}"),
                ));
            }
        }
    }

    let mut input: Box<dyn Read> = match path {
        Some(p) => Box::new(std::fs::File::open(p)?),
        None => Box::new(io::stdin().lock()),
    };
    let mut tracer = Tracer {
        out: BufWriter::new(io::stdout().lock()),
        text: String::new(),
    };
    let mut buf = [0u8; 8192];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        parser.advance(&mut tracer, &buf[..n]);
    }
    tracer.flush_text();
    tracer.out.flush()
}

struct Tracer<W: Write> {
    out: W,
    text: String,
}

impl<W: Write> Tracer<W> {
    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            let text = std::mem::take(&mut self.text);
            self.line(format_args!("PRINT {text:?}"));
        }
    }

    fn line(&mut self, args: std::fmt::Arguments<'_>) {
        // Tracing to a closed pipe (e.g. `| head`) is not an error worth reporting.
        let _ = writeln!(self.out, "{args}");
    }

    fn event(&mut self, args: std::fmt::Arguments<'_>) {
        self.flush_text();
        self.line(args);
    }
}

fn seq_text(seq: &Sequence<'_>) -> String {
    let mut s = String::new();
    if let Some(p) = seq.private {
        s.push(char::from(p));
    }
    let _ = write!(s, "{:?}", seq.params);
    s.extend(seq.intermediates.iter().map(|&b| char::from(b)));
    s.push(char::from(seq.final_byte));
    s
}

fn control_name(b: u8) -> &'static str {
    const C0: [&str; 32] = [
        "NUL", "SOH", "STX", "ETX", "EOT", "ENQ", "ACK", "BEL", "BS", "HT", "LF", "VT", "FF", "CR",
        "SO", "SI", "DLE", "DC1", "DC2", "DC3", "DC4", "NAK", "SYN", "ETB", "CAN", "EM", "SUB",
        "ESC", "FS", "GS", "RS", "US",
    ];
    const C1: [&str; 32] = [
        "PAD", "HOP", "BPH", "NBH", "IND", "NEL", "SSA", "ESA", "HTS", "HTJ", "VTS", "PLD", "PLU",
        "RI", "SS2", "SS3", "DCS", "PU1", "PU2", "STS", "CCH", "MW", "SPA", "EPA", "SOS", "SGCI",
        "SCI", "CSI", "ST", "OSC", "PM", "APC",
    ];
    match b {
        0x00..=0x1F => C0[usize::from(b)],
        0x80..=0x9F => C1[usize::from(b - 0x80)],
        _ => "?",
    }
}

impl<W: Write> Perform for Tracer<W> {
    fn print(&mut self, byte: u8) {
        self.text.push(char::from(byte));
    }

    fn print_char(&mut self, ch: char) {
        self.text.push(ch);
    }

    fn execute(&mut self, byte: u8) {
        self.event(format_args!("EXEC  {}", control_name(byte)));
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], final_byte: u8) {
        let i: String = intermediates.iter().map(|&b| char::from(b)).collect();
        self.event(format_args!("ESC   {i}{}", char::from(final_byte)));
    }

    fn csi_dispatch(&mut self, seq: Sequence<'_>) {
        self.event(format_args!("CSI   {}", seq_text(&seq)));
    }

    fn dcs_hook(&mut self, seq: Sequence<'_>) {
        self.event(format_args!("DCS   {}", seq_text(&seq)));
    }

    fn dcs_put(&mut self, byte: u8) {
        self.print(byte);
    }

    fn dcs_unhook(&mut self, end: StringEnd) {
        self.event(format_args!("DCS-END {end:?}"));
    }

    fn osc_start(&mut self) {
        self.event(format_args!("OSC"));
    }

    fn osc_put(&mut self, byte: u8) {
        self.print(byte);
    }

    fn osc_end(&mut self, end: StringEnd) {
        self.event(format_args!("OSC-END {end:?}"));
    }

    fn string_start(&mut self, kind: StringKind) {
        self.event(format_args!("{kind:?}"));
    }

    fn string_put(&mut self, byte: u8) {
        self.print(byte);
    }

    fn string_end(&mut self, end: StringEnd) {
        self.event(format_args!("STRING-END {end:?}"));
    }

    fn vt52_cursor(&mut self, line: u8, column: u8) {
        self.event(format_args!("VT52-CUP {line};{column}"));
    }
}
