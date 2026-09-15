# Changelog

All notable changes to veetee are listed here. Until 1.0 the minor version follows the project
milestones (0.3 = M3). The format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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

[Unreleased]: https://github.com/issinoho/veetee/compare/v0.8.5...HEAD
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
