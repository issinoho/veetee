//! Command-line options and opening the requested connection.

use std::io;

use vt_core::{Config, Model};
use vt_transport::Transport;
use vt_transport::pty::Pty;
use vt_transport::serial::{FlowControl, Parity, Serial, SerialConfig};
use vt_transport::ssh::{self, SshConfig};
use vt_transport::telnet::{ComPort, Telnet, TelnetConfig};

pub const USAGE: &str = "\
usage: veetee [--profile NAME] [--model MODEL] [--record FILE] [CONNECTION]

connections (default: your login shell):
  --profile NAME         a saved connection (options given with it override it)
  --list-profiles        list the saved connections
  --telnet HOST[:PORT]   Telnet, e.g. --telnet vms1 or telnet://vms1:2323
  --ssh [USER@]HOST      SSH via OpenSSH (uses ~/.ssh/config), e.g. ssh://system@vms1
  --serial DEVICE        serial line, e.g. --serial /dev/ttyUSB0 or --serial COM3
  --lat NODE             LAT, DEC's own protocol, on the local segment (Linux only)
  --command COMMAND      run COMMAND through the system shell (sh -c, or cmd /C)

options:
  --model MODEL          vt100 vt102 vt220 vt320 vt420 (default) vt510 vt520 vt525
  --port PORT            TCP port for --telnet or --ssh
  --interface NAME       the interface --lat speaks on; needed only where more
                         than one Ethernet interface is up
  --service NAME         the LAT service to ask for (default: the node's own name)
  --telnet-binary        negotiate 8-bit Telnet (BINARY). OpenVMS answers it by
                         setting the terminal PASSALL, where DELETE stops erasing
  --record FILE          record the session to FILE (.vtrec) for replay and tests;
                         Ctrl+Shift+M marks a checkpoint
  --record-keys          also record typed keys (includes passwords)
  --log FILE             add the session's text to FILE (~ and %Y %m %d %H %M %S
                         are expanded)
  --log-timestamps       start each logged line with the date and time
  --log-raw              log the host's bytes as received instead of the text
  --sessions N           open 1 or 2 sessions (2 splits the window, F4 switches)
  --phosphor COLOUR      white (P4, default), green (P1) or amber (P3)
  --keymap FILE          PC-to-DEC keymap (default: the one saved from the Keyboard
                         Map window, else the built-in LK401 map)

line options (picocom style; defaults are DEC factory Set-Up). With --serial they set
the port; with --telnet they ask a terminal server for those settings (RFC 2217):
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
        /// Negotiate the Telnet BINARY option, for 8-bit controls.
        binary: bool,
        /// Line settings to ask a terminal server for (RFC 2217), from the
        /// serial line options given with `--telnet`.
        com_port: Option<ComPort>,
    },
    Ssh(SshConfig),
    /// A LAT service on a node, over raw Ethernet. Linux only.
    Lat {
        /// The interface to speak on. LAT does not route, so it has to be the
        /// one sharing a segment with the node; when it is not given, and
        /// there is only one it could be, that one is used.
        interface: Option<String>,
        /// The node, as it announces itself.
        node: String,
        /// The service wanted, the node's own name when it is not given.
        service: Option<String>,
    },
}

impl Connection {
    /// Network and serial sessions keep their window open when the line
    /// drops, so the screen can still be read (and copied).
    pub fn keep_open_on_close(&self) -> bool {
        matches!(
            self,
            Connection::Serial(_)
                | Connection::Telnet { .. }
                | Connection::Ssh(_)
                | Connection::Lat { .. }
        )
    }

    /// Short label for the window subtitle.
    pub fn label(&self) -> String {
        match self {
            Connection::Shell => "Local shell".into(),
            Connection::Command(cmd) => cmd.clone(),
            Connection::Serial(s) => s.to_string(),
            Connection::Telnet { host, port: 23, .. } => format!("telnet {host}"),
            Connection::Telnet { host, port, .. } => format!("telnet {host}:{port}"),
            Connection::Ssh(s) => match s.port {
                Some(port) => format!("ssh {}:{port}", s.destination),
                None => format!("ssh {}", s.destination),
            },
            Connection::Lat { node, service, .. } => match service {
                Some(service) if service != node => format!("lat {node}/{service}"),
                _ => format!("lat {node}"),
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub connection: Connection,
    pub record: Option<crate::session::RecordOptions>,
    /// Sessions to open at start: 1, or 2 for a split window.
    pub sessions: u8,
    /// Phosphor colour: "white", "green" or "amber".
    pub phosphor: String,
    /// Keymap file to use instead of the saved or built-in keymap.
    pub keymap: Option<std::path::PathBuf>,
    /// The saved connection the window was opened from.
    pub profile: Option<String>,
    /// Log the first session to a file.
    pub log: Option<crate::log::LogOptions>,
}

// Parsed once at start-up, so the size difference does not matter.
#[allow(clippy::large_enum_variant)]
pub enum Parsed {
    Run(Config, Options),
    Help,
    ListProfiles,
}

pub fn parse_args(args: impl Iterator<Item = String>) -> Result<Parsed, String> {
    parse_args_with(args, crate::profiles::find)
}

/// Parses the arguments, looking up `--profile` with `find`.
pub fn parse_args_with(
    args: impl Iterator<Item = String>,
    find: impl Fn(&str) -> Result<crate::profiles::Profile, String>,
) -> Result<Parsed, String> {
    let mut config = Config::default();
    let mut options = Options {
        sessions: 1,
        phosphor: "white".into(),
        ..Options::default()
    };
    // A profile sets the starting point; the other options change it.
    let args: Vec<String> = args.collect();
    let mut rest = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--profile" {
            let name = iter.next().ok_or("--profile needs a value")?;
            find(&name)?.apply(&mut config, &mut options);
        } else {
            rest.push(arg);
        }
    }
    let mut line: Vec<(String, String)> = Vec::new();
    let mut port: Option<u16> = None;
    // LAT is named a piece at a time, as a serial line is, and the pieces can
    // come in any order.
    let mut interface: Option<String> = None;
    let mut service: Option<String> = None;
    let mut chosen = 0;
    let mut record_keys = false;
    let mut telnet_binary = false;
    let (mut log_timestamps, mut log_raw) = (false, false);
    let mut args = rest.into_iter().peekable();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        let connection = match arg.as_str() {
            "--model" => {
                let v = value()?;
                config.model = parse_model(&v).ok_or_else(|| format!("unknown model {v:?}"))?;
                None
            }
            "--record" => {
                let keys = options.record.as_ref().is_some_and(|r| r.keys);
                options.record = Some(crate::session::RecordOptions {
                    path: value()?.into(),
                    keys,
                });
                None
            }
            "--record-keys" => {
                record_keys = true;
                None
            }
            "--log" => {
                let path = crate::log::expand_path(&value()?);
                options.log = Some(crate::log::LogOptions {
                    path,
                    raw: false,
                    timestamps: false,
                    append: true,
                });
                None
            }
            "--log-timestamps" => {
                log_timestamps = true;
                None
            }
            "--log-raw" => {
                log_raw = true;
                None
            }
            "--keymap" => {
                options.keymap = Some(value()?.into());
                None
            }
            "--phosphor" => {
                let v = value()?;
                if !matches!(v.as_str(), "white" | "green" | "amber") {
                    return Err(format!("--phosphor: white, green or amber, not {v:?}"));
                }
                options.phosphor = v;
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
            "--telnet-binary" => {
                telnet_binary = true;
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
            "--lat" => Some(Connection::Lat {
                interface: None,
                node: value()?,
                service: None,
            }),
            "--interface" => {
                interface = Some(value()?);
                None
            }
            "--service" => {
                service = Some(value()?);
                None
            }
            "--ssh" => Some(ssh(&value()?)?),
            "-b" | "--baud" | "-d" | "--databits" | "-p" | "--parity" | "-s" | "--stopbits"
            | "-f" | "--flow" => {
                let v = value()?;
                line.push((arg, v));
                None
            }
            "-h" | "--help" => return Ok(Parsed::Help),
            "--list-profiles" => return Ok(Parsed::ListProfiles),
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
    if record_keys {
        match options.record.as_mut() {
            Some(r) => r.keys = true,
            None => return Err("--record-keys needs --record FILE".into()),
        }
    }
    if log_timestamps || log_raw {
        match options.log.as_mut() {
            Some(log) => {
                log.timestamps |= log_timestamps;
                log.raw |= log_raw;
            }
            None => return Err("--log-timestamps and --log-raw need --log FILE".into()),
        }
    }
    if chosen > 1 {
        return Err("choose one connection: --telnet, --ssh, --serial, --lat or --command".into());
    }

    if interface.is_some() || service.is_some() {
        match &mut options.connection {
            Connection::Lat {
                interface: i,
                service: s,
                ..
            } => {
                if interface.is_some() {
                    *i = interface;
                }
                if service.is_some() {
                    *s = service;
                }
            }
            _ => return Err("--interface and --service apply to --lat".into()),
        }
    }

    if let Some(p) = port {
        match &mut options.connection {
            Connection::Telnet { port, .. } => *port = p,
            Connection::Ssh(s) => s.port = Some(p),
            _ => return Err("--port applies to --telnet and --ssh".into()),
        }
    }

    if telnet_binary {
        match &mut options.connection {
            Connection::Telnet { binary, .. } => *binary = true,
            _ => return Err("--telnet-binary applies to --telnet".into()),
        }
    }

    match &mut options.connection {
        Connection::Serial(serial) => apply_line(
            line,
            &mut serial.baud,
            &mut serial.data_bits,
            &mut serial.parity,
            &mut serial.stop_bits,
            &mut serial.flow,
        )?,
        // The same options over Telnet ask a terminal server for the line
        // settings with RFC 2217.
        Connection::Telnet { com_port, .. } if !line.is_empty() => {
            let port = com_port.get_or_insert_with(ComPort::default);
            apply_line(
                line,
                &mut port.baud,
                &mut port.data_bits,
                &mut port.parity,
                &mut port.stop_bits,
                &mut port.flow,
            )?;
        }
        _ if !line.is_empty() => {
            return Err("line options need --serial DEVICE or --telnet HOST".into());
        }
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

/// Applies the picocom-style line options to a serial port or, over Telnet,
/// to the settings asked of a terminal server.
fn apply_line(
    line: Vec<(String, String)>,
    baud: &mut u32,
    data_bits: &mut u8,
    parity: &mut Parity,
    stop_bits: &mut u8,
    flow: &mut FlowControl,
) -> Result<(), String> {
    for (flag, v) in line {
        let number = |v: &str| {
            v.parse::<u32>()
                .map_err(|_| format!("{flag}: not a number: {v:?}"))
        };
        match flag.as_str() {
            "-b" | "--baud" => *baud = number(&v)?,
            "-d" | "--databits" => *data_bits = number(&v)? as u8,
            "-p" | "--parity" => *parity = v.parse()?,
            "-s" | "--stopbits" => *stop_bits = number(&v)? as u8,
            _ => *flow = v.parse()?,
        }
    }
    Ok(())
}

fn telnet(spec: &str) -> Result<Connection, String> {
    let (host, port) = split_host_port(spec)?;
    if host.is_empty() {
        return Err("--telnet needs a host name".into());
    }
    Ok(Connection::Telnet {
        host,
        port: port.unwrap_or(23),
        binary: false,
        com_port: None,
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

#[cfg(unix)]
fn login_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

#[cfg(windows)]
fn login_shell() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
}

/// The shell that runs `--command`, and its flag for a command string.
#[cfg(unix)]
fn command_shell() -> (String, &'static str) {
    ("/bin/sh".into(), "-c")
}

#[cfg(windows)]
fn command_shell() -> (String, &'static str) {
    (login_shell(), "/C")
}

/// Opens the connection. May block (DNS, TCP connect), so call it off the UI thread.
pub fn open_transport(config: &Config, connection: &Connection) -> io::Result<Box<dyn Transport>> {
    let (rows, cols, term) = (
        config.rows as u16,
        config.cols as u16,
        config.model.term_name(),
    );
    Ok(match connection {
        // In Flatpak the shell runs on the host: the user's shell from the
        // host's password database, since the sandbox has no $SHELL.
        Connection::Shell if vt_transport::pty::in_flatpak() => Box::new(Pty::spawn(
            "sh",
            &[
                "-c",
                r#"s=$(getent passwd "$(id -un)" | cut -d: -f7); exec "${s:-/bin/sh}""#,
            ],
            rows,
            cols,
            term,
        )?),
        Connection::Shell => Box::new(Pty::spawn::<&str>(&login_shell(), &[], rows, cols, term)?),
        Connection::Command(cmd) => {
            let (shell, flag) = command_shell();
            Box::new(Pty::spawn(&shell, &[flag, cmd.as_str()], rows, cols, term)?)
        }
        Connection::Serial(serial) => Box::new(Serial::open(serial.clone())?),
        Connection::Telnet {
            host,
            port,
            binary,
            com_port,
        } => Box::new(Telnet::connect(TelnetConfig {
            port: *port,
            rows,
            cols,
            binary: *binary,
            com_port: *com_port,
            // Telnet terminal types are conventionally upper case (RFC 1091).
            ..TelnetConfig::new(host.clone(), model_name(config.model))
        })?),
        Connection::Ssh(s) => Box::new(ssh::connect(s, rows, cols, term)?),
        #[cfg(target_os = "linux")]
        Connection::Lat {
            interface,
            node,
            service,
        } => Box::new(vt_transport::lat::Lat::connect(
            vt_transport::lat::LatConfig {
                interface: lat_interface(interface.as_deref())?,
                node: node.clone(),
                service: service.clone(),
                address: None,
                from: None,
                rows,
                cols,
            },
        )?),
        #[cfg(not(target_os = "linux"))]
        Connection::Lat { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "LAT needs raw Ethernet, which veetee only has on Linux",
            ));
        }
    })
}

/// The interface to speak LAT on: the one named, or the only one it could be.
///
/// LAT does not route, so this is not a matter of picking a default: a node is
/// reachable on the segment it is on and nowhere else. Choosing for the user
/// is only safe when there is nothing to choose between.
#[cfg(target_os = "linux")]
pub fn lat_interface(named: Option<&str>) -> io::Result<String> {
    if let Some(named) = named {
        return Ok(named.to_string());
    }
    choose_interface(&vt_transport::lat::interfaces())
}

/// Which of the interfaces that are up LAT should speak on.
#[cfg(target_os = "linux")]
fn choose_interface(found: &[vt_transport::lat::Interface]) -> io::Result<String> {
    // LAT over wireless works — a session to OpenVMS has been run over it —
    // but where there is a wire as well, the wire is the likelier segment to
    // find DEC equipment on. So a wireless interface is not weighed against a
    // wired one; it is only passed over when there is something else to take.
    let wired: Vec<&str> = found
        .iter()
        .filter(|i| !i.wireless)
        .map(|i| i.name.as_str())
        .collect();
    if let [only] = wired[..] {
        return Ok(only.to_string());
    }
    // Nothing but wireless, and only one of it: there is no choice to make,
    // and refusing to make it would be obstruction rather than caution.
    if wired.is_empty()
        && let [only] = found
    {
        return Ok(only.name.clone());
    }
    if found.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no Ethernet interface is up for LAT to speak on",
        ));
    }
    let all: Vec<String> = found
        .iter()
        .map(|i| {
            if i.wireless {
                format!("{} (wireless)", i.name)
            } else {
                i.name.clone()
            }
        })
        .collect();
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "say which interface LAT should use, with --interface: {}",
            all.join(", ")
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_transport::serial::{FlowControl, Parity};

    fn parse(args: &[&str]) -> Result<Options, String> {
        parse_with_model(args).map(|(_, o)| o)
    }

    fn parse_with_model(args: &[&str]) -> Result<(Model, Options), String> {
        let profiles = crate::profiles::parse(
            "[[profile]]\nname = \"vms1\"\nconnection = \"telnet\"\nhost = \"vms1\"\nmodel = \"vt520\"\nphosphor = \"amber\"\n",
        )
        .unwrap();
        let find = |name: &str| {
            profiles
                .iter()
                .find(|p| p.name == name)
                .cloned()
                .ok_or_else(|| format!("no profile {name:?}"))
        };
        match parse_args_with(args.iter().map(|a| a.to_string()), find)? {
            Parsed::Run(c, o) => Ok((c.model, o)),
            _ => Err("not run".into()),
        }
    }

    #[test]
    fn profiles_are_a_starting_point() {
        let (model, o) = parse_with_model(&["--profile", "vms1"]).unwrap();
        assert_eq!((model, o.phosphor.as_str()), (Model::Vt520, "amber"));
        assert_eq!(o.connection.label(), "telnet vms1");
        let (model, o) =
            parse_with_model(&["--model", "vt420", "--profile", "vms1", "--port", "2323"]).unwrap();
        assert_eq!(model, Model::Vt420, "options override the profile");
        assert_eq!(o.connection.label(), "telnet vms1:2323");
        let o = parse(&["--profile", "vms1", "--ssh", "alpha"]).unwrap();
        assert_eq!(o.connection.label(), "ssh alpha");
        assert!(parse(&["--profile", "nope"]).is_err());
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

    #[cfg(target_os = "linux")]
    #[test]
    fn lat_takes_the_only_wired_interface_and_asks_about_the_rest() {
        use vt_transport::lat::Interface;
        let wired = |name: &str| Interface {
            name: name.into(),
            wireless: false,
        };
        let wireless = |name: &str| Interface {
            name: name.into(),
            wireless: true,
        };

        assert_eq!(
            choose_interface(&[wired("enp0s31f6")]).unwrap(),
            "enp0s31f6"
        );
        assert_eq!(
            choose_interface(&[wired("enp0s31f6"), wireless("wlp59s0")]).unwrap(),
            "enp0s31f6",
            "a LAT segment is a wired one, so wireless is not weighed against it"
        );

        // Two of the same kind is a choice nobody else can make.
        let e = choose_interface(&[wired("eth0"), wired("eth1")]).unwrap_err();
        assert!(
            e.to_string().contains("eth0") && e.to_string().contains("eth1"),
            "{e}"
        );

        // LAT over wireless works, so the only interface there is gets used
        // whatever kind it is: refusing would be obstruction, not caution.
        assert_eq!(choose_interface(&[wireless("wlp59s0")]).unwrap(), "wlp59s0");

        // Two of them is a choice again, and each is named for what it is.
        let e = choose_interface(&[wireless("wlan0"), wireless("wlan1")]).unwrap_err();
        assert!(e.to_string().contains("wlan0 (wireless)"), "{e}");

        let e = choose_interface(&[]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn lat_names_a_node_and_may_name_the_rest() {
        let o = parse(&["--lat", "MYI64"]).unwrap();
        assert_eq!(
            o.connection,
            Connection::Lat {
                interface: None,
                node: "MYI64".into(),
                service: None,
            },
            "the interface is worked out at the time, and the service is the node"
        );
        assert_eq!(o.connection.label(), "lat MYI64");

        let o = parse(&[
            "--lat",
            "MYI64",
            "--interface",
            "eth0",
            "--service",
            "TERMINALS",
        ])
        .unwrap();
        assert_eq!(
            o.connection,
            Connection::Lat {
                interface: Some("eth0".into()),
                node: "MYI64".into(),
                service: Some("TERMINALS".into()),
            }
        );
        assert_eq!(o.connection.label(), "lat MYI64/TERMINALS");
    }

    #[test]
    fn the_lat_options_belong_to_lat() {
        let e = parse(&["--telnet", "vms1", "--interface", "eth0"]).unwrap_err();
        assert!(e.contains("--interface"), "{e}");
        let e = parse(&["--service", "TERMINALS"]).unwrap_err();
        assert!(e.contains("--service"), "{e}");
    }

    #[test]
    fn line_options_over_telnet_ask_a_terminal_server() {
        let o = parse(&["--telnet", "decserver", "-b", "19200", "-p", "e", "-f", "h"]).unwrap();
        let Connection::Telnet { com_port, .. } = o.connection else {
            panic!()
        };
        let port = com_port.expect("line options over Telnet mean RFC 2217");
        assert_eq!(
            (port.baud, port.parity, port.flow),
            (19200, Parity::Even, FlowControl::RtsCts)
        );
        // The rest keep DEC's factory values.
        assert_eq!((port.data_bits, port.stop_bits), (8, 1));
    }

    #[test]
    fn telnet_without_line_options_does_not_ask_for_com_port_control() {
        let Connection::Telnet { com_port, .. } = parse(&["--telnet", "vms1"]).unwrap().connection
        else {
            panic!()
        };
        assert_eq!(com_port, None);
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
                port: 23,
                binary: false,
                com_port: None,
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
                port: 24,
                binary: false,
                com_port: None,
            }
        );
        assert_eq!(
            parse(&["--telnet", "vms1", "--port", "2001"])
                .unwrap()
                .connection,
            Connection::Telnet {
                host: "vms1".into(),
                port: 2001,
                binary: false,
                com_port: None,
            }
        );
        assert_eq!(
            parse(&["--telnet", "[fe80::1]:23"]).unwrap().connection,
            Connection::Telnet {
                host: "fe80::1".into(),
                port: 23,
                binary: false,
                com_port: None,
            }
        );
        assert_eq!(
            parse(&["--telnet", "fe80::1"]).unwrap().connection,
            Connection::Telnet {
                host: "fe80::1".into(),
                port: 23,
                binary: false,
                com_port: None,
            }
        );
    }

    #[test]
    fn telnet_binary_is_off_unless_asked_for() {
        assert_eq!(
            parse(&["--telnet", "vms1", "--telnet-binary"])
                .unwrap()
                .connection,
            Connection::Telnet {
                host: "vms1".into(),
                port: 23,
                binary: true,
                com_port: None,
            }
        );
        assert!(parse(&["--telnet-binary"]).is_err());
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
