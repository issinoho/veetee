# Changelog

All notable changes to veetee are listed here. Until 1.0 the minor version follows the project
milestones (0.3 = M3). The format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

- `--phosphor white|green|amber` selects the phosphor colour at start.
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

[Unreleased]: https://github.com/issinoho/veetee/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/issinoho/veetee/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/issinoho/veetee/releases/tag/v0.3.0
