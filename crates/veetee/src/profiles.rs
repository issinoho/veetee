//! Saved connections ("profiles"): a name, a connection, the terminal model
//! and the window's options, kept in `$XDG_CONFIG_HOME/veetee/profiles.toml`
//! so they can be opened from the Connections window or with
//! `veetee --profile NAME`.

use std::io;
use std::path::PathBuf;

use gtk::glib;
use serde::{Deserialize, Serialize};
use vt_core::{Config, Model};
use vt_transport::serial::{FlowControl, Parity, SerialConfig};
use vt_transport::ssh::SshConfig;
use vt_transport::telnet::ComPort;

use crate::cli::{self, Connection, Options};

/// A saved connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub model: Model,
    pub connection: Connection,
    /// "white", "green" or "amber".
    pub phosphor: String,
    /// 1, or 2 for a split window.
    pub sessions: u8,
    pub keymap: Option<PathBuf>,
    /// A log file template (see [`crate::log::expand_path`]).
    pub log: Option<String>,
    pub log_timestamps: bool,
    pub log_raw: bool,
    /// Opened when veetee starts without being told a connection. At most one
    /// saved connection has it; [`put`] and [`toggle_default`] keep it so.
    pub default: bool,
}

impl Profile {
    /// A profile for the connection and options a window was opened with.
    pub fn from_options(name: &str, config: &Config, options: &Options) -> Profile {
        Profile {
            name: name.into(),
            model: config.model,
            connection: options.connection.clone(),
            phosphor: options.phosphor.clone(),
            sessions: options.sessions.clamp(1, 2),
            keymap: options.keymap.clone(),
            log: None,
            log_timestamps: false,
            log_raw: false,
            default: false,
        }
    }

    /// Applies the profile to a configuration and window options.
    pub fn apply(&self, config: &mut Config, options: &mut Options) {
        config.model = self.model;
        options.connection = self.connection.clone();
        options.phosphor = self.phosphor.clone();
        options.sessions = self.sessions;
        options.keymap = self.keymap.clone();
        options.profile = Some(self.name.clone());
        options.log = self.log.as_deref().map(|template| crate::log::LogOptions {
            path: crate::log::expand_path(template),
            raw: self.log_raw,
            timestamps: self.log_timestamps,
            append: true,
        });
    }

    /// "VT420 · telnet vms1", for lists.
    pub fn summary(&self) -> String {
        format!(
            "{} · {}",
            cli::model_name(self.model),
            self.connection.label()
        )
    }
}

/// The file layout: one `[[profile]]` table per connection.
///
/// The default is named at the top of the file rather than marked in its
/// profile, so that an older veetee, which refuses a profile with a key it
/// does not know, still reads a file that has one.
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
struct File {
    /// The connection opened when veetee starts without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_profile: Option<String>,
    #[serde(default)]
    profile: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Entry {
    name: String,
    /// shell, command, telnet, ssh, serial or lat.
    connection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    telnet_binary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    /// LAT: the interface to speak on, when one was chosen, and the service
    /// when it is not the node's own name.
    #[serde(skip_serializing_if = "Option::is_none")]
    interface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baud: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_bits: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_bits: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phosphor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sessions: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keymap: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    log: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    log_timestamps: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    log_raw: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

impl Entry {
    fn to_profile(&self) -> Result<Profile, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("a profile has no name".into());
        }
        let context = |msg: String| format!("profile {name:?}: {msg}");
        let need = |field: &Option<String>, what: &str| {
            field
                .clone()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| context(format!("{} needs {what}", self.connection)))
        };
        let connection = match self.connection.as_str() {
            "shell" => Connection::Shell,
            "command" => Connection::Command(need(&self.command, "a command")?),
            "telnet" => Connection::Telnet {
                host: need(&self.host, "a host")?,
                port: self.port.unwrap_or(23),
                binary: self.telnet_binary.unwrap_or(false),
                com_port: com_port_from(
                    self.baud,
                    self.data_bits,
                    &self.parity,
                    self.stop_bits,
                    &self.flow,
                )
                .map_err(context)?,
            },
            "ssh" => Connection::Ssh(SshConfig {
                destination: need(&self.host, "a host")?,
                port: self.port,
            }),
            "lat" => Connection::Lat {
                interface: self.interface.clone().filter(|i| !i.trim().is_empty()),
                node: need(&self.host, "a node")?,
                service: self.service.clone().filter(|s| !s.trim().is_empty()),
            },
            "serial" => {
                let mut serial = SerialConfig::new(need(&self.device, "a device")?);
                if let Some(baud) = self.baud {
                    serial.baud = baud;
                }
                if let Some(bits) = self.data_bits {
                    serial.data_bits = bits;
                }
                if let Some(parity) = &self.parity {
                    serial.parity = parity.parse().map_err(context)?;
                }
                if let Some(stop) = self.stop_bits {
                    serial.stop_bits = stop;
                }
                if let Some(flow) = &self.flow {
                    serial.flow = flow.parse().map_err(context)?;
                }
                Connection::Serial(serial)
            }
            other => {
                return Err(context(format!(
                    "unknown connection {other:?} (shell, command, telnet, ssh, serial or lat)"
                )));
            }
        };
        let model = match &self.model {
            Some(m) => {
                cli::parse_model(m).ok_or_else(|| context(format!("unknown model {m:?}")))?
            }
            None => Config::default().model,
        };
        let phosphor = self.phosphor.clone().unwrap_or_else(|| "white".into());
        if !matches!(phosphor.as_str(), "white" | "green" | "amber") {
            return Err(context(format!("unknown phosphor {phosphor:?}")));
        }
        Ok(Profile {
            name: name.into(),
            model,
            connection,
            phosphor,
            sessions: self.sessions.unwrap_or(1).clamp(1, 2),
            keymap: self.keymap.clone(),
            log: self.log.clone().filter(|l| !l.trim().is_empty()),
            log_timestamps: self.log_timestamps,
            log_raw: self.log_raw,
            default: false,
        })
    }

    fn from_profile(p: &Profile) -> Entry {
        let mut e = Entry {
            name: p.name.clone(),
            model: Some(p.model.term_name_exact().into()),
            phosphor: (p.phosphor != "white").then(|| p.phosphor.clone()),
            sessions: (p.sessions != 1).then_some(p.sessions),
            keymap: p.keymap.clone(),
            log: p.log.clone(),
            log_timestamps: p.log_timestamps,
            log_raw: p.log_raw,
            ..Entry::default()
        };
        match &p.connection {
            Connection::Shell => e.connection = "shell".into(),
            Connection::Command(c) => {
                e.connection = "command".into();
                e.command = Some(c.clone());
            }
            Connection::Telnet {
                host,
                port,
                binary,
                com_port,
            } => {
                e.connection = "telnet".into();
                e.host = Some(host.clone());
                e.port = (*port != 23).then_some(*port);
                e.telnet_binary = binary.then_some(true);
                if let Some(p) = com_port {
                    e.baud = Some(p.baud);
                    e.data_bits = Some(p.data_bits);
                    e.parity = Some(parity_name(p.parity).into());
                    e.stop_bits = Some(p.stop_bits);
                    e.flow = Some(flow_name(p.flow).into());
                }
            }
            Connection::Ssh(s) => {
                e.connection = "ssh".into();
                e.host = Some(s.destination.clone());
                e.port = s.port;
            }
            Connection::Lat {
                interface,
                node,
                service,
            } => {
                e.connection = "lat".into();
                // The node is the host of a LAT connection, and the service
                // is only worth keeping when it is not the node's own name.
                e.host = Some(node.clone());
                e.interface = interface.clone();
                e.service = service.clone().filter(|s| s != node);
            }
            Connection::Serial(s) => {
                e.connection = "serial".into();
                e.device = Some(s.device.to_string_lossy().into_owned());
                e.baud = Some(s.baud);
                e.data_bits = Some(s.data_bits);
                e.parity = Some(parity_name(s.parity).into());
                e.stop_bits = Some(s.stop_bits);
                e.flow = Some(flow_name(s.flow).into());
            }
        }
        e
    }
}

/// The line settings of a Telnet profile, present when any of them is: the
/// same keys a serial profile uses, asked of a terminal server with RFC 2217.
fn com_port_from(
    baud: Option<u32>,
    data_bits: Option<u8>,
    parity: &Option<String>,
    stop_bits: Option<u8>,
    flow: &Option<String>,
) -> Result<Option<ComPort>, String> {
    if baud.is_none()
        && data_bits.is_none()
        && parity.is_none()
        && stop_bits.is_none()
        && flow.is_none()
    {
        return Ok(None);
    }
    let mut port = ComPort::default();
    if let Some(b) = baud {
        port.baud = b;
    }
    if let Some(b) = data_bits {
        port.data_bits = b;
    }
    if let Some(p) = parity {
        port.parity = p.parse()?;
    }
    if let Some(s) = stop_bits {
        port.stop_bits = s;
    }
    if let Some(f) = flow {
        port.flow = f.parse()?;
    }
    Ok(Some(port))
}

fn parity_name(parity: Parity) -> &'static str {
    match parity {
        Parity::None => "n",
        Parity::Even => "e",
        Parity::Odd => "o",
        Parity::Mark => "m",
        Parity::Space => "s",
    }
}

fn flow_name(flow: FlowControl) -> &'static str {
    match flow {
        FlowControl::None => "n",
        FlowControl::XonXoff => "x",
        FlowControl::RtsCts => "h",
    }
}

/// Reads profiles from TOML text.
pub fn parse(text: &str) -> Result<Vec<Profile>, String> {
    let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut profiles = file
        .profile
        .iter()
        .map(Entry::to_profile)
        .collect::<Result<Vec<_>, _>>()?;
    for (i, p) in profiles.iter().enumerate() {
        if profiles[..i].iter().any(|q| q.name == p.name) {
            return Err(format!("two profiles are named {:?}", p.name));
        }
    }
    // A name that matches nothing is a mistake to report, not a default to
    // lose quietly: the terminal would open something else and say nothing.
    if let Some(name) = file.default_profile.as_deref().map(str::trim) {
        match profiles.iter_mut().find(|p| p.name == name) {
            Some(p) => p.default = true,
            None => {
                return Err(format!(
                    "default-profile is {name:?}, which is not a saved connection"
                ));
            }
        }
    }
    Ok(profiles)
}

/// Stores `profile` at `index`, or at the end, and leaves at most one
/// default: a profile that is the default takes it from the others.
pub fn put(all: &mut Vec<Profile>, index: Option<usize>, profile: Profile) {
    if profile.default {
        all.iter_mut().for_each(|p| p.default = false);
    }
    match index {
        Some(i) if i < all.len() => all[i] = profile,
        _ => all.push(profile),
    }
}

/// Makes the profile at `index` the default, or, if it already is, leaves
/// there being none, so veetee goes back to opening the login shell.
pub fn toggle_default(all: &mut [Profile], index: usize) {
    let was = all.get(index).is_some_and(|p| p.default);
    all.iter_mut().for_each(|p| p.default = false);
    if let Some(p) = all.get_mut(index) {
        p.default = !was;
    }
}

/// The profiles as TOML text.
pub fn to_toml(profiles: &[Profile]) -> String {
    let file = File {
        default_profile: profiles.iter().find(|p| p.default).map(|p| p.name.clone()),
        profile: profiles.iter().map(Entry::from_profile).collect(),
    };
    let body = toml::to_string_pretty(&file).unwrap_or_default();
    format!(
        "# veetee saved connections. Edit here or in the Connections window.\n\
         # connection: shell, command, telnet, ssh, serial or lat.\n\
         # default-profile: the one opened when veetee starts without a connection.\n\n{body}"
    )
}

pub fn path() -> PathBuf {
    glib::user_config_dir().join("veetee").join("profiles.toml")
}

/// The saved profiles; none if the file does not exist.
pub fn load() -> Result<Vec<Profile>, String> {
    match std::fs::read_to_string(path()) {
        Ok(text) => parse(&text).map_err(|e| format!("{}: {e}", path().display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path().display())),
    }
}

pub fn save(profiles: &[Profile]) -> io::Result<()> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, to_toml(profiles))
}

/// The name of the connection to open when none is asked for, if one is set.
pub fn default_name() -> Result<Option<String>, String> {
    Ok(load()?.into_iter().find(|p| p.default).map(|p| p.name))
}

/// The saved profile called `name`.
pub fn find(name: &str) -> Result<Profile, String> {
    let profiles = load()?;
    profiles
        .iter()
        .find(|p| p.name == name)
        .or_else(|| profiles.iter().find(|p| p.name.eq_ignore_ascii_case(name)))
        .cloned()
        .ok_or_else(|| {
            let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
            if names.is_empty() {
                format!("no profile {name:?}: no connections are saved yet")
            } else {
                format!("no profile {name:?} (saved: {})", names.join(", "))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[profile]]
name = "vms1"
connection = "telnet"
host = "vms1"
model = "vt520"
phosphor = "green"

[[profile]]
name = "console"
connection = "serial"
device = "/dev/ttyUSB0"
baud = 19200
parity = "e"
data-bits = 7
sessions = 2

[[profile]]
name = "alpha"
connection = "ssh"
host = "system@alpha"
port = 2222
log = "~/logs/alpha-%Y%m%d.log"
log-timestamps = true

[[profile]]
name = "myi64"
connection = "lat"
host = "MYI64"
interface = "enp0s31f6"
service = "TERMINALS"
"#;

    #[test]
    fn reads_every_kind_of_connection() {
        let p = parse(SAMPLE).unwrap();
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].summary(), "VT520 · telnet vms1");
        assert_eq!(p[0].phosphor, "green");
        assert_eq!(p[1].summary(), "VT420 · /dev/ttyUSB0 19200 7E1");
        assert_eq!(p[1].sessions, 2);
        assert_eq!(p[2].summary(), "VT420 · ssh system@alpha:2222");
        assert_eq!(p[3].summary(), "VT420 · lat MYI64/TERMINALS");
        assert_eq!(
            p[3].connection,
            Connection::Lat {
                interface: Some("enp0s31f6".into()),
                node: "MYI64".into(),
                service: Some("TERMINALS".into()),
            }
        );
        let (mut config, mut options) = (Config::default(), Options::default());
        p[2].apply(&mut config, &mut options);
        let log = options.log.unwrap();
        assert!(log.timestamps && log.append && !log.raw);
        let name = log.path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("alpha-2") && name.ends_with(".log"),
            "{name}"
        );
        assert!(log.path.parent().unwrap().ends_with("logs"));
    }

    #[test]
    fn saved_text_reads_back() {
        let p = parse(SAMPLE).unwrap();
        assert_eq!(parse(&to_toml(&p)).unwrap(), p);
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(parse("[[profile]]\nname = \"x\"\nconnection = \"lat\"\n").is_err());
        assert!(parse("[[profile]]\nname = \"x\"\nconnection = \"telnet\"\n").is_err());
        assert!(
            parse("[[profile]]\nname = \"x\"\nconnection = \"shell\"\nmodel = \"vt999\"\n")
                .is_err()
        );
        assert!(parse("[[profile]]\nname = \"x\"\nconnection = \"shell\"\ncolour = 1\n").is_err());
        let twice = "[[profile]]\nname = \"x\"\nconnection = \"shell\"\n".repeat(2);
        assert!(parse(&twice).unwrap_err().contains("two profiles"));
    }

    #[test]
    fn the_default_is_named_at_the_top_of_the_file() {
        let text = format!("default-profile = \"alpha\"\n{SAMPLE}");
        let p = parse(&text).unwrap();
        assert_eq!(
            p.iter().map(|p| p.default).collect::<Vec<_>>(),
            [false, false, true, false]
        );
        let saved = to_toml(&p);
        assert!(saved.contains("\ndefault-profile = \"alpha\"\n"), "{saved}");
        assert_eq!(parse(&saved).unwrap(), p);
        // Nothing is the default until something is made so.
        assert!(!to_toml(&parse(SAMPLE).unwrap()).contains("\ndefault-profile ="));
        assert!(parse(SAMPLE).unwrap().iter().all(|p| !p.default));
        // The file is TOML with the key before the first table, so it is
        // still one an older veetee reads: it ignores keys it does not know
        // at the top, and there is nothing new inside a profile.
        assert!(!saved.contains("default = "));
    }

    #[test]
    fn a_default_that_names_nothing_is_reported() {
        let text = format!("default-profile = \"nope\"\n{SAMPLE}");
        let e = parse(&text).unwrap_err();
        assert!(
            e.contains("\"nope\"") && e.contains("not a saved connection"),
            "{e}"
        );
    }

    #[test]
    fn one_default_at_most() {
        let mut all = parse(SAMPLE).unwrap();
        let named = |all: &mut Vec<Profile>, i: usize| {
            let mut p = all[i].clone();
            p.default = true;
            put(all, Some(i), p);
        };
        named(&mut all, 0);
        named(&mut all, 2);
        assert_eq!(all.iter().filter(|p| p.default).count(), 1);
        assert!(all[2].default);

        // Editing the default and switching it off leaves none.
        let mut off = all[2].clone();
        off.default = false;
        put(&mut all, Some(2), off);
        assert!(all.iter().all(|p| !p.default));

        // A new one that is the default takes it from the rest.
        named(&mut all, 1);
        let mut new = all[0].clone();
        new.name = "new".into();
        new.default = true;
        put(&mut all, None, new);
        assert_eq!(all.iter().filter(|p| p.default).count(), 1);
        assert!(all.last().unwrap().default);
    }

    #[test]
    fn the_star_moves_the_default_and_takes_it_away() {
        let mut all = parse(SAMPLE).unwrap();
        toggle_default(&mut all, 1);
        assert_eq!(default_of(&all), Some("console"));
        toggle_default(&mut all, 3);
        assert_eq!(default_of(&all), Some("myi64"));
        toggle_default(&mut all, 3);
        assert_eq!(default_of(&all), None);
        toggle_default(&mut all, 99);
        assert_eq!(default_of(&all), None);
    }

    fn default_of(all: &[Profile]) -> Option<&str> {
        all.iter().find(|p| p.default).map(|p| p.name.as_str())
    }
}
