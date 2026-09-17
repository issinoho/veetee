//! `veetee-lat-helper` — opens a LAT socket, so that veetee need not be
//! privileged itself.
//!
//! LAT rides directly on Ethernet, which takes `CAP_NET_RAW`. Giving that to
//! the whole terminal is far more than it should have, and it would not work
//! anyway: the kernel sets `AT_SECURE` for a process raised by file
//! capabilities, and GLib refuses to go on when it sees that. So this opens
//! the socket, hands it back through its own standard input and exits.
//! Everything after that — the circuit, the session, every frame — is veetee's
//! own work, unprivileged.
//!
//! It is not given the capability when installed. Grant it with:
//!
//! ```sh
//! sudo setcap cap_net_raw+ep /usr/libexec/veetee-lat-helper
//! ```
//!
//! A capability nobody asked for is a poor default, and LAT is wanted by few.
//! Note what the grant means: anyone who can run this can open a socket for
//! LAT frames on one interface, and send them. That is narrower than
//! `CAP_NET_RAW` — the socket carries one protocol, `0x6004` — but it is not
//! nothing, and it is why the capability is opt-in rather than shipped.

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("LAT needs raw Ethernet, which veetee only has on Linux");
    std::process::ExitCode::FAILURE
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(interface) = args.next() else {
        eprintln!(
            "usage: veetee-lat-helper INTERFACE\n\
             The socket goes back through standard input, which veetee makes a \
             socket pair of; there is nothing here to run by hand."
        );
        return std::process::ExitCode::FAILURE;
    };
    match hand_over(&interface) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // veetee reads this and shows it: it is the only thing the helper
            // has to say, and usually says the capability has not been given.
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Opens the socket and passes it back, which is the whole of the job.
#[cfg(target_os = "linux")]
fn hand_over(interface: &str) -> std::io::Result<()> {
    use std::io::IoSlice;

    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
    use std::os::fd::AsFd;

    let listener = vt_transport::lat::Listener::open(interface)?;

    // A descriptor travels as ancillary data beside a byte of nothing, since
    // a message with no body is not a message.
    let sockets = [listener.as_fd()];
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    if !ancillary.push(SendAncillaryMessage::ScmRights(&sockets)) {
        return Err(std::io::Error::other("no room for the socket"));
    }
    sendmsg(
        rustix::stdio::stdin(),
        &[IoSlice::new(b"L")],
        &mut ancillary,
        SendFlags::empty(),
    )?;
    Ok(())
}
