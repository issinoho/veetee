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

## Getting two nodes to talk at all

Both of these cost an afternoon here, and neither announces itself: the symptom of each is
silence, which reads like a protocol that does not work.

**OpenVMS refuses connections it has not been told to allow.** A node that offers a service still
answers nothing if its connection setting says so. LATCP shows and sets it:

```
$ MCR LATCP SHOW NODE          ! among the rest: Connections: ...
$ MCR LATCP SET NODE/CONNECTIONS=BOTH
```

`BOTH` is incoming and outgoing; `INCOMING` is enough for a node that only offers services, and
`OUTGOING` for one that only calls out. Put it in `LAT$SYSTARTUP.COM` to survive a reboot.

**A virtual machine may never be given the multicast.** LAT announcements go to
`09-00-2B-00-00-0F`, and a guest on QEMU/KVM behind **macvtap** does not receive them by default:
the guest asks for the group, but libvirt does not pass that request to the host interface unless
it is told to trust the guest's filters. The guest hears nothing, announces into the void, and
looks like a node with LAT switched off.

On the host, to fix it now:

```sh
ip link set macvtap0 allmulticast on
```

and in the domain XML, so it survives a redefine:

```xml
<interface type='direct' trustGuestRxFilters='yes'>
```

The same applies to veetee itself in a guest: no announcements arrive, so the service browser
stays empty and `--lat NODE` waits for a node that never announces. It is the host's filtering,
not the wire.

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

The whole of a session has since been driven this way — a login, a 667-file `DIRECTORY`,
`SHOW TERMINAL` and `LOGOUT` — which is where most of what follows was read.

**A session ends with a slot of type 13 from slot 0, carrying nothing**, with a type 11 slot
holding a single `@` beside it. It is the last slot the host sends; after it OpenVMS acknowledges
for as long as it is asked to and says nothing more, so the circuit outlives the session and is
the caller's to take down. Seen twice: on `LOGOUT`, and when a login was timed out for want of a
password.

`SHOW TERMINAL` on the far end reports the name from the start message — `LAT Server/Port:
VEETEE` — which is one more field of it confirmed. Behind the terminal itself it reports:

```
Terminal: _LTA5052:   Device_Type: VT400_Series  Owner: TEST
   Input:    9600     Width:  80      Output:   9600     Page:   24
   Eightbit ... Soft Characters ... DEC_CRT  DEC_CRT2  DEC_CRT3  DEC_CRT4
```

So `SET TERMINAL/INQUIRE` at login identifies veetee as a VT420 over LAT, and sets the line up
for 8-bit controls and soft character sets. 🔎 Whether the page size comes from the slot that
asked for the service or from the cursor-position probe VMS makes at login is still not told
apart, both saying the same thing now that a real terminal answers the probe.

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
| 11 | `0xb0` | One byte, `@`. 🔎 Unread; it comes with the end of a session |
| 13 | `0xd1` | **The session is over.** From slot 0, with nothing in it |

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

**Credit is flow control, and running out of it stops a session dead.** A slot spends one of the
credits its sender has been granted; a node with none left sends nothing but acknowledgements. That
is not a deduction from the shape of the byte — veetee granted fifteen at the start of a session
and none afterwards, and MYI64 broke off in the middle of a word:

```
circuit 0x9001: 15 sent, 30 heard, 1 slot
      session 1 to 1  control 0x00  254 bytes  ... Hardware type: HP rx2660 ...       Software
circuit 0x9001: heard 31
circuit 0x9001: heard 32
```

It had more to send, it was still acknowledging, and it never sent another byte.

🔎 A grant **adds** to what the far end has rather than replacing it, because OpenVMS grants
zero on most of its slots and a session carries on through them. veetee grants the far end back up
to fifteen whenever it is down to seven, on a slot that is going anyway or on an empty one if
nothing is — a terminal has nothing to say for as long as the user is reading, and the host sends
empty slots for the same reason (`control 0x03` and `0x0f` with no data). How much a node ought to
grant, and whether anything but a slot spends credit, are not read.

🔎 veetee does not count its own credit, only what it has granted. Typing is a trickle beside a
listing, so nothing has run it out; pasting into a session might.

🔎 The byte that pads an odd-length slot is **not** zeroed: OpenVMS sent `0x25` in one
observed, which looks like whatever was in its buffer rather than anything meant. A reader should
ignore it, and a writer can send zero.

This reading parses every one of the 137 run messages in a session capture — login, a directory
listing, idle keepalives and logout — with nothing left over, which is the strongest evidence in
this document.

## Watching a session

A session that fails after an hour leaves nothing behind unless it was asked to. Name a file in
`VEETEE_LAT_TRACE` and every frame of every session goes into it:

```sh
VEETEE_LAT_TRACE=$HOME/lat.log veetee --lat MYI64
```

An environment variable rather than an option, because the long sessions worth watching are the
ones opened from the connection dialog, which no command line reaches.

Each line is milliseconds since the trace opened, a direction, and the frame:

```
        0 --  lat MYI64 on eth0, unix 1789430400, slot data left out
       12 out call ours=1a3f theirs=0000 to=MYI64 from=VEETEE keepalive=20s frame=1500
       47 in  agree ours=e001 theirs=1a3f to=VEETEE from=MYI64 keepalive=20s frame=1500
       48 out run seq=1 ack=3 ours=1a3f theirs=e001 [1->0 start credit=15 len=12]
      210 in  run seq=4 ack=1 ours=e001 theirs=1a3f [1->1 data credit=0 len=61]
      211 out ack seq=2 ack=4 ours=1a3f theirs=e001
    60048 sum in=412 out=196 acks=18/94 slots=102/3 bytes=8841/17 dup=0 rewind=0 missed=0 ...
```

`sum` is a line of running totals, written once a minute and once more as the session ends. The
numbers worth reading first:

| Number | Means |
|---|---|
| `missed` | messages the host sent that never arrived, counted from the gaps in its numbering |
| `dup` | messages that arrived twice, which is the host repeating itself for want of an acknowledgement |
| `rewind` | messages that arrived out of order |
| `credit=ours/theirs` | what each end has left to spend. `theirs` at nought and staying there is a session about to go quiet |
| `unacked` | messages sent and not acknowledged. Above one or two, the host has stopped listening |
| `kernel=in/dropped` | what the kernel took in for the socket, and what it threw away before veetee read it |

`kernel`'s second number is the one that tells a loss veetee inflicted on itself from a loss on
the wire, which a gap in the far end's numbering cannot: both look identical from here. It rises
in bursts — a screenful arriving faster than the reader drains it — so a `missed` that climbs
while `dropped` stays flat is the wire's doing, and one that climbs with it is veetee's. The
counters come from `getsockopt(SOL_PACKET, PACKET_STATISTICS)`, which clears them as it reads
them, so they are read every time round the loop and added up.

Slot contents are left out unless `VEETEE_LAT_TRACE_DATA` is set as well. **A trace with data in
it holds the password typed into the session**, in clear, exactly as the wire carries it.

## Not yet observed

- **What a slot of type 10 says.** Thirty-five bytes of terminal parameters, of which only the
  coded lines and columns are read. It goes both ways: the host sent one beside the login banner
  and again as it tried to set the terminal type.
- **The five slots a session opens with**: the `LTA` device name, a parameter block, three bytes,
  and two empty ones. Only the device name is read.
- **How much credit to grant, and what spends it.** That a slot spends one, and that running out
  stops a sender, is settled; the rest of the policy is veetee's own choice.
- **The flag bits** that make a run message `0x00`, `0x01` or `0x02`.
- **Stop (`0x0a`)** is seen once, eleven bytes, and not understood beyond its shape. veetee builds
  one from those bytes as it leaves, and OpenVMS does release the `LTA` device — but the sessions
  that were watched had been logged out of first, which is reason enough on its own for VMS to
  delete it. Leaving a session still logged in and letting veetee exit would separate the two.
- **`0x3c`**, sent by both nodes, carrying a node address and the same version and frame size as an
  announcement. It answers a solicit, on position alone.
- **Much of the start message**, including fourteen bytes before the address.
- **The 80 ms circuit timer, and retransmission.** Nothing was ever lost in a captured session,
  so no retransmission was ever seen, and veetee does none of its own: it keeps no copy of what it
  has sent, so typing lost on the wire is lost. What it does do is survive the host's losses. A
  gap in the host's numbering is counted as credit the host spent on a message that never arrived
  — without that, every loss leaves veetee's reckoning of the host's allowance one too high for
  good, and after eight or so the host runs out of credit and goes quiet with nothing said by
  either end. The allowance is also worked out from scratch every thirty-two messages, so any
  other way of losing count rights itself.

  **A message carrying nothing takes no sequence number.** Each end acknowledges only the messages
  that carry slots — neither acknowledges an acknowledgement — so a number spent on one is a number
  that will never be acknowledged, and the distance between what this end has sent and what the far
  end has acknowledged grows by one every keepalive, for ever, whatever else is happening. MYI64's
  `Queue Limit` is 24, and past that it stopped accepting anything veetee sent: typing went
  unacknowledged and unechoed, `SET TERM/INQUIRE` timed out into "unknown terminal type", and the
  terminal was dead a few minutes into every session, regardless of load, credit or loss. The count
  reached 133, 75 and 43 in three traces before it was read as anything but an artefact.

  **Which number is acknowledged and whether a frame is sent back are separate questions.** A node
  whose last message carried no slots waits to hear that number back before it sends anything
  else. Answering an acknowledgement directly draws another answer and goes on for ever, so veetee
  does not — but it must still take the number, or the wait never ends. It did not, and MYI64
  repeated `seq=86` every ten seconds for eleven minutes while veetee answered `ack=85`, both ends
  healthy, full credit either way, `LTA5074:` still `Online`, and the terminal frozen. The number
  now follows everything the far end sends and goes out with the next keepalive.

  🔎 Advancing the number past a gap tells the far end that something arrived when it did not, so
  anything lost stays lost. Holding it until the missing message is repeated is the stricter
  reading, and risks the same deadlock whenever what went missing is never repeated. Deadlock
  being much the worse failure, veetee advances.

  **The gap has to be measured across everything the host sends, its own acknowledgements
  included.** They carry a sequence number like any other message, and 1.1.0 followed the
  numbering of only the messages with slots in them — so it read every acknowledgement as a
  message lost. On a real session that came to 165 phantom losses, and each one bought the host
  credit it had never spent: 1361 granted against 107 received, sent as about 170 empty slots to
  carry 138 bytes of typing.

- **What credit costs, and what spends it.** That a slot spends one and that running out stops a
  sender are settled. Beyond that veetee assumes only session data with something in it spends an
  allowance, and it has to assume something: the slot asking for a service goes before the far end
  has granted anything, so a session could never open otherwise, and an empty slot is how credit
  itself is granted, so if those spent too then two ends that had both run out could never grant
  each other any. 🔎 Whether a real node reckons it the same way is unread.

  veetee now spends against what it has been granted rather than sending regardless, and holds
  typing when there is nothing left — but only for three seconds, after which it sends anyway.
  Holding strictly is the correct reading and the wrong behaviour: a host that stops granting
  would otherwise take the keyboard with it.

- **Deciding the far end has gone.** Each end has a keepalive timer and a retransmit limit so it
  can tell. OpenVMS gives up after eight retransmissions at an 80 ms circuit timer — about two
  thirds of a second — and releases the `LTA` device. veetee sent keepalives from the first and
  did none of the deciding, so a host that dropped the circuit left a terminal that stopped
  updating, kept clicking its keys, and never said why; one such session was still sending
  keepalives into nothing an hour later. Silence for a minute, three of the host's own keepalive
  intervals, now ends the session.

  All of this is reproduced in `crates/vt-lat/tests/soak.rs`, against a host that is not there —
  one that acknowledges as it goes and refuses to accept more than it granted, both of which an
  earlier version of that host did not do, which is why it missed all of the above.
