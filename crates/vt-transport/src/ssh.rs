//! SSH through the system OpenSSH client, so `~/.ssh/config`, agents, keys,
//! jump hosts, known hosts and Kerberos all work exactly as they do in a shell.

use std::io::{self, Write};
use std::time::Duration;

use crate::pty::Pty;
use crate::{Transport, TransportWriter};

/// X11 forwarding: whether `ssh` is asked to carry the host's X clients
/// back to a display here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum X11 {
    /// No option given: `ForwardX11` in `~/.ssh/config` decides, and is off
    /// unless set there.
    #[default]
    AsConfigured,
    /// `ssh -X`: forwarded, with the X security extension's restrictions.
    On,
    /// `ssh -Y`: forwarded, with the clients trusted as local ones.
    Trusted,
}

impl X11 {
    /// The name saved connections use.
    pub fn name(self) -> Option<&'static str> {
        match self {
            X11::AsConfigured => None,
            X11::On => Some("on"),
            X11::Trusted => Some("trusted"),
        }
    }
}

impl std::str::FromStr for X11 {
    type Err = String;

    fn from_str(s: &str) -> Result<X11, String> {
        match s {
            "on" => Ok(X11::On),
            "trusted" => Ok(X11::Trusted),
            _ => Err(format!("x11: on or trusted, not {s:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SshConfig {
    /// `[user@]host`, or a `Host` alias from `~/.ssh/config`.
    pub destination: String,
    pub port: Option<u16>,
    pub x11: X11,
    /// The X display forwarded clients appear on, as `DISPLAY` for `ssh`;
    /// none leaves `DISPLAY` as veetee has it.
    pub display: Option<String>,
}

impl SshConfig {
    /// Arguments for `ssh`: force a remote TTY so full-screen applications work.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec!["-tt".to_string()];
        if let Some(port) = self.port {
            args.push("-p".into());
            args.push(port.to_string());
        }
        match self.x11 {
            X11::AsConfigured => {}
            X11::On => args.push("-X".into()),
            X11::Trusted => args.push("-Y".into()),
        }
        args.push("--".into());
        args.push(self.destination.clone());
        args
    }
}

/// An SSH session: the OpenSSH client running on a PTY.
#[derive(Debug)]
pub struct Ssh {
    pty: Pty,
    description: String,
}

/// Starts `ssh` on a PTY. `term` becomes the remote `TERM`. On Windows the
/// session runs over pipes, the pseudo console carrying only OpenSSH's
/// prompts: a pseudo console rewrites what passes through it, losing DECCOLM,
/// DEC line drawing and the keypad's application sequences.
pub fn connect(config: &SshConfig, rows: u16, cols: u16, term: &str) -> io::Result<Ssh> {
    #[cfg(windows)]
    let spawn = Pty::spawn_piped_with_env::<String>;
    #[cfg(not(windows))]
    let spawn = Pty::spawn_with_env::<String>;
    let env: Vec<(&str, &str)> = config
        .display
        .as_deref()
        .map(|d| ("DISPLAY", d))
        .into_iter()
        .collect();
    let pty = spawn("ssh", &config.args(), rows, cols, term, &env).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            io::Error::new(e.kind(), "the OpenSSH client (ssh) is not installed")
        } else {
            e
        }
    })?;
    let description = match config.port {
        Some(port) => format!("ssh {}:{port}", config.destination),
        None => format!("ssh {}", config.destination),
    };
    Ok(Ssh { pty, description })
}

impl Transport for Ssh {
    fn flow_in_band(&self) -> bool {
        true
    }

    fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.pty.read_timeout(buf, timeout)
    }

    fn writer(&self) -> io::Result<Box<dyn TransportWriter>> {
        Ok(Box::new(SshWriter(Transport::writer(&self.pty)?)))
    }

    fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        self.pty.resize(rows, cols)
    }

    fn description(&self) -> String {
        self.description.clone()
    }
}

struct SshWriter(Box<dyn TransportWriter>);

impl Write for SshWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl TransportWriter for SshWriter {
    fn send_break(&mut self) -> io::Result<()> {
        // OpenSSH only sends BREAK through its escape sequence, which must
        // start a line; sending it ourselves would inject a Return.
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "over SSH, press Return then type ~B",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_ssh_arguments() {
        let c = SshConfig {
            destination: "system@vms1".into(),
            port: Some(2222),
            ..SshConfig::default()
        };
        assert_eq!(c.args(), ["-tt", "-p", "2222", "--", "system@vms1"]);
        let c = SshConfig {
            destination: "-oProxyCommand=evil".into(),
            ..SshConfig::default()
        };
        assert_eq!(
            c.args(),
            ["-tt", "--", "-oProxyCommand=evil"],
            "destination is never parsed as an option"
        );
    }

    #[test]
    fn x11_forwarding_asks_for_x_or_y() {
        let mut c = SshConfig {
            destination: "user@server".into(),
            x11: X11::On,
            ..SshConfig::default()
        };
        assert_eq!(c.args(), ["-tt", "-X", "--", "user@server"]);
        c.x11 = X11::Trusted;
        assert_eq!(c.args(), ["-tt", "-Y", "--", "user@server"]);
        assert_eq!("trusted".parse(), Ok(X11::Trusted));
        assert!("yes".parse::<X11>().is_err());
    }
}
