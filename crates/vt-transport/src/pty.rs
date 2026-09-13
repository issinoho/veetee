//! Local pseudo-terminal connection: runs a program with a terminal of its
//! own, a Unix PTY or a Windows pseudo console (ConPTY).

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::Pty;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Pty;
