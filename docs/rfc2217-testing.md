# Testing RFC 2217 against a terminal server

veetee can set up the serial line behind a terminal server — speed, word size, parity, stop bits,
flow control and a line break — with RFC 2217 (Telnet COM Port Control). It is written, and its
tests assert that the bytes on the wire match the RFC, which is a weaker claim than it sounds:
**none of it has ever met real hardware.** This page is for somebody who has some.

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
  plug.

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
