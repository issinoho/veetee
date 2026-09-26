# Testing RFC 2217 against a terminal server

veetee can set up the serial line behind a terminal server — speed, word size, parity, stop bits,
flow control and a line break — with RFC 2217 (Telnet COM Port Control).

**It has since been proved against `ser2net`**, which answered every setting with the same value
it was asked for; see [what was proved](#what-was-proved). What has not been tried is a terminal
server — a DECserver, a Lantronix, a Moxa — and whether one speaks the option at all. This page is
for somebody who has one.

Everything here is a question rather than an instruction. A result of "the server refused the
option" is as useful as a result of "it worked", and far more useful than no result.

Please read [What veetee does not do](#what-veetee-does-not-do) before starting, so that a gap is
not mistaken for a fault.

## What you need

- veetee 0.8.7 or later, on Linux or Windows. *About veetee* in the window menu says which.
- A terminal server that speaks Telnet on a TCP port for one of its serial ports: a DECserver,
  Lantronix, Moxa NPort, Digi, or anything else of the kind.
- Something on the far end of that serial line whose speed you can change or know — a VAX or Alpha
  console, a PDP-11, a modem, a second computer running `screen` or `picocom`, even a loopback
  plug. For the `ser2net` route below, **nothing at all**: see
  [what to plug in](#what-to-plug-the-cable-into).

## Which servers speak it

**This project does not know, and that is part of what is being asked.** No list of models is
given here because none can be given honestly: nothing has been tested, and a wrong model number
would send somebody looking for hardware that cannot answer.

What can be said:

- **DEC's own terminal servers probably do not.** RFC 2217 is from October 1997, and the DECserver
  line is older; their Telnet servers pass bytes, and a port's speed is set on the server itself
  (`SET PORT n SPEED 9600` and the like) rather than by the client. Nobody has shown one answering
  option 44. If yours does, that is a genuinely new fact — please say so.
- **Vendors that document the option** include Moxa, whose NPort devices have an RFC 2217 mode by
  that name, and some Lantronix and Digi devices. That is from their documentation rather than
  from testing here.
- **You do not need a terminal server at all.** `ser2net` on Linux exposes a serial port over
  Telnet and implements RFC 2217, so a spare machine and a USB serial adapter make a test bed for
  the price of the adapter. A result from `ser2net` is worth having: it proves veetee against a
  known-good implementation, which is the other half of the question.

Test 1 below settles it for whatever you have, and "it refuses the option" is a complete answer.

## With ser2net and a USB serial adapter

This wants no terminal server and no far-end machine, and it answers the central question more
directly than hardware does: **`stty` shows what the line was actually set to.** A DECserver can
only be watched from a distance; a local serial port can be asked.

Install `ser2net` and give it the adapter. Version 4, which is what current distributions ship
(`ser2net -v` says), reads `/etc/ser2net.yaml`:

```yaml
connection: &usb
  accepter: telnet(rfc2217),tcp,localhost,4001
  connector: serialdev,/dev/ttyUSB0,9600n81,local
```

`telnet(rfc2217)` is the part that matters: without it ser2net serves plain Telnet and refuses the
option, which is worth trying too — it is test 2 from the other side.

**Pick a port the packaged configuration does not already claim.** Debian and Ubuntu ship an
`/etc/ser2net.yaml` with connections on **2000, 2001, 3000 and 3001**, wired to `/dev/ttyS0` and
`/dev/ttyS1`. Adding a second connection on one of those does not replace it: the packaged one
answers, tries to open a serial port the machine has not got, and drops the connection with
`Device open failure: Internal I/O error` — which reaches a client as `Connection reset by peer`,
and looks for all the world like a fault in the client. 4001 is out of the way.

Version 3 uses `/etc/ser2net.conf` and one line per port, where the option is enabled by adding
`remctl`:

```
2001:telnet:600:/dev/ttyUSB0:9600 8DATABITS NONE 1STOPBIT remctl
```

**Restart ser2net after editing** — `sudo systemctl restart ser2net` — or the running one keeps
the configuration it started with. A stale one still accepts the connection and then drops it,
which veetee reports as `Connection reset by peer`; `ser2net -n -d -c /etc/ser2net.yaml` in the
foreground says what it is really doing.

Then ask for something distinctive and watch the wire. **`stty` cannot be used for this**, which
was the first thing tried here: ser2net opens the port exclusively, so nothing else can read its
settings while a session is up, and it puts them back when the session ends. The conversation is
the evidence, and it is better evidence, since it shows what the server said as well as what
veetee asked.

All of it in one terminal — `tcpdump` writes a capture in the background, veetee holds the
foreground until its window is closed:

```sh
telnet localhost 4001              # ten seconds well spent: if this drops, so will veetee
sudo -v                            # or the backgrounded sudo waits for a password nobody types
sudo tcpdump -i lo -n -w /tmp/rfc2217.pcap 'tcp port 4001' 2>/dev/null &
veetee --telnet localhost:4001 -b 19200 -d 7 -p e -s 2 -f h
#   ... close the window, and then:
sudo pkill tcpdump
tcpdump -r /tmp/rfc2217.pcap -n -X | tail -50
```

Write a capture file rather than piping `-X` to one: `tcpdump` block-buffers its text output, so a
plain redirection loses the end of the conversation — which is exactly the part with the answers
in it.

The `telnet` first tells a configuration problem from anything worth reporting. A stale or
misconfigured ser2net accepts the connection and drops it, which reaches a client as
`Connection reset by peer` and looks for all the world like a fault in the client.

The second `stty` should report `speed 19200 baud`, `cs7`, `parenb -parodd`, `cstopb` and
`crtscts`. Anything veetee asked for that is not there is a fault worth reporting, and anything
ser2net set differently is test 8 answered on the spot.

Work through the settings one at a time — `-b 1200`, `-b 115200`, `-p o`, `-d 8 -s 1`, `-f x`,
`-f n`. From 1.6.0 there is no need to reconnect: leaving Communications Set-Up with new settings
sends them again, as does DECSCS, DECSPP or DECSFC from the host (docs/serial-setup.md).

A capture is easy here too, the whole conversation being on the loopback interface:

```sh
sudo tcpdump -i lo -w rfc2217.pcap tcp port 4001
```

### What was proved

Against `ser2net` 4 on Ubuntu, with an FTDI USB adapter and nothing on the far end, on
17 September 2026. veetee asked for 19200, 7 data bits, even parity, 2 stop bits and RTS/CTS
(`-b 19200 -d 7 -p e -s 2 -f h`), and the whole conversation was:

```
veetee → ff fb 2c                          WILL COM-PORT-OPTION
ser2net→ ff fd 2c                          DO
veetee → ff fa 2c 01 00 00 4b 00 ff f0     SET-BAUDRATE 19200
         ff fa 2c 02 07 ff f0              SET-DATASIZE 7
         ff fa 2c 03 03 ff f0              SET-PARITY even
         ff fa 2c 04 02 ff f0              SET-STOPSIZE 2
         ff fa 2c 05 03 ff f0              SET-CONTROL hardware
ser2net→ ff fa 2c 6f ff ff ff f0           MODEMSTATE-MASK IS 0xff   (unasked for)
         ff fa 2c 6b 00 ff f0              NOTIFY-MODEMSTATE 0       (unasked for)
         ff fa 2c 65 00 00 4b 00 ff f0     BAUDRATE IS 19200
         ff fa 2c 66 07 ff f0              DATASIZE IS 7
         ff fa 2c 67 03 ff f0              PARITY   IS even
         ff fa 2c 68 02 ff f0              STOPSIZE IS 2
         ff fa 2c 69 03 ff f0              CONTROL  IS hardware
```

Every setting came back as it was sent, including the four-byte speed, which is the encoding most
likely to be got wrong and the hardest to check without a server to answer.

Two things worth taking from it beyond "it works":

- **A server may send notifications nobody asked for.** ser2net sent the modem-state mask and a
  modem-state notification unbidden. veetee ignores both, as it says it does, and the session was
  unaffected — so that gap costs nothing against this implementation.
- **`telnet` is enough to see it.** No terminal server was needed to prove the client side. What a
  terminal server would answer is a different question: whether it speaks the option at all.

Tests 5 and 6 — flow control and break — were not done, having no far end to feel them.

### Changing the line while connected

Against `ser2net` 4.6.5 on 26 September 2026, with no adapter at all: a `socat` pseudo-terminal
pair stood in for the serial port (`connector: serialdev,<pty>,9600n81,local`), and a second
`socat -x -v` between veetee and the server logged the conversation, so no `sudo` was needed.
veetee's Telnet transport connected at the factory 9600 8N1 and then changed the line four times
without reconnecting, as leaving Set-Up does:

| Asked for | ser2net answered |
|---|---|
| 19200 7E2, RTS/CTS | 19200, 7, even, 2, hardware |
| 1200 8O1, XON/XOFF transmit only | 1200, 8, odd, 1, XON/XOFF |
| 115200 8N1, XON/XOFF receive only | 115200, 8, none, 1, XON/XOFF |
| 9600 8N1, XON/XOFF | 9600, 8, none, 1, XON/XOFF |

Every setting was taken while connected. The one-way flow controls — a VT420 set to *No XOFF*
stops at the host's XOFF but sends none — are asked for as XON/XOFF both ways, because the first
attempt found that ser2net does not keep the directions apart: asked for XON/XOFF and then the
RFC's *inbound* none (`SET-CONTROL 14`), it answered `CONTROL IS none`, both ways off; asked for
none and then inbound XON/XOFF (`15`), it left none. A local serial port sets each direction
exactly, through termios.

### What to plug the cable into

**Nothing, for the test above.** The adapter goes in a USB port and its serial end can hang free.
ser2net opens `/dev/ttyUSB0` and applies what veetee asks to the port; `stty` reads it back out of
the driver. No byte has to arrive anywhere for that to be a real answer, because the question is
whether the settings reach the line, not whether something is listening on it.

Connecting something buys more, in order of effort:

| On the far end | What it adds |
|---|---|
| Nothing | Tests 1, 2, 7, 8 and every setting check. Most of this page |
| A loopback plug | Typing echoes back, so data is shown flowing at the settings that were asked for |
| A second machine, or a real device | A wrong speed shows as rubbish; break and flow control become testable (tests 5 and 6) |

A loopback plug is a DB9 with **pins 2 and 3 joined** — a paperclip will do. Add **7 to 8**
(RTS to CTS) if you want to try `-f h` without a stall, since hardware flow control waits on CTS,
and **4 to 6** (DTR to DSR) for completeness.

One caveat on `-f h` with nothing attached: `stty` will still show `crtscts`, because the request
did reach the driver. Whether the wires then do anything is a separate question, and needs wires.

## How veetee speaks it

The option is offered **only when line settings are given**. An ordinary Telnet connection never
mentions it, deliberately: raising an option uninvited caused a real bug in 0.8.4, and a terminal
server is the only thing that can answer this one.

```sh
veetee --telnet decserver:2001 -b 9600 -d 8 -p n -s 1 -f x
```

The long forms are `--baud`, `--databits`, `--parity`, `--stopbits` and `--flow`. Parity is
`n`, `o`, `e`, `m` or `s`; flow is `n` (none), `x` (XON/XOFF) or `h` (RTS/CTS). Defaults are DEC's
factory Set-Up values: 9600, 8 data bits, no parity, 1 stop bit, XON/XOFF.

On the wire that means:

1. In the opening hello, `IAC WILL 44` alongside the usual terminal type and window size.
2. If the server answers `IAC DO 44`, five subnegotiations at once, in this order:

   | | Command | Data |
   |---|---|---|
   | Speed | `SET-BAUDRATE` (1) | four bytes, most significant first |
   | Word size | `SET-DATASIZE` (2) | 5, 6, 7 or 8 |
   | Parity | `SET-PARITY` (3) | 1 none, 2 odd, 3 even, 4 mark, 5 space |
   | Stop bits | `SET-STOPSIZE` (4) | 1 or 2 |
   | Flow control | `SET-CONTROL` (5) | 1 none, 2 XON/XOFF, 3 hardware |

3. **F5 sends a real line break**: `SET-CONTROL` with 4 (break on), then 5 (break off), so the
   server holds the line down for the gap between them. Without the option F5 sends a Telnet
   `BRK` instead.

A server answers each command with the command plus 100 — `101` for the speed it actually set, and
so on. **veetee ignores those replies.** See below.

## The tests

Take a packet capture of each if you can: `tcpdump -i any -w test1.pcap tcp port 2001`, or
Wireshark, which decodes the option by name. ⚠️ **A capture holds everything typed, passwords
included.** Look before sending one on, or redact it.

### 1. Does the server take the option at all?

```sh
veetee --telnet decserver:2001 -b 9600
```

In the capture, find `IAC WILL 44` from veetee and look for the answer.

- `IAC DO 44` — it speaks RFC 2217. Carry on.
- `IAC DONT 44`, or no answer — it does not. **Stop here and say so**, with the make, model and
  firmware version: that is the result. veetee should then behave as an ordinary Telnet client
  and the session should work normally, which is worth confirming.

### 2. Does an ordinary connection stay quiet?

```sh
veetee --telnet decserver:2001
```

With no line settings, veetee must not mention option 44 at all. If `WILL 44` appears in this
capture, that is a bug — please report it.

### 3. Is the speed actually set?

Set the far-end device to a known speed, then connect at the **wrong** one:

```sh
veetee --telnet decserver:2001 -b 1200     # device is at 9600
```

Expect rubbish, or nothing. Then connect at the right one and expect clean text. The point is that
the speed veetee asked for is the speed the line ran at, which nothing but the far end can show.

Try a rate outside the common set if the server allows it — 76800, or 115200 — since the four-byte
encoding is where an implementation is most likely to differ.

### 4. Word size, parity and stop bits

Same again, one at a time: set the far end to 7E1 and connect as 7E1, then as 8N1. 8N1 against a
7E1 device usually shows the text with the top bit set or every eighth character wrong, rather
than nothing, which is a good sign the parity setting reached the line.

### 5. Flow control

With `-f x`, send enough output to fill a buffer — `TYPE` a long file on the far end, or `ls -R /`
— and press **F1 (Hold Screen)**. The line should stop, and start again when F1 is pressed a
second time, without losing characters in the middle. With `-f n` on both ends, expect dropped
characters instead: that is the control working.

`-f h` asks for RTS/CTS, which needs the cable to have those wires.

### 6. Break

Connect to something that reacts to a break — a VAX or Alpha console, where break halts the
machine, or a Linux console with magic SysRq.

Press **F5**. Expect the far end to see a genuine line break. In the capture, expect
`SET-CONTROL 4` then `SET-CONTROL 5`.

⚠️ On a VAX or Alpha console **a break halts the machine.** Use one you can afford to halt, or a
device where a break is harmless.

### 7. A saved connection

Set the line up, then *Save as Connection…* in the window menu. Open it again from *Connections…*
and confirm the settings are still applied — they are kept in the same `profiles.toml` keys a
serial connection uses.

### 8. What the server says back

In the capture, find the server's answers (`101`, `102`, `103`, `104`, `105`) and compare each
with what veetee asked for. A server is allowed to answer with something different — the nearest
speed it supports, say.

If they differ, **that is the most valuable result on this page**, because veetee neither checks
nor reports it: it will tell you the line is 76800 when the server settled on 57600.

### 9. A line that goes away

With a session open, unplug the serial cable or turn off the attached device.

veetee is expected **not to notice**: it does not subscribe to the notifications that would tell
it. What matters is what it does instead — a frozen window is acceptable, a crash or a busy loop
is not. Please say which.

## What veetee does not do

None of these are faults to report; they are known, and knowing whether they matter in practice is
part of what this testing is for.

- **It ignores the server's replies.** What veetee shows and saves is what it asked for, not what
  the server set. Test 8 is about how far apart those can be.
- **It does not subscribe to `NOTIFY-LINESTATE` or `NOTIFY-MODEMSTATE`**, so a dropped line, a lost
  carrier or a break *from* the far end go unnoticed.
- **It does not send `SIGNATURE`, `PURGE-DATA` or `SET-LINESTATE-MASK`**, and does not ask the
  server for its current settings before setting them.
- **It sets the line once**, when the connection opens. There is no way to change the speed
  mid-session without reconnecting.
- **Only RTS/CTS is offered as hardware flow control** — DTR/DSR is not.

## Reporting back

Please open an issue at <https://github.com/issinoho/veetee/issues> with:

- the make, model and firmware version of the terminal server, and what was on the serial line;
- the exact `veetee` command for each test, and what happened;
- the captures, if you are able to share them — **checked for passwords first**;
- anything that surprised you, including things that worked.

A report saying only that a particular server refuses option 44, and that a plain Telnet session
works normally, is complete and useful: it settles a question that has been open since 0.8.7, and
would let this move out of [ROADMAP.md](ROADMAP.md) one way or the other.
