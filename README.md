# veetee

A DEC VT terminal emulator for the Linux desktop, aiming at SmarTerm/Reflection-class
compatibility: VT52 through VT525, DECforms and FMS applications, DEC-faithful fonts,
and SSH, Telnet, serial and LAT connections.

**Status:** early development — milestone M2: VT220/VT320 features (protected fields, national
and DEC Technical character sets, soft fonts, user-defined keys, status line, state reports) on
top of the VT100/VT102/VT52 core. vttest VT100–VT320 menus pass headless.

## Layout

| Crate | Purpose |
|-------|---------|
| `crates/vt-parser` | Allocation-free DEC STD 070 / ECMA-48 control function parser (7-bit, 8-bit, UTF-8, VT52) |
| `crates/vt-core` | The terminal model: screen, modes, character sets, reports, DEC keyboard codes |
| `crates/vt-transport` | Host connections (local PTY so far) |
| `crates/vt-fonts` | Original DEC-style bitmap fonts (SIL OFL) and their parser |
| `crates/vt-keyboard` | PC keyboard → DEC LK401 key map |
| `crates/vt-render` | OpenGL renderer: dot stretching, scan lines, double-size lines, 132 columns |
| `crates/veetee` | The GTK4/libadwaita application |
| `crates/vt-headless` | CLI driver: `trace` parser actions; `run` scripted sessions with golden screen snapshots |
| `xtask` | `cargo xtask vttest` builds a pinned vttest and runs the conformance suite |
| `docs/compat-matrix.md` | Per-function DEC compatibility status and sources |
| `fuzz/` | cargo-fuzz targets (nightly) |

## Running

Requires GTK 4.12+ and libadwaita 1.5+ development packages
(`sudo apt install libgtk-4-dev libadwaita-1-dev` on Ubuntu).

```sh
cargo run -p veetee                  # local shell, VT420
cargo run -p veetee -- --model vt102 # emulate another model
```

The phosphor colour (white P4, green P1, amber P3) is in the window menu.

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
| Ctrl+Shift+V | Paste |

## Development

```sh
cargo test --workspace
cargo xtask vttest                    # vttest conformance (needs curl, a C compiler, make)
cargo xtask vttest --bless vt102/menu2  # re-record snapshots after reviewing a change
cargo run -p vt-headless -- trace --utf8 some-capture.bin
cargo +nightly fuzz run parser        # requires cargo-fuzz
```

## License

Code is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Only permissively-licensed dependencies are accepted (enforced by `cargo deny`).
Bundled fonts will be original designs released under the SIL Open Font License.
