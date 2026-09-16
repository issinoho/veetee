# LAT on the wire

What veetee knows about DEC's Local Area Transport, written from frames captured between two
OpenVMS systems. It is not a specification: it records what was observed, and says so where it
stops.

**Provenance.** Every byte here comes from a packet capture taken on a Linux machine sharing a
switch with the two nodes, of OpenVMS V8.4-2L3 on IA64 and OpenVMS V9.2-3 on x86_64, both
reporting LAT protocol version 5.3. No part of `latd` was read: it is GPL and veetee is
MIT OR Apache-2.0, so the implementation is clean-room (see [CLAUDE.md](../CLAUDE.md)).

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

**No response has been captured.** A solicit built to this shape, differing only in the asking
node's name, went unanswered by two separate OpenVMS nodes with `Service Responder` enabled — and
the one real solicit we captured, from OpenVMS itself, was never answered either. So this format
is known only from messages that failed, and something in it is unread. The response message type
is unknown.

## Not yet observed

Everything needed to hold a session:

- the circuit messages — start, run, stop — and their sequence and acknowledgement numbers
- slots within a run message: data, credit, start and stop
- the credit scheme that paces a session
- keepalives, which OpenVMS times at 20 seconds, and the 80 ms circuit timer
- the response to a solicit

Capturing them needs two LAT nodes that can see each other. On the network this was written from,
the two OpenVMS nodes each list only their own service, so no connection between them could be
made — which is a fault on those hosts rather than anything about the protocol, but it does mean
this document stops at discovery.
