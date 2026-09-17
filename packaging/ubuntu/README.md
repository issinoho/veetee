# Publishing veetee to a PPA

veetee is published to [`ppa:issinoho/veetee`](https://launchpad.net/~issinoho/+archive/ubuntu/veetee)
for the two current LTS releases. Launchpad builds from a **source** package, not the binary `.deb`
that `cargo xtask dist` makes, and it builds in a chroot with **no network** — so every crate in
`Cargo.lock` is vendored into the orig tarball and `.cargo/config.toml` redirects crates.io at it.
That is the same problem `packaging/flatpak/cargo-sources.json` solves, by a different route.

## Which series, and why only resolute

**Rust decides this, and it is not veetee's own `rust-version` that matters.** The gtk-rs stack —
`glib`, `gio`, `gdk4`, `cairo-rs`, `graphene`, `gdk-pixbuf` — requires **rustc 1.92**, well beyond
the 1.85 the workspace declared until 1.0. The real question for any series is whether the archive
can offer 1.92.

| Series | newest rustc in the archive | libadwaita | Verdict |
|---|---|---|---|
| **resolute** 26.04 LTS | 1.93, the default | 1.9.0 | builds |
| noble 24.04 LTS | 1.91 (`rustc-1.91`, noble-updates) | 1.5.0 | **one version short**; cannot build |
| questing 25.10 | 1.85.1 | — | end of life |
| jammy 22.04 LTS | 1.85 | 1.1.0 | far too old on both counts |

Noble was tried and rejected on evidence rather than on paper: the upload was accepted and the
build failed with `rustc 1.85.1 is not supported by the following packages`, listing the whole
gtk-rs stack. `rustc-1.89` and `rustc-1.91` exist in noble-updates but `rustc-1.92` does not, so
there is nothing to reach for. If Ubuntu ever backports 1.92 or newer to noble, the packaging here
needs no change beyond adding the series back to the build command — `debian/control` already asks
for `rustc (>= 1.92) | rustc-1.92`, and `debian/rules` probes for a toolchain that really is new
enough rather than assuming where it lives.

## Building

```sh
./build-source.sh VERSION SERIES [KEYID]     # e.g. ./build-source.sh 1.0.0 noble 07C28D1449CA9FB8
```

It takes the source from the **release tag** rather than the working tree, so what is uploaded is
what was released, vendors the crates, writes the orig tarball, drops `debian/` in with a changelog
for that series, and builds the source package — signed if a key id is given.

Output lands in `target/ppa/SERIES/`. Upload with:

```sh
dput ppa:issinoho/veetee target/ppa/SERIES/veetee_VERSION-0ubuntu1~SERIES1_source.changes
```

Each series needs its own upload, and a version can only be uploaded once — a rejected or
superseded upload needs the `~series2` suffix bumped.

## Three traps worth knowing

**One orig tarball serves every series, and it has to be byte-identical.** Launchpad keeps a single
`veetee_VERSION.orig.tar.xz` per archive: the first series to be accepted defines it, and a later
upload carrying the same name with different bytes is rejected. `cargo vendor` is not reproducible
enough to survive being run twice, so `build-source.sh` vendors once, caches the tarball at
`target/ppa/veetee_VERSION.orig.tar.xz` and unpacks every series' tree from that exact file. If a
rejection ever mentions the orig tarball, delete the cached one and rebuild every series together.


**`dh_clean` deletes the vendored manifests.** Its `find` removes every `*.orig` in the tree, which
takes out the `Cargo.toml.orig` that each crate's `.cargo-checksum.json` accounts for, and cargo
then refuses the whole vendored source with `failed to calculate checksum of`. `debian/rules`
overrides `dh_clean` with `-Xvendor/`. This fails identically on Launchpad, so it is not something a
local build being green would have saved you from — it *is* what a local build caught.

**`dpkg-source` reads the same `*.orig` default as deletions.** Harmless in itself, but it warns
once per crate. `debian/source/options` excludes `vendor/` and `.cargo/` from the Debian diff, which
is correct anyway: both come wholly from the orig tarball.

## The conformance suites do not run here

`debian/rules` runs the unit tests during the package build but not `cargo xtask vttest` or
`esctest`: both fetch their suites from upstream, which a network-less builder cannot do. Those run
in CI on every push instead.

## Setting up, once

1. A Launchpad account, with the Ubuntu Code of Conduct signed.
2. A signing key registered at <https://launchpad.net/~issinoho/+editpgpkeys> — the key must be on
   `keyserver.ubuntu.com` first (`gpg --keyserver hkps://keyserver.ubuntu.com:443 --send-keys KEYID`;
   the plain `hkp` port is usually blocked). Launchpad then emails an encrypted confirmation to
   decrypt and follow.
3. The PPA itself, created at <https://launchpad.net/~issinoho/+activate-ppa>.

Uploads are signed with the veetee-specific key `07C28D1449CA9FB8`, kept separate from the
loadbearer release key.
