# Kermit file transfer

A plan, with the first part built. Written 2026-09-24 against veetee 1.2.0 and the `vt-kermit`
crate as of `9a5bb68`; check both before acting on it.

## Why Kermit, and why first

A terminal on a serial console or a LAT line has one channel and nothing else: no SCP, no FTP.
Kermit is how a file leaves such a machine, and it is the DEC answer as much as anything is —
OpenVMS has KERMIT-32, and C-Kermit for OpenVMS on newer systems, and the protocol was built for
seven-bit lines with parity, XON/XOFF, and hosts that will not pass a control character. Of the
file transfers on the roadmap it is the one with a DEC reason to be there. X/Y/ZMODEM come after,
if at all.

No DEC terminal did file transfer, so this is an addition to the terminal, not an emulation of
one. It must not change anything a DEC terminal does when no transfer is running.

## Where it stands

`vt-kermit` is the protocol layer on the same footing as `vt-lat`: bytes in and out, no files,
sockets or timers. Clean-room from da Cruz, *Kermit: A File Transfer Protocol* (Digital Press
1987) and the Kermit Protocol Manual; `gkermit` and C-Kermit are GPL, so they are counterparties
to test against and never a reference.

| Part | State |
|---|---|
| Packets: build, read, the three block checks, the packet types | Done |
| Send-init parameters: read, build, agree between two ends | Done; `gkermit`'s real send-init pinned as a fixture |
| Data encoding: control, eighth-bit and repeat prefixes, filling a packet to the room agreed | Done |
| The 16-bit CRC (check type 3) | Written, never checked against another implementation, so not asked for |
| Transfers: `Sender` and `Receiver` (K1) | Done, unreleased; tested end to end over a simulated line, not yet against a real Kermit |
| The capability field | Carried, not read |
| Long packets, sliding windows, attribute packets | Not supported; a far end offering them gets short packets, one at a time |

## The plan

Five steps, each usable and tested on its own. The first two need nothing from the window.

### K1. The transfer state machine, in `vt-kermit`

**Done** (24 September 2026), as `transfer.rs`, `text.rs` and `names.rs`. What building it found:

- **The check type was agreed wrongly.** `Params::agreed` took the far end's choice outright,
  so against `gkermit`, which asks for the CRC while veetee answers `1`, veetee would have read
  every packet after the send-init with the CRC while `gkermit` sent the one-character check. The
  protocol uses a check only where both ends name it and falls back to type 1 otherwise; it now
  does, and the test that expected the CRC with `gkermit` expects type 1.
- **A nak cannot answer a send-init.** A nak for packet *n*+1 acknowledges packet *n*, except when
  *n* is the send-init, whose answer carries the receiver's parameters. Taking a nak for it left
  the ends disagreeing about the prefixes — found by the lossy soak, as files arriving the right
  length with the wrong bytes.
- **The one-character check lets damage through.** With one packet in five damaged, about two
  transfers in a hundred arrived wrong and reported success; with the CRC, none in a thousand.
  Damage to the length moves where the check is read from, which a six-bit check misses one time
  in sixty-four. So once K2 has proved the CRC against `gkermit`, veetee should ask for it.
- **A short packet waited for ever.** A length too small to hold a sequence, a type and a check
  was read as incomplete, which holds a line until the timeout; it is now damaged and skipped.

The sketch that follows is what was planned; the API as built is close to it, with `Store` and
`Source` traits in place of file actions.

Sans-I/O like the rest of the crate, so every retry and timeout is testable without a clock or
a line. One type per direction, driven by the caller:

```rust
// Sketch, not a settled API.
let mut rx = Receiver::new(ours(), now);
let actions = rx.feed(bytes_from_line, now);  // packets in
let actions = rx.tick(now);                   // timeouts: NAK or resend
// Action::Send(bytes)          — write to the line
// Action::OpenFile(name)       — the caller decides where, or refuses
// Action::Write(data)          — decoded file bytes
// Action::CloseFile { complete: bool }
// Action::Progress { bytes }
// Action::Done(Result<(), String>)
```

- **Receive**: answer S with our parameters; F opens a file (the caller can refuse, which becomes
  an E packet); D is decoded and written; Z closes; B ends. A packet arriving twice is
  acknowledged again and not written twice; a bad check is NAKed.
- **Send**: S, wait for its ack and agree; F; D until the file is empty, filling each packet with
  `encode`'s `room`; Z; the next file or B.
- **Timeouts and retries**: resend on the far end's timeout, NAK on ours, give up after a limit
  (10 by default) with an E packet that says why.
- **Cancel**: the user can stop a transfer. First interrupt the file (the X/Z discard mechanism),
  then the batch; if the far end does not answer, send E and stop.
- **Text and binary**: text mode translates line endings between the line's CR LF and the local
  convention; binary passes bytes untouched and needs eighth-bit prefixing on a seven-bit line.
  The mode is the user's choice, as attribute packets are not supported to carry it.

Tests: a sender and receiver in the same process over a lossy, reordering, duplicating pipe (the
lesson of LAT's soak test: model the far end's rules, not an obliging peer), plus every exchange
with a captured fixture.

### K2. `vt-headless kermit`, and interop with a real Kermit

A command-line transfer over any transport veetee already has:

```sh
vt-headless kermit receive --telnet vms1 --into ~/incoming
vt-headless kermit send FILE.TXT --serial /dev/ttyUSB0 -b 9600 -d 7 -p e
```

This is both a useful tool and the interop harness. `gkermit` will not run on a pipe ("Can't set
packet mode") but runs on a pty, which is exactly what `--command` gives:

```sh
vt-headless kermit receive --command "gkermit -s testfile" --into target/k
vt-headless kermit send testfile --command "gkermit -r"
```

That is enough for CI: files of every awkward shape (empty, one byte, all 256 byte values, long
runs, CR LF and bare LF text) sent both ways, text and binary, with each check type. It settles
the CRC against `gkermit` the first time it runs. C-Kermit can be added the same way where it
installs.

### K3. In the window

- **Menu**: *Receive File…* and *Send File…* in the window menu, beside *Log to File…*. The user
  starts `SEND` or `RECEIVE` on the host first, then picks the matching item, which is how every
  terminal emulator with Kermit has worked.
- **While it runs**, the session's host bytes go to the transfer and not to the terminal, and
  typed keys are held back except the ones that cancel. A bar above the screen shows the file
  name, bytes so far, and a Cancel button. The screen is left as it was and picks up where it
  left off.
- **Files**: GTK's file dialog for what to send and where to receive, which works through the
  portal in the Flatpak and natively on Windows.
- **Recording and logs**: a recording keeps the bytes (it is a record of the line), and a text
  log is paused, because packets are not text.

The change to `session.rs` is contained: `io_loop` already reads with a 250 ms timeout, which is
the tick, and all writes go through the one writer; the transfer is an `Option` in `Shared` that
`io_loop` checks before feeding the terminal.

### K4. Acceptance on OpenVMS

A transfer is not proved until it has run against KERMIT-32 on OpenVMS, both ways, text and
binary, over Telnet, SSH, serial and LAT. **The user runs these**; they are not run from here. What
to check:

- VMS file names (`LOGIN.COM;3`) and how they arrive: the version stripped, the case lowered?
- VMS record formats against text mode: variable-length records arriving as lines.
- A seven-bit serial line with parity, which forces eighth-bit prefixing for a binary file.
- A transfer long enough to see a timeout recovered from, on the lossiest link to hand.

### K5. After that, only if wanted

Attribute packets (file size, so progress can be a percentage; dates; text or binary chosen by
the sender), long packets and sliding windows (speed on a fast link), server mode (`GET` from the
terminal end), and autodownload (starting a receive when a send-init arrives unasked).

## Decisions to make

Recommendations first; these are the user's to settle.

1. **Receive-side file names are untrusted input**, since the host chooses them. Keep the last
   path component only, refuse names with nothing left after that, and never overwrite: a
   clash gets a suffix. *Recommended, and not really optional.*
2. **VMS names**: strip the `;version` and lower the case (`LOGIN.COM;3` → `login.com`), as the
   Kermit "normal form" expects. *Recommended*, with an option to keep them as sent.
3. **Default mode**: text, as Kermit itself defaults to, with binary chosen in the dialog.
4. **Autodownload**: off by default and left to K5. It is an xterm-era convenience, not DEC
   behaviour, and a terminal that starts writing files because the host sent a particular packet
   is one to be careful about.
5. **Where it lives in the tree**: the state machine in `vt-kermit` (headless-testable, like
   `vt-lat`); the command in `vt-headless`; the window work in `veetee`. `vt-core` does not change
   and does not learn about Kermit.
6. **Release**: K1 and K2 can be released on their own as a minor (a headless command is new
   behaviour); K3 is the release worth announcing.
