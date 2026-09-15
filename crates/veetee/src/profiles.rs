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
#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    profile: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Entry {
    name: String,
    /// shell, command, telnet, ssh or serial.
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
            },
            "ssh" => Connection::Ssh(SshConfig {
                destination: need(&self.host, "a host")?,
                port: self.port,
            }),
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
                    "unknown connection {other:?} (shell, command, telnet, ssh or serial)"
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
            Connection::Telnet { host, port, binary } => {
                e.connection = "telnet".into();
                e.host = Some(host.clone());
                e.port = (*port != 23).then_some(*port);
                e.telnet_binary = binary.then_some(true);
            }
            Connection::Ssh(s) => {
                e.connection = "ssh".into();
                e.host = Some(s.destination.clone());
                e.port = s.port;
            }
            Connection::Serial(s) => {
                e.connection = "serial".into();
                e.device = Some(s.device.to_string_lossy().into_owned());
                e.baud = Some(s.baud);
                e.data_bits = Some(s.data_bits);
                e.parity = Some(
                    match s.parity {
                        Parity::None => "n",
                        Parity::Even => "e",
                        Parity::Odd => "o",
                        Parity::Mark => "m",
                        Parity::Space => "s",
                    }
                    .into(),
                );
                e.stop_bits = Some(s.stop_bits);
                e.flow = Some(
                    match s.flow {
                        FlowControl::None => "n",
                        FlowControl::XonXoff => "x",
                        FlowControl::RtsCts => "h",
                    }
                    .into(),
                );
            }
        }
        e
    }
}

/// Reads profiles from TOML text.
pub fn parse(text: &str) -> Result<Vec<Profile>, String> {
    let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
    let profiles = file
        .profile
        .iter()
        .map(Entry::to_profile)
        .collect::<Result<Vec<_>, _>>()?;
    for (i, p) in profiles.iter().enumerate() {
        if profiles[..i].iter().any(|q| q.name == p.name) {
            return Err(format!("two profiles are named {:?}", p.name));
        }
    }
    Ok(profiles)
}

/// The profiles as TOML text.
pub fn to_toml(profiles: &[Profile]) -> String {
    let file = File {
        profile: profiles.iter().map(Entry::from_profile).collect(),
    };
    let body = toml::to_string_pretty(&file).unwrap_or_default();
    format!(
        "# veetee saved connections. Edit here or in the Connections window.\n\
         # connection: shell, command, telnet, ssh or serial.\n\n{body}"
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
"#;

    #[test]
    fn reads_every_kind_of_connection() {
        let p = parse(SAMPLE).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].summary(), "VT520 · telnet vms1");
        assert_eq!(p[0].phosphor, "green");
        assert_eq!(p[1].summary(), "VT420 · /dev/ttyUSB0 19200 7E1");
        assert_eq!(p[1].sessions, 2);
        assert_eq!(p[2].summary(), "VT420 · ssh system@alpha:2222");
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
}
