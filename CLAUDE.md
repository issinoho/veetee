# Working on veetee

veetee is a DEC VT terminal emulator (VT52 to VT525) in Rust with a GTK 4 / libadwaita front end,
for Linux and Windows. Start with [README.md](README.md) for the layout and commands,
[docs/ROADMAP.md](docs/ROADMAP.md) for what is left, and [docs/compat-matrix.md](docs/compat-matrix.md)
for the state of every control function.

## DEC and OpenVMS first

Every default is DEC and OpenVMS behaviour, not xterm or Unix convention. The user was emphatic:
"I cannot emphasise enough how much this should default to everything DEC and OpenVMS".

- DEC documentation wins: DEC STD 070, the VT420/VT510/VT520 programmer references
  (EK-VT420-RM, EK-VT510-RM, EK-VT520-RM) and *Installing and Using* guides. Cite the manual and
  section for behaviour, in code comments and in compat-matrix.md.
- DEC factory Set-Up values, DEC-strict emulation (xterm extensions such as UTF-8, xterm SGR and
  mouse reporting are off unless enabled), 7-bit controls, DEC Supplemental in GR, LK401 keyboard
  semantics (the backarrow key sends DEL), TERM names such as `vt420`.
- The default model is the **VT420**.
- Where xterm or esctest disagree with DEC, DEC wins; record the difference in
  `tests/conformance/esctest/expected-failures.txt` with the manual reference, and offer xterm's
  behaviour only as an option.
- Deliberate exception: Auto Wrap is on at power-up (0.8.4), because OpenVMS sets terminals
  `/WRAP` and EDT depends on it. Keep any new exception this well justified and documented.

## Project decisions

- Rust workspace; the emulator core (`vt-parser`, `vt-core`) has no GTK dependency and stays
  headless-testable; `vt-core` depends only on `vt-parser`.
- Licence MIT OR Apache-2.0: permissive dependencies only (enforced by `cargo deny`); no GPL code,
  so LAT is a clean-room implementation (not from latd).
- Fonts are original pixel designs on DEC character cells, released under the SIL OFL; no ROM dumps.
- SSH uses the system OpenSSH client in a PTY (for `~/.ssh/config`, agents and ProxyJump); Telnet is
  veetee's own client.

## How the user wants to work

- **Commit or push only when asked** ("commit and push"). Never force-push without explicit consent.
- **Never log in to the user's OpenVMS server** (192.168.0.156) or other hosts; ask the user to run
  sessions and recordings there.
- **Recordings** (`.vtrec`) stay outside the repository unless the user asks to add one; they exclude
  typed keys unless `--record-keys`. Root `*.vtrec` files are gitignored.
- **Website** (`site/`): no DEC, Digital, Compaq or HP logos or trademarks.
- Releases: bump the workspace version, move the changelog's Unreleased notes under the new
  version, update version references in README.md, `site/index.html` and
  `data/com.issinoho.Veetee.metainfo.xml`, tag `vX.Y.Z` and push the tag; the Release workflow
  builds the .deb, tarball, Windows zip and Flatpak. The wiki (a separate git repository,
  `issinoho/veetee.wiki`) is updated alongside.

## Checks before committing

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets     # CI builds with -D warnings
cargo test --workspace
cargo xtask vttest                          # vttest must stay all PASS
cargo xtask esctest                         # must match expected-failures.txt exactly
```

Throughput: `cargo run --release -p vt-core --example terminal_throughput` (text lines around
100 MB/s). After changing dependencies, regenerate `packaging/flatpak/cargo-sources.json` with
flatpak-builder-tools' `flatpak-cargo-generator.py`.
