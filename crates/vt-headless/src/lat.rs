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
pub fn lat(args: impl Iterator<Item = String>) -> io::Result<()> {
    use std::time::Duration;

    use vt_lat::Message;
    use vt_transport::lat::{Listener, split};

    let bad = |what: String| io::Error::new(io::ErrorKind::InvalidInput, what);
    let usage = "usage: vt-headless lat INTERFACE [SECONDS] [--connect NODE] [--type TEXT]";
    let mut interface = None;
    let mut seconds: Option<u64> = None;
    let mut wanted: Option<String> = None;
    let mut typing: Option<String> = None;
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--connect" => wanted = Some(rest.next().ok_or_else(|| bad(usage.into()))?),
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

    let mut frame = vec![0u8; 2048];
    // What we know of the circuit once one is open, so its messages can be
    // acknowledged: without that the far end repeats itself for ever.
    let mut circuit: Option<Circuit> = None;
    if let Some(node) = &wanted {
        ask_for_a_circuit(&mut listener, &mut frame, node)?;
    }
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
                if !c.calling && wanted.is_some() {
                    // The first message on a new circuit both acknowledges the
                    // agreement and asks for a service, which is what OpenVMS
                    // sends at this point. Their end is named first, as it is
                    // in every message on the circuit.
                    let data = vt_lat::session_start(c.to);
                    let open = vt_lat::Run {
                        flags: 2,
                        theirs: c.ours,
                        ours: c.theirs,
                        sequence: 1,
                        acknowledged: 0,
                        slots: vec![vt_lat::Slot {
                            to: 0,
                            from: 1,
                            control: vt_lat::SLOT_START,
                            data: &data,
                        }],
                    };
                    listener.send(source, &open.build())?;
                    circuit = Some(Circuit {
                        peer: source,
                        theirs: c.ours,
                        ours: c.theirs,
                        sequence: 1,
                        heard: 0,
                    });
                    println!("      circuit open; asking for {}", c.to);
                }
            }
            Ok(Message::Run(r)) => {
                // Say what we have heard, or it will be said again. The
                // acknowledgement is the highest sequence number from the far
                // end, and our own number counts up with every message sent.
                // A message with no slots carries nothing to acknowledge, and
                // answering one only draws another: the two ends will
                // acknowledge each other for ever.
                if let Some(c) = &mut circuit
                    && r.ours == c.theirs
                    && r.sequence != c.heard
                    && !r.slots.is_empty()
                {
                    c.heard = r.sequence;
                    c.sequence = c.sequence.wrapping_add(1);
                    let ack = vt_lat::Run::acknowledgement(c.theirs, c.ours, c.sequence, c.heard);
                    listener.send(c.peer, &ack)?;
                }
                // Type once there is something to type at: the far end sends
                // its prompt before it will read anything.
                if let (Some(text), Some(c)) = (&typing, &mut circuit)
                    && r.ours == c.theirs
                    && r.slots.iter().any(|slot| !slot.data.is_empty())
                {
                    let mut line = text.clone().into_bytes();
                    line.push(b'\r');
                    c.sequence = c.sequence.wrapping_add(1);
                    let typed = vt_lat::Run {
                        flags: 2,
                        theirs: c.theirs,
                        ours: c.ours,
                        sequence: c.sequence,
                        acknowledged: c.heard,
                        slots: vec![vt_lat::Slot {
                            to: 1,
                            from: 1,
                            control: 0, // data, against SLOT_START for the request
                            data: &line,
                        }],
                    };
                    listener.send(c.peer, &typed.build())?;
                    println!("      typed {text:?}");
                    typing = None;
                }
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

/// An open circuit, and enough of its state to keep it open.
#[cfg(target_os = "linux")]
struct Circuit {
    peer: [u8; 6],
    theirs: u16,
    ours: u16,
    sequence: u8,
    heard: u8,
}

/// Waits to hear a node announce itself, then asks it for a circuit.
///
/// Announcements are the only way we have of learning a node's address: a
/// solicit built by hand has never been answered, so this waits for one to
/// arrive of its own accord, which takes up to a multicast timer.
#[cfg(target_os = "linux")]
fn ask_for_a_circuit(
    listener: &mut vt_transport::lat::Listener,
    frame: &mut [u8],
    node: &str,
) -> io::Result<()> {
    use std::time::Duration;

    use vt_lat::{Message, Start};
    use vt_transport::lat::split;

    eprintln!("waiting for {node} to announce itself, which can take a minute");
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    let address = loop {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{node} did not announce itself"),
            ));
        }
        let n = listener.recv_timeout(frame, Duration::from_millis(500))?;
        let Some((source, payload)) = split(&frame[..n]) else {
            continue;
        };
        if let Ok(Message::Announcement(a)) = vt_lat::parse(payload)
            && a.node.eq_ignore_ascii_case(node)
        {
            break source;
        }
    };

    let start = Start {
        calling: true,
        theirs: 0, // we cannot name their end until they tell us
        ours: 0x1001,
        max_frame: 1500,
        version: (5, 3),
        keepalive: 20,
        to: node,
        from: "VEETEE",
    };
    println!("{}  asking {node} for a circuit", mac(address));
    listener.send(address, &start.build(listener.address()))
}

#[cfg(target_os = "linux")]
fn mac(address: [u8; 6]) -> String {
    address
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}
