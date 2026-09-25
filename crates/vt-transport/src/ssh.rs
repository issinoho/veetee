//! SSH through the system OpenSSH client, so `~/.ssh/config`, agents, keys,
//! jump hosts, known hosts and Kerberos all work exactly as they do in a shell.

use std::io::{self, Write};
use std::time::Duration;

use crate::pty::Pty;
use crate::{Transport, TransportWriter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshConfig {
    /// `[user@]host`, or a `Host` alias from `~/.ssh/config`.
    pub destination: String,
    pub port: Option<u16>,
}

impl SshConfig {
    /// Arguments for `ssh`: force a remote TTY so full-screen applications work.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec!["-tt".to_string()];
        if let Some(port) = self.port {
            args.push("-p".into());
            args.push(port.to_string());
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

/// Starts `ssh` on a PTY. `term` becomes the remote `TERM`.
pub fn connect(config: &SshConfig, rows: u16, cols: u16, term: &str) -> io::Result<Ssh> {
    let pty = Pty::spawn("ssh", &config.args(), rows, cols, term).map_err(|e| {
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
        };
        assert_eq!(c.args(), ["-tt", "-p", "2222", "--", "system@vms1"]);
        let c = SshConfig {
            destination: "-oProxyCommand=evil".into(),
            port: None,
        };
        assert_eq!(
            c.args(),
            ["-tt", "--", "-oProxyCommand=evil"],
            "destination is never parsed as an option"
        );
    }
}
