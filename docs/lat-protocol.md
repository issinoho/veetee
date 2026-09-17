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

The two multicast message types start the same way.

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

### Response information (`0x3c`)

A solicit is answered with `0x3c`, unicast. It is an announcement with the answering node's
address in front of it:

```
3c 00 | 05 05 05 03 | dc 05 | 90 52 | 00 00 | 0e 00 |
00 17 a4 ab 62 50          the answering node's address
3c 00 00 01 01             `3c` = 60 again, the multicast timer
05 "MYI64"                 node name
40 " Welcome to VMS ..."   node identification
01 4e 01 01 03 | 52        🔎 then the rating, as an announcement has
01 01 | 05 "MYI64" | 40 " Welcome to ..."      the service
01 10 80 f2 b4 78 6a 29 bc 00 | 00 17 a4 ab 62 50 | 00 00 00   🔎 as an announcement's tail
```

Offsets 8–9 look like the identifier the solicit carried, echoed back.

A node that has been asked for a circuit is solicited by the far end for its own details: after
veetee opened a circuit to OpenVMS, OpenVMS solicited **veetee**, with an empty service name, and
repeated it. 🔎 What it does with the answer is unknown, and veetee has not sent one.

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

The acknowledgement is **the highest sequence number heard from the far end**, and a sender's own
number counts up with every message it sends, acknowledgements included. Until a message is
acknowledged it is sent again, so a client that never answers is told the same thing for ever.

Answer only what carries slots. Acknowledging an acknowledgement draws another back, and the two
ends will then acknowledge each other indefinitely.

### A session, end to end

Opening a circuit and asking for a service is enough to be given a terminal. OpenVMS answers with
five slots naming the device it has created — an `LTA` unit — then its login banner, then
`Username:`, and holds the circuit for as long as it is acknowledged. Left alone it ends the login
itself with *Error reading command input* and *Timeout period expired*, which is the ordinary
behaviour of a VMS terminal nobody types at.

Typing back is a slot like any other, with a control byte of `0x00` and the characters as data.
Sending a username produced the echo and then `Password:`, so a session carries traffic both ways
on the reading here. **The far end will not read before it has prompted**: a slot sent before it
says anything is ignored.

### Starting a session

The first run message the calling node sends carries a slot whose data names the service wanted:

```
01 01 fe | 05 "MYI64" | 00 01 02 04 00 05 10 "UIC_000200000101" 06 01 01 00
```

🔎 with a control byte of `0x9f`, against `0x00` for the slots that carry session data. Only
one such slot has been seen, so the layout around the name is unread.

### Slots

Each slot is a four-byte header and its data:

```
01 | 01 | 0e | 00 | "\n\r\n\rUsername: "
^    ^    ^    ^
|    |    |    the type in the high nibble, credit in the low
|    |    byte count
|    the sending session
the receiving session
```

**Data is padded to an even length**, and the whole message is then padded to the Ethernet
minimum with zeros.

**The type is the high nibble and the credit the low**, which a login to OpenVMS settles. Three
types have been seen:

| Type | Seen as | Carries |
|------|---------|---------|
| 0 | `0x00`, `0x01`, `0x03`, `0x0f` | Session data, in both directions |
| 9 | `0x9f` | A session starting: the service a caller wants, and the `LTA` device the host created for it |
| 10 | `0xa0`, `0xa1`, `0xaf` | Thirty-five bytes of terminal parameters, the coded page size among them |

The evidence is that the low nibble varies while the meaning does not. The same parameter block
arrived as `0xa0`, `0xa1` and `0xaf` in one session, and the login banner, the echo of a username
and a VMS error message came through as `0x00`, `0x01` and `0x0f` alike. A number that changes
while what it labels stays the same is a count, not a name — and it is the way round DEC documents
a LAT slot.

This was read the other way round at first, which cost real data: veetee showed the parameter
block on screen as thirty-five bytes of rubbish and dropped the banner OpenVMS prints after a
login, because both decisions turned on the wrong nibble.

🔎 veetee shows the terminal type 0 and nothing else, so a data slot of a type never seen
would be dropped rather than displayed.

🔎 **Credit is not understood.** veetee grants fifteen in the slot that asks for a service and
never grants any again, and a session survives a login and several commands that way. Whether a
long enough burst of output would stall for want of it is untested.

🔎 The byte that pads an odd-length slot is **not** zeroed: OpenVMS sent `0x25` in one
observed, which looks like whatever was in its buffer rather than anything meant. A reader should
ignore it, and a writer can send zero.

This reading parses every one of the 137 run messages in a session capture — login, a directory
listing, idle keepalives and logout — with nothing left over, which is the strongest evidence in
this document.

## Not yet observed

- **What a slot of type 10 says.** Thirty-five bytes of terminal parameters, of which only the
  coded lines and columns are read. It goes both ways: the host sent one beside the login banner
  and again as it tried to set the terminal type.
- **The slots either end of a session.** A session opens with five slots — the `LTA` device name,
  a parameter block, three bytes, and two empty ones — and ends with a slot **from session 0**
  carrying nothing, sent as VMS closes the login. The second is very likely the session ending,
  but only its position says so, so veetee does not act on it.
- **Credit**, as above: granted once and never again, with no sign yet of that being too few.
- **The flag bits** that make a run message `0x00`, `0x01` or `0x02`.
- **Stop (`0x0a`)** is seen once, eleven bytes, and not understood beyond its shape.
- **`0x3c`**, sent by both nodes, carrying a node address and the same version and frame size as an
  announcement. It answers a solicit, on position alone.
- **Much of the start message**, including fourteen bytes before the address.
- **The 80 ms circuit timer**: retransmission was never provoked, because nothing was ever lost.
