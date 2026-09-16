//! Hearing LAT on the wire: a raw Ethernet socket joined to the LAT group.
//!
//! Linux only, because LAT is not IP. It needs `AF_PACKET` and so `CAP_NET_RAW`;
//! a helper holding that capability, so the interface stays unprivileged, is
//! still to come (`docs/ROADMAP.md`).
//!
//! Joining the multicast group is not optional. The card filters
//! `09-00-2B-00-00-0F` out otherwise and nothing arrives at all — `tcpdump`
//! hides that by putting the interface in promiscuous mode, so a capture and an
//! application can disagree about whether there is any traffic.
#![allow(unsafe_code)]

use std::ffi::CString;
use std::io::{self, Read};
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

use vt_lat::{ETHERTYPE, GROUP};

/// The Ethernet header a `SOCK_RAW` packet socket keeps on the front.
pub const HEADER: usize = 14;

/// Listens for LAT messages on one interface.
#[derive(Debug)]
pub struct Listener {
    socket: Socket,
}

impl Listener {
    /// Opens a packet socket for LAT and joins the group on `interface`.
    ///
    /// The socket is left unbound, so it hears LAT on every interface; the
    /// group is joined only on this one. Sending will want a bound socket, and
    /// can have one when there is something to send.
    pub fn open(interface: &str) -> io::Result<Listener> {
        // AF_PACKET, and the protocol in network order, as the kernel wants it.
        let socket = Socket::new(
            Domain::from(libc::AF_PACKET),
            Type::from(libc::SOCK_RAW),
            Some(Protocol::from(i32::from(ETHERTYPE.to_be()))),
        )?;
        join_group(&socket, interface)?;
        Ok(Listener { socket })
    }

    /// Reads one frame, giving up after `timeout`. A timeout reads no bytes
    /// rather than failing, since nothing arriving is the ordinary case
    /// between announcements.
    pub fn recv_timeout(&mut self, frame: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.socket
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        match self.socket.read(frame) {
            Ok(n) => Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(0)
            }
            Err(e) => Err(e),
        }
    }
}

/// The address a frame came from, and the LAT message in it.
pub fn split(frame: &[u8]) -> Option<([u8; 6], &[u8])> {
    let source = frame.get(6..12)?.try_into().ok()?;
    let kind = u16::from_be_bytes(frame.get(12..14)?.try_into().ok()?);
    (kind == ETHERTYPE).then(|| (source, &frame[HEADER..]))
}

/// Asks the card to keep LAT's multicast frames rather than filter them out.
fn join_group(socket: &Socket, interface: &str) -> io::Result<()> {
    let name = CString::new(interface)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "interface name"))?;
    // SAFETY: `name` is a valid C string for the length of the call.
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    if index == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{interface}: no such interface"),
        ));
    }
    let mut mreq: libc::packet_mreq = unsafe { std::mem::zeroed() };
    mreq.mr_ifindex = index as i32;
    mreq.mr_type = libc::PACKET_MR_MULTICAST as u16;
    mreq.mr_alen = GROUP.len() as u16;
    mreq.mr_address[..GROUP.len()].copy_from_slice(&GROUP);
    // SAFETY: the socket is open for the length of the call, and `mreq` is a
    // whole `packet_mreq` whose length is passed with it.
    let rc = unsafe {
        libc::setsockopt(
            std::os::fd::AsRawFd::as_raw_fd(socket),
            libc::SOL_PACKET,
            libc::PACKET_ADD_MEMBERSHIP,
            std::ptr::from_ref(&mreq).cast(),
            size_of::<libc::packet_mreq>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        let e = io::Error::last_os_error();
        return Err(io::Error::new(
            e.kind(),
            format!("joining the LAT multicast group on {interface}: {e}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_gives_up_its_sender_and_message() {
        let mut frame = vec![0x09, 0x00, 0x2b, 0x00, 0x00, 0x0f];
        frame.extend_from_slice(&[0x00, 0x17, 0xa4, 0xab, 0x62, 0x50]);
        frame.extend_from_slice(&ETHERTYPE.to_be_bytes());
        frame.extend_from_slice(&[0x28, 0x08]);
        let (source, payload) = split(&frame).expect("a LAT frame");
        assert_eq!(source, [0x00, 0x17, 0xa4, 0xab, 0x62, 0x50]);
        assert_eq!(payload, [0x28, 0x08]);
    }

    #[test]
    fn another_protocol_is_not_ours() {
        let mut frame = vec![0u8; 12];
        frame.extend_from_slice(&0x0800u16.to_be_bytes()); // IPv4
        frame.push(0x45);
        assert_eq!(split(&frame), None);
        assert_eq!(split(&[0u8; 8]), None, "too short to have a header");
    }

    #[test]
    fn an_unknown_interface_is_named_in_the_error() {
        // Opening the socket needs CAP_NET_RAW, so only check the name when it
        // is refused for the reason we can test without privilege.
        if let Err(e) = Listener::open("no-such-interface-0") {
            let text = e.to_string();
            assert!(
                text.contains("no-such-interface-0") || e.kind() == io::ErrorKind::PermissionDenied,
                "{text}"
            );
        }
    }
}
