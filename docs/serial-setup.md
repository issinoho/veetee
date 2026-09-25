# Serial line settings from Set-Up

A plan, written 25 September 2026, for the 1.6.0 feature: the Communications Set-Up settings
drive the serial line, as they do on the terminal. The decisions were settled as recommended the
same day, and S1 to S3 are built; S4's tests are written, and the hardware test is to come.

## Why

On a VT420 there is nowhere else to set the line: the Communications Set-Up screen holds the
transmit and receive speeds, the data format, the stop bits and XOFF, and leaving Set-Up puts
them on the line. On the VT500 series the host can set them too, with DECSCS (speed), DECSPP
(data bits, parity, stop bits) and DECSFC (flow control) (EK-VT520-RM; 🔎 section to cite when
built).

veetee keeps all of these — shown, saved, restored, reported to DECRQSS — and acts on none of
them. A serial line takes its settings from the command line (`-b -d -p -s -f`, picocom style)
or from the saved connection, and Set-Up goes on showing 9600 whatever the line runs at. It is
the first entry under *Stored only* in [`compat-matrix.md`](compat-matrix.md), and the first of
the limitations 1.0 shipped with.

## Where it stood

Before S1 to S3:

| Part | State |
|---|---|
| Set-Up Comm screen (VT420) and Communication menu (VT500) | Transmit and receive speed, data format, stop bits, XOFF, VT500 transmit and receive flow control: kept and saved, acting on nothing |
| DECSCS, DECSPP, DECSFC from the host | Validated, stored and reported; the line does not change |
| `--serial DEVICE` | Opens at DEC's factory settings (9600 8N1, XON/XOFF), or what `-b -d -p -s -f` say |
| A saved serial connection | Keeps its own line settings, chosen in the Connections window |
| `--telnet` with line options (RFC 2217) | Sends them to the terminal server once, at connect |
| Changing the line while connected | Not possible |

## The plan

### S1. The line from Set-Up, at connect

A function from Set-Up's features to the line settings, in `veetee` (`vt-core` knows nothing of
serial ports, and `vt-transport` nothing of Set-Up):

| Set-Up | Line |
|---|---|
| Transmit speed | Baud rate |
| Data format: 8 bits, or 7 bits | 8 or 7 data bits |
| Parity: none, even, odd, mark, space; even and odd *unchecked* | The same; unchecked sends parity and does not check it — as checked does today, see decision 5 |
| Stop bits | 1 or 2 |
| XOFF at 64 or 128 (VT420), receive flow control XON/XOFF (VT500) | The port sends XOFF when its buffer fills (termios `IXOFF`) |
| Transmit flow control XON/XOFF (VT500; always on for the VT420) | The port stops at the host's XOFF (termios `IXON`) |
| DSR, DTR flow control (VT500) | Hardware flow control — see decision 3 |
| Receive speed, XOFF threshold | Not applied — see decision 4 |

`--serial DEVICE` then opens with the session's Set-Up settings, which at power-up are the saved
ones. Options on the command line, or a saved connection's own settings, win for that
connection, and are put into Set-Up's current values so that Set-Up shows what the line is
doing (decision 1).

### S2. Changing the line while connected

A new `TransportWriter::set_line`, for the serial port (termios on Linux, `SetCommState` on
Windows) and for Telnet with COM Port Control (the RFC 2217 SET-BAUDRATE, SET-DATASIZE,
SET-PARITY, SET-STOPSIZE and SET-CONTROL it already sends at connect). The session compares
the line settings after leaving Set-Up and after the host's input, and when they differ, sets the
line: so a change in Set-Up, and DECSCS, DECSPP or DECSFC from the host, take effect at once.

### S3. Showing it

The window title and the Connections window's description (`/dev/ttyUSB0 19200 7E1`) follow
the line, and a line the port refuses — a speed the adapter cannot do — is said in the status
bar, with the line left as it was.

### S4. Acceptance

Written: the termios and DCB computed for the one-way flow controls; `set_line` on an open port
(a pty standing in) and refused for a data format no port has; RFC 2217 settings sent again while
connected, and refused without COM Port Control; the mapping both ways between Set-Up and the
line; which wins at connect; and a session that sets the line on leaving Set-Up and on DECSCS,
keeps a speed Set-Up cannot show, and leaves line and Set-Up alone when the port refuses. Still
to do: `ser2net`, and real hardware.


Planned: the computed termios and DCB for every Set-Up combination, as the serial tests already
check `-b -d -p -s -f`; a session test with a scripted transport that sees `set_line` called for
Set-Up and for DECSCS, DECSPP and DECSFC; RFC 2217 against `ser2net`, as in
[`rfc2217-testing.md`](rfc2217-testing.md). On hardware, run by the user: a USB serial adapter
to an OpenVMS console or terminal port, changing speed in Set-Up at both ends.

## Decisions

Recommendations first; these are the user's to settle.

1. **Which wins at connect.** *Recommended*: the command line, then a saved connection's own
   settings, then Set-Up's — and whichever wins goes into Set-Up's current values, not its
   saved ones, so Set-Up shows the line as it is and Save makes it the power-up setting. The
   alternative, Set-Up alone, would make a saved connection to a 19200 console forget its speed.
2. **Live changes.** *Recommended*: yes, as the terminal does — leaving Set-Up with a new speed
   sets the port at once, and so does DECSCS from the host. A host that changes its own speed
   and the terminal's together is how a DEC line was moved to a faster rate.
3. **DSR and DTR flow control** (VT500 only). Windows has DTR/DSR handshaking; Linux has only
   RTS/CTS. *Recommended*: RTS/CTS on both, since it is the hardware flow a PC's serial port and
   USB adapters carry and `-f h` already means it; say so in Set-Up's documentation.
4. **Receive speed.** Split speeds are rare, Windows cannot set them and most USB adapters
   cannot either. *Recommended*: the transmit speed sets the line and the receive speed stays
   stored, as today; the XOFF threshold likewise, as the kernel decides when to send XOFF.
5. **Checked parity.** Today no parity is checked, *unchecked* or not. A VT420 shows a character
   received with bad parity as the error character. *Recommended*: leave it for after S1 to S3,
   as its own step: termios `INPCK` with `PARMRK` on Linux, and the error character shown.
6. **RFC 2217.** *Recommended*: Telnet with COM Port Control follows Set-Up in the same way, but
   only on a connection that turned COM Port Control on; a plain Telnet connection never starts
   sending it.
