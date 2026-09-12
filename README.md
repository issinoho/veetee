# veetee

A DEC VT terminal emulator for the Linux desktop, aiming at SmarTerm/Reflection-class
compatibility: VT52 through VT525, DECforms and FMS applications, DEC-faithful fonts,
and SSH, Telnet, serial and LAT connections.

**Status:** early development (milestone M0 — foundations).

## Layout

| Crate | Purpose |
|-------|---------|
| `crates/vt-parser` | Allocation-free DEC STD 070 / ECMA-48 control function parser (7-bit, 8-bit, UTF-8, VT52) |
| `crates/vt-headless` | CLI driver: `vt-headless trace` prints parser actions for a byte stream |
| `fuzz/` | cargo-fuzz targets (nightly) |

## Development

```sh
cargo test --workspace
cargo run -p vt-headless -- trace --utf8 some-capture.bin
cargo +nightly fuzz run parser        # requires cargo-fuzz
```

## License

Code is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Only permissively-licensed dependencies are accepted (enforced by `cargo deny`).
Bundled fonts will be original designs released under the SIL Open Font License.
