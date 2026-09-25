//! Host connections for veetee.

use std::io::{self, Write};
use std::time::Duration;

/// LAT discovery. Linux only: LAT is raw Ethernet, not IP.
#[cfg(target_os = "linux")]
pub mod lat;
pub mod pty;
pub mod serial;
pub mod ssh;
pub mod telnet;

/// A byte-stream connection to a host.
pub trait Transport: Send {
    /// Waits up to `timeout` for data. Returns `Ok(0)` on timeout and
    /// `Err(UnexpectedEof)` once the connection has closed.
    fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize>;

    /// An independent handle for sending data (and line signals) from another thread.
    fn writer(&self) -> io::Result<Box<dyn TransportWriter>>;

    /// The terminal page size changed (only meaningful for a local PTY).
    fn resize(&mut self, _rows: u16, _cols: u16) -> io::Result<()> {
        Ok(())
    }

    /// Short human-readable description, e.g. `/dev/ttyUSB0 9600 8N1`.
    fn description(&self) -> String;

    /// Whether the host's XON and XOFF arrive in the data, for the terminal
    /// to act on as a DEC terminal does. True of a network connection to a
    /// host's terminal driver — Telnet, SSH, LAT — which is how OpenVMS asks
    /// a terminal to stop sending when its type-ahead buffer is full. Not of
    /// a serial line, whose driver acts on them and takes them out of the
    /// data itself, nor of a local program on a pty, which has no line to
    /// overrun and would stop the keyboard by printing a binary file.
    fn flow_in_band(&self) -> bool {
        false
    }

    /// The line settings of a serial port, or those asked of a terminal
    /// server with RFC 2217; `None` for a connection with no line to set.
    fn line(&self) -> Option<serial::Line> {
        None
    }
}

/// Sending side of a [`Transport`].
pub trait TransportWriter: Write + Send {
    /// Sends a break signal (the DEC Break key).
    fn send_break(&mut self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this connection has no break signal",
        ))
    }

    /// Changes the line settings while connected, as leaving a DEC
    /// terminal's Communications Set-Up does. A setting the port refuses
    /// leaves the line as it was.
    fn set_line(&mut self, _line: &serial::Line) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this connection has no line settings",
        ))
    }
}
