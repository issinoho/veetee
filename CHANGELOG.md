# Changelog

All notable changes to veetee are listed here. Until 1.0 the minor version follows the project
milestones (0.3 = M3). The format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

Milestone M8 begins.

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

[Unreleased]: https://github.com/issinoho/veetee/compare/v0.7.1...HEAD
[0.7.1]: https://github.com/issinoho/veetee/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/issinoho/veetee/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/issinoho/veetee/compare/v0.4.0...v0.6.0
[0.4.0]: https://github.com/issinoho/veetee/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/issinoho/veetee/releases/tag/v0.3.0
