//! Unix pseudo-terminals: the child leads a session with the PTY as its
//! controlling terminal.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{OFlags, open};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};

/// A running child process attached to a PTY.
#[derive(Debug)]
pub struct Pty {
    master: File,
    child: Child,
    description: String,
}

impl Pty {
    /// Spawns `program` with `args` on a new PTY of the given size. `TERM`
    /// is set to `term`; the rest of the environment is inherited.
    pub fn spawn<S: AsRef<OsStr>>(
        program: &str,
        args: &[S],
        rows: u16,
        cols: u16,
        term: &str,
    ) -> io::Result<Pty> {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let name = ptsname(&master, Vec::new())?;
        let slave: OwnedFd = open(name.as_c_str(), OFlags::RDWR | OFlags::NOCTTY, 0.into())?;
        tcsetwinsize(&slave, winsize(rows, cols))?;

        let mut cmd = Command::new(program);
        cmd.args(args)
            .env("TERM", term)
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        // The child must lead a new session with the PTY as controlling terminal,
        // so job control and SIGWINCH behave as on a real terminal line.
        #[allow(unsafe_code)]
        // SAFETY: the closure only makes async-signal-safe system calls.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())?;
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        let mut description = program.to_string();
        for arg in args {
            description.push(' ');
            description.push_str(&arg.as_ref().to_string_lossy());
        }
        Ok(Pty {
            master: File::from(master),
            child,
            description,
        })
    }

    /// Informs the child of a new terminal size (delivers SIGWINCH).
    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        tcsetwinsize(&self.master, winsize(rows, cols))?;
        Ok(())
    }

    /// Waits up to `timeout` for output. Returns `Ok(0)` on timeout and
    /// `Err(UnexpectedEof)` once the child has closed the PTY.
    pub fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        let ts = Timespec {
            tv_sec: timeout.as_secs() as _,
            tv_nsec: timeout.subsec_nanos() as _,
        };
        let mut fds = [PollFd::new(&self.master, PollFlags::IN)];
        if poll(&mut fds, Some(&ts))? == 0 {
            return Ok(0);
        }
        match self.master.read(buf) {
            Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
            // Linux reports EIO on the master after the last slave closes.
            Err(e) if e.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => {
                Err(io::ErrorKind::UnexpectedEof.into())
            }
            other => other,
        }
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.master.write_all(bytes)
    }

    pub fn master(&self) -> impl AsFd + '_ {
        &self.master
    }

    /// Returns the exit status if the child has finished.
    pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()?;
        self.child.wait().map(|_| ())
    }
}

impl crate::Transport for Pty {
    fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        Pty::read_timeout(self, buf, timeout)
    }

    fn writer(&self) -> io::Result<Box<dyn crate::TransportWriter>> {
        Ok(Box::new(PtyWriter(self.master.try_clone()?)))
    }

    fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        Pty::resize(self, rows, cols)
    }

    fn description(&self) -> String {
        self.description.clone()
    }
}

struct PtyWriter(File);

impl Write for PtyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl crate::TransportWriter for PtyWriter {}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.kill();
        }
    }
}

fn winsize(rows: u16, cols: u16) -> Winsize {
    Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_sees_tty_and_size() {
        let mut pty = Pty::spawn(
            "sh",
            &["-c", "tty -s && stty size; echo \"$TERM\""],
            30,
            100,
            "vt420",
        )
        .expect("spawn");
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            match pty.read_timeout(&mut buf, Duration::from_secs(5)) {
                Ok(0) => panic!("timed out; got {:?}", String::from_utf8_lossy(&out)),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(String::from_utf8_lossy(&out), "30 100\r\nvt420\r\n");
    }
}
