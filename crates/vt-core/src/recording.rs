//! Session recordings (`.vtrec`): what a host sent, what the terminal
//! answered, optionally what was typed, and named checkpoints, so a real
//! session can be replayed through the emulator and its screens compared.
//!
//! The format is line-oriented text:
//!
//! ```text
//! vtrec 1
//! config model=vt420 rows=24 cols=80 autowrap=0 newline=0 status=indicator ...
//! H 1520 G1tIAA==        bytes from the host, milliseconds from the start
//! R 1521 G1s/NjRjCg==    replies the terminal sent
//! K 3100 cw==            typed keys (only when keys are recorded)
//! M 4200 edt-help        a checkpoint
//! ```
//!
//! Data is base64. Unknown record types and configuration keys are ignored,
//! so later versions can add to the format.

use std::fmt::Write as _;
use std::io::{self, BufRead, Write};
use std::time::Instant;

use crate::charset::Nrc;
use crate::{Config, Model, StatusDisplay, Supplemental};

/// One line of a recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Host { ms: u64, data: Vec<u8> },
    Reply { ms: u64, data: Vec<u8> },
    Keys { ms: u64, data: Vec<u8> },
    Mark { ms: u64, name: String },
}

/// Writes a recording as a session runs.
#[derive(Debug)]
pub struct Recorder<W: Write> {
    out: W,
    start: Instant,
    keys: bool,
    marks: u32,
}

impl<W: Write> Recorder<W> {
    /// Starts a recording; typed keys are stored only when `keys` is set,
    /// because they include passwords.
    pub fn new(mut out: W, config: &Config, keys: bool) -> io::Result<Recorder<W>> {
        writeln!(out, "vtrec 1")?;
        writeln!(out, "config {}", config_line(config))?;
        out.flush()?;
        Ok(Recorder {
            out,
            start: Instant::now(),
            keys,
            marks: 0,
        })
    }

    fn ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn data(&mut self, tag: char, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let ms = self.ms();
        writeln!(self.out, "{tag} {ms} {}", base64_encode(data))
    }

    pub fn host(&mut self, data: &[u8]) -> io::Result<()> {
        self.data('H', data)
    }

    pub fn reply(&mut self, data: &[u8]) -> io::Result<()> {
        self.data('R', data)
    }

    pub fn keys(&mut self, data: &[u8]) -> io::Result<()> {
        if self.keys {
            self.data('K', data)
        } else {
            Ok(())
        }
    }

    /// Adds a checkpoint; returns its name (`checkpoint-01`, …) when none is given.
    pub fn mark(&mut self, name: Option<&str>) -> io::Result<String> {
        self.marks += 1;
        let name = match name {
            Some(n) => sanitize(n),
            None => format!("checkpoint-{:02}", self.marks),
        };
        let ms = self.ms();
        writeln!(self.out, "M {ms} {name}")?;
        self.out.flush()?;
        Ok(name)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// A checkpoint name usable as a file name.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "checkpoint".into()
    } else {
        cleaned
    }
}

/// Reads a recording: its configuration and records.
pub fn read(input: impl BufRead) -> io::Result<(Config, Vec<Record>)> {
    let invalid = |msg: String| io::Error::new(io::ErrorKind::InvalidData, msg);
    let mut lines = input.lines();
    match lines.next().transpose()? {
        Some(header) if header.trim() == "vtrec 1" => {}
        other => return Err(invalid(format!("not a vtrec 1 recording: {other:?}"))),
    }
    let mut config = Config::default();
    let mut records = Vec::new();
    for (number, line) in lines.enumerate() {
        let line = line?;
        let mut parts = line.splitn(3, ' ');
        let (tag, rest) = (parts.next().unwrap_or(""), parts.collect::<Vec<_>>());
        let at = |i: usize| rest.get(i).copied().unwrap_or("");
        let bad = |what: &str| invalid(format!("line {}: {what}: {line:?}", number + 2));
        match tag {
            "config" => {
                let text = line.trim_start_matches("config").trim();
                apply_config(&mut config, text).map_err(|e| bad(&e))?;
            }
            "H" | "R" | "K" => {
                let ms = at(0).parse().map_err(|_| bad("bad time"))?;
                let data = base64_decode(at(1)).ok_or_else(|| bad("bad data"))?;
                records.push(match tag {
                    "H" => Record::Host { ms, data },
                    "R" => Record::Reply { ms, data },
                    _ => Record::Keys { ms, data },
                });
            }
            "M" => {
                let ms = at(0).parse().map_err(|_| bad("bad time"))?;
                records.push(Record::Mark {
                    ms,
                    name: sanitize(at(1)),
                });
            }
            "" => {}
            _ => {}
        }
    }
    Ok((config, records))
}

fn config_line(c: &Config) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "model={} rows={} cols={} autowrap={} newline={} status={} national={} supplemental={} udk-locked={}",
        c.model.term_name_exact(),
        c.rows,
        c.cols,
        u8::from(c.autowrap),
        u8::from(c.new_line),
        match c.status_display {
            StatusDisplay::None => "none",
            StatusDisplay::Indicator => "indicator",
            StatusDisplay::HostWritable => "host",
        },
        u8::from(c.national_mode),
        match c.supplemental {
            Supplemental::DecSupplemental => "dec",
            Supplemental::IsoLatin1 => "latin1",
        },
        u8::from(c.udk_locked),
    );
    if let Some(language) = c.keyboard_language {
        let _ = write!(s, " keyboard={}", nrc_name(language));
    }
    let e = c.extensions;
    let _ = write!(
        s,
        " utf8={} xterm-sgr={} xterm-compat={}",
        u8::from(e.utf8),
        u8::from(e.xterm_sgr),
        u8::from(e.xterm_compat)
    );
    if !c.answerback.is_empty() {
        let _ = write!(s, " answerback={}", base64_encode(&c.answerback));
    }
    s
}

fn apply_config(c: &mut Config, text: &str) -> Result<(), String> {
    for item in text.split_whitespace() {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| format!("bad setting {item}"))?;
        let flag = || value == "1";
        let number = || {
            value
                .parse::<usize>()
                .map_err(|_| format!("bad number {item}"))
        };
        match key {
            "model" => {
                c.model = Model::from_name(value).ok_or_else(|| format!("unknown model {value}"))?
            }
            "rows" => c.rows = number()?,
            "cols" => c.cols = number()?,
            "autowrap" => c.autowrap = flag(),
            "newline" => c.new_line = flag(),
            "national" => c.national_mode = flag(),
            "udk-locked" => c.udk_locked = flag(),
            "status" => {
                c.status_display = match value {
                    "none" => StatusDisplay::None,
                    "host" => StatusDisplay::HostWritable,
                    _ => StatusDisplay::Indicator,
                }
            }
            "supplemental" => {
                c.supplemental = if value == "latin1" {
                    Supplemental::IsoLatin1
                } else {
                    Supplemental::DecSupplemental
                }
            }
            "keyboard" => c.keyboard_language = nrc_from_name(value),
            "utf8" => c.extensions.utf8 = flag(),
            "xterm-sgr" => c.extensions.xterm_sgr = flag(),
            "xterm-compat" => c.extensions.xterm_compat = flag(),
            "answerback" => c.answerback = base64_decode(value).unwrap_or_default(),
            _ => {}
        }
    }
    Ok(())
}

const NRC_NAMES: [(Nrc, &str); 12] = [
    (Nrc::British, "british"),
    (Nrc::Dutch, "dutch"),
    (Nrc::Finnish, "finnish"),
    (Nrc::French, "french"),
    (Nrc::FrenchCanadian, "french-canadian"),
    (Nrc::German, "german"),
    (Nrc::Italian, "italian"),
    (Nrc::NorwegianDanish, "norwegian-danish"),
    (Nrc::Portuguese, "portuguese"),
    (Nrc::Spanish, "spanish"),
    (Nrc::Swedish, "swedish"),
    (Nrc::Swiss, "swiss"),
];

fn nrc_name(n: Nrc) -> &'static str {
    NRC_NAMES
        .iter()
        .find(|(x, _)| *x == n)
        .map_or("", |(_, s)| s)
}

fn nrc_from_name(name: &str) -> Option<Nrc> {
    NRC_NAMES.iter().find(|(_, s)| *s == name).map(|(n, _)| *n)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = ALPHABET.iter().position(|&a| a == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"a", b"ab", b"abc", b"\x1b[?64;1;2c\xff\x00"] {
            assert_eq!(base64_decode(&base64_encode(data)).unwrap(), data);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn recording_round_trip() {
        let config = Config {
            model: Model::Vt525,
            rows: 25,
            autowrap: true,
            keyboard_language: Some(Nrc::German),
            answerback: b"VMS1".to_vec(),
            ..Config::default()
        };
        let mut buf = Vec::new();
        {
            let mut r = Recorder::new(&mut buf, &config, false).unwrap();
            r.host(b"\x1b[c").unwrap();
            r.reply(b"\x1b[?65c").unwrap();
            r.keys(b"secret\r").unwrap();
            assert_eq!(r.mark(None).unwrap(), "checkpoint-01");
            assert_eq!(r.mark(Some("EDT help")).unwrap(), "EDT-help");
        }
        let (read_config, records) = read(&buf[..]).unwrap();
        assert_eq!(read_config, config);
        assert!(matches!(&records[0], Record::Host { data, .. } if data == b"\x1b[c"));
        assert!(matches!(&records[1], Record::Reply { data, .. } if data == b"\x1b[?65c"));
        assert!(matches!(&records[2], Record::Mark { name, .. } if name == "checkpoint-01"));
        assert_eq!(records.len(), 4, "keys are not recorded unless asked");
    }
}
