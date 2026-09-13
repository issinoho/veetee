# veetee

A DEC VT terminal emulator for the Linux desktop, aiming at SmarTerm/Reflection-class
compatibility: VT52 through VT525, DECforms and FMS applications, DEC-faithful fonts,
and SSH, Telnet, serial and LAT connections.

**Status:** early development — milestone M3: VT420 features (left/right margins, rectangular
area operations and checksums, page memory, macros, terminal state reports) on top of the
VT220/VT320 and VT100/VT102/VT52 layers. vttest VT100–VT420 menus pass headless, and esctest2
runs with every difference from xterm explained against DEC documentation.

## Layout

| Crate | Purpose |
|-------|---------|
| `crates/vt-parser` | Allocation-free DEC STD 070 / ECMA-48 control function parser (7-bit, 8-bit, UTF-8, VT52) |
| `crates/vt-core` | The terminal model: screen, modes, character sets, reports, DEC keyboard codes |
| `crates/vt-transport` | Host connections: local PTY, serial lines, Telnet, SSH (via OpenSSH) |
| `crates/vt-fonts` | Original DEC-style bitmap fonts (SIL OFL) and their parser |
| `crates/vt-keyboard` | PC keyboard → DEC LK401 key map |
| `crates/vt-render` | OpenGL renderer: dot stretching, scan lines, double-size lines, 132 columns |
| `crates/veetee` | The GTK4/libadwaita application |
| `crates/vt-headless` | CLI driver: `trace` parser actions; `run` scripted sessions with golden screen snapshots |
| `xtask` | `cargo xtask vttest` / `esctest` run the conformance suites against pinned upstream versions; `dist` builds release packages |
| `tests/conformance` | vttest session scripts with golden screens; esctest2 expected failures |
| `docs/compat-matrix.md` | Per-function DEC compatibility status and sources |
| `data/` | Desktop entry |
| `fuzz/` | cargo-fuzz targets (nightly) |

## Installing

[Releases](https://github.com/issinoho/veetee/releases) provide a Debian/Ubuntu package and an
x86_64 Linux tarball, both needing GTK 4.12+ and libadwaita 1.5+:

```sh
sudo apt install ./veetee_0.3.0-1_amd64.deb
```

Changes are listed in [CHANGELOG.md](CHANGELOG.md).

## Running

Building from source requires GTK 4.12+ and libadwaita 1.5+ development packages
(`sudo apt install libgtk-4-dev libadwaita-1-dev` on Ubuntu).

```sh
cargo run -p veetee                              # local shell, VT420
cargo run -p veetee -- --telnet vms1             # Telnet (or telnet://vms1:2323)
cargo run -p veetee -- --ssh system@vms1         # SSH via your OpenSSH client and ~/.ssh/config
cargo run -p veetee -- --serial /dev/ttyUSB0     # serial line (see below)
cargo run -p veetee -- --model vt102 --telnet vms1
cargo run -p veetee -- --command 'vttest'        # any program, via /bin/sh -c
cargo run -p veetee -- --record session.bin --telnet vms1   # keep the host output for replay
```

`cargo run -p veetee -- --help` lists every option. More documentation, including OpenVMS
tips, is in the [wiki](https://github.com/issinoho/veetee/wiki).

Telnet negotiates BINARY (so 8-bit DEC controls pass unchanged), terminal type (`VT420`, or the
selected model), window size and suppress-go-ahead; F5 sends a Telnet BREAK. SSH runs the system
`ssh -tt`, so keys, agents, `ProxyJump` and `known_hosts` behave exactly as in a shell, with
`TERM` set to the emulated model. Network and serial sessions keep their window open when the
connection closes, so the final screen can still be read and copied.

The phosphor colour (white P4, green P1, amber P3) and full screen are in the window menu. Below
the page is the VT420 indicator status line (reverse video: printer, Hold Screen, keyboard lock,
page number and cursor position); hosts can switch it to a host-writable status line.

### Copy and paste

Drag to select (double-click selects a word, including whole VMS file specifications such as
`DKA0:[SYS0.SYSCOMMON]LOGIN.COM;1`; triple-click selects a line). The selection is copied to the
primary selection, so middle-click pastes it. Ctrl+Shift+C copies to the clipboard and
Ctrl+Shift+V pastes; both are also on the right-click menu.

### Serial lines

```sh
cargo run -p veetee -- --serial /dev/ttyUSB0                          # 9600 8N1, XON/XOFF
cargo run -p veetee -- --serial /dev/ttyUSB0 -b 9600 -d 8 -p n -s 1 -f n   # picocom-style options
```

Defaults are the DEC factory Set-Up values: 9600 baud, 8 data bits, no parity, 1 stop bit,
XON/XOFF flow control. `-f h` selects RTS/CTS. F5 sends a line break; F1 (Hold Screen) stops
reading so the line is flow-controlled. The port is opened for exclusive use; add yourself to the
`dialout` group (`sudo usermod -aG dialout $USER`, then log in again) rather than running as root.

### Keyboard

The PC keyboard is mapped to LK401 key positions:

| PC key | DEC key |
|--------|---------|
| F1 F2 F3 F4 F5 | Hold Screen, Print Screen, Set-Up, Data/Talk, Break |
| F6–F12, Shift+F1–F10 | F6–F12, F11–F20 (Shift+F5 = Help, Shift+F6 = Do) |
| Ctrl+F5 | Answerback |
| Ctrl+F6–F12, Ctrl+Shift+F1–F10 | User-defined keys (DEC Shift+F6–F20) |
| Insert Home PgUp / Delete End PgDn | Find, Insert Here, Remove / Select, Prev Screen, Next Screen |
| NumLock / * − | PF1 PF2 PF3 PF4 |
| Keypad + (Shift: −) | Keypad , (−) |
| Backspace | `<X]` (sends DEL) |
| Pause, Print Screen, Break (Ctrl+Break) | Hold Screen, Print Screen, Break (Answerback) |
| Ctrl+Shift+C, Ctrl+Shift+V | Copy, Paste |

Set-Up (F3), Print Screen and Data/Talk are not implemented yet.

## Development

```sh
cargo test --workspace
cargo xtask vttest                    # vttest conformance (needs curl, a C compiler, make)
cargo xtask vttest --bless vt102/menu2  # re-record snapshots after reviewing a change
cargo xtask esctest                   # esctest2 against expected-failures.txt (needs git, python3)
cargo xtask dist                      # release tarball, .deb (needs cargo-deb) and SHA256SUMS
cargo run -p vt-headless -- trace --utf8 some-capture.bin
cargo +nightly fuzz run parser        # requires cargo-fuzz
```

## License

Code is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Only permissively-licensed dependencies are accepted (enforced by `cargo deny`).
The bundled fonts are original designs released under the SIL Open Font License
(`crates/vt-fonts/fonts/OFL.txt`).
