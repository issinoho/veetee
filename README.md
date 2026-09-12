# veetee

A DEC VT terminal emulator for the Linux desktop, aiming at SmarTerm/Reflection-class
compatibility: VT52 through VT525, DECforms and FMS applications, DEC-faithful fonts,
and SSH, Telnet, serial and LAT connections.

**Status:** early development — milestone M1 (VT100/VT102/VT52 core; vttest menus 1–8 pass headless).

## Layout

| Crate | Purpose |
|-------|---------|
| `crates/vt-parser` | Allocation-free DEC STD 070 / ECMA-48 control function parser (7-bit, 8-bit, UTF-8, VT52) |
| `crates/vt-core` | The terminal model: screen, modes, character sets, reports, DEC keyboard codes |
| `crates/vt-transport` | Host connections (local PTY so far) |
| `crates/vt-headless` | CLI driver: `trace` parser actions; `run` scripted sessions with golden screen snapshots |
| `xtask` | `cargo xtask vttest` builds a pinned vttest and runs the conformance suite |
| `docs/compat-matrix.md` | Per-function DEC compatibility status and sources |
| `fuzz/` | cargo-fuzz targets (nightly) |

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
