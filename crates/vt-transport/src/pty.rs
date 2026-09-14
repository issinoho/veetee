//! Local pseudo-terminal connection: runs a program with a terminal of its
//! own, a Unix PTY or a Windows pseudo console (ConPTY).

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{Pty, in_flatpak};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Pty;

/// Whether veetee runs inside a Flatpak sandbox (never on Windows).
#[cfg(windows)]
pub fn in_flatpak() -> bool {
    false
}
