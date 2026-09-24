//! `vt-headless kermit`: a Kermit transfer over any connection veetee has,
//! with no window.
//!
//! It is a tool in its own right — getting a file off an OpenVMS system over a
//! serial console from a script — and it is how veetee's Kermit is tested
//! against a real one: `gkermit` refuses to run on a pipe but runs happily on
//! a pty, which is exactly what `--command` gives it.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vt_kermit::{
    Check, Mode, Names, Params, Receiver, Sender, Settings, Source, Status, Store, ours,
};
use vt_transport::Transport;
use vt_transport::pty::Pty;
use vt_transport::serial::{Serial, SerialConfig};
use vt_transport::ssh::{self, SshConfig};
use vt_transport::telnet::{Telnet, TelnetConfig};

pub const USAGE: &str = "\
usage:
  vt-headless kermit receive [--into DIR] CONNECTION [OPTIONS]
  vt-headless kermit send FILE... CONNECTION [OPTIONS]

connections:
  --command COMMAND      run COMMAND on a pty, e.g. \"gkermit -s notes.txt\"
  --telnet HOST[:PORT]   Telnet
  --ssh [USER@]HOST      SSH, through the system OpenSSH client
  --serial DEVICE        a serial line, with -b RATE -d BITS -p PARITY -s STOP -f FLOW
                         as for veetee (DEC factory settings: 9600 8N1 XON/XOFF)

options:
  --binary               send the bytes exactly (default: text, CR LF on the line)
  --names as-sent        keep a received name as sent, version and case included
                         (default: LOGIN.COM;3 arrives as login.com)
  --check 1|2|3          the block check to ask for (default 3, the CRC; a Kermit
                         that cannot do it answers otherwise, and both use 1)
  --retries N            tries before giving up on a packet (default 10)
  --verbose              print every packet to stderr

Start the other end first: SEND or RECEIVE at the host's Kermit prompt.";

/// Runs `vt-headless kermit ...`. `Ok(false)` is a transfer that did not
/// finish, which the caller turns into a failing exit status.
pub fn kermit(args: impl Iterator<Item = String>) -> Result<bool, String> {
    let args: Vec<String> = args.collect();
    let command = parse(&args)?;
    let transport = open(&command.connection)?;
    run(command, transport)
}

enum Direction {
    Receive { into: PathBuf },
    Send { files: Vec<PathBuf> },
}

enum Connection {
    Command(String),
    Telnet(String, u16),
    Ssh(SshConfig),
    Serial(SerialConfig),
}

struct Command {
    direction: Direction,
    connection: Connection,
    settings: Settings,
    verbose: bool,
}

fn parse(args: &[String]) -> Result<Command, String> {
    let mut args = args.iter();
    let receiving = match args.next().map(String::as_str) {
        Some("receive") => true,
        Some("send") => false,
        _ => return Err(USAGE.into()),
    };
    let mut into = PathBuf::from(".");
    let mut files = Vec::new();
    let mut connection = None;
    let mut line: Vec<(String, String)> = Vec::new();
    let mut settings = Settings::default();
    let mut verbose = false;
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        let chosen = match arg.as_str() {
            "--into" => {
                into = value()?.into();
                None
            }
            "--command" => Some(Connection::Command(value()?)),
            "--telnet" => {
                let v = value()?;
                Some(match v.rsplit_once(':') {
                    Some((host, port)) => Connection::Telnet(
                        host.into(),
                        port.parse().map_err(|_| format!("not a port: {port:?}"))?,
                    ),
                    None => Connection::Telnet(v, 23),
                })
            }
            "--ssh" => Some(Connection::Ssh(SshConfig {
                destination: value()?,
                port: None,
            })),
            "--serial" => Some(Connection::Serial(SerialConfig::new(value()?))),
            "-b" | "-d" | "-p" | "-s" | "-f" => {
                line.push((arg.clone(), value()?));
                None
            }
            "--binary" => {
                settings.mode = Mode::Binary;
                None
            }
            "--names" => {
                settings.names = match value()?.as_str() {
                    "as-sent" => Names::AsSent,
                    "converted" => Names::Converted,
                    other => return Err(format!("--names: as-sent or converted, not {other:?}")),
                };
                None
            }
            "--check" => {
                let v = value()?;
                let check = v
                    .bytes()
                    .next()
                    .filter(|_| v.len() == 1)
                    .and_then(Check::from_byte)
                    .ok_or_else(|| format!("--check: 1, 2 or 3, not {v:?}"))?;
                settings.params = Params { check, ..ours() };
                None
            }
            "--retries" => {
                let v = value()?;
                settings.retries = v.parse().map_err(|_| format!("--retries: {v:?}"))?;
                None
            }
            "--verbose" => {
                verbose = true;
                None
            }
            file if !receiving && !file.starts_with('-') => {
                files.push(PathBuf::from(file));
                None
            }
            other => return Err(format!("unexpected argument {other:?}\n\n{USAGE}")),
        };
        if let Some(c) = chosen {
            if connection.is_some() {
                return Err("choose one connection".into());
            }
            connection = Some(c);
        }
    }
    let mut connection = connection.ok_or_else(|| format!("which connection?\n\n{USAGE}"))?;
    match &mut connection {
        Connection::Serial(serial) => {
            for (flag, v) in line {
                let bad = || format!("{flag}: {v:?}");
                match flag.as_str() {
                    "-b" => serial.baud = v.parse().map_err(|_| bad())?,
                    "-d" => serial.data_bits = v.parse().map_err(|_| bad())?,
                    "-p" => serial.parity = v.parse()?,
                    "-s" => serial.stop_bits = v.parse().map_err(|_| bad())?,
                    _ => serial.flow = v.parse()?,
                }
            }
        }
        _ if !line.is_empty() => return Err("line settings need --serial".into()),
        _ => {}
    }
    let direction = if receiving {
        Direction::Receive { into }
    } else {
        if files.is_empty() {
            return Err("send what?".into());
        }
        for file in &files {
            if !file.is_file() {
                return Err(format!("{}: not a file", file.display()));
            }
        }
        Direction::Send { files }
    };
    Ok(Command {
        direction,
        connection,
        settings,
        verbose,
    })
}

fn open(connection: &Connection) -> Result<Box<dyn Transport>, String> {
    let (rows, cols, term) = (24, 80, "vt420");
    let opened: io::Result<Box<dyn Transport>> = match connection {
        Connection::Command(cmd) => {
            let (shell, flag) = if cfg!(windows) {
                ("cmd", "/C")
            } else {
                ("/bin/sh", "-c")
            };
            Pty::spawn(shell, &[flag, cmd.as_str()], rows, cols, term)
                .map(|p| Box::new(p) as Box<dyn Transport>)
        }
        Connection::Telnet(host, port) => Telnet::connect(TelnetConfig {
            port: *port,
            rows,
            cols,
            ..TelnetConfig::new(host.clone(), "VT420")
        })
        .map(|t| Box::new(t) as Box<dyn Transport>),
        Connection::Ssh(config) => {
            ssh::connect(config, rows, cols, term).map(|s| Box::new(s) as Box<dyn Transport>)
        }
        Connection::Serial(config) => {
            Serial::open(config.clone()).map(|s| Box::new(s) as Box<dyn Transport>)
        }
    };
    opened.map_err(|e| format!("cannot connect: {e}"))
}

/// Either end of a transfer, so the loop below can drive both.
enum End {
    Receiving(Receiver, Received),
    Sending(Sender, Files),
}

impl End {
    fn feed(&mut self, bytes: &[u8], now: Instant) -> Vec<u8> {
        match self {
            End::Receiving(r, store) => r.feed(bytes, now, store),
            End::Sending(s, source) => s.feed(bytes, now, source),
        }
    }
    fn tick(&mut self, now: Instant) -> Vec<u8> {
        match self {
            End::Receiving(r, store) => r.tick(now, store),
            End::Sending(s, _) => s.tick(now),
        }
    }
    fn status(&self) -> &Status {
        match self {
            End::Receiving(r, _) => r.status(),
            End::Sending(s, _) => s.status(),
        }
    }
    fn files(&self) -> u32 {
        match self {
            End::Receiving(r, _) => r.progress().files,
            End::Sending(s, _) => s.progress().files,
        }
    }
}

fn run(command: Command, mut transport: Box<dyn Transport>) -> Result<bool, String> {
    let mut writer = transport.writer().map_err(|e| e.to_string())?;
    let verbose = command.verbose;
    let mut send = |bytes: &[u8]| -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        if verbose {
            eprintln!("> {}", visible(bytes));
        }
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .map_err(|e| format!("cannot send: {e}"))
    };
    let now = Instant::now();
    let mut end = match command.direction {
        Direction::Receive { into } => {
            if !into.is_dir() {
                return Err(format!("{}: not a directory", into.display()));
            }
            End::Receiving(Receiver::new(command.settings, now), Received::new(into))
        }
        Direction::Send { files } => {
            let mut sender = Sender::new(command.settings);
            send(&sender.start(now))?;
            End::Sending(sender, Files::new(files))
        }
    };

    let mut buf = vec![0u8; 4096];
    while *end.status() == Status::Running {
        let n = match transport.read_timeout(&mut buf, Duration::from_millis(100)) {
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                // Whatever the far end said last has been heard; if the
                // transfer is not over, it never will be.
                if *end.status() == Status::Running {
                    return Err("the connection closed part way through".into());
                }
                break;
            }
            Err(e) => return Err(e.to_string()),
        };
        let now = Instant::now();
        if n > 0 {
            if verbose {
                eprintln!("< {}", visible(&buf[..n]));
            }
            let out = end.feed(&buf[..n], now);
            send(&out)?;
        }
        let out = end.tick(now);
        send(&out)?;
    }

    // A receiver's last answer may be lost, and the sender will ask again;
    // stay a moment to answer it, which costs nothing if nothing comes.
    if matches!(end, End::Receiving(..)) && *end.status() == Status::Done {
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            match transport.read_timeout(&mut buf, Duration::from_millis(100)) {
                Ok(n) if n > 0 => {
                    let out = end.feed(&buf[..n], Instant::now());
                    send(&out)?;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }

    let files = end.files();
    let plural = if files == 1 { "" } else { "s" };
    match end.status() {
        Status::Done => {
            eprintln!("{files} file{plural} transferred");
            Ok(true)
        }
        Status::Cancelled => {
            eprintln!("cancelled after {files} file{plural}");
            Ok(false)
        }
        Status::Failed(why) => {
            eprintln!("vt-headless: {why} ({files} file{plural} transferred)");
            Ok(false)
        }
        Status::Running => unreachable!("the loop runs until it is not"),
    }
}

/// Bytes on the line, with control characters shown as `^A` and the like.
fn visible(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        match b {
            0x00..=0x1f => {
                out.push('^');
                out.push(char::from(b + 0x40));
            }
            0x7f => out.push_str("^?"),
            0x80.. => out.push_str(&format!("\\x{b:02x}")),
            _ => out.push(char::from(b)),
        }
    }
    out
}

/// Received files, written into one directory.
///
/// A file is created under its own name only if nothing has that name: an
/// existing file is never overwritten. The next free of `login.1.com`,
/// `login.2.com` and so on is used instead. A file that does not arrive
/// complete is removed.
struct Received {
    dir: PathBuf,
    open: Option<(File, PathBuf)>,
}

impl Received {
    fn new(dir: PathBuf) -> Received {
        Received { dir, open: None }
    }
}

/// `login.com` as `login.N.com`, or `readme` as `readme.N`.
fn numbered(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}.{n}.{ext}"),
        _ => format!("{name}.{n}"),
    }
}

fn create_new(dir: &Path, name: &str) -> io::Result<(File, PathBuf)> {
    for n in 0..1000 {
        let candidate = if n == 0 {
            name.to_string()
        } else {
            numbered(name, n)
        };
        let path = dir.join(&candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        format!("{name} and a thousand numbered copies of it already exist"),
    ))
}

impl Store for Received {
    fn create(&mut self, name: &str) -> Result<(), String> {
        let (file, path) = create_new(&self.dir, name).map_err(|e| format!("{name}: {e}"))?;
        eprintln!("receiving {}", path.display());
        self.open = Some((file, path));
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        let (file, path) = self.open.as_mut().ok_or("no file is open")?;
        file.write_all(data)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    fn finish(&mut self, complete: bool) -> Result<(), String> {
        let Some((file, path)) = self.open.take() else {
            return Ok(());
        };
        if complete {
            file.sync_all()
                .map_err(|e| format!("{}: {e}", path.display()))
        } else {
            drop(file);
            eprintln!("{}: incomplete, removed", path.display());
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
        }
    }
}

/// Files to send, one after another.
struct Files {
    paths: VecDeque<PathBuf>,
    open: Option<File>,
    size: Option<u64>,
}

impl Files {
    fn new(paths: Vec<PathBuf>) -> Files {
        Files {
            paths: paths.into(),
            open: None,
            size: None,
        }
    }
}

impl Source for Files {
    fn next_file(&mut self) -> Result<Option<String>, String> {
        let Some(path) = self.paths.pop_front() else {
            self.open = None;
            return Ok(None);
        };
        let file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.size = file.metadata().ok().map(|m| m.len());
        self.open = Some(file);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{}: no file name", path.display()))?;
        eprintln!("sending {}", path.display());
        Ok(Some(name))
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let file = self.open.as_mut().ok_or("no file is open")?;
        file.read(buf).map_err(|e| e.to_string())
    }

    fn size(&self) -> Option<u64> {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_name_gets_a_number_and_keeps_its_type() {
        assert_eq!(numbered("login.com", 1), "login.1.com");
        assert_eq!(numbered("readme", 2), "readme.2");
        assert_eq!(numbered(".profile", 1), ".profile.1");
        assert_eq!(numbered("a.tar.gz", 3), "a.tar.3.gz");
    }

    #[test]
    fn nothing_is_overwritten() {
        let dir = std::env::temp_dir().join(format!("vt-headless-kermit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("login.com"), b"keep me").unwrap();
        let mut store = Received::new(dir.clone());
        store.create("login.com").unwrap();
        store.write(b"new").unwrap();
        store.finish(true).unwrap();
        assert_eq!(std::fs::read(dir.join("login.com")).unwrap(), b"keep me");
        assert_eq!(std::fs::read(dir.join("login.1.com")).unwrap(), b"new");

        store.create("partial.dat").unwrap();
        store.write(b"half").unwrap();
        store.finish(false).unwrap();
        assert!(
            !dir.join("partial.dat").exists(),
            "an incomplete file is removed"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn options_are_read() {
        let args = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        let c = parse(&args(
            "receive --into /tmp --serial /dev/ttyUSB0 -b 19200 -p e -d 7 --binary --check 3",
        ))
        .unwrap();
        let Connection::Serial(serial) = c.connection else {
            panic!()
        };
        assert_eq!((serial.baud, serial.data_bits), (19200, 7));
        assert_eq!(c.settings.mode, Mode::Binary);
        assert_eq!(c.settings.params.check, Check::Three);
        assert!(parse(&args("receive")).is_err(), "no connection");
        assert!(parse(&args("receive --telnet a --ssh b")).is_err(), "two");
        assert!(
            parse(&args("receive --telnet a -b 9600")).is_err(),
            "line settings"
        );
        assert!(parse(&args("send --telnet a")).is_err(), "nothing to send");
        assert!(parse(&args("receive --telnet a --check 4")).is_err());
    }
}
