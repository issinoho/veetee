# veetee roadmap

The working roadmap: what is done, what 1.0 ships with, and what is parked. The wiki's
[Roadmap](https://github.com/issinoho/veetee/wiki/Roadmap) page is the public summary; details of
every control function are in [compat-matrix.md](compat-matrix.md), and released changes in
[CHANGELOG.md](../CHANGELOG.md).

Latest release: **1.0.0** (17 September 2026). From 1.0 the version follows Semantic Versioning:
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

  Left: nothing. Several fields of the messages are still copied rather than understood; they are
  marked in [`lat-protocol.md`](lat-protocol.md). Not available in the Flatpak, which has no raw
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
  [winget-pkgs#436670](https://github.com/microsoft/winget-pkgs/pull/436670) offers 0.8.12, is
  past URL validation and waiting on a moderator. It was installed from the local manifests first
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

- **Stored-only Set-Up settings**: serial line settings in Set-Up are not applied to `--serial`
  connections; zero style, energy saver, host wake-up, overscan, transmit rate limits, modem
  control and the compose/Alt/F5 key options are saved and reported but do not change behaviour.
- **Sessions**: a window holds two sessions (a VT520 has four), with no session management over one
  line (TD/SMP, SSU).
- **Indicator status line** field layout not yet checked against hardware; DECTST resets without a
  visible self-test.
- **Printing**: printer controller and print screen data are swallowed; printing is post-1.0.
- **Hardware nobody here has.** RFC 2217 is proved against `ser2net`, and whether a DECserver,
  Lantronix or Moxa answers the option at all is unknown —
  [`rfc2217-testing.md`](rfc2217-testing.md) is written for whoever has one. Three details of LAT
  are read no further than their shape, and are marked in
  [`lat-protocol.md`](lat-protocol.md): what a stop message does, what a type-10 slot says beyond
  the page size, and the run-message flag bits.
- **Protected fields have no acceptance coverage**, as the OpenVMS section above records: nothing
  on OpenVMS appears to drive DECSCA, DECSED, DECSEL or DECSERA, so they are tested against vttest
  and esctest and never against an application.

### Every release

- **Windows signing**: each release's zip is signed after publishing with
  `pwsh -File packaging\windows\sign-release.ps1 -Version X.Y.Z` on Windows with Certum SimplySign
  Desktop signed in (see `packaging/windows/SIGNING.md`).
- **The wiki** (`issinoho/veetee.wiki`, a separate repository) is updated alongside a release.

## After 1.0

VT340 Sixel and ReGIS graphics, Tektronix 4010/4014, printer controller output to CUPS or PDF,
Kermit and X/Y/ZMODEM file transfer, scripting and macros. The parser already accepts and safely
ignores their sequences.
