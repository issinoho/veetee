//! `vt-headless lat INTERFACE` — what LAT is saying on a wire, and a session
//! on it.
//!
//! Two things, and the interface is all they share. Without `--connect` it
//! prints every LAT message that arrives, which is how the protocol was read
//! and is still the way to watch a session frame by frame — a second copy of
//! this can listen while the first one connects. With `--connect NODE` it opens
//! a session through [`vt_transport::lat::Lat`], the same transport a terminal
//! uses, and prints what the host sends.

use std::io;

#[cfg(not(target_os = "linux"))]
pub fn lat(_args: impl Iterator<Item = String>) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "LAT needs raw Ethernet, which veetee only has on Linux",
    ))
}

#[cfg(target_os = "linux")]
pub fn lat(args: impl Iterator<Item = String>) -> io::Result<()> {
    let bad = |what: String| io::Error::new(io::ErrorKind::InvalidInput, what);
    let usage = "usage: vt-headless lat INTERFACE [SECONDS] \
                 [--connect NODE [--service NAME] [--type TEXT]]";
    let mut interface = None;
    let mut seconds: Option<u64> = None;
    let mut node: Option<String> = None;
    let mut service: Option<String> = None;
    let mut typing: Option<String> = None;
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--connect" => node = Some(rest.next().ok_or_else(|| bad(usage.into()))?),
            "--service" => service = Some(rest.next().ok_or_else(|| bad(usage.into()))?),
            "--type" => typing = Some(rest.next().ok_or_else(|| bad(usage.into()))?),
            _ if interface.is_none() => interface = Some(arg),
            _ => {
                seconds = Some(
                    arg.parse()
                        .map_err(|_| bad(format!("not a number: {arg:?}")))?,
                )
            }
        }
    }
    let interface = interface.ok_or_else(|| bad(usage.into()))?;
    match node {
        Some(node) => connect(interface, node, service, typing, seconds),
        None => listen(&interface, seconds),
    }
}

/// Opens a session and prints what the host sends, as a terminal would show it.
#[cfg(target_os = "linux")]
fn connect(
    interface: String,
    node: String,
    service: Option<String>,
    typing: Option<String>,
    seconds: Option<u64>,
) -> io::Result<()> {
    use std::io::Write;
    use std::time::{Duration, Instant};

    use vt_transport::Transport;
    use vt_transport::lat::{Lat, LatConfig};

    eprintln!("waiting for {node} to announce itself, which can take a minute");
    let mut session = Lat::connect(LatConfig {
        interface,
        node,
        service,
        // A service browser would have heard the address already; this has
        // only a name, so it waits for an announcement.
        address: None,
        from: None,
        rows: 24,
        cols: 80,
    })?;
    eprintln!("{}: circuit open", session.description());

    let mut writer = session.writer()?;
    if let Some(text) = &typing {
        // A DEC keyboard sends a carriage return for Return, and OpenVMS
        // submits a line on one: a line feed leaves it sitting at the prompt.
        // No waiting for the prompt either, since the session holds typing
        // back until the far end has spoken.
        let mut line = text.clone().into_bytes();
        line.push(b'\r');
        writer.write_all(&line)?;
        eprintln!("typed {text:?}");
    }
    // The host echoes what it is sent, so a terminal echoing as well shows
    // everything twice, and a DEC application wants each key as it is pressed
    // rather than a line at a time. Raw mode is both. It also hands Ctrl-C to
    // OpenVMS, where it belongs, which leaves the SECONDS argument as the way
    // out of here -- and the way that closes the circuit properly.
    if let Some(n) = seconds {
        eprintln!("raw mode: Ctrl-C goes to the host, and this ends after {n}s");
    } else {
        eprintln!("raw mode: Ctrl-C goes to the host, so kill this from elsewhere");
    }
    let _cooked = raw_mode();

    // Anything typed goes to the host, a password included -- which LAT
    // carries in clear on the wire in any case.
    std::thread::spawn(move || {
        use std::io::Read;
        let mut stdin = io::stdin().lock();
        let mut typed = [0u8; 256];
        while let Ok(n) = stdin.read(&mut typed) {
            if n == 0 {
                break;
            }
            for byte in &mut typed[..n] {
                if *byte == b'\n' {
                    *byte = b'\r';
                }
            }
            if writer.write_all(&typed[..n]).is_err() {
                break;
            }
        }
    });

    let until = seconds.map(|n| Instant::now() + Duration::from_secs(n));
    let mut buf = [0u8; 4096];
    loop {
        if until.is_some_and(|end| Instant::now() >= end) {
            // On a line of its own, and with the carriage returns written out:
            // the host may have left the cursor anywhere, and raw mode has
            // stopped the terminal adding one to a line feed.
            eprint!("\r\nclosing the circuit\r\n");
            return Ok(());
        }
        match session.read_timeout(&mut buf, Duration::from_millis(500)) {
            Ok(0) => {}
            Ok(n) => {
                io::stdout().write_all(&buf[..n])?;
                io::stdout().flush()?;
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                eprint!("\r\n{e}\r\n");
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }
}

/// Puts the terminal into raw mode for as long as a session lasts.
///
/// Gives back what it was, so a shell is not left with its echo off however
/// the session ends.
#[cfg(target_os = "linux")]
fn raw_mode() -> Cooked {
    use rustix::termios::{OptionalActions, isatty, tcgetattr, tcsetattr};

    let stdin = io::stdin();
    // A pipe is not a terminal and has nothing to put into raw mode, which is
    // a fair way to drive this: echo SYSTEM | vt-headless lat ... --connect.
    if !isatty(&stdin) {
        return Cooked(None);
    }
    let Ok(cooked) = tcgetattr(&stdin) else {
        return Cooked(None);
    };
    let mut raw = cooked.clone();
    raw.make_raw();
    match tcsetattr(&stdin, OptionalActions::Now, &raw) {
        Ok(()) => Cooked(Some(cooked)),
        Err(_) => Cooked(None),
    }
}

/// What the terminal was before the session, put back when it ends.
#[cfg(target_os = "linux")]
struct Cooked(Option<rustix::termios::Termios>);

#[cfg(target_os = "linux")]
impl Drop for Cooked {
    fn drop(&mut self) {
        use rustix::termios::{OptionalActions, tcsetattr};

        if let Some(cooked) = &self.0 {
            // Nothing useful to do if it fails, and `reset` is the answer.
            let _ = tcsetattr(io::stdin(), OptionalActions::Now, cooked);
        }
    }
}

/// Prints every LAT message on the wire, whoever it is for.
#[cfg(target_os = "linux")]
fn listen(interface: &str, seconds: Option<u64>) -> io::Result<()> {
    use std::time::Duration;

    use vt_lat::Message;
    use vt_transport::lat::{Listener, split};

    let mut listener = Listener::open(interface)?;
    match seconds {
        Some(n) => eprintln!("listening on {interface} for {n} seconds"),
        None => eprintln!("listening on {interface}; announcements come about once a minute"),
    }

    let mut frame = vec![0u8; 2048];
    let until = seconds.map(|n| std::time::Instant::now() + Duration::from_secs(n));
    loop {
        if until.is_some_and(|end| std::time::Instant::now() >= end) {
            return Ok(());
        }
        let n = listener.recv_timeout(&mut frame, Duration::from_millis(500))?;
        let Some((source, payload)) = split(&frame[..n]) else {
            continue;
        };
        let from = mac(source);
        match vt_lat::parse(payload) {
            Ok(Message::Announcement(a)) => {
                println!(
                    "{from}  {} offers {} service{}, up to {} bytes, every {}s",
                    a.node,
                    a.services.len(),
                    if a.services.len() == 1 { "" } else { "s" },
                    a.max_frame,
                    a.multicast_timer
                );
                for service in &a.services {
                    println!(
                        "      {:<16} rating {:<4} {}",
                        service.name,
                        service.rating,
                        service.identification.trim()
                    );
                }
            }
            Ok(Message::Solicit(s)) => {
                println!("{from}  {} asks for {}/{}", s.from, s.node, s.service);
            }
            Ok(Message::Start(c)) => {
                // The names keep the sense they were sent with: the node
                // called and the node calling, whichever way the message goes.
                let line = if c.calling {
                    format!("{} asks {} for a circuit", c.from, c.to)
                } else {
                    format!("{} agrees a circuit with {}", c.to, c.from)
                };
                println!(
                    "{from}  {line}: {} bytes, LAT {}.{}, keepalive {}s",
                    c.max_frame, c.version.0, c.version.1, c.keepalive
                );
            }
            Ok(Message::Run(r)) => {
                if r.slots.is_empty() {
                    // An idle circuit says only what it has heard.
                    println!("{from}  circuit {:#06x}: heard {}", r.ours, r.acknowledged);
                    continue;
                }
                println!(
                    "{from}  circuit {:#06x}: {} sent, {} heard, {} slot{}",
                    r.ours,
                    r.sequence,
                    r.acknowledged,
                    r.slots.len(),
                    if r.slots.len() == 1 { "" } else { "s" }
                );
                for slot in &r.slots {
                    let text: String = slot
                        .data
                        .iter()
                        .map(|&c| {
                            if (32..127).contains(&c) {
                                c as char
                            } else {
                                '.'
                            }
                        })
                        .collect();
                    // The control byte is the one field of a slot still being
                    // read, and what veetee shows the terminal turns on it, so
                    // it is worth having in front of the data it goes with.
                    println!(
                        "      session {} to {}  control {:#04x}  {:3} bytes  {text}",
                        slot.from,
                        slot.to,
                        slot.control,
                        slot.data.len()
                    );
                }
            }
            Ok(Message::Stop(c)) => {
                println!("{from}  circuit {:#06x} closed", c.theirs);
            }
            Ok(Message::Other { kind, body }) => {
                // Worth seeing: several message types have never been seen at
                // all, and one arriving is how we would learn it exists.
                println!("{from}  message type {kind:#04x}, {} bytes", body.len());
            }
            Err(e) => println!("{from}  unreadable: {e:?}"),
        }
    }
}

#[cfg(target_os = "linux")]
fn mac(address: [u8; 6]) -> String {
    address
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}
