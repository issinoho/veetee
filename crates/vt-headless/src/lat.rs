//! `vt-headless lat INTERFACE` — what LAT is saying on a wire.
//!
//! Discovery, and a way to see it without the application: nodes announce
//! every minute or so, and this prints each message as it arrives.

use std::io;

#[cfg(not(target_os = "linux"))]
pub fn lat(_args: impl Iterator<Item = String>) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "LAT needs raw Ethernet, which veetee only has on Linux",
    ))
}

#[cfg(target_os = "linux")]
pub fn lat(mut args: impl Iterator<Item = String>) -> io::Result<()> {
    use std::time::Duration;

    use vt_lat::Message;
    use vt_transport::lat::{Listener, split};

    let bad = |what: String| io::Error::new(io::ErrorKind::InvalidInput, what);
    let interface = args
        .next()
        .ok_or_else(|| bad("usage: vt-headless lat INTERFACE [SECONDS]".into()))?;
    let seconds: Option<u64> = match args.next() {
        Some(s) => Some(s.parse().map_err(|_| bad(format!("not a number: {s:?}")))?),
        None => None,
    };

    let mut listener = Listener::open(&interface).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("{e}\nLAT needs CAP_NET_RAW: try sudo, or setcap cap_net_raw+ep"),
        )
    })?;
    match seconds {
        Some(n) => eprintln!("listening on {interface} for {n} seconds"),
        None => eprintln!("listening on {interface}; announcements come about once a minute"),
    }

    let until = seconds.map(|n| std::time::Instant::now() + Duration::from_secs(n));
    let mut frame = vec![0u8; 2048];
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
                let what = if c.calling { "asks" } else { "agrees" };
                println!(
                    "{from}  {} {what} {} for a circuit: {} bytes, LAT {}.{}, keepalive {}s",
                    c.from, c.to, c.max_frame, c.version.0, c.version.1, c.keepalive
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
                    println!(
                        "      session {} to {}  {:3} bytes  {text}",
                        slot.from,
                        slot.to,
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
