//! LAT on the wire: a raw Ethernet socket joined to the LAT group, and a
//! [`Transport`](crate::Transport) over one session on it.
//!
//! [`Listener`] is the datalink — frames in and out on one interface — and
//! [`Lat`] drives a session over it, with the protocol itself in `vt-lat`,
//! which has no sockets and is tested against captured frames.
//!
//! Linux only, because LAT is not IP. It needs `AF_PACKET` and so `CAP_NET_RAW`;
//! a helper holding that capability, so the interface stays unprivileged, is
//! still to come (`docs/ROADMAP.md`).
//!
//! A page size goes in the slot that asks for a service, so a session is the
//! size the terminal was when it opened; there is no read way to tell the far
//! end that it has changed, which is why [`Lat`] does not answer a resize.
//!
//! Joining the multicast group is not optional. The card filters
//! `09-00-2B-00-00-0F` out otherwise and nothing arrives at all — `tcpdump`
//! hides that by putting the interface in promiscuous mode, so a capture and an
//! application can disagree about whether there is any traffic.
#![allow(unsafe_code)]

use std::ffi::CString;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};

use vt_lat::{ETHERTYPE, Event, GROUP, Message, Session, SessionConfig};

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

    /// Another handle on the same socket, so one thread can send while
    /// another waits for a frame.
    pub fn try_clone(&self) -> io::Result<Listener> {
        Ok(Listener {
            socket: self.socket.try_clone()?,
            address: self.address,
        })
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

/// How long to wait for a node to announce itself, when its address is not
/// already known. Announcements come about once a minute.
const ANNOUNCE_WAIT: Duration = Duration::from_secs(90);

/// How long to wait for the node to agree a circuit, and how often to ask
/// again meanwhile.
///
/// 🔎 The 80 ms circuit timer that governs retransmission is unread, and
/// nothing was ever lost in the captured session, so asking again on a timer
/// of our own is the safe way round it.
const AGREE_WAIT: Duration = Duration::from_secs(15);
const CALL_AGAIN: Duration = Duration::from_secs(3);

/// Silence after which an idle circuit is kept alive. The start message asks
/// for twenty seconds and OpenVMS sends one every ten to twenty, so this is
/// comfortably inside what the far end expects.
const KEEPALIVE: Duration = Duration::from_secs(10);

/// How long a single wait for a frame lasts, so that a keepalive falling due
/// during a long read is not late.
const POLL: Duration = Duration::from_millis(100);

/// The name veetee gives itself on a circuit when nothing else is asked for.
const FROM: &str = "VEETEE";

/// Which node to call, on which interface, and what to ask it for.
#[derive(Debug, Clone)]
pub struct LatConfig {
    /// The interface to speak LAT on. There is no routing, so it has to be
    /// the one sharing a segment with the node.
    pub interface: String,
    /// The node to call, as it announces itself.
    pub node: String,
    /// The service to ask for. The node's own name if none is given, which is
    /// how OpenVMS offers its terminal service.
    pub service: Option<String>,
    /// The node's Ethernet address, if it is already known — from a service
    /// browser that has heard it announce itself. Learned by waiting for an
    /// announcement otherwise, which takes up to a multicast timer.
    pub address: Option<[u8; 6]>,
    /// The name this end goes by on the circuit.
    pub from: Option<String>,
    /// The page size, which the slot asking for a service carries.
    pub rows: u16,
    pub cols: u16,
}

/// The session, and when anything was last sent on it.
///
/// Both sides hold this: reading answers what arrives, writing sends what is
/// typed, and each counts the sequence numbers up, so they cannot be allowed
/// to do it at once. A keepalive is due only after silence from both.
#[derive(Debug)]
struct Shared {
    session: Session,
    sent: Instant,
}

/// A LAT session on an interface: one circuit to a node, and a terminal on a
/// service of it.
#[derive(Debug)]
pub struct Lat {
    listener: Listener,
    shared: Arc<Mutex<Shared>>,
    /// The node's address. Every frame of the circuit goes to it directly.
    peer: [u8; 6],
    /// Session data read but not yet given to the caller, and how much of it
    /// has gone: a run message can carry more than one read asks for.
    pending: Vec<u8>,
    at: usize,
    frame: Vec<u8>,
    description: String,
}

impl Lat {
    /// Opens a circuit to the node and asks it for a service, returning once
    /// the far end has agreed.
    ///
    /// The terminal is not quite ready at that point: the far end sends its
    /// banner in its own time, and will not read before it has prompted, so
    /// anything typed meanwhile is held back until it does.
    pub fn connect(config: LatConfig) -> io::Result<Lat> {
        let service = config
            .service
            .clone()
            .unwrap_or_else(|| config.node.clone());
        let mut listener = Listener::open(&config.interface).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("{e}\nLAT needs CAP_NET_RAW: try sudo, or setcap cap_net_raw+ep"),
            )
        })?;
        let mut frame = vec![0u8; 2048];
        let peer = match config.address {
            Some(address) => address,
            None => announced(&mut listener, &mut frame, &config.node)?,
        };

        let mut session = Session::new(SessionConfig {
            node: config.node.clone(),
            service: service.clone(),
            from: config.from.clone().unwrap_or_else(|| FROM.to_string()),
            rows: config.rows,
            cols: config.cols,
            address: listener.address(),
        });
        session.call();
        let mut shared = Shared {
            session,
            sent: Instant::now(),
        };
        flush(&mut listener, peer, &mut shared)?;

        // Whatever the far end says before the caller first reads is kept:
        // OpenVMS names the terminal it has created straight away.
        let mut pending = Vec::new();
        let deadline = Instant::now() + AGREE_WAIT;
        let mut again = Instant::now() + CALL_AGAIN;
        while !shared.session.is_open() {
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{} did not agree a circuit", config.node),
                ));
            }
            if now >= again {
                shared.session.call();
                flush(&mut listener, peer, &mut shared)?;
                again = now + CALL_AGAIN;
            }
            let n = listener.recv_timeout(&mut frame, POLL)?;
            let Some((_, payload)) = split(&frame[..n]) else {
                continue;
            };
            let event = shared.session.receive(payload, &mut pending);
            flush(&mut listener, peer, &mut shared)?;
            if event == Event::Closed {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("{} took the circuit down", config.node),
                ));
            }
        }

        let description = if service == config.node {
            format!("lat {} on {}", config.node, config.interface)
        } else {
            format!("lat {}/{service} on {}", config.node, config.interface)
        };
        Ok(Lat {
            listener,
            shared: Arc::new(Mutex::new(shared)),
            peer,
            pending,
            at: 0,
            frame,
            description,
        })
    }

    /// Whether anything read has still to be handed to the caller.
    fn unread(&self) -> bool {
        self.at < self.pending.len()
    }

    /// Hands the caller as much of what has arrived as it asked for.
    fn drain(&mut self, buf: &mut [u8]) -> usize {
        let n = (self.pending.len() - self.at).min(buf.len());
        buf[..n].copy_from_slice(&self.pending[self.at..self.at + n]);
        self.at += n;
        if self.at == self.pending.len() {
            self.pending.clear();
            self.at = 0;
        }
        n
    }

    /// Keeps an idle circuit open. The far end takes down one that goes quiet,
    /// and a terminal is quiet for as long as the user is reading.
    fn keepalive(&mut self) -> io::Result<()> {
        let mut shared = lock(&self.shared);
        if shared.sent.elapsed() < KEEPALIVE {
            return Ok(());
        }
        shared.session.keepalive();
        flush(&mut self.listener, self.peer, &mut shared)
    }

    fn closed(&self) -> bool {
        lock(&self.shared).session.is_closed()
    }
}

impl crate::Transport for Lat {
    fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        if self.unread() {
            return Ok(self.drain(buf));
        }
        if self.closed() {
            return Err(down());
        }
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let wait = deadline.saturating_duration_since(now).min(POLL);
            let n = self.listener.recv_timeout(&mut self.frame, wait)?;
            if n > 0 {
                // A packet socket hears every LAT frame on the wire, this
                // circuit or not, so most of what arrives is ignored here.
                let mut shared = lock(&self.shared);
                if let Some((_, payload)) = split(&self.frame[..n]) {
                    shared.session.receive(payload, &mut self.pending);
                    flush(&mut self.listener, self.peer, &mut shared)?;
                }
                let closed = shared.session.is_closed();
                drop(shared);
                if self.unread() {
                    return Ok(self.drain(buf));
                }
                if closed {
                    break;
                }
            }
            self.keepalive()?;
        }
        self.keepalive()?;
        if self.unread() {
            // Whatever the far end said last is worth having even if it then
            // took the circuit down; the read after this one reports that.
            Ok(self.drain(buf))
        } else if self.closed() {
            Err(down())
        } else {
            Ok(0)
        }
    }

    fn writer(&self) -> io::Result<Box<dyn crate::TransportWriter>> {
        Ok(Box::new(LatWriter {
            listener: self.listener.try_clone()?,
            shared: Arc::clone(&self.shared),
            peer: self.peer,
        }))
    }

    fn description(&self) -> String {
        self.description.clone()
    }
}

impl Drop for Lat {
    fn drop(&mut self) {
        // Take the circuit down, so the far end releases the terminal it
        // created rather than waiting out its own timer. Nothing can be done
        // if it fails, and the timer is there for exactly that case.
        let mut shared = lock(&self.shared);
        shared.session.close();
        let _ = flush(&mut self.listener, self.peer, &mut shared);
    }
}

/// Sending side of a [`Lat`] session.
#[derive(Debug)]
struct LatWriter {
    listener: Listener,
    shared: Arc<Mutex<Shared>>,
    peer: [u8; 6],
}

impl Write for LatWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut shared = lock(&self.shared);
        if shared.session.is_closed() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the circuit is down",
            ));
        }
        shared.session.write(data);
        flush(&mut self.listener, self.peer, &mut shared)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // A slot goes as soon as it is built; there is nothing held back but
        // typing the far end has not asked for yet.
        Ok(())
    }
}

impl crate::TransportWriter for LatWriter {
    fn send_break(&mut self) -> io::Result<()> {
        // 🔎 A real LAT terminal has a break, but which slot type carries it
        // is unread: guessing would send the host something arbitrary.
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the LAT slot that carries a break has not been read yet",
        ))
    }
}

/// Sends whatever the session has queued, and notes that the circuit has been
/// spoken on.
fn flush(listener: &mut Listener, peer: [u8; 6], shared: &mut Shared) -> io::Result<()> {
    for frame in shared.session.take_outgoing() {
        listener.send(peer, &frame)?;
        shared.sent = Instant::now();
    }
    Ok(())
}

/// What a read gives once the circuit has gone, which is how a terminal is
/// told a session has ended.
fn down() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "the host took the circuit down",
    )
}

/// A poisoned lock is no reason to lose a session: the state behind it is a
/// circuit, and the panic that poisoned it has already been reported.
fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// Waits to hear a node announce itself, for its Ethernet address.
///
/// That is the only way veetee has of learning one: a solicit built by hand
/// has never been answered, so this waits for an announcement to arrive of its
/// own accord, which takes up to the sender's multicast timer.
fn announced(listener: &mut Listener, frame: &mut [u8], node: &str) -> io::Result<[u8; 6]> {
    let deadline = Instant::now() + ANNOUNCE_WAIT;
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{node} did not announce itself"),
            ));
        }
        let n = listener.recv_timeout(frame, POLL)?;
        let Some((source, payload)) = split(&frame[..n]) else {
            continue;
        };
        if let Ok(Message::Announcement(a)) = vt_lat::parse(payload)
            && a.node.eq_ignore_ascii_case(node)
        {
            return Ok(source);
        }
    }
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
