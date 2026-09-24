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
| The 16-bit CRC (check type 3) | Proved against C-Kermit and G-Kermit both ways (K2), and asked for by default |
| Transfers: `Sender` and `Receiver` (K1) | Released in 1.3.0; tested end to end over a simulated line |
| `vt-headless kermit` and interop (K2) | Released in 1.3.0; both ways against C-Kermit and G-Kermit, text and binary, check 1 and the CRC |
| Attribute packets: file type and size | Released in 1.3.0; brought forward from K5, proved against both |
| The capability field | Carried, not read |
| Long packets, sliding windows | Not supported; a far end offering them gets short packets, one at a time |

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

**Done** (24 September 2026). `crates/vt-headless/tests/kermit_interop.rs` sends files of every
awkward shape both ways, text and binary, with check 1 and the CRC, against C-Kermit 10.0 Beta.12
and G-Kermit 2.01 locally; CI installs both from the Ubuntu archive (noble has C-Kermit Beta.11)
and fails if either is missing. What it found:

- **Every combination worked first time except one**, and the check-type fix from K1 is why:
  both Kermits ask for the CRC, are answered `1`, and then send with the one-character check.
- **C-Kermit sends one character over.** Asked for packets of 94 and sending with the CRC, it
  sends 95, the length travelling as DEL. veetee now reads a length of 95; nothing longer.
- **Without an attribute packet, a receiving Kermit assumes binary.** Both say so in their
  documentation, and both kept veetee's CR LF lines as they arrived until told `-T`. So until
  veetee sends attribute packets (K5), the user has to put the host's Kermit in text mode as
  well as veetee — `SET FILE TYPE TEXT`, or its equivalent — which K3's dialog should say. It is
  a strong reason to bring attribute packets forward.
- **A file with CR LF already in it survives.** A Unix Kermit sends its CR as data, so CR CR LF;
  veetee keeps the lone CR and the file arrives exactly as it was.
- **The CRC is proved**, and veetee now asks for it (decided 24 September 2026). With G-Kermit
  and C-Kermit, which ask for it themselves, it is what is used. A Kermit that cannot do it —
  possibly KERMIT-32, which has not been seen either way — answers with another check, and the
  protocol has both ends fall back to type 1, so asking costs nothing. The one-character check
  stays tested: the soak that loses and repeats packets runs with it.

The plan as written before it was built:

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

### Attribute packets, brought forward

**Done** (24 September 2026), as `attributes.rs`, because K2 showed that without them a receiving
Kermit assumes binary: text sent to a host kept its CR LF unless the host's Kermit was put in text
mode as well. veetee now offers attributes in its send-init (capability bit 8), and where the far
end offers them too it sends the file's type (`AMJ` for text, `B8` for binary) and its size in
bytes and in kilobytes. Receiving, it takes a sender's type over its own setting, as both Kermits
do, and reads the size for progress. Interop tests prove both halves: C-Kermit and G-Kermit told
nothing store veetee's text as text, and C-Kermit left to choose per file sends a mixed batch that
veetee, set to binary, receives intact.

Three things came from watching what the two Kermits send rather than from the page:

- **The data is not prefix-encoded.** C-Kermit's date goes out under the tag `#`, which as file
  data would be the control prefix; decoding the field would have lost the tag and everything
  after it. It is read raw.
- **veetee does not send the sending system** (`.`, which C-Kermit sends as `U1` for Unix). The
  G-Kermit manual says two Kermits that recognise each other as the same system switch to binary,
  and veetee's text travels in the line's form, so being recognised would put CR LF in the host's
  file.
- **A refusal is `N` in the answer**, followed by the tags objected to; veetee then ends the file
  unsent, marked discard, and offers the next.

Not sent yet: the file's date, which OpenVMS would keep, and protection. Neither is needed to get
a file across intact.

### K3. In the window

**Built** (24 September 2026), and **not yet seen working in the window**: the session under it
is tested end to end against G-Kermit on a pty (`session.rs`), but the menu items, the file
dialogs and the bar have been run by nothing but the compiler. What was built follows the plan
below, with these differences:

- **Text or binary is decided per file** when sending, by looking at it as C-Kermit does (no
  nulls, and nearly all printable or the controls text uses), and the host told in an attribute
  packet; there is no mode to choose. Receiving follows the sender's word, text otherwise.
- **The glue to files moved into `vt_kermit::files`**, which both the window and `vt-headless`
  now use: the folder that never overwrites, the files to send, and a transfer to drive.
- **A finished receiver keeps only packets.** It holds the line for two seconds in case the
  sender asks again for the end of the batch, but only what starts with a packet mark: the
  host's prompt, printed the moment its Kermit finishes, goes to the screen. The first version
  took everything for those two seconds, and the session test caught it losing the text the host
  printed after the transfer.
- **Not checked in the Flatpak**, where the file dialogs go through the portal.

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

Dates and protection in attribute packets, long packets and sliding windows (speed on a fast
link), server mode (`GET` from the terminal end), and autodownload (starting a receive when a
send-init arrives unasked).

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
