//! `vt-headless kermit`: a Kermit transfer over any connection veetee has,
//! with no window.
//!
//! It is a tool in its own right — getting a file off an OpenVMS system over a
//! serial console from a script — and it is how veetee's Kermit is tested
//! against a real one: `gkermit` refuses to run on a pipe but runs happily on
//! a pty, which is exactly what `--command` gives it.

use std::io::{self, ErrorKind, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use vt_kermit::files::{Folder, Paths, Transfer};
use vt_kermit::{Check, Mode, Names, Params, Settings, Status, ours};
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
    let mut transfer = match command.direction {
        Direction::Receive { into } => {
            if !into.is_dir() {
                return Err(format!("{}: not a directory", into.display()));
            }
            Transfer::receive(Folder::new(into), command.settings, now)
        }
        Direction::Send { files } => {
            let (transfer, first) = Transfer::send(Paths::new(files), command.settings, now);
            send(&first)?;
            transfer
        }
    };

    let mut buf = vec![0u8; 4096];
    let mut announced: Option<String> = None;
    // Past the end of the transfer itself, a receiver stays a moment to
    // answer the sender asking again for the end of the batch.
    while transfer.wants_line(Instant::now()) {
        let n = match transport.read_timeout(&mut buf, Duration::from_millis(100)) {
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                // Whatever the far end said last has been heard; if the
                // transfer is not over, it never will be.
                if *transfer.status() == Status::Running {
                    return Err("the connection closed part way through".into());
                }
                break;
            }
            Err(e) => return Err(e.to_string()),
        };
        if verbose && n > 0 {
            eprintln!("< {}", visible(&buf[..n]));
        }
        let out = transfer.feed(&buf[..n], Instant::now());
        send(&out)?;
        let file = &transfer.progress().file;
        if *file != announced {
            if let Some(name) = file {
                let doing = if transfer.is_receiving() {
                    "receiving"
                } else {
                    "sending"
                };
                eprintln!("{doing} {name}");
            }
            announced.clone_from(file);
        }
    }

    for path in transfer.saved() {
        eprintln!("saved {}", path.display());
    }
    let files = transfer.progress().files;
    let plural = if files == 1 { "" } else { "s" };
    match transfer.status() {
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

#[cfg(test)]
mod tests {
    use super::*;

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
