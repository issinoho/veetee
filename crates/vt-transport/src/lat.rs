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
use std::io::{self, Read, Write};
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

use vt_lat::{ETHERTYPE, GROUP};

/// The Ethernet header a `SOCK_RAW` packet socket keeps on the front.
pub const HEADER: usize = 14;

/// Listens for LAT messages on one interface, and sends on it.
#[derive(Debug)]
pub struct Listener {
    socket: Socket,
    /// This interface's own address, which every frame we send comes from.
    address: [u8; 6],
}

impl Listener {
    /// Opens a packet socket for LAT on `interface`, bound to it and joined to
    /// the group, so it can both hear and send.
    pub fn open(interface: &str) -> io::Result<Listener> {
        // AF_PACKET, and the protocol in network order, as the kernel wants it.
        let socket = Socket::new(
            Domain::from(libc::AF_PACKET),
            Type::from(libc::SOCK_RAW),
            Some(Protocol::from(i32::from(ETHERTYPE.to_be()))),
        )?;
        let index = interface_index(interface)?;
        bind_to(&socket, index)?;
        join_group(&socket, index, interface)?;
        Ok(Listener {
            socket,
            address: hardware_address(interface)?,
        })
    }

    /// This interface's own address.
    pub fn address(&self) -> [u8; 6] {
        self.address
    }

    /// Sends a LAT message, wrapping it in an Ethernet header.
    pub fn send(&mut self, to: [u8; 6], message: &[u8]) -> io::Result<()> {
        let mut frame = Vec::with_capacity(HEADER + message.len());
        frame.extend_from_slice(&to);
        frame.extend_from_slice(&self.address);
        frame.extend_from_slice(&ETHERTYPE.to_be_bytes());
        frame.extend_from_slice(message);
        self.socket.write_all(&frame)
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

fn interface_index(interface: &str) -> io::Result<u32> {
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
    Ok(index)
}

/// The interface's own address, read from sysfs rather than with an ioctl:
/// this is Linux only anyway, and the file needs no unsafe code at all.
fn hardware_address(interface: &str) -> io::Result<[u8; 6]> {
    let path = format!("/sys/class/net/{interface}/address");
    let text = std::fs::read_to_string(&path)?;
    let mut address = [0u8; 6];
    let mut parts = text.trim().split(':');
    for byte in &mut address {
        let part = parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{path}: short")))?;
        *byte = u8::from_str_radix(part, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("{path}: {part:?}")))?;
    }
    Ok(address)
}

/// Ties the socket to one interface, which is what lets it send.
fn bind_to(socket: &Socket, index: u32) -> io::Result<()> {
    // SAFETY: a zeroed sockaddr_ll is a valid one.
    let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    addr.sll_family = libc::AF_PACKET as u16;
    addr.sll_protocol = ETHERTYPE.to_be();
    addr.sll_ifindex = index as i32;
    // SAFETY: the socket is open for the length of the call, and `addr` is a
    // whole sockaddr_ll whose length is passed with it.
    let rc = unsafe {
        libc::bind(
            std::os::fd::AsRawFd::as_raw_fd(socket),
            std::ptr::from_ref(&addr).cast(),
            size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Asks the card to keep LAT's multicast frames rather than filter them out.
fn join_group(socket: &Socket, index: u32, interface: &str) -> io::Result<()> {
    // SAFETY: a zeroed packet_mreq is a valid one.
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
