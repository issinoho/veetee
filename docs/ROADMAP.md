# veetee roadmap

The working roadmap: what is done, what 1.0 ships with, and what is parked. The wiki's
[Roadmap](https://github.com/issinoho/veetee/wiki/Roadmap) page is the public summary; details of
every control function are in [compat-matrix.md](compat-matrix.md), and released changes in
[CHANGELOG.md](../CHANGELOG.md).

Latest release: **1.5.0** (25 September 2026); 1.0.0 was released on 17 September 2026 (see
[Since 1.0](#since-10)). From 1.0 the version follows Semantic Versioning:
a breaking change to the crates' public API or to saved settings takes the major, new terminal
behaviour takes the minor, fixes take the patch. Before 1.0 the minor version followed the
milestone reached.

## Goal

A DEC VT terminal for the Linux (and Windows) desktop with SmarTerm/Reflection-class compatibility:
VT52 through VT525, DECforms, FMS, EDT, EVE and SMG$ applications on OpenVMS without glitches,
fonts drawn on DEC's own character cells, and the depth power users expect. It passes vttest and
esctest2, and defaults to DEC and OpenVMS behaviour everywhere (see [CLAUDE.md](../CLAUDE.md)).

## Milestones

| Milestone | Scope | Status |
|-----------|-------|--------|
| M0 Foundations | Workspace, licences, CI, parser, fuzzing, headless driver | Done |
| M1 VT100/VT102/VT52 | Screen model, GTK window, OpenGL renderer, PTY, first font, vttest 1–8 | Done |
| M2 VT220/VT320 | Character sets and NRCS, soft fonts, UDKs, 8-bit controls, selective erase, status line, reports | Done |
| M3 VT420 | Left/right margins, rectangles, checksums, pages, macros, state reports, esctest2 | Done (0.3.0) |
| M4 VT510/VT520/VT525 | Colour, dual sessions, cursor styles, VT500 modes, reports, keyboard controls, character sets | Done (0.4.0) |
| M5 Transports | PTY, serial, Telnet, SSH; RFC 2217; LAT | Done (0.8.9–0.8.11) |
| M6 Keyboard and DEC applications | Keymap and LK401 editor, DECFNK, VT520 key programming, recordings; OpenVMS acceptance recordings | Done (0.6.0) |
| M7 Fonts and look | DEC-cell fonts for every model and width, VT420 and VT500 Set-Up, sound, smooth scroll, CRT picture, Display Controls | Done (0.7.1) |
| M8 Polish and 1.0 | Saved connections, logs, history search, copy and paste translation, screen readers, throughput, Flatpak, signed Windows builds | Planned work done (0.8.1–0.8.8) |

## OpenVMS acceptance

Complete: the milestone shipped in 0.6.0 and the recordings cover the checklist. What is
left here is a limitation rather than work.

- **Recordings**: the checklist in `tests/conformance/openvms/README.md` is complete. Nine are in
  that directory — `SET TERMINAL/INQUIRE` over Telnet and over SSH, the EDT keypad, EVE/TPU, MAIL,
  the FMS sample application, the DECforms sample, `SHOW CLUSTER/CONTINUOUS` and MONITOR — made by
  the user with `veetee --record FILE.vtrec` on their OpenVMS system and replayed in CI by
  `cargo xtask openvms` (they are kept outside the repository until reviewed, and exclude typed
  keys unless `--record-keys`). The last two added no control function the others did not already
  reach: coverage was near its useful extent after FMS and DECforms, and further recordings are
  worth making only for an application that exercises something new.
- **Protected fields have no acceptance coverage.** DECSCA, DECSED, DECSEL and DECSERA are
  implemented and pass vttest 11.1.2.4, but no recording exercises them, and none can: both DEC
  forms products keep field protection to themselves. Neither the FMS sample application nor the
  DECforms sample sends a single one of those sequences — they track which fields are writable
  and redraw. So the terminal's protected-field functions are tested only against vttest and
  esctest, never against a real OpenVMS application. Finding one that drives them would close the
  last gap of its kind; it may be that nothing on OpenVMS does.
- PCTerm mode and key position reports (DECPCTERM, DECKPM, DECEKBD): not planned for 1.0; OpenVMS
  does not use them.

## M5: connections

Complete: LAT shipped in 0.8.9 and RFC 2217 is proved against a reference implementation. What is
left here is other people's hardware rather than work.


- **RFC 2217** (Telnet COM Port Control) is **proved against `ser2net`**: veetee asked for 19200,
  7 data bits, even parity, 2 stop bits and RTS/CTS, and every one came back from the server as it
  was sent, the four-byte speed included. The exchange is written out in
  [`rfc2217-testing.md`](rfc2217-testing.md), which is also the page for anyone wanting to repeat
  it — it wants a USB serial adapter and nothing on the other end of it.

  What remains is other people's hardware rather than veetee: whether a DECserver, Lantronix or
  Moxa answers the option at all. The option is from 1997 and DEC's own servers are older, so a
  DECserver may well refuse it, and that would be a result worth having.

  Two known gaps, neither of which cost anything against ser2net: nothing subscribes to
  NOTIFY-LINESTATE or NOTIFY-MODEMSTATE, so a dropped line goes unnoticed — though ser2net sends
  modem state unbidden and veetee tolerates it — and the server's replies are ignored rather than
  checked, so veetee would report what it asked for if a server settled on something else.
  Break and flow control are untested for want of anything on the far end to feel them.
- **LAT** (Local Area Transport), clean-room from packet captures of OpenVMS LATACP: `latd` is
  GPL, so none of it is read. Still the largest item left, but no longer untouched.

  Done: [`docs/lat-protocol.md`](lat-protocol.md) records what the captures show; `vt-lat` reads a
  service announcement and a solicit and builds a solicit, with the captured frames as fixtures;
  and `vt-headless lat INTERFACE` finds the services announcing on a wire, which is discovery
  working end to end against two OpenVMS nodes.

  Also done, and further than expected: `vt-headless lat --connect NODE` opens a circuit with an
  OpenVMS host, asks for a service, and is given a terminal and the login prompt. The host creates
  an `LTA` device, sends its banner, and holds the circuit for as long as veetee acknowledges it.
  Everything needed for that — circuit start, run messages, slots, sequence and acknowledgement —
  is read and written by `vt-lat`.

  Traffic goes both ways: typing a username gets the echo and then `Password:`, so the protocol
  work is done in substance.

  The `Transport` is written and **it has carried a real session**: `vt_transport::lat::Lat` opens
  the circuit, asks for the service and carries the session both ways, keeping it alive while it
  is idle and taking it down on the way out, so a terminal can use a LAT session as it uses
  Telnet. Against MYI64 on Linux that means a login, a 667-file `DIRECTORY SYS$SYSTEM`,
  `SHOW TERMINAL` and a clean `LOGOUT`, with `SHOW DEVICE LTA` showing no devices left behind.

  Meeting a host is what read the last of the protocol, and the three things it found were all
  veetee misreading it: a slot's type is the high nibble and its credit the low, not the other way
  about, which had been showing a parameter block on screen and swallowing the banner; credit is
  flow control, and granting it once left OpenVMS stopping mid-word; and a session ends with a
  slot of type 13 from slot 0, which veetee now acts on rather than acknowledging a dead login.
  The `Transport` itself needed no change through any of it.

  The **helper** is written: `veetee-lat-helper` opens the socket and hands it back through a Unix
  socket pair, so nothing that draws a terminal ever holds `CAP_NET_RAW`. That is not only good
  manners — GTK refuses to start at all when it has file capabilities, the kernel setting
  `AT_SECURE`, so LAT in the window was impossible without it. The capability is **not** granted by
  the package: `sudo setcap cap_net_raw+ep /usr/libexec/veetee-lat-helper` turns it on, and veetee
  says exactly that when it is missing, naming the path it looked in. A socket has since crossed
  from it into a session in the window, which is the whole of it proved.

  **LAT is a connection like any other**: `--lat NODE`, with `--interface` only where more than
  one Ethernet interface is up and `--service` only where the service is not the node's own name;
  a saved-connection kind in `profiles.toml`; and an entry in the connection dialog beside Telnet
  and SSH.

  In the window it is a DEC terminal on a DEC protocol, which is the whole point of the project.
  `SET TERMINAL/INQUIRE` at login identifies veetee as `Device_Type: VT400_Series` and then sets
  the line up for it — `CSI 62 " p` for the VT400 conformance level, `CSI 24 * |` for the page,
  `CSI ? 3 l` for eighty columns, `ESC SP F` for 7-bit controls — and `SHOW TERMINAL` reports
  80 × 24, `Eightbit`, `Soft Characters` and `DEC_CRT` through `DEC_CRT4`. A file of sixty lines
  types out without a pause.

  The **service browser** is written: a button beside the node in the connection dialog opens a
  window that listens and fills in as the announcements arrive, a row for each node and service
  with its rating, and picking one fills in the connection. Nothing can be asked for — a node
  announces itself about once a minute and a solicit built by hand has never been answered — so
  the window says as much rather than looking broken while it waits.

  Left: no planned work, but this is the least settled part of the project: it took three
  releases (1.1.0–1.1.2, see [Since 1.0](#since-10)) to hold a session for hours, and a change is
  proved by a long `VEETEE_LAT_TRACE` run against a real host, not by the tests. Several fields of
  the messages are still copied rather than understood; they are marked in
  [`lat-protocol.md`](lat-protocol.md). Not available in the Flatpak, which has no raw
  sockets, nor on Windows, which has no raw Ethernet without a driver.

## 1.0, and what it ships with

**1.0.0 was released on 17 September 2026.** Every milestone from the plan is closed — M5 was the
last, with LAT in 0.8.9 and RFC 2217 proved against `ser2net` in 0.8.11 — and the distribution
decisions are settled: winget alone on Windows, no Flathub. What follows is what 1.0 ships with
rather than fixes; none of it was judged a reason to hold the release.

### Decisions

- **1.0 itself: called on 17 September 2026**, with the tree green (`fmt`, `clippy`, the workspace
  tests, vttest all PASS and esctest matching `expected-failures.txt` exactly) and no open issues.
  The winget pull request was still in review and was deliberately not waited for; the manifests
  are pointed at each release afterwards with `cargo xtask winget VERSION`.
- **Windows: winget alone** (17 September 2026). No Inno Setup or WiX installer. A release ships
  a zip holding the GTK runtime, and winget takes that zip as a portable package:
  [winget-pkgs#436670](https://github.com/microsoft/winget-pkgs/pull/436670) opened with 0.8.12
  and was superseded in place with 1.1.2 on 18 September 2026, keeping its number and its place in
  the queue: 0.8.12 predates the LAT work below, and 1.1.0 and 1.1.1 each kill a LAT session
  within minutes, so publishing any of the three first would have put a known-bad version in front
  of first-time installers. It is revalidating and waiting on a moderator; the new-package queue
  has been running at about a fortnight. It was installed from the local manifests first
  and does work — winget verifies the checksum, unpacks it, and both `veetee` and `vt-headless`
  run from an unrelated directory with GTK resolving beside them. `winget uninstall` removes it
  and it registers an Add/Remove Programs entry, so uninstall and upgrade are answered.

  What veetee gives up by not building an installer is a **Start menu shortcut**, which for a
  windowed application is a real gap: a first-time user has to know to type `veetee` in a shell.
  That was judged not worth a second artifact to build and code-sign every release.
  [`packaging/winget`](../packaging/winget) has the manifests, `cargo xtask winget VERSION` points
  them at a release, and its README covers the submission.
- **Flathub: decided against** (17 September 2026). The manifest in
  [`packaging/flathub`](../packaging/flathub) was finished and Flathub's own
  `flatpak-builder-lint` run against it, which returned three errors. `appid-url-not-reachable`
  is the app ID's own domain: `com.issinoho.Veetee` obliges `https://issinoho.com` to answer, and
  it does not — it resolves but serves nothing on 80 or 443, confirmed from three networks, and it
  is the same fault that failed winget's URL validation. The other two,
  `finish-args-flatpak-spawn-access` and `finish-args-home-filesystem-access`, need exceptions
  granted by pull request, and Flathub's documentation says sandbox-escape exceptions "will not be
  granted if there are signs of LLM usage in the software or in the exception PR"; this repository
  is full of `Co-Authored-By` lines. Flathub further bars AI agents from opening or writing its
  submission pull requests at all.

  Those two permissions are host shells and SSH, and writing logs where the user asks — the things
  that make veetee worth installing — so narrowing them to suit one store was not worth it. The
  metainfo passes `appstreamcli validate` and `--device=all` drew no complaint, so nothing here
  reflects on the packaging. **`packaging/flatpak` is unaffected**: the bundle still builds in CI
  and ships with every release. The README in `packaging/flathub` keeps the full reasoning and the
  procedure, should this ever be reopened.

### Limitations 1.0 ships with (from compat-matrix.md)

- **Stored-only Set-Up settings**: zero style, energy saver, host wake-up, overscan, transmit
  rate limits, modem control, the receive speed and the compose/Alt/F5 key options are saved and
  reported but do not change behaviour. Communications Set-Up sets the serial line from 1.6.0
  ([`serial-setup.md`](serial-setup.md)).
- **Sessions**: a window holds two sessions (a VT520 has four), with no session management over one
  line (TD/SMP, SSU).
- **Indicator status line** field layout not yet checked against hardware; DECTST resets without a
  visible self-test.
- **Printing**: released in 1.5.0 ([`printing.md`](printing.md)). The terminal's printer
  functions, printing to PDF and printing to a real printer are done; passing a host's printer
  data through untouched is optional. Accepted on OpenVMS (every printer function, from DCL and
  in EVE and MONITOR); an application that prints to the terminal's printer, such as ALL-IN-1,
  is waited for.
- **Hardware nobody here has.** RFC 2217 is proved against `ser2net`, and whether a DECserver,
  Lantronix or Moxa answers the option at all is unknown —
  [`rfc2217-testing.md`](rfc2217-testing.md) is written for whoever has one. Several details of
  LAT are read no further than their shape and are marked in
  [`lat-protocol.md`](lat-protocol.md): what a stop message does, what a type-10 slot says beyond
  the page size, and the run-message flag bits. Three of veetee's own rules are a *reading* of
  unread protocol rather than a documented fact — what spends an allowance, whether to advance an
  acknowledgement past a gap, and which messages take a sequence number. Each is chosen so that
  the session cannot wedge, which is the failure worth avoiding, and each is argued where it is
  made.
- **Protected fields have no acceptance coverage**, as the OpenVMS section above records: nothing
  on OpenVMS appears to drive DECSCA, DECSED, DECSEL or DECSERA, so they are tested against vttest
  and esctest and never against an application.

### Every release

- **Windows signing**: each release's zip is signed after publishing with
  `pwsh -File packaging\windows\sign-release.ps1 -Version X.Y.Z` on Windows with Certum SimplySign
  Desktop signed in (see `packaging/windows/SIGNING.md`).
- **winget, after signing and not before**: `cargo xtask winget VERSION`. Signing replaces both
  the zip and `SHA256SUMS`, so manifests pointed at a release first pin a file that is no longer
  there. Got wrong on 1.0.0, and the asset timestamps are worth a glance first.
- **The wiki** (`issinoho/veetee.wiki`, a separate repository) is updated alongside a release.

## Since 1.0

**1.1.0, 1.1.1 and 1.1.2 are all LAT** (18 September 2026), and 1.1.2 is the one to have: the
other two are worse than 1.0.0 was. Four faults, found from traces of a real session rather than
from the captures — a session that goes quiet on a wire that loses frames, credit granted for
traffic that never existed, a deadlock over which numbers get acknowledged, and underneath them
all a sequence number taken for every message sent, which ran veetee past the host's queue limit
of 24 and killed every session within minutes of a login. `CHANGELOG.md` has each in full.

Away from LAT, 1.1.0 gave a split window a way back out of itself: **Close Session** in the
window menu, and a second session that ends now gives the whole window to the other rather
than leaving half of it dead. Neither has been seen working by anything but a compiler — a
GTK dialog cannot be driven from the machine this was written on.

1.1.0 also added **`VEETEE_LAT_TRACE`**, which is what found the rest: every frame to a file with
a line of running totals each minute, and from 1.1.2 the kernel's own `PACKET_STATISTICS` beside
them, so a frame veetee lost to itself can be told from one the wire lost. The soak in
`crates/vt-lat/tests/soak.rs` now models a host that acknowledges as it goes, refuses more credit
than it granted, waits to hear its own numbers back and enforces a queue limit — all four being
things it did not do, each of which let a fault through to a release.

**A LAT change is not proved by the soak.** Three releases in a row passed it and were disproved
by MYI64 within minutes. What proves one is `VEETEE_LAT_TRACE` on a session running
`MONITOR SYSTEM` for an hour or more: 1.1.2 was held until a run of two hours thirty-eight minutes
showed `unacked` flat at nought, two lost messages both attributable to the wire, and nothing
dropped in veetee's own receive buffer across 1.35 MB.

**Still to be seen fire**: the dead-peer timer, which ends a session after a minute of silence. It
is unit-tested and has never run in anger, no host having dropped a circuit since it was written.

**1.2.0** (24 September 2026) added a **default connection**: a star beside a saved connection makes
it what veetee opens when started without one, in place of the login shell, and `--shell` is the
way back. Like Close Session it has been seen only by a compiler and its unit tests.

**1.3.0** (24 September 2026) added **Kermit file transfer**, in the window and from the command
line, and fixed LAT for any paste longer than 255 characters: veetee filled a slot to 255 and
OpenVMS, which never sends more than 254, dropped the circuit. That was found through Kermit, the
first time a transfer over LAT had to recover.

**1.4.0** (25 September 2026) made LAT survive lost frames: veetee sends again what the host has
not acknowledged and takes the host's messages in order, where before one lost frame froze the
session and a lost host message left a hole on the screen. With long packets for Kermit and five
slots to a LAT message, 20 MB to OpenVMS over LAT, which 1.3.0 could not finish, took 16 minutes
over Wi-Fi. All of it was found and proved against MYI64 with `VEETEE_LAT_TRACE`.

**1.5.0** (25 September 2026) added **printing**: the printer port, so printer controller data no
longer lands on the screen, and each print job — from the host or the Print Screen key — as a PDF
or on a real printer, seen on paper including a job printed from OpenVMS. It also honours XOFF
from the host, so a long paste into OpenVMS with HOSTSYNC set no longer overruns.

## After 1.0

VT340 Sixel and ReGIS graphics, Tektronix 4010/4014, X/Y/ZMODEM file transfer, scripting and
macros. The parser already accepts and safely ignores
their sequences.

### Kermit: released, acceptance under way

Kermit is the one file transfer with a DEC reason to be here: OpenVMS ships KERMIT-32, and on a
serial or LAT line there is no SCP or FTP to fall back on. Released in 1.3.0: *Send File…* and
*Receive File…* in the window, and `vt-headless kermit`. The plan, in five steps from the state machine to acceptance
on OpenVMS, is [`kermit.md`](kermit.md).

**`vt-kermit`** is the protocol layer on the same footing as `vt-lat`: bytes in, bytes out, no
files, sockets or timers, so all of it is testable anywhere. It is clean-room, written from
da Cruz's *Kermit: A File Transfer Protocol* and the Kermit Protocol Manual; `gkermit` and
C-Kermit are GPL, so they are counterparties to test against and never a reference.

- **Done**: whole transfers both ways (K1), with retries, cancelling, text and binary, and
  received names made safe; `vt-headless kermit`, over any connection veetee has (K2); attribute
  packets, so the host is told whether a file is text; and the CRC, asked for by default. All of
  it is proved both ways against C-Kermit and G-Kermit, which CI installs.
- **Acceptance on OpenVMS (K4), under way**: against C-Kermit 9.0.300 on MYI64, everything passes
  over Telnet and LAT — text and binary both ways, a mixed batch, cancelling from either end.
  Still to try: SSH, serial, C-Kermit 8.0.211, and KERMIT-32 where a system has it.
- **Long packets** (1.4.0): up to 9 KB where both ends offer them. 20 MB to OpenVMS over LAT takes
  16 minutes over Wi-Fi and 31 on the cable, the pace now the host's; over Telnet it is still to
  be timed.
- **XOFF from the host** (1.5.0): pasting faster than OpenVMS reads overran its type-ahead buffer;
  veetee now stops at XOFF and goes on at XON over Telnet, SSH and LAT, with the terminal set
  `SET TERMINAL/HOSTSYNC`.
- **Not supported**: sliding windows, so a far end offering them gets one packet at a time.

The lesson of LAT applies: a peer written here is too well behaved to find anything. The real
Kermits found three faults the simulated line did not; KERMIT-32 on OpenVMS is the one left.

Printing came in 1.5.0: the printer functions in `vt-core`, and print jobs to a PDF or a real
printer. The plan, and its acceptance on OpenVMS, are in
[`printing.md`](printing.md).

An Android port is planned in outline in [`android.md`](android.md).
