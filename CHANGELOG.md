# Changelog

All notable changes to veetee are listed here. From 1.0 the version follows
[Semantic Versioning](https://semver.org/): a breaking change to the crates' public API or to
saved settings takes the major, new terminal behaviour takes the minor, fixes take the patch.
Before 1.0 the minor version followed the project milestones (0.3 = M3). The format follows
[Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

- **Added: a default connection.** A star beside each saved connection in the Connections window
  makes it the one veetee opens when it is started without being told where to connect — a click on
  the launcher, or `veetee` with no connection option — instead of your login shell. Click the star
  again and it goes back to the login shell. The connection editor has the same choice as a
  *Default connection* switch. `--model`, `--phosphor` and the like still apply to it as they do to
  `--profile`, while any of `--profile`, `--telnet`, `--ssh`, `--serial`, `--lat` or `--command` is
  taken at its word, and the new `--shell` opens your login shell whatever is set. It is saved as
  `default-profile = "NAME"` at the top of `profiles.toml`, and `--list-profiles` marks it. A name
  there that is not a saved connection is reported rather than ignored; started that way, veetee
  says so on stderr and opens the login shell, so a mistake in the file never keeps the terminal
  from opening.

## [1.1.2] - 2026-09-18

**Upgrade if you use LAT.** 1.1.1 kills a LAT session within minutes of any login: the terminal
stops echoing, `SET TERM/INQUIRE` reports an unknown terminal type and the screen freezes, on a
circuit that is perfectly healthy. Confirmed fixed against the same OpenVMS node — half an hour of
`MONITOR SYSTEM`, typing echoed, `SET TERM/INQUIRE` answering, and not one frame lost.

- **Fixed: a LAT terminal went dead a few minutes into every session.** Typing stopped being
  echoed, `SET TERM/INQUIRE` reported an unknown terminal type, and the screen froze — on a
  circuit that was up, with full credit, nothing lost and nothing dropped. veetee took a new
  sequence number for every message it sent, including bare acknowledgements, and a far end
  acknowledges only the messages that carry slots. So the gap between what veetee had sent and
  what the host had acknowledged grew by one every keepalive, for ever; past MYI64's queue limit
  of 24 the host stopped accepting anything at all. A message carrying nothing now takes no number
  of its own. That figure had appeared as `unacked` in three traces — 133, 75 and 43 — and been
  read as an artefact each time.
- **Fixed: a LAT session froze with both ends healthy and nothing wrong.** A node whose last
  message carries no slots waits to hear that number acknowledged before it sends anything else,
  and veetee advanced the number it acknowledged only on messages that *did* carry slots. So MYI64
  repeated `seq=86` every ten seconds for eleven minutes while veetee answered `ack=85`, each
  politely acknowledging a stale number at the other: full credit both ways, nothing lost, nothing
  duplicated, the `LTA` device still online, and the screen stopped. Which number is acknowledged
  and whether a frame is sent back are now separate decisions — an acknowledgement still draws no
  answer, since answering one draws another for ever, but the number it carries is taken and goes
  out with the next keepalive.
- **Added: the kernel's own frame counts in a LAT trace**, as `kernel=in/dropped` at the end of
  each summary line. A frame the kernel drops for want of room in the socket's receive buffer is
  one veetee lost to itself rather than to the wire, and a gap in the far end's numbering looks
  identical either way — an ambiguity that cost an afternoon's guessing while the fault behind
  1.1.1 was being tracked down. `missed` climbing while `dropped` stays flat is the wire; climbing
  together is veetee not reading fast enough.

## [1.1.1] - 2026-09-18

**Upgrade if you use LAT.** 1.1.0 made LAT worse rather than better: it counted every one of the
host's acknowledgements as a lost message, and granted the host credit to make up for traffic that
had never existed. The three faults that came out of that are fixed here, all of them found from a
trace of a real session, and the fix is confirmed against the same OpenVMS node — sixteen minutes
with no phantom losses and an allowance that does not drift, where 1.1.0 degraded from the first
minute.

- **Fixed: a LAT session froze with the screen stopped and the keys still clicking.** Three faults
  in a row, found from a trace of a real session that failed this way on 1.1.0.

  The first: a host's own acknowledgements carry a sequence number like any other message, and
  1.1.0 followed the numbering of only the messages carrying slots — so it read every
  acknowledgement as a message lost. A loss believed is credit believed spent, so veetee granted
  the host credit to make up for traffic that had never existed, and sent an empty slot for every
  phantom. On the session that failed: 165 phantom losses, 1361 credits granted against 107
  received, about 170 empty slots to carry 138 bytes of typing.

  The second: veetee never read the credit the host granted *it*, and sent whenever there was
  something to send. With all those empty slots going out it ran 74 slots past its allowance.
  It now spends against what it has been granted, holds typing when there is none, and coalesces
  what is waiting into one slot rather than one slot for every keystroke. It holds for three
  seconds and then sends regardless: holding strictly is the correct reading of the protocol and
  the wrong behaviour, because a host that stopped granting would take the keyboard with it.

  The third is why it looked frozen rather than disconnected. LAT gives both ends a keepalive
  timer and a retransmit limit so either can decide the other has gone. veetee sent the keepalives
  from the first and did none of the deciding, so when OpenVMS gave up — eight retransmissions at
  an 80 ms circuit timer, about two thirds of a second — and released the `LTA` device, veetee
  kept acknowledging into a circuit that no longer existed. The session that prompted this was
  still doing so an hour later. A minute of silence, three of the host's own keepalive intervals,
  now ends the session and says so.

  The soak in `crates/vt-lat/tests/soak.rs` missed all three because its host never acknowledged
  and never checked what it had granted. It does both now.


## [1.1.0] - 2026-09-18

A LAT session survives a wire that loses frames, and a window with two sessions can get back to
one. The first of those was a real fault found by looking for it: sessions were suspected of
failing over long periods, and they were, silently, whenever the wire dropped a frame.

- **Added: Close Session**, in the window menu beside Open Second Session. There was no way to
  close one session of two: the window had a way in and no way out.
- **Fixed: a second session that ended left half the window dead.** A connection that drops keeps
  its screen up on purpose, so what was on it can still be read and copied — but that only makes
  sense for the last session, where the alternative is the window vanishing. With the window split,
  logging out of one session now gives the whole window back to the other, and says why in a toast
  rather than on a dead screen. Serial, Telnet, SSH and LAT were all affected, those being the
  connections whose screens are kept.
- **Fixed: a LAT session stopped dead after a while on a lossy wire.** Credit is how a LAT node is
  told it may keep sending, and veetee worked out what the host had spent by counting the slots
  that arrived. A frame that never arrived spent the host's credit all the same, so every loss
  left veetee's reckoning one too high for the rest of the session; once the drift passed what it
  holds back before granting, the host ran out of credit and went quiet for good — no error, no
  circuit taken down, output simply stopping. Gaps in the host's numbering are now counted as
  credit spent, and the allowance is worked out from scratch every thirty-two messages so that any
  other way of losing count rights itself. A soak with a seventh of the frames dropped now runs
  100,000 messages without stalling; it used to stop at message 7,503 with a thousandth of that
  loss rate.
- **Fixed: a repeated message was read twice**, so text the host resent — which it does whenever an
  acknowledgement goes missing — was painted on the terminal a second time. A message no newer than
  the last one heard is now acknowledged again, which is what the host is waiting for, and its
  slots are dropped.
- **Fixed: a message arriving out of order dragged the acknowledgement backwards**, asking the host
  to send everything since all over again. Sequence numbers are one byte and run out about every
  three quarters of an hour, so "newer" now counts the wrap instead of asking whether the number
  differs.
- **Added: `VEETEE_LAT_TRACE`**, naming a file to write every frame of a LAT session to, with a
  line of running totals every minute — missed messages, duplicates, out-of-order arrivals, credit
  at each end, and messages sent but not acknowledged. An environment variable rather than an
  option because the sessions worth watching are opened from the connection dialog.
  `VEETEE_LAT_TRACE_DATA` adds what each slot carried, and is off by default because a LAT session
  carries its password in clear. See `docs/lat-protocol.md`.
- **Added: an Ubuntu PPA**, `ppa:issinoho/veetee`, for resolute (26.04 LTS). Launchpad builds from
  a source package in a chroot with no network, so `packaging/ubuntu` vendors every crate in
  `Cargo.lock` into the orig tarball and points `.cargo/config.toml` at it. `build-source.sh`
  builds from the release tag rather than the working tree.

  Noble (24.04 LTS) is **not** supported and the script refuses it. Its newest archive rustc is
  1.91 and the gtk-rs crates require 1.92; the noble build was attempted and failed on exactly
  that. If Ubuntu backports a newer rustc the packaging needs no change beyond allowing the series.

- **Fixed: the declared minimum Rust version was wrong.** `rust-version` said 1.85, but the gtk-rs
  stack -- `glib`, `gio`, `gdk4`, `cairo-rs`, `graphene`, `gdk-pixbuf` -- requires 1.92, so anyone
  building with 1.85 through 1.91 got a wall of crate errors rather than one clear message. It now
  says 1.92, which is what the tree has really needed for some time. CI, the Flatpak SDK and a
  rustup toolchain are all newer than that, which is why nothing had caught it.

## [1.0.0] - 2026-09-17

**veetee 1.0.** Every milestone from the plan is closed: the VT52 and VT100 through the VT420 and
the colour VT525, DEC factory Set-Up defaults, fonts drawn on DEC's own character cells, an LK401
keyboard, SSH, Telnet, serial lines, local shells and LAT. vttest passes across the VT100–VT520
menus headless, and esctest2 runs at VT level 5 with every difference from xterm explained against
a DEC manual reference. What 1.0 does not do is written down in
[`docs/ROADMAP.md`](docs/ROADMAP.md) and [`docs/compat-matrix.md`](docs/compat-matrix.md):
printing, a third and fourth session, and the Set-Up settings that are stored but not yet applied.

- **Added: winget manifests**, and veetee is now submitted to winget.
  `packaging/winget` treats the Windows zip as a portable package — winget unpacks it and puts
  `veetee` and `vt-headless` on the path, the GTK runtime being in the zip already — and
  `cargo xtask winget VERSION` points the manifests at a release, taking the checksum from its
  `SHA256SUMS` and the date from its tag. Installing from them locally works: the checksum
  verifies, and both programs run from an unrelated directory. 0.8.12 went to
  [winget-pkgs#436670](https://github.com/microsoft/winget-pkgs/pull/436670).

  This is the whole of the Windows distribution story: **no Inno Setup or WiX installer** will be
  built. `winget uninstall` works and an Add/Remove Programs entry is registered, so only a Start
  menu shortcut is given up.

- **Fixed: winget's `PublisherUrl` pointed at a domain that does not answer.** The validation bot
  could not reach `https://issinoho.com`, which resolves but serves nothing on port 80 or 443, so
  the publisher is now `https://github.com/issinoho`.

- **Decided against Flathub.** `packaging/flathub` holds the manifest, pinned to a tag and its
  commit as Flathub requires, but veetee will not be submitted. Flathub's own
  `flatpak-builder-lint` returns three errors: `appid-url-not-reachable`, because the
  `com.issinoho.Veetee` app ID obliges `https://issinoho.com` to answer and it does not, and
  `finish-args-flatpak-spawn-access` and `finish-args-home-filesystem-access`, which need
  exceptions Flathub does not grant where there are signs of LLM usage. Those two permissions are
  host shells, SSH and writing logs where the user asks, so narrowing them for one store was not
  worth it. The directory and its README are kept as the record. `packaging/flatpak` is
  unaffected and releases still ship a Flatpak bundle. The metainfo gains `<branding>` colours and
  passes `appstreamcli validate`.

## [0.8.12] - 2026-09-17

An RPM, so the Debian package is no longer the only one.

- **Added: an RPM.** Releases carry one beside the Debian package and the tarball, built with
  `cargo-generate-rpm` from the same assets. It asks for its libraries by soname rather than by
  package name, so the one package resolves on Fedora, RHEL and openSUSE alike, whatever each
  calls the thing holding `libgtk-4.so.1`.

  CI builds the tarball, the Debian package and the RPM on every push as well. Packaging is the
  part nobody looks at until a release is being cut, which is the worst moment to find it broken.

## [0.8.11] - 2026-09-17

An interface to pick from rather than type for LAT, and RFC 2217 proved against a real server,
which completes M5 and with it every milestone.

- **Added: an interface picker for LAT.** The connection dialog lists the Ethernet interfaces that
  are up, so a LAT connection is chosen rather than typed and nobody has to leave the window to
  run `ip addr`. A name a saved connection carries is kept and shown even on a machine where that
  interface is not present, since connections travel between machines.

- **Fixed: a lone wireless interface is used rather than refused.** LAT over wireless works — a
  session to OpenVMS has been run over one — and veetee was declining to choose a wireless
  interface at all, so a machine with nothing else up was told to name it by hand. A wire is still
  preferred where there is one, a segment with DEC equipment on it being the likelier of the two.

- **RFC 2217 is proved against a real implementation.** The serial line options over `--telnet`
  had only ever been checked against the RFC itself, which says what the bytes should be and
  nothing about whether a server agrees. Against `ser2net`, veetee asked for 19200, 7 data bits,
  even parity, 2 stop bits and RTS/CTS, and every setting came back from the server as it was
  sent — the four-byte speed included, which is the encoding most likely to be got wrong.
  [docs/rfc2217-testing.md](docs/rfc2217-testing.md) has the exchange written out, and what to do
  to repeat it: a USB serial adapter, with nothing on the far end of it.

  That completes M5. What is left is other people's hardware — whether a DECserver, Lantronix or
  Moxa answers the option at all — rather than anything in veetee.

## [0.8.10] - 2026-09-17

Says what to do when LAT cannot open a socket, having told the first people to try it very little
of use.

- **Fixed: LAT in the Flatpak says so.** Its sandbox refuses a raw Ethernet socket outright rather
  than for want of privilege, which veetee passed on as `Address family not supported by protocol
  (os error 97)` — true, and no help to anybody. It now says that the Flatpak does not allow the
  socket and to use the package or the tarball instead. The fallback to the helper was matching on
  being refused permission alone, so a refusal of any other kind never reached it.

- **Fixed: the capability is asked for once, and can be copied.** The message said the same thing
  twice, and began by suggesting `sudo` — which is right for `vt-headless` and wrong for a GTK
  terminal, since one raised by file capabilities does not start at all. What is left is the
  reason and the remedy:

  ```
  Operation not permitted (os error 1): LAT needs CAP_NET_RAW
  Grant it with: sudo setcap cap_net_raw+ep /usr/libexec/veetee-lat-helper
  ```

  The command could not be taken out of the window, an `AdwActionRow` subtitle not being
  selectable, so the one thing to run had to be typed out by hand. It is selectable now, and where
  a capability is what is wanted the row carries a button that puts the command on the clipboard.

- **Documented: the capability is granted again after every upgrade.** It is an attribute of the
  file, so it lasts across reboots and logins but not across a new package, which is a new file.
  The README says so, and says in the Flatpak section — rather than only further down — that LAT
  cannot work there at all.

## [0.8.9] - 2026-09-17

LAT, DEC's own terminal protocol: veetee opens a session on an OpenVMS node over raw Ethernet,
and OpenVMS sets the line up as a VT420.

- **Added: LAT, DEC's own terminal protocol.** `veetee --lat MYI64` opens a session on an OpenVMS
  node the way `--telnet` opens a Telnet one, and it saves like one: `connection = "lat"` in
  `profiles.toml`, and an entry in the connection dialog beside Telnet and SSH, where a LAT
  connection names a node rather than a host and has no port, LAT not being IP at all. It rides
  directly on Ethernet, so nothing routes and the node has to be on the same segment.

  In the window it is a DEC terminal on a DEC protocol, which is what the whole of this was for.
  `SET TERMINAL/INQUIRE` at login identifies veetee as `Device_Type: VT400_Series` and sets the
  line up for it — the VT400 conformance level, eighty columns, a twenty-four line page and 7-bit
  controls — and `SHOW TERMINAL` reports `Eightbit`, `Soft Characters` and `DEC_CRT` through
  `DEC_CRT4` on an `LTA` device OpenVMS created for it.

  `--interface` is wanted only where more than one Ethernet interface is up: the interface is not
  a preference but the one segment the node is on, and where there is only one it is worked out.
  A wireless interface is offered but never chosen, a LAT segment being a wired one. `--service`
  asks for a service other than the node's own name, which is the usual one.

  Linux only: there is no raw Ethernet in the Flatpak, and none on Windows without a driver.

- **Added: a LAT service browser.** A button beside the node in the connection dialog opens a
  window listing the services announcing themselves, a row each with its rating and what the node
  says about itself; picking one fills the connection in. Nothing is asked for and nothing waits
  on an answer: a node announces itself about once a minute, and a solicit built by hand has never
  been answered, so the list fills in as they arrive and says as much while it is empty.

- **Added: a helper, so that nothing drawing a terminal holds `CAP_NET_RAW`.**
  `veetee-lat-helper` opens the LAT socket and hands it straight back through a Unix socket pair,
  then exits; the circuit, the session and every frame after that are veetee's own work,
  unprivileged. It is more than good manners: GTK refuses to start at all with file capabilities,
  the kernel setting `AT_SECURE` for a process they raise, so LAT in a window could not have
  worked any other way.

  The capability is not granted by the package — a niche protocol is no reason to ship one nobody
  asked for — so `sudo setcap cap_net_raw+ep /usr/libexec/veetee-lat-helper` turns it on, and
  veetee says precisely that, naming the path it looked in, when it is missing. What the grant
  allows is worth knowing: anyone who can run the helper can open a socket for LAT frames on one
  interface and send them. That is far narrower than `CAP_NET_RAW` itself, the socket carrying one
  protocol, but it is not nothing.

- **The protocol, clean-room from packet captures.** `latd` is GPL, so none of it is read: what
  veetee knows of LAT comes from watching OpenVMS LATACP on a wire, and is written down in
  [docs/lat-protocol.md](docs/lat-protocol.md) — circuits, run messages, slots, sequence and
  acknowledgement, credit, and the fields still copied rather than understood, which are marked as
  guesses there. The UIC the captured client sent is left out: it is the identity of the account
  that was calling, which veetee has not got and should not invent.

  It lives in `vt-lat` as a state machine with no sockets in it, frames in and frames out, so it is
  tested against the captured frames of a real login on any platform rather than only on a wire.
  `vt-transport` adds the datalink and a `Transport`, which keeps an idle circuit alive, takes it
  down on the way out so OpenVMS releases the `LTA` device, and holds typing back until the far end
  has prompted, since a slot sent earlier is ignored.

  Meeting a host is what read the last of it, and each thing it found was veetee misreading the
  protocol: a slot's type is the high nibble and its credit the low, not the other way about, which
  had been putting a parameter block on the screen and swallowing the login banner; credit is flow
  control, and granting it once left OpenVMS breaking off mid-word; and a session ends with a slot
  of a type of its own, which veetee had been acknowledging as though the login were still there.
  Two of the three could not have appeared in a short capture at all.

- **Added: `vt-headless lat`**, which is how the protocol was read and how it is watched.
  `vt-headless lat INTERFACE` prints every LAT message on the wire, slot by slot with its control
  byte; `--connect NODE` opens a session without a window, `--type TEXT` types into it, and
  anything else typed goes the same way. Neither needs `sudo`, both asking the helper for a socket.

## [0.8.8] - 2026-09-16

Finds the LAT services announcing themselves on a wire, which is the first half of reaching an
OpenVMS host over DEC's own protocol rather than over IP.

- **Added: LAT service discovery.** `vt-headless lat INTERFACE` lists the services announcing
  themselves on a wire, with their ratings, maximum frame size and announcement interval, read by
  veetee's own code rather than by a packet sniffer. Linux only — LAT is raw Ethernet rather than
  IP, so it needs `CAP_NET_RAW`, has no Windows equivalent without a driver, and cannot work in
  the Flatpak. Connecting to a service is not implemented; see
  [docs/lat-protocol.md](docs/lat-protocol.md) for how far the protocol has been read.

## [0.8.7] - 2026-09-16

A selection can be dragged out of the scrollback at last, so a copy longer than a screenful is
possible, and Telnet can set up the line behind a terminal server.

- **Fixed: a selection can be dragged out of the scrollback.** Dragging past the top or bottom of
  the page selected only as far as the rows on screen, so copying anything longer than a screenful
  was impossible. The screen now moves through the history while the drag is held past an edge, a
  line for each cell height beyond it, and keeps moving while the pointer is held still — a drag
  that stops moving sends no more events, so it takes a timer rather than the gesture alone.

- **Added: RFC 2217 COM Port Control over Telnet**, for terminal servers such as DECserver,
  Lantronix and Moxa. The line options that set up a serial port — `--baud`, `--databits`,
  `--parity`, `--stopbits`, `--flow` — now also work with `--telnet`, where they ask the server
  for those settings on the line behind it, and a saved connection keeps them in the same keys a
  serial connection uses. F5 then sends a real line break rather than a Telnet one.

  The option is offered only when line settings are given. An ordinary host never sees it: veetee
  learned in 0.8.4 what raising an option uninvited can do, and a terminal server is the only
  thing that can answer this one.

## [0.8.6] - 2026-09-16

Keeps an idle Telnet session from being dropped while you read the screen, and gives EVE and TPU a
Do key of their own.

- **Added: Scroll Lock is the Do key.** A PC keyboard has no Do key, so DEC applications that lean
  on it — EVE and TPU ask for a command on it constantly — needed Shift+F6, a two-handed
  reach. Scroll Lock has no DEC meaning and veetee bound nothing to it, so it now sends Do.
  Shift+F6 still does as well, and Hold Screen stays on F1 and Pause. `Scroll_Lock` is also a
  name the keymap accepts now, so it can be bound to anything else in the Keyboard Map window.

- **Fixed (Telnet): idle connections are held open with TCP keepalives.** A terminal sits idle for
  as long as the user is reading, and OpenVMS sends nothing meanwhile, so a firewall or NAT between
  the two is free to forget the connection; the next keystroke then failed with "an established
  connection was aborted by the software in your host machine", losing the session. The socket now
  asks for a keepalive after a minute of silence, repeated every fifteen seconds. SSH is unaffected:
  it runs through the OpenSSH client, which has its own `ServerAliveInterval`.

## [0.8.5] - 2026-09-15

Corrects the identity veetee reports in VT100 mode, and documents why OpenVMS gives an SSH session
a VT102 whatever model is selected.

- **Fixed: VT100 mode no longer changes the terminal's identity.** A VT220 or later in VT100 mode
  (DECSCL level 1) answered Primary DA with `CSI ? 6 c`, claiming to be a VT102. The DA1 identity
  comes from Set-Up's *Terminal ID to host* (DECTID), whose default is the terminal's own, and
  which "has no effect when the terminal is in VT52 mode" — VT100 mode is not an exception
  (EK-VT510-RM 2.6.2). EK-VT220-RM 4.17.1.1 agrees: the VT100, VT101 and VT102 responses apply in
  VT100 mode only when that ID is selected. A VT420 in VT100 mode now answers
  `CSI ? 64 ; 1 ; 2 ; 6 ; 7 ; 8 ; 9 ; 15 ; 18 ; 21 c` as it does at every other level.

## [0.8.4] - 2026-09-15

Fixes two ways veetee and OpenVMS disagreed: Backspace ran the command it should have been
editing, and EDT lost a line of the file for every line longer than the screen.

- **Changed: Auto Wrap is on at power-up**, the one place veetee's Set-Up differs from DEC's
  factory table. Hosts assume a wrapping terminal: VMS sets terminals `/WRAP` by default, and its
  editors rely on it, EDT writing the line after an 80-column one with no CR LF and expecting the
  wrap to start it. With Auto Wrap off those characters piled into the last column and EDT's
  `ESC [ K` then erased it, so a long line lost its last character and the line after it vanished.
  Set-Up still turns it off, and `SET TERM/NOWRAP` makes VMS insert the CR LF itself.

- **Changed: a soft reset (DECSTR) returns Auto Wrap to the Set-Up value** instead of resetting it,
  as DEC's table has it. EDT sends DECSTR as it exits, so with the old behaviour turning Auto Wrap
  on fixed one editing session and the next was broken again.

- **Fixed (Telnet)**: veetee asked for the Telnet BINARY option on every connection, and OpenVMS
  answers that by putting the terminal in PASSALL, where the driver passes input through untouched:
  DELETE stopped erasing (it ended the line instead, so `DIR` ran as soon as Backspace was pressed)
  and DCL command recall went with it. BINARY is now offered only when asked for, with
  `--telnet-binary` or `telnet-binary = true` in a saved connection; a host that proposes it is
  still answered. On OpenVMS, `SET TERMINAL/INTERACTIVE` undoes an earlier session's PASSALL.

## [0.8.3] - 2026-09-15

Fixes veetee failing to start on Windows computers whose graphics driver left an old
Vulkan loader in System32.

- **Fixed (Windows)**: veetee would not start on computers whose graphics driver leaves an old
  Vulkan loader in System32 ("The procedure entry point vkBindImageMemory2 could not be located
  in the dynamic link library ...\bin\libgtk-4-1.dll"). GTK imports the Vulkan loader, which
  `ldd` resolves to the System32 copy, so it was never bundled; the zip now ships MSYS2's
  `vulkan-1.dll` next to the other libraries, and `bundle.sh` fails if anything else GTK imports
  is left to the machine's own copy.

## [0.8.2] - 2026-09-14

Fixes local shells and SSH in the Flatpak, and adds signing of Windows releases.

- **Fixed (Flatpak)**: local shells started as `/bin/sh` without job control ("can't access tty"),
  and `--ssh` could not ask for a password (it tried `ssh-askpass` instead). The host program now
  gets the terminal as its controlling terminal, and the local shell is the user's login shell
  from the host.

- **Signing Windows releases**: `packaging/windows/sign-release.ps1` signs a published release's
  executables with the project's Certum SimplySign certificate on a Windows computer, rebuilds the
  zip with identical contents and replaces it and `SHA256SUMS` on the release.

## [0.8.1] - 2026-09-14

Completes the planned work of milestone M8: screen reader support, much faster output, a Flatpak
bundle and the means to sign Windows builds.

- **Flatpak (M8)**: a Flatpak manifest (`packaging/flatpak`) on the GNOME 50 runtime, built in CI
  and attached to releases as a `.flatpak` bundle. In the sandbox, local shells, commands and SSH
  run on the host through `flatpak-spawn --host`; Telnet, serial lines and sound work directly.
- **Signed Windows builds (M8)**: release builds sign `veetee.exe` and `vt-headless.exe` when the
  repository has a code-signing certificate (see `packaging/windows/SIGNING.md`).
- An application icon and AppStream metadata, installed by the Debian package and tarball too.
- **Accessibility (M8)**: the terminal is exposed to screen readers such as Orca as a terminal
  named "Terminal": its text is the lines on the screen (or Set-Up while it is open), the caret is
  the cursor, and output is reported as the text removed and inserted, so new lines and typed
  characters are spoken rather than the whole screen. This works while the window is hidden too.
- **Changed**: veetee now needs GTK 4.14 or later (for accessible text).
- **Faster output (M8)**: plain text and DEC line drawing are written a run at a time, lines that
  scroll off reuse memory instead of allocating, smooth scroll is only recorded where it is
  shown, and control sequence parameters are parsed in a tight loop. The terminal model now
  handles about 105 MB/s of text lines (from 29), 82 MB/s of cursor-addressed forms (from 39)
  and 60 MB/s of line drawing, renditions or UTF-8 (from 23–32), measured with
  `cargo run --release -p vt-core --example terminal_throughput`. A local flood now runs within
  about 15% of the speed of the pseudo-terminal itself.

## [0.8.0] - 2026-09-14

The first release of milestone M8, power-user features: saved connections, session logs,
history review and search, and copy and paste that translate characters.

- **Saved connections (M8)**: *Connections…* in the window menu lists saved connections (Telnet,
  SSH, serial, local shell or command, with the terminal model, phosphor and sessions) to open in
  a new window, add, edit or delete; *Save as Connection…* saves the current window's. They live
  in `profiles.toml` in the configuration directory. `--profile NAME` opens one from the command
  line, with other options overriding it, and `--list-profiles` lists them. A window opened from a
  saved connection is titled with its name.
- **History review and search (M8)**: the mouse wheel and Shift+PgUp/PgDn move the screen back
  through the lines that scrolled off the page; typing or host output returns to the page, as
  with the VT520's review of previous lines. *Find…* (Ctrl+Shift+F) searches the history and page
  newest first, highlighting and scrolling to each match, and keeps its place while output
  continues. A saved keymap from an earlier version needs Shift+Page_Up and Shift+Page_Down bound
  to *Review Back* and *Review Forward* in *Keyboard Map…* (or *Restore Defaults*).
- **Copy and paste (M8)**: pasted text is translated for the host's character sets instead of
  losing what they cannot send: typographic quotes, dashes and ellipses become ASCII, accented
  letters without an encoding their base letter, separately written accents are joined, and
  anything else is `?`. Control characters other than tab and line breaks are no longer pasted.
  Copied text turns DEC Technical pieces, soft-font characters and Display Controls symbols into
  Unicode or names. Selections can be made in the reviewed history and stay with their text as
  output scrolls.
- **Session logs (M8)**: *Log to File…* in the window menu logs the active session's text as it
  is shown — character sets translated to Unicode, a line break for each new line, the status
  line left out — with optional timestamps on each line (*Timestamp Log Lines*). `--log FILE`
  (with `~` and date placeholders), `--log-timestamps` and `--log-raw` (the host's bytes as
  received) do the same from the command line, and saved connections can log every time they
  open.

## [0.7.1] - 2026-09-14

Finishes milestone M7.

- **VT500 Set-Up (M7)**: the VT510, VT520 and VT525 open the menu-driven Set-Up of their
  reference manual (EK-VT520-RM chapter 2): pull-right menus with check boxes and radio buttons,
  Answerback and Tab Set-Up dialog boxes, and the Set-Up summary line (line settings, character
  set, keyboard, emulation) on the status line. Features veetee does not have are dimmed. The
  VT100 to VT420 models keep the VT420 Set-Up screens.
- **More Set-Up features**: word size and parity separately (with the unchecked parities), 57.6K
  to 115.2K baud, transmit and receive flow control and threshold, transmit and function key rate
  limits, modem speeds, clear on column change (DECNCSM), CRT saver time, energy saver, zero
  style, host wake-up, overscan, NUL handling, half duplex and auto repeat rate. They are saved
  and follow the host's DECSPP, DECSFC, DECSTRL, DECSCS and VT500 modes.
- **Display Controls (M7)**: Display Set-Up's Display Controls (CRM) shows control characters as
  DEC's small stacked names instead of performing them, for debugging host output; DECSR leaves
  it.
- **Fonts (M7)**: hand-drawn ASCII for 48-line screens (10×8 and 6×8 dots) and hand-drawn Greek,
  Cyrillic and Hebrew for 132 columns.
- **Changed**: DECSR (secure reset) now works on the VT420, as its programmer reference documents.

## [0.7.0] - 2026-09-14

Milestone M7, the look and feel of the terminal: DEC character cell fonts (in 0.6.0), Set-Up,
sound, smooth scrolling and a CRT picture. The VT500 menu-driven Set-Up and more hand-drawn
fonts are still to come.

- **CRT look (M7)**: the page is drawn through a post-processing pass with a soft glow around lit
  dots and phosphor afterglow that fades with the phosphor's persistence (short for P4 white,
  longer for P1 green and P3 amber), both on by default, and an optional curved screen with
  darker corners. *Screen* in the window menu switches them, and the choice is remembered.
- **CRT saver**: the screen blanks after the Set-Up idle time (30 minutes on a VT420, DECCRTST on
  a VT500); a key or host output wakes it.
- **Visible bell**: optionally flashes "Bell" on the status line when the bell sounds.
- **Smooth scroll (M7)**: with DECSCLM set, each line scrolls smoothly at DEC speed — Smooth 2
  at 9 lines a second, Smooth 4 (DECSSCLS or Display Set-Up) at 18 — and host output waits
  meanwhile, so the host is flow-controlled as on the terminal. Scrolling regions and left/right
  margins scroll within their bounds.
- **Changed**: the VT420 and VT510 now power up in smooth scroll, as their programmer references
  document; the VT520 and VT525 keep jump scroll (their factory Set-Up). Choose *Jump Scroll* in
  Display Set-Up (F3) and Save for fast output.
- **Sound (M7)**: the warning bell and margin bell sound as 125 ms beeps and keys click with a
  2 ms beep (EK-VT520-RM section 2.17), at the keyclick, warning bell and margin bell volumes of
  Keyboard Set-Up (factory: click and warning bell high, margin bell off) or DECSKCV, DECSWBV and
  DECSMBV. DECPS plays notes C5–C7 in turn. Sound uses the system's default audio output (cpal);
  building on Linux needs the ALSA development package.
- **Set-Up (M7)**: F3 opens the terminal's Set-Up screens, laid out like a VT420's: the Set-Up
  Directory with Clear Display, Clear Comm, Reset Session, Recall, Save, Default and Screen Align,
  and the Global, Display, General, Communications, Printer, Keyboard and Tab screens. The host is
  held while Set-Up is open, changes take effect on leaving it, and Save keeps the settings per
  model and session as the power-up settings that RIS and Recall restore. Global Set-Up's Local
  puts the host on hold and echoes typed characters. A VT420's General Set-Up terminal ID
  changes its device attributes reply. *Set-Up* is also in the window menu.
- Fixed: box-drawing and other lines that run across cells showed a faint step half-way along
  each cell.
- Website screenshots are recaptured with the new fonts, and the site covers the Windows build.

## [0.6.0] - 2026-09-13

Milestone M6 (keyboard and OpenVMS applications) and the first part of M7 (fonts), and the first
Windows build. M5 (RFC 2217 and LAT) is still to come.

- `--phosphor white|green|amber` selects the phosphor colour at start.
- **Keyboard (M6)**: the PC-to-LK401 map is a TOML keymap with a visual *Keyboard Map* editor
  (`~/.config/veetee/keymap.toml`, `--keymap FILE`); VT500 modified function, editing and cursor
  keys send DECFNK sequences; Ctrl with the cursor and Prev/Next keys pans through page memory.
- **VT520 key programming**: DECPFK, DECPAK, DECCKD, DECPKA, DECRQKD with DECRPFK/DECRPAK,
  DECRQPKFM and DECRQKT, by LK411 key station; programmed local functions run veetee's own.
- **Session recordings**: `--record` writes `.vtrec` recordings with checkpoints (Ctrl+Shift+M),
  leaving typed keys out unless `--record-keys` is given; `vt-headless replay` and
  `cargo xtask openvms` compare checkpoint screens.
- Glyph dots are box-filtered, so strokes keep an even weight at any window size.
- **Fonts (M7)**: the VT320, VT420 and VT500 series draw with new original fonts on DEC's
  character cells: 10×16 dots for 80 columns and 6×16 for 132 columns at 24 lines, 10 and 8
  dots high at 36 and 48 lines (EK-VT420-RM table 5-5). The page has the proportions of their
  800×400 raster of 1:1.4 pixels, with one scan line per dot row. The fonts cover DEC Greek,
  Hebrew, Turkish and Cyrillic and the ISO Latin-2, Greek, Hebrew, Cyrillic and Latin-5 sets.
  132-column mode uses the narrow font instead of squeezing the 80-column glyphs. Screen sizes
  without a hand-drawn font, and 132-column VT100/VT220 text, are resampled so separate strokes
  stay separate. VT52–VT220 keep the 10×10 font.
- Fixed: keypad digits, `.`, `/`, `*` and `−` went to the input method as text, so EDT and EVE
  received digits instead of application keypad sequences; host-programmed main keypad keys
  (DECPAK) were bypassed the same way. veetee now handles mapped keys first.
- **Windows**: a zip for 64-bit Windows 10 (1809) and later, with the GTK runtime included. Local
  command windows run on a Windows pseudo console (ConPTY), SSH uses Windows' OpenSSH client and
  serial lines use COM ports. CI builds and tests on Windows.
- Project website in `site/`, published with GitHub Pages; screenshots are regenerated from
  veetee's renderer with `site/capture/capture.sh`.

## [0.4.0] - 2026-09-13

Milestone M4: the VT510, VT520 and VT525.

### Emulation

- **VT510/VT520/VT525**: CHA, HPA, VPA, HPR, VPR, CNL, CPL, CHT, CBT;
  DECST8C; cursor styles (DECSCUSR); DECNCSM; the VT500 private modes; Set-Up selections with
  DECRQSS reports; DECTID, DECTME, DECSR; answerback, banner and time-of-day loading; session
  names as the window title (DECSWT); VT500 page and screen sizes; keyboard language, local
  function key and local function controls; DECES, DECUS, DECSPMA; DECPS parsing.
- **VT525 colour**: SGR colours, DECAC, DECATC and alternate text colour modes, DECSTGLT,
  DECCTR/DECRSTS colour tables in RGB or HLS, DECECM, DECBBSM.
- **Dual sessions**: `--sessions 2` or *Open Second Session* splits the window into two
  sessions with their own connections; F4 (Session) switches the keyboard, DECES activates a
  session, session names from DECSWT label each half and title the window.
- **VT500 character sets**: DEC Greek, Hebrew, Turkish and Cyrillic; ISO Latin-2, Greek, Hebrew,
  Latin-Cyrillic and Latin-5; Greek, Hebrew, Turkish, Serbo-Croatian and Russian NRCS.
- Replies sent before a reset in the same data are no longer lost.

### Development

- vttest 11.4 scripts; esctest2 runs at VT level 5 against a VT525. Screen snapshots show colour
  indexes.

## [0.3.0] - 2026-09-13

The first release: VT100 through VT420 emulation with local, Telnet, SSH and serial connections.

### Emulation

- **VT100/VT102/VT52** — cursor control, scrolling regions, tabs, double-width and double-height
  lines, DEC Special Graphics, insert/delete, screen alignment, VT52 mode; vttest menus 1–8 pass.
- **VT220/VT320** — 8-bit controls and DECSCL levels; G0–G3 with locking and single shifts;
  national replacement character sets, DEC Supplemental, DEC Technical, ISO Latin-1 and the
  user-preferred supplemental set; soft fonts (DECDLD); user-defined keys (DECUDK); protected
  fields with selective erase; indicator and host-writable status lines; DECRQM, DECRQSS,
  DECRQPSR/DECRSPS and other reports; vttest 11.1 and 11.2 pass.
- **VT420** — left/right margins; DECIC/DECDC and DECBI/DECFI; rectangular area operations
  (DECCRA, DECERA, DECSERA, DECFRA, DECCARA, DECRARA, DECSACE); DECRQCRA checksums matching the
  hardware; page memory with page movement, DECSLPP, DECSCPP and 24/36/48-line screens (SU/SD
  pan as on DEC terminals); macros (DECDMAC/DECINVM); terminal state reports (DECRQTSR/DECRSTS);
  vttest 11.3 passes.
- Models VT100, VT102, VT220, VT320 and VT420 (default); VT510/VT520/VT525 identify themselves but
  add no VT500 features yet.
- DEC factory defaults throughout; xterm behaviour is an opt-in extension used for esctest2.

### Application

- GTK4/libadwaita window with an OpenGL renderer: original DEC-style bitmap font, dot stretching,
  scan lines, 132-column mode, soft fonts, blinking, white/green/amber phosphors, full screen.
- LK401 keyboard layout on PC keyboards, including PF1–PF4, the editing keypad, Help/Do, F6–F20,
  user-defined keys, Hold Screen, Break and Answerback.
- Mouse selection (word selection understands VMS file specifications), primary selection,
  clipboard copy and paste.
- Connections: local shell or command (PTY); native Telnet with binary mode, terminal type, window
  size and BREAK; SSH through the system OpenSSH client; serial lines with picocom-style options,
  DEC default 9600 8N1 XON/XOFF, and line break. Network and serial windows stay open after the
  connection closes.
- `--record` saves host output for bug reports and replay.

### Development

- `vt-headless` session scripts with golden screen snapshots; `cargo xtask vttest` and
  `cargo xtask esctest` against pinned upstream versions, run in CI together with unit tests,
  `cargo deny` licence checks and parser fuzzing.
- `cargo xtask dist` builds the release tarball and Debian package.

[Unreleased]: https://github.com/issinoho/veetee/compare/v1.1.2...HEAD
[1.1.2]: https://github.com/issinoho/veetee/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/issinoho/veetee/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/issinoho/veetee/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/issinoho/veetee/compare/v0.8.12...v1.0.0
[0.8.12]: https://github.com/issinoho/veetee/compare/v0.8.11...v0.8.12
[0.8.11]: https://github.com/issinoho/veetee/compare/v0.8.10...v0.8.11
[0.8.10]: https://github.com/issinoho/veetee/compare/v0.8.9...v0.8.10
[0.8.9]: https://github.com/issinoho/veetee/compare/v0.8.8...v0.8.9
[0.8.8]: https://github.com/issinoho/veetee/compare/v0.8.7...v0.8.8
[0.8.7]: https://github.com/issinoho/veetee/compare/v0.8.6...v0.8.7
[0.8.6]: https://github.com/issinoho/veetee/compare/v0.8.5...v0.8.6
[0.8.5]: https://github.com/issinoho/veetee/compare/v0.8.4...v0.8.5
[0.8.4]: https://github.com/issinoho/veetee/compare/v0.8.3...v0.8.4
[0.8.3]: https://github.com/issinoho/veetee/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/issinoho/veetee/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/issinoho/veetee/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/issinoho/veetee/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/issinoho/veetee/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/issinoho/veetee/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/issinoho/veetee/compare/v0.4.0...v0.6.0
[0.4.0]: https://github.com/issinoho/veetee/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/issinoho/veetee/releases/tag/v0.3.0
