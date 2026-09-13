//! Host connections for veetee.

use std::io::{self, Write};
use std::time::Duration;

pub mod pty;
pub mod serial;

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
}
