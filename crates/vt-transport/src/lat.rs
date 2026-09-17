//! LAT on the wire: a raw Ethernet socket joined to the LAT group, and a
//! [`Transport`](crate::Transport) over one session on it.
//!
//! [`Listener`] is the datalink — frames in and out on one interface — and
//! [`Lat`] drives a session over it, with the protocol itself in `vt-lat`,
//! which has no sockets and is tested against captured frames.
//!
//! Linux only, because LAT is not IP. It needs `AF_PACKET` and so `CAP_NET_RAW`.
//! [`open`] gets a socket without this process holding that: it opens one
//! directly if it can, and otherwise asks `veetee-lat-helper`, which holds the
//! capability and hands the socket back. A terminal has no business holding it
//! — and GTK will not start at all when it does, the kernel setting
//! `AT_SECURE` for a process raised by file capabilities.
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
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
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
        )
        .map_err(|e| {
            // Say what to do about it, but only when this is what went wrong:
            // an interface that does not exist is not a question of privilege.
            if e.kind() == io::ErrorKind::PermissionDenied {
                io::Error::new(
                    e.kind(),
                    format!("{e}: LAT needs CAP_NET_RAW, so try sudo, or setcap cap_net_raw+ep"),
                )
            } else {
                e
            }
        })?;
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

    /// Takes on a socket somebody else opened — the helper, which holds the
    /// capability this process does not.
    ///
    /// The interface is named again because the address is read from sysfs,
    /// which needs no privilege and so is no reason to ask the helper.
    pub fn adopt(socket: OwnedFd, interface: &str) -> io::Result<Listener> {
        Ok(Listener {
            socket: Socket::from(socket),
            address: hardware_address(interface)?,
        })
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

impl AsFd for Listener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.socket.as_fd()
    }
}

/// Ethernet, as `/sys/class/net/*/type` reports it. LAT rides on Ethernet
/// and nothing else, so anything of another kind is no use to it.
const ARPHRD_ETHER: &str = "1";

/// An interface LAT could speak on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    /// Wireless, which is Ethernet enough to carry LAT but not where it is
    /// found: a segment with DEC equipment on it is a wired one. Offered, but
    /// never chosen for the user.
    pub wireless: bool,
}

/// The interfaces that could carry LAT: real Ethernet, and up.
///
/// There is no routing in LAT, so the one sharing a segment with the node is
/// the only one that will do. Where there is exactly one, it needs no saying.
pub fn interfaces() -> Vec<Interface> {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let mut found: Vec<Interface> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let at = |what: &str| {
                std::fs::read_to_string(entry.path().join(what))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            };
            // Loopback is another type, so it goes without saying, and a line
            // that is down carries nothing.
            if at("type") != ARPHRD_ETHER || at("operstate") != "up" {
                return None;
            }
            Some(Interface {
                name: entry.file_name().to_string_lossy().into_owned(),
                // Either of these marks a wireless interface; which one
                // depends on how old the driver is.
                wireless: entry.path().join("wireless").exists()
                    || entry.path().join("phy80211").exists(),
            })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// A service heard announcing itself, as a browser shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announced {
    /// The node offering it, which is what `--lat` wants.
    pub node: String,
    /// The service, often the node's own name.
    pub service: String,
    /// How willing the node is, recalculated from its load, so worth reading
    /// afresh rather than remembering.
    pub rating: u8,
    /// What the node says about itself, truncated to 64 characters by the
    /// sender.
    pub identification: String,
}

/// Listens for the services announcing themselves on one interface.
///
/// There is nothing to ask: a solicit built by hand has never been answered,
/// so a browser waits, and a node announces itself about once a minute. What
/// arrives, arrives.
#[derive(Debug)]
pub struct Browser {
    listener: Listener,
    frame: Vec<u8>,
}

impl Browser {
    /// Opens a socket to listen on, through the helper as a session does.
    pub fn open(interface: &str) -> io::Result<Browser> {
        Ok(Browser {
            listener: open(interface)?,
            frame: vec![0u8; 2048],
        })
    }

    /// Waits up to `timeout` for a node to announce itself.
    ///
    /// Gives back every service of the one that did, and nothing at all when
    /// the time runs out, which between announcements is most of the time.
    pub fn next(&mut self, timeout: Duration) -> io::Result<Vec<Announced>> {
        let n = self.listener.recv_timeout(&mut self.frame, timeout)?;
        let Some((_, payload)) = split(&self.frame[..n]) else {
            return Ok(Vec::new());
        };
        let Ok(Message::Announcement(a)) = vt_lat::parse(payload) else {
            return Ok(Vec::new());
        };
        Ok(a.services
            .iter()
            .map(|service| Announced {
                node: a.node.to_string(),
                service: service.name.to_string(),
                rating: service.rating,
                identification: service.identification.trim().to_string(),
            })
            .collect())
    }
}

/// The helper that holds `CAP_NET_RAW`, as it is installed and as it sits
/// beside a binary that has not been installed at all.
const HELPER: &str = "veetee-lat-helper";

/// Opens a LAT socket on `interface`, through the helper if this process has
/// no privilege of its own.
///
/// `vt-headless` run under `sudo` opens one directly and never spawns
/// anything; veetee opens no raw socket ever, and always goes the long way
/// round. Both end up with the same socket.
pub fn open(interface: &str) -> io::Result<Listener> {
    // The Flatpak has no raw sockets at all, and no helper either: its
    // seccomp filter refuses the address family outright, which is a poor
    // thing to hand a reader as "os error 97".
    if crate::pty::in_flatpak() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "LAT needs a raw Ethernet socket, which the Flatpak sandbox does not allow.              Install the package or the tarball instead.",
        ));
    }
    match Listener::open(interface) {
        // An interface that is not there is not something privilege fixes,
        // and the helper would only say so at greater length. Anything else
        // is worth a process to find out: refusing an address family, which
        // a sandbox does, reads nothing like being refused permission, and
        // matching on permission alone left the helper unasked.
        Err(missing) if missing.kind() == io::ErrorKind::NotFound => Err(missing),
        Err(refused) => from_helper(interface, &refused),
        opened => opened,
    }
}

/// Asks the helper for a socket and takes it over its standard input.
fn from_helper(interface: &str, refused: &io::Error) -> io::Result<Listener> {
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::process::{Command, Stdio};

    let (ours, theirs) = UnixStream::pair()?;
    let helper = helper_path();
    let mut child = Command::new(&helper)
        .arg(interface)
        // The socket goes back this way, so there is no descriptor to name
        // and nothing to keep open across the exec but the one.
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            // Not being allowed to open a socket is the ordinary case here
            // and no use saying: what matters is the helper that would have.
            // Never suggest raising veetee itself — GTK would then refuse to
            // start at all, which is a worse afternoon than this one.
            if e.kind() == io::ErrorKind::NotFound {
                io::Error::new(
                    e.kind(),
                    format!(
                        "LAT needs veetee-lat-helper to open a socket, and there is none at \
                         {0}.\nIn a build tree, build it:  cargo build -p vt-lat-helper\nThen \
                         grant it the capability:  sudo setcap cap_net_raw+ep {0}",
                        helper.display()
                    ),
                )
            } else {
                io::Error::new(
                    e.kind(),
                    format!("{refused}\nand {}: {e}", helper.display()),
                )
            }
        })?;

    match take_socket(&ours)? {
        Some(socket) => {
            // It has done its work and has nothing else to say.
            let _ = child.wait();
            Listener::adopt(socket, interface)
        }
        None => {
            // Whatever went wrong, the helper said so on its way out, and
            // what it says is more use than anything that could be said here.
            let mut why = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut why);
            }
            let _ = child.wait();
            let why = why.trim();
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                if why.is_empty() {
                    format!("{} gave back no socket", helper.display())
                } else {
                    format!(
                        "{why}\nGrant it with: sudo setcap cap_net_raw+ep {}",
                        helper.display()
                    )
                },
            ))
        }
    }
}

/// Takes the descriptor sent across a socket, if one came.
///
/// It travels as ancillary data beside a byte of nothing: a message with no
/// body at all is not a message, and would be the end of the socket instead.
fn take_socket(from: &std::os::unix::net::UnixStream) -> io::Result<Option<OwnedFd>> {
    use std::io::IoSliceMut;

    use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, recvmsg};

    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let mut byte = [0u8; 1];
    recvmsg(
        from,
        &mut [IoSliceMut::new(&mut byte)],
        &mut ancillary,
        RecvFlags::empty(),
    )?;
    Ok(ancillary.drain().find_map(|message| match message {
        RecvAncillaryMessage::ScmRights(fds) => fds.into_iter().next(),
        _ => None,
    }))
}

/// Where to look for the helper: beside the running program, where a build
/// tree and a tarball both put it, then where the package puts it, then
/// whatever the path turns up.
fn helper_path() -> std::path::PathBuf {
    use std::path::PathBuf;

    // Set by hand, which is how a build tree elsewhere is used.
    if let Some(named) = std::env::var_os("VEETEE_LAT_HELPER") {
        return PathBuf::from(named);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let beside = dir.join(HELPER);
        if beside.exists() {
            return beside;
        }
        // Installed, veetee in /usr/bin and the helper out of the way in
        // /usr/libexec, where a thing nobody runs by hand belongs.
        if let Some(prefix) = dir.parent() {
            let libexec = prefix.join("libexec").join(HELPER);
            if libexec.exists() {
                return libexec;
            }
        }
        // Nowhere to be found, so name where it was looked for first: an
        // error about a bare name is no help to anyone who has to act on it.
        return beside;
    }
    PathBuf::from(HELPER)
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
        let mut listener = open(&config.interface)?;
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

    /// How the session ended, for the terminal to show.
    fn ending(&self) -> &'static str {
        lock(&self.shared).session.ending()
    }
}

impl crate::Transport for Lat {
    fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        if self.unread() {
            return Ok(self.drain(buf));
        }
        if self.closed() {
            return Err(down(self.ending()));
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
            Err(down(self.ending()))
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

/// What a read gives once the session has gone, which is how a terminal is
/// told it is over. The words come from the session, which knows whether it
/// was the session that ended or the circuit that was taken down.
fn down(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, why.to_string())
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

    /// Sends a descriptor the way the helper does, so the receiving side can
    /// be tested without the capability the helper needs.
    fn hand_over(down: &std::os::unix::net::UnixStream, what: BorrowedFd<'_>) {
        use std::io::IoSlice;

        use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};

        let fds = [what];
        let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        assert!(ancillary.push(SendAncillaryMessage::ScmRights(&fds)));
        sendmsg(
            down,
            &[IoSlice::new(b"L")],
            &mut ancillary,
            SendFlags::empty(),
        )
        .expect("sending a descriptor");
    }

    #[test]
    fn a_socket_crosses_a_unix_socket_and_still_works() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;

        let (ours, theirs) = UnixStream::pair().expect("a socket pair");
        // Any descriptor will do to prove the passing: a pipe can be written
        // at one end and read at the other, which a socket cannot be on its
        // own, so what comes back can be shown to be the very same thing.
        let (read, mut write) = std::io::pipe().expect("a pipe");
        hand_over(&theirs, read.as_fd());
        drop(read);

        let got = take_socket(&ours)
            .expect("receiving")
            .expect("a descriptor came with it");
        write.write_all(b"MYI64").expect("writing to the pipe");
        drop(write);

        let mut said = String::new();
        std::fs::File::from(got)
            .read_to_string(&mut said)
            .expect("reading what came through");
        assert_eq!(said, "MYI64", "the descriptor is the one that was sent");
    }

    #[test]
    fn a_byte_with_nothing_attached_is_not_a_socket() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;

        let (ours, mut theirs) = UnixStream::pair().expect("a socket pair");
        theirs.write_all(b"L").expect("a byte on its own");
        assert!(
            take_socket(&ours).expect("receiving").is_none(),
            "a helper that sent nothing is not a helper that sent a socket"
        );
    }

    #[test]
    fn the_helper_is_looked_for_beside_this_program() {
        // Whatever is found, it is the helper being looked for, and an
        // override is honoured: that is all this can say without installing.
        unsafe { std::env::set_var("VEETEE_LAT_HELPER", "/nowhere/lat-helper") };
        assert_eq!(helper_path(), std::path::Path::new("/nowhere/lat-helper"));
        unsafe { std::env::remove_var("VEETEE_LAT_HELPER") };
        assert!(
            helper_path().to_string_lossy().contains(HELPER),
            "and it is the helper that is looked for"
        );
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
