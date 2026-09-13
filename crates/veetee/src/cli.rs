//! Command-line options and opening the requested connection.

use std::io;
use std::path::PathBuf;

use vt_core::{Config, Model};
use vt_transport::Transport;
use vt_transport::pty::Pty;
use vt_transport::serial::{Serial, SerialConfig};
use vt_transport::ssh::{self, SshConfig};
use vt_transport::telnet::{Telnet, TelnetConfig};

pub const USAGE: &str = "\
usage: veetee [--model MODEL] [--record FILE] [CONNECTION]

connections (default: your login shell):
  --telnet HOST[:PORT]   Telnet, e.g. --telnet vms1 or telnet://vms1:2323
  --ssh [USER@]HOST      SSH via OpenSSH (uses ~/.ssh/config), e.g. ssh://system@vms1
  --serial DEVICE        serial line, e.g. --serial /dev/ttyUSB0
  --command COMMAND      run COMMAND (via /bin/sh -c)

options:
  --model MODEL          vt100 vt102 vt220 vt320 vt420 (default) vt510 vt520 vt525
  --port PORT            TCP port for --telnet or --ssh
  --record FILE          append everything the host sends to FILE
  --sessions N           open 1 or 2 sessions (2 splits the window, F4 switches)

serial line options (picocom style; defaults are DEC factory Set-Up):
  -b, --baud RATE        bits per second (9600)
  -d, --databits N       5, 6, 7 or 8 (8)
  -p, --parity P         n none, e even, o odd, m mark, s space (n)
  -s, --stopbits N       1 or 2 (1)
  -f, --flow F           x XON/XOFF, h RTS/CTS, n none (x)";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Connection {
    #[default]
    Shell,
    Command(String),
    Serial(SerialConfig),
    Telnet {
        host: String,
        port: u16,
    },
    Ssh(SshConfig),
}

impl Connection {
    /// Network and serial sessions keep their window open when the line
    /// drops, so the screen can still be read (and copied).
    pub fn keep_open_on_close(&self) -> bool {
        matches!(
            self,
            Connection::Serial(_) | Connection::Telnet { .. } | Connection::Ssh(_)
        )
    }

    /// Short label for the window subtitle.
    pub fn label(&self) -> String {
        match self {
            Connection::Shell => "Local shell".into(),
            Connection::Command(cmd) => cmd.clone(),
            Connection::Serial(s) => s.to_string(),
            Connection::Telnet { host, port: 23 } => format!("telnet {host}"),
            Connection::Telnet { host, port } => format!("telnet {host}:{port}"),
            Connection::Ssh(s) => match s.port {
                Some(port) => format!("ssh {}:{port}", s.destination),
                None => format!("ssh {}", s.destination),
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub connection: Connection,
    pub record: Option<PathBuf>,
    /// Sessions to open at start: 1, or 2 for a split window.
    pub sessions: u8,
}

pub enum Parsed {
    Run(Config, Options),
    Help,
}

pub fn parse_args(args: impl Iterator<Item = String>) -> Result<Parsed, String> {
    let mut config = Config::default();
    let mut options = Options {
        sessions: 1,
        ..Options::default()
    };
    let mut line: Vec<(String, String)> = Vec::new();
    let mut port: Option<u16> = None;
    let mut chosen = 0;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        let connection = match arg.as_str() {
            "--model" => {
                let v = value()?;
                config.model = parse_model(&v).ok_or_else(|| format!("unknown model {v:?}"))?;
                None
            }
            "--record" => {
                options.record = Some(value()?.into());
                None
            }
            "--sessions" => {
                let v = value()?;
                options.sessions = match v.as_str() {
                    "1" => 1,
                    "2" => 2,
                    _ => return Err(format!("--sessions: 1 or 2, not {v:?}")),
                };
                None
            }
            "--port" => {
                let v = value()?;
                port = Some(
                    v.parse()
                        .map_err(|_| format!("--port: not a port number: {v:?}"))?,
                );
                None
            }
            "--command" => Some(Connection::Command(value()?)),
            "--serial" => Some(Connection::Serial(SerialConfig::new(value()?))),
            "--telnet" => Some(telnet(&value()?)?),
            "--ssh" => Some(ssh(&value()?)?),
            "-b" | "--baud" | "-d" | "--databits" | "-p" | "--parity" | "-s" | "--stopbits"
            | "-f" | "--flow" => {
                let v = value()?;
                line.push((arg, v));
                None
            }
            "-h" | "--help" => return Ok(Parsed::Help),
            url if url.starts_with("telnet://") => Some(telnet(
                url.trim_start_matches("telnet://").trim_end_matches('/'),
            )?),
            url if url.starts_with("ssh://") => {
                Some(ssh(url.trim_start_matches("ssh://").trim_end_matches('/'))?)
            }
            _ => return Err(format!("unexpected argument {arg:?}")),
        };
        if let Some(c) = connection {
            chosen += 1;
            options.connection = c;
        }
    }
    if chosen > 1 {
        return Err("choose one connection: --telnet, --ssh, --serial or --command".into());
    }

    if let Some(p) = port {
        match &mut options.connection {
            Connection::Telnet { port, .. } => *port = p,
            Connection::Ssh(s) => s.port = Some(p),
            _ => return Err("--port applies to --telnet and --ssh".into()),
        }
    }

    match &mut options.connection {
        Connection::Serial(serial) => {
            for (flag, v) in line {
                let number = |v: &str| {
                    v.parse::<u32>()
                        .map_err(|_| format!("{flag}: not a number: {v:?}"))
                };
                match flag.as_str() {
                    "-b" | "--baud" => serial.baud = number(&v)?,
                    "-d" | "--databits" => serial.data_bits = number(&v)? as u8,
                    "-p" | "--parity" => serial.parity = v.parse()?,
                    "-s" | "--stopbits" => serial.stop_bits = number(&v)? as u8,
                    _ => serial.flow = v.parse()?,
                }
            }
        }
        _ if !line.is_empty() => return Err("line options need --serial DEVICE".into()),
        _ => {}
    }
    Ok(Parsed::Run(config, options))
}

/// `HOST`, `HOST:PORT` or `[IPv6]:PORT`.
fn split_host_port(spec: &str) -> Result<(String, Option<u16>), String> {
    let bad_port = |p: &str| format!("not a port number: {p:?}");
    if let Some(rest) = spec.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| format!("unterminated [ in {spec:?}"))?;
        return match after.strip_prefix(':') {
            Some(p) => Ok((host.into(), Some(p.parse().map_err(|_| bad_port(p))?))),
            None if after.is_empty() => Ok((host.into(), None)),
            None => Err(format!("unexpected {after:?} after address")),
        };
    }
    match spec.rsplit_once(':') {
        // A bare IPv6 address has several colons and no port.
        Some((host, p)) if !host.contains(':') => {
            Ok((host.into(), Some(p.parse().map_err(|_| bad_port(p))?)))
        }
        _ => Ok((spec.into(), None)),
    }
}

fn telnet(spec: &str) -> Result<Connection, String> {
    let (host, port) = split_host_port(spec)?;
    if host.is_empty() {
        return Err("--telnet needs a host name".into());
    }
    Ok(Connection::Telnet {
        host,
        port: port.unwrap_or(23),
    })
}

fn ssh(spec: &str) -> Result<Connection, String> {
    let (destination, port) = split_host_port(spec)?;
    if destination.is_empty() || destination.starts_with('-') {
        return Err(format!("not an SSH destination: {spec:?}"));
    }
    Ok(Connection::Ssh(SshConfig { destination, port }))
}

pub fn parse_model(name: &str) -> Option<Model> {
    Some(match name.to_ascii_lowercase().as_str() {
        "vt100" => Model::Vt100,
        "vt102" => Model::Vt102,
        "vt220" => Model::Vt220,
        "vt320" => Model::Vt320,
        "vt420" => Model::Vt420,
        "vt510" => Model::Vt510,
        "vt520" => Model::Vt520,
        "vt525" => Model::Vt525,
        _ => return None,
    })
}

pub fn model_name(model: Model) -> &'static str {
    match model {
        Model::Vt100 => "VT100",
        Model::Vt102 => "VT102",
        Model::Vt220 => "VT220",
        Model::Vt320 => "VT320",
        Model::Vt420 => "VT420",
        Model::Vt510 => "VT510",
        Model::Vt520 => "VT520",
        Model::Vt525 => "VT525",
    }
}

/// Opens the connection. May block (DNS, TCP connect), so call it off the UI thread.
pub fn open_transport(config: &Config, connection: &Connection) -> io::Result<Box<dyn Transport>> {
    let (rows, cols, term) = (
        config.rows as u16,
        config.cols as u16,
        config.model.term_name(),
    );
    Ok(match connection {
        Connection::Shell => {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            Box::new(Pty::spawn::<&str>(&shell, &[], rows, cols, term)?)
        }
        Connection::Command(cmd) => {
            Box::new(Pty::spawn("/bin/sh", &["-c", cmd], rows, cols, term)?)
        }
        Connection::Serial(serial) => Box::new(Serial::open(serial.clone())?),
        Connection::Telnet { host, port } => Box::new(Telnet::connect(TelnetConfig {
            port: *port,
            rows,
            cols,
            // Telnet terminal types are conventionally upper case (RFC 1091).
            ..TelnetConfig::new(host.clone(), model_name(config.model))
        })?),
        Connection::Ssh(s) => Box::new(ssh::connect(s, rows, cols, term)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_transport::serial::{FlowControl, Parity};

    fn parse(args: &[&str]) -> Result<Options, String> {
        match parse_args(args.iter().map(|a| a.to_string()))? {
            Parsed::Run(_, o) => Ok(o),
            Parsed::Help => Err("help".into()),
        }
    }

    #[test]
    fn picocom_style_serial_options() {
        let o = parse(&[
            "--serial",
            "/dev/ttyUSB0",
            "-b",
            "19200",
            "-d",
            "7",
            "-p",
            "e",
            "-s",
            "2",
            "-f",
            "n",
        ])
        .unwrap();
        let Connection::Serial(s) = o.connection else {
            panic!()
        };
        assert_eq!(
            (s.baud, s.data_bits, s.parity, s.stop_bits, s.flow),
            (19200, 7, Parity::Even, 2, FlowControl::None)
        );
    }

    #[test]
    fn serial_defaults_are_dec_factory_settings() {
        let o = parse(&["--serial", "/dev/ttyS0"]).unwrap();
        assert_eq!(o.connection.label(), "/dev/ttyS0 9600 8N1");
    }

    #[test]
    fn telnet_forms() {
        assert_eq!(
            parse(&["--telnet", "vms1"]).unwrap().connection,
            Connection::Telnet {
                host: "vms1".into(),
                port: 23
            }
        );
        assert_eq!(
            parse(&["--telnet", "192.168.0.156:2323"])
                .unwrap()
                .connection
                .label(),
            "telnet 192.168.0.156:2323"
        );
        assert_eq!(
            parse(&["telnet://vms1:24/"]).unwrap().connection,
            Connection::Telnet {
                host: "vms1".into(),
                port: 24
            }
        );
        assert_eq!(
            parse(&["--telnet", "vms1", "--port", "2001"])
                .unwrap()
                .connection,
            Connection::Telnet {
                host: "vms1".into(),
                port: 2001
            }
        );
        assert_eq!(
            parse(&["--telnet", "[fe80::1]:23"]).unwrap().connection,
            Connection::Telnet {
                host: "fe80::1".into(),
                port: 23
            }
        );
        assert_eq!(
            parse(&["--telnet", "fe80::1"]).unwrap().connection,
            Connection::Telnet {
                host: "fe80::1".into(),
                port: 23
            }
        );
    }

    #[test]
    fn ssh_forms() {
        let o = parse(&["--ssh", "system@vms1"]).unwrap();
        assert_eq!(o.connection.label(), "ssh system@vms1");
        let o = parse(&["ssh://system@vms1:2222"]).unwrap();
        assert_eq!(
            o.connection,
            Connection::Ssh(SshConfig {
                destination: "system@vms1".into(),
                port: Some(2222)
            })
        );
        assert!(parse(&["--ssh", "-oProxyCommand=x"]).is_err());
    }

    #[test]
    fn rejects_conflicting_or_orphaned_options() {
        assert!(parse(&["--serial", "/dev/ttyS0", "--command", "sh"]).is_err());
        assert!(parse(&["--telnet", "a", "--ssh", "b"]).is_err());
        assert!(parse(&["-b", "9600"]).is_err());
        assert!(parse(&["--port", "23"]).is_err());
        assert!(parse(&["--serial", "/dev/ttyS0", "-p", "q"]).is_err());
    }
}
