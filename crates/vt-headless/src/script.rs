//! `run` subcommand: scripted terminal sessions with golden screen snapshots.
//!
//! A script is one command per line (`#` starts a comment):
//!
//! ```text
//! model vt102                 # terminal model (before spawn)
//! size 24 80                  # page size (before spawn)
//! autowrap on                 # Set-Up autowrap (before spawn)
//! answerback "veetee"         # Set-Up answerback (before spawn)
//! spawn ${VTTEST} 24x80.132   # start a program; ${VAR} expands from the environment
//! prompt "Push <RETURN>"      # wait for text ending at the cursor, output idle
//! wait-text "Test of"         # wait for text anywhere on the page, output idle
//! send "1\r"                  # send raw bytes; escapes \r \n \t \e \\ \" \xHH
//! type "hello\r"              # type on the main keypad (honours SRM local echo)
//! key pf1 kp7 up f16          # press DEC keys: see `parse_key`
//! snapshot menu1-box80        # compare with GOLDEN/menu1-box80.screen
//! step menu1-wrap80           # prompt "Push <RETURN>", snapshot, send "\r"
//! walk menu11-1 "choice (0 - 7): "  # step through every Push <RETURN> screen
//!                             # (snapshots NAME-01, NAME-02, …) until PROMPT
//! settle                      # wait for output after the last input to go quiet
//! send-answerback             # the DEC Ctrl+Break local function
//! sleep 200                   # milliseconds, still processing output
//! timeout 60                  # seconds to wait in prompt/wait-text/walk (default 15)
//! expect-exit                 # wait for the program to finish
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vt_core::dump::{dump, row_text};
use vt_core::{Config, Event, Key, Model, Terminal};
use vt_transport::pty::Pty;

use crate::USAGE;

const IDLE: Duration = Duration::from_millis(60);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

pub fn run(args: impl Iterator<Item = String>) -> io::Result<bool> {
    let mut script = None;
    let mut golden = None;
    let mut bless = false;
    let mut record = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--golden" => golden = Some(PathBuf::from(value(&mut args, "--golden")?)),
            "--record" => record = Some(PathBuf::from(value(&mut args, "--record")?)),
            "--bless" => bless = true,
            _ if script.is_none() && !arg.starts_with("--") => script = Some(PathBuf::from(arg)),
            _ => return Err(invalid(format!("unexpected argument {arg:?}\n{USAGE}"))),
        }
    }
    let script = script.ok_or_else(|| invalid(USAGE.to_string()))?;
    let golden =
        golden.unwrap_or_else(|| script.parent().unwrap_or(Path::new(".")).join("snapshots"));
    let text = fs::read_to_string(&script)?;
    let mut session = Session {
        config: Config::default(),
        term: None,
        pty: None,
        golden,
        bless,
        record: Vec::new(),
        output_since_send: false,
        failures: 0,
        timeout: DEFAULT_TIMEOUT,
    };

    for (lineno, line) in text.lines().enumerate() {
        let words = tokenize(line)
            .map_err(|e| invalid(format!("{}:{}: {e}", script.display(), lineno + 1)))?;
        let Some((cmd, rest)) = words.split_first() else {
            continue;
        };
        if let Err(e) = session.command(cmd, rest) {
            let screen = session.term.as_ref().map(dump).unwrap_or_default();
            return Err(io::Error::new(
                e.kind(),
                format!("{}:{}: {line}\n{e}\n{screen}", script.display(), lineno + 1),
            ));
        }
    }
    if let Some(path) = record {
        fs::write(path, &session.record)?;
    }
    if session.failures > 0 {
        eprintln!("{} snapshot(s) differ", session.failures);
    }
    Ok(session.failures == 0)
}

struct Session {
    config: Config,
    term: Option<Terminal>,
    pty: Option<Pty>,
    golden: PathBuf,
    bless: bool,
    record: Vec<u8>,
    output_since_send: bool,
    failures: usize,
    timeout: Duration,
}

impl Session {
    fn command(&mut self, cmd: &str, args: &[String]) -> io::Result<()> {
        let arg = |i: usize| {
            args.get(i)
                .map(String::as_str)
                .ok_or_else(|| invalid(format!("{cmd}: missing argument")))
        };
        match cmd {
            "model" => self.config.model = parse_model(arg(0)?)?,
            "size" => {
                self.config.rows = parse_num(arg(0)?)?;
                self.config.cols = parse_num(arg(1)?)?;
            }
            "autowrap" => self.config.autowrap = arg(0)? == "on",
            // Opt-in departures from DEC behaviour, e.g. `extensions xterm-compat`.
            "extensions" => {
                for name in args {
                    let ext = &mut self.config.extensions;
                    match name.as_str() {
                        "utf8" => ext.utf8 = true,
                        "xterm-sgr" => ext.xterm_sgr = true,
                        "xterm-compat" => ext.xterm_compat = true,
                        other => return Err(invalid(format!("unknown extension {other}"))),
                    }
                }
            }
            "answerback" => self.config.answerback = unescape(arg(0)?)?,
            "spawn" => {
                let program = expand(arg(0)?)?;
                let rest: Vec<String> = args[1..]
                    .iter()
                    .map(|a| expand(a))
                    .collect::<io::Result<_>>()?;
                let term = Terminal::new(self.config.clone());
                let pty = Pty::spawn(
                    &program,
                    &rest,
                    self.config.rows as u16,
                    self.config.cols as u16,
                    self.config.model.term_name(),
                )?;
                self.term = Some(term);
                self.pty = Some(pty);
            }
            "send" => {
                let bytes = unescape(arg(0)?)?;
                self.pump(Duration::ZERO)?;
                self.pty()?.write_all(&bytes)?;
                self.output_since_send = false;
            }
            "type" => {
                let text = String::from_utf8_lossy(&unescape(arg(0)?)?).into_owned();
                self.pump(Duration::ZERO)?;
                let term = self
                    .term
                    .as_mut()
                    .ok_or_else(|| invalid("no program spawned".into()))?;
                term.type_text(&text);
                self.flush_keys()?;
            }
            "key" => {
                if args.is_empty() {
                    return Err(invalid("key: missing key name".into()));
                }
                self.pump(Duration::ZERO)?;
                for name in args {
                    let key = parse_key(name)?;
                    let term = self
                        .term
                        .as_mut()
                        .ok_or_else(|| invalid("no program spawned".into()))?;
                    term.key(key);
                    self.flush_keys()?;
                }
            }
            "prompt" => {
                let text = String::from_utf8_lossy(&unescape(arg(0)?)?).into_owned();
                self.wait_for(|t| prompt_at_cursor(t, &text)).map_err(|e| {
                    io::Error::new(e.kind(), format!("{e} waiting for prompt {text:?}"))
                })?;
            }
            "wait-text" => {
                let text = String::from_utf8_lossy(&unescape(arg(0)?)?).into_owned();
                self.wait_for(|t| (0..t.grid().rows()).any(|r| row_text(t, r).contains(&text)))
                    .map_err(|e| {
                        io::Error::new(e.kind(), format!("{e} waiting for text {text:?}"))
                    })?;
            }
            "settle" => self.wait_for(|_| true)?,
            "timeout" => self.timeout = Duration::from_secs(parse_num(arg(0)?)? as u64),
            "send-answerback" => {
                self.pump(Duration::ZERO)?;
                let term = self
                    .term
                    .as_mut()
                    .ok_or_else(|| invalid("no program spawned".into()))?;
                term.send_answerback();
                self.flush_keys()?;
            }
            "sleep" => {
                let end = Instant::now() + Duration::from_millis(parse_num(arg(0)?)? as u64);
                while Instant::now() < end {
                    self.pump(Duration::from_millis(10))?;
                }
            }
            "snapshot" => self.snapshot(arg(0)?)?,
            "step" => {
                let name = arg(0)?.to_string();
                self.command("prompt", &["Push <RETURN>".to_string()])?;
                self.snapshot(&name)?;
                self.command("send", &["\\r".to_string()])?;
            }
            "walk" => {
                let prefix = arg(0)?.to_string();
                let menu = String::from_utf8_lossy(&unescape(arg(1)?)?).into_owned();
                let mut page = 0;
                loop {
                    self.wait_for(|t| {
                        prompt_at_cursor(t, "Push <RETURN>") || prompt_at_cursor(t, &menu)
                    })
                    .map_err(|e| io::Error::new(e.kind(), format!("{e} during walk {prefix}")))?;
                    let term = self
                        .term
                        .as_ref()
                        .ok_or_else(|| invalid("no program spawned".into()))?;
                    if prompt_at_cursor(term, &menu) {
                        break;
                    }
                    page += 1;
                    self.snapshot(&format!("{prefix}-{page:02}"))?;
                    self.command("send", &["\\r".to_string()])?;
                }
            }
            "expect-exit" => {
                let deadline = Instant::now() + self.timeout;
                loop {
                    match self.pump(IDLE) {
                        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                        Err(e) => return Err(e),
                        Ok(_) if Instant::now() > deadline => {
                            return Err(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "program did not exit",
                            ));
                        }
                        Ok(_) => {}
                    }
                }
            }
            _ => return Err(invalid(format!("unknown command {cmd:?}"))),
        }
        Ok(())
    }

    /// Sends queued keyboard output to the program.
    fn flush_keys(&mut self) -> io::Result<()> {
        let bytes = self
            .term
            .as_mut()
            .map(Terminal::take_output)
            .unwrap_or_default();
        self.pty()?.write_all(&bytes)?;
        self.output_since_send = false;
        Ok(())
    }

    fn pty(&mut self) -> io::Result<&mut Pty> {
        self.pty
            .as_mut()
            .ok_or_else(|| invalid("no program spawned".into()))
    }

    /// Reads whatever arrives within `timeout`, feeding the terminal and
    /// answering its reports. Returns the number of bytes processed.
    fn pump(&mut self, timeout: Duration) -> io::Result<usize> {
        let (Some(pty), Some(term)) = (self.pty.as_mut(), self.term.as_mut()) else {
            return Err(invalid("no program spawned".into()));
        };
        let mut buf = [0u8; 16 * 1024];
        let mut total = 0;
        let mut wait = timeout;
        loop {
            let n = pty.read_timeout(&mut buf, wait)?;
            if n == 0 {
                break;
            }
            total += n;
            self.record.extend_from_slice(&buf[..n]);
            term.advance(&buf[..n]);
            let reply = term.take_output();
            if !reply.is_empty() {
                pty.write_all(&reply)?;
            }
            for event in term.take_events() {
                if let Event::ColumnsChanged(cols) = event {
                    pty.resize(term.grid().rows() as u16, cols as u16)?;
                }
            }
            wait = Duration::ZERO;
        }
        if total > 0 {
            self.output_since_send = true;
        }
        Ok(total)
    }

    /// Waits until output has gone quiet with `done` true, having seen output
    /// since the last `send`.
    fn wait_for(&mut self, done: impl Fn(&Terminal) -> bool) -> io::Result<()> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let n = self.pump(IDLE)?;
            if n == 0 && self.output_since_send && self.term.as_ref().is_some_and(&done) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"));
            }
        }
    }

    fn snapshot(&mut self, name: &str) -> io::Result<()> {
        let term = self
            .term
            .as_ref()
            .ok_or_else(|| invalid("no program spawned".into()))?;
        if !crate::golden::compare(&self.golden, name, &dump(term), self.bless)? {
            self.failures += 1;
        }
        Ok(())
    }
}

/// True when `text` appears on the cursor row immediately before the cursor.
fn prompt_at_cursor(term: &Terminal, text: &str) -> bool {
    let cursor = term.cursor();
    let line: String = term
        .grid()
        .line(cursor.row)
        .cells()
        .iter()
        .map(|c| c.ch)
        .collect();
    let end = if cursor.pending_wrap {
        cursor.col + 1
    } else {
        cursor.col
    };
    let before: String = line.chars().take(end).collect();
    before.ends_with(text)
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> io::Result<String> {
    args.next()
        .ok_or_else(|| invalid(format!("{flag} needs a value")))
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

fn parse_num(s: &str) -> io::Result<usize> {
    s.parse()
        .map_err(|_| invalid(format!("not a number: {s:?}")))
}

fn parse_model(s: &str) -> io::Result<Model> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "vt100" => Model::Vt100,
        "vt102" => Model::Vt102,
        "vt220" => Model::Vt220,
        "vt320" => Model::Vt320,
        "vt420" => Model::Vt420,
        "vt510" => Model::Vt510,
        "vt520" => Model::Vt520,
        "vt525" => Model::Vt525,
        _ => return Err(invalid(format!("unknown model {s:?}"))),
    })
}

/// DEC key names: `return delete tab linefeed escape backspace up down left
/// right find insert remove select prev next pf1`–`pf4 kp0`–`kp9 kp- kp, kp.
/// enter f6`–`f20 help do udk6`–`udk20` (shifted function keys).
fn parse_key(name: &str) -> io::Result<Key> {
    let lower = name.to_ascii_lowercase();
    Ok(match lower.as_str() {
        "return" => Key::Return,
        "delete" => Key::Delete,
        "tab" => Key::Tab,
        "linefeed" => Key::LineFeed,
        "escape" => Key::Escape,
        "backspace" => Key::Backspace,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "find" => Key::Find,
        "insert" => Key::InsertHere,
        "remove" => Key::Remove,
        "select" => Key::Select,
        "prev" => Key::PrevScreen,
        "next" => Key::NextScreen,
        "pf1" => Key::Pf1,
        "pf2" => Key::Pf2,
        "pf3" => Key::Pf3,
        "pf4" => Key::Pf4,
        "kp-" => Key::KeypadMinus,
        "kp," => Key::KeypadComma,
        "kp." => Key::KeypadPeriod,
        "enter" => Key::KeypadEnter,
        "help" => Key::Function(15),
        "do" => Key::Function(16),
        _ => {
            if let Some(d) = lower
                .strip_prefix("kp")
                .and_then(|d| d.parse::<u8>().ok())
                .filter(|d| *d <= 9)
            {
                Key::Keypad(d)
            } else if let Some(f) = lower
                .strip_prefix("udk")
                .and_then(|f| f.parse::<u8>().ok())
                .filter(|f| (6..=20).contains(f))
            {
                Key::UserDefined(f)
            } else if let Some(f) = lower
                .strip_prefix('f')
                .and_then(|f| f.parse::<u8>().ok())
                .filter(|f| (6..=20).contains(f))
            {
                Key::Function(f)
            } else {
                return Err(invalid(format!("unknown key {name:?}")));
            }
        }
    })
}

fn expand(s: &str) -> io::Result<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let end = rest[start..]
            .find('}')
            .ok_or_else(|| invalid(format!("unterminated ${{ in {s:?}")))?;
        let name = &rest[start + 2..start + end];
        let val = std::env::var(name)
            .map_err(|_| invalid(format!("environment variable {name} is not set")))?;
        out.push_str(&val);
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Splits a line into words; double-quoted words keep spaces and escapes.
fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '#' {
            break;
        } else if c == '"' {
            chars.next();
            let mut word = String::new();
            loop {
                match chars.next() {
                    None => return Err("unterminated string".into()),
                    Some('"') => break,
                    Some('\\') => {
                        word.push('\\');
                        word.push(chars.next().ok_or("dangling backslash")?);
                    }
                    Some(ch) => word.push(ch),
                }
            }
            words.push(word);
        } else {
            let mut word = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() {
                    break;
                }
                word.push(ch);
                chars.next();
            }
            words.push(word);
        }
    }
    Ok(words)
}

fn unescape(s: &str) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('r') => out.push(b'\r'),
            Some('n') => out.push(b'\n'),
            Some('t') => out.push(b'\t'),
            Some('e') => out.push(0x1B),
            Some('\\') => out.push(b'\\'),
            Some('"') => out.push(b'"'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                let byte = u8::from_str_radix(&hex, 16)
                    .map_err(|_| invalid(format!("bad \\x escape in {s:?}")))?;
                out.push(byte);
            }
            other => {
                return Err(invalid(format!(
                    "bad escape \\{} in {s:?}",
                    other.unwrap_or(' ')
                )));
            }
        }
    }
    Ok(out)
}
