# LAT on the wire

What veetee knows about DEC's Local Area Transport, written from frames captured between two
OpenVMS systems. It is not a specification: it records what was observed, and says so where it
stops.

**Provenance.** Every byte here comes from packet captures of OpenVMS V8.4-2L3 on IA64 and
OpenVMS V9.2-3 on x86_64, both reporting LAT protocol version 5.3. No part of `latd` was read: it
is GPL and veetee is MIT OR Apache-2.0, so the implementation is clean-room (see
[CLAUDE.md](../CLAUDE.md)).

> **A LAT capture contains passwords.** There is no encryption anywhere in the protocol, and a
> login is carried as ordinary session data, so the first thing a capture of a real session holds
> is somebody's username and password in clear. Capture with an account you are willing to
> publish, and treat the files accordingly.

**Where to capture.** A switch floods multicast but forwards unicast only to its destination port,
so a third machine sees announcements and solicits and none of a session. Capture on one of the
two nodes, or on the interface of the virtual machine running one of them.

## The datalink

LAT rides directly on Ethernet. There is no IP, so nothing routes and both ends must share a
segment.

| | |
|---|---|
| EtherType | `0x6004` |
| Announcements and solicits | multicast to `09-00-2B-00-00-0F` |
| Replies and circuits | unicast |
| Frames | padded to the 60-byte Ethernet minimum |

**A receiver must join the multicast group.** A raw socket alone is not enough: the card filters
`09-00-2B-00-00-0F` out before the socket sees it. On Linux that is `PACKET_ADD_MEMBERSHIP` with
`PACKET_MR_MULTICAST` on an `AF_PACKET` socket. `tcpdump` sees the frames without it only because
it puts the interface in promiscuous mode, which is a trap worth remembering when a capture and an
application disagree about whether anything is arriving.

## Common header

Both message types observed start the same way.

| Offset | Bytes | Meaning |
|-------:|-------|---------|
| 0 | `28` / `38` | message type: `0x28` announcement, `0x38` solicit information |
| 1 | `08` / `00` | 🔎 constant per message type in every frame seen |
| 2–5 | `05 05 05 03` | protocol version, matching the 5.3 both nodes report |

Counted strings run throughout: one length byte, then that many characters. Node identifications
are truncated to 64 characters by the sender, which is why OpenVMS shows them cut short too.

## Service announcement (`0x28`)

Sent to the multicast group every 60 seconds, matching LAT's *multicast timer*.

```
28 08 05 05 05 03 | 54 ff | dc 05 | 3c | 02 | 01 | 01 |
05 "MYI64"                                    node name
40 " Welcome to VMS Software, Inc. …"         node identification, 64 characters
01                                            number of services
52                                            service rating
05 "MYI64"                                    service name
40 " Welcome to VMS Software, Inc. …"         service identification
01 01 01 08 01 10 | 80 1a | ac 93 50 27 | bc 00 | 00 17 a4 ab 62 51 | 00 00 00
```

| Field | Notes |
|---|---|
| offset 6–7 | 🔎 differs between frames from the same node (`20 7f` then `21 77`): a counter or timestamp |
| offset 8–9 | `dc 05` = 1500, the maximum frame size, matching the 1500 MTU |
| offset 10 | `3c` = 60, the multicast timer in seconds |
| offset 11 | 🔎 `02`, seen as `03` once on the first frame after a restart |
| rating | drifts frame to frame (`52`, `53`, `51`): the dynamic rating OpenVMS recalculates from load. A client choosing between nodes offering the same service picks on this, so it should be treated as live rather than cached |
| tail | 🔎 largely unread. It ends in a six-byte MAC address; on the x86 node that is its own source address, on the IA64 node it is one greater than the source address, so it is not simply the sender |

## Solicit information (`0x38`)

Sent to the multicast group when a node wants a service it has not heard announced. Observed
retransmitting every two seconds; OpenVMS's *retransmit limit* is 8 messages.

```
38 00 05 05 05 03 | dc 05 | 26 5a | 02 | 00 |
05 "MYI64"             the node asked about
01 01                  🔎 group mask: one byte, bit 0 set, matching "User Groups: 0"
06 "X86VMS"            the node asking
05 "MYI64"             the service wanted
00 …                   padded to 46 bytes of payload
```

| Field | Notes |
|---|---|
| offset 6–7 | `dc 05` = 1500 again |
| offset 8–9 | 🔎 a request identifier, presumably echoed by the response |
| offset 10–11 | 🔎 `02 00` in every frame seen |

A solicit is answered with **`0x3c`**, unicast, carrying the answering node's address along with
the same version and maximum frame size an announcement has. Both nodes send them while a
connection is being made — twelve of each in the session captured.

🔎 A solicit built to this shape by hand, differing from a real one only in the asking
node's name, has never been answered. Something in it is still unread.

## Circuits

A connection is a *virtual circuit* between two nodes, carrying one or more sessions. Each end
names the circuit with a two-byte identifier of its own choosing, and every later message carries
both: the other end's first, then its own.

### Start (`0x06`), and its reply (`0x04`)

```
06 00 | 00 00 | 01 50 | 00 | ff | dc 05 | 05 03 | 10 09 08 14 | 00 00 | 03 03 |
05 "MYI64"        the node being called
06 "X86VMS"       the node calling
00                🔎 an empty string
01 02 64 00 02 10 00 73 9b 3f a8 29 bc 00      🔎 unread
aa 00 04 00 01 04 | 00 00 00                   the caller's own address
```

The reply is the same shape with type `0x04`, and fills in the circuit identifier the answering
node has chosen: the caller sent `00 00` for a circuit it could not yet name, and the reply carries
`01 50` and `01 c0`, both ends now known.

| Field | Meaning |
|---|---|
| offset 2–3 | the other end's circuit identifier, zero when calling |
| offset 4–5 | this end's circuit identifier |
| offset 8–9 | `dc 05` = 1500, the maximum frame size |
| offset 10–11 | `05 03`, protocol version 5.3 |
| offset 15 | `14` = 20, the keepalive timer in seconds |

### Run (`0x00`, `0x01`, `0x02`)

Everything else rides in run messages, which carry the session data and the acknowledgements.

```
02 | 01 | 01 c0 | 01 50 | 03 | 03 | <slots>
^    ^     ^       ^      ^    ^
|    |     |       |      |    acknowledgement: the last sequence number heard
|    |     |       |      sequence number, counting up
|    |     |       this end's circuit identifier
|    |     the other end's
|    number of slots
message type
```

The three type bytes are one message with flags: the calling node sent `0x02` throughout, the
answering node `0x00` and `0x01`. 🔎 Which bit means what is unread.

**A message with no slots is an acknowledgement**, and doubles as the keepalive: with the circuit
idle, one goes out every ten to twenty seconds, eight bytes long and padded to the Ethernet
minimum.

### Slots

Each slot is a four-byte header and its data:

```
01 | 01 | 0e | 00 | "\r\n\r\nUsername: "
^    ^    ^    ^
|    |    |    🔎 credit in the high nibble, type in the low, on the evidence of the values seen
|    |    byte count
|    the sending session
the receiving session
```

**Data is padded to an even length**, and the whole message is then padded to the Ethernet
minimum with zeros.

This reading parses every one of the 137 run messages in a session capture — login, a directory
listing, idle keepalives and logout — with nothing left over, which is the strongest evidence in
this document.

## Not yet observed

- **The slot type and credit byte.** The values fall into two groups, small ones and ones with a
  high nibble set, which is why the split above is read as credit and type — but which type is
  which is unread.
- **The flag bits** that make a run message `0x00`, `0x01` or `0x02`.
- **Stop (`0x0a`)** is seen once, eleven bytes, and not understood beyond its shape.
- **`0x3c`**, sent by both nodes, carrying a node address and the same version and frame size as an
  announcement. It answers a solicit, on position alone.
- **Much of the start message**, including fourteen bytes before the address.
- **The 80 ms circuit timer**: retransmission was never provoked, because nothing was ever lost.
