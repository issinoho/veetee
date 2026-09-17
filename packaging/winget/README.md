# Submitting veetee to winget

The Windows zip already holds the GTK runtime beside the program, so winget takes it as a
**portable** package: it unpacks the zip and makes aliases to `bin\veetee.exe` and
`bin\vt-headless.exe`. Windows resolves the DLLs from the directory the executable sits in, so
nothing else is needed and nothing is installed into the system.

That means no new artifact to build or sign — the manifests here point at the zip a release
already publishes.

## What is here

They sit in `manifests/` rather than beside this file because `winget validate` reads *every*
file in the directory it is given, and a README is not YAML.

| File in `manifests/` | Holds |
|------|-------|
| `issinoho.veetee.yaml` | the version manifest |
| `issinoho.veetee.installer.yaml` | the zip's URL, its SHA-256, and the paths inside it |
| `issinoho.veetee.locale.en-US.yaml` | name, publisher, licence, description and tags |

## Updating them for a release

```sh
cargo xtask winget 0.8.12
```

It fetches `SHA256SUMS` from that release and rewrites the version, the checksum, the paths inside
the zip (which carry the version) and the release date. Check the result:

```powershell
winget validate --manifest packaging\winget\manifests
```

To try it before submitting anything — this installs veetee on the machine you run it on:

```powershell
winget install --manifest packaging\winget\manifests
veetee                     # the alias should be on the path, in a new shell
winget uninstall issinoho.veetee
```

`--manifest` needs developer mode, or `winget settings` with `"localManifestFiles": true`.

## Submitting

1. Fork [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).
2. Copy the three files to `manifests/i/issinoho/veetee/VERSION/` in the fork, keeping their
   names.
3. Open a pull request against `master`. A bot validates the manifests, installs the package on a
   virtual machine and comments with what it found; a human reviews after that.

The first submission takes longer than later ones, the publisher identity being new. After that
each release is the same three files with a new version and checksum.

## Notes on the manifests

- `MinimumOSVersion: 10.0.17763.0` is Windows 10 1809, which is what the GTK runtime in the zip
  needs and what the README claims.
- `PortableCommandAlias` puts `veetee` and `vt-headless` on the path. Removing the package removes
  the aliases and the unpacked directory.
- The relative paths inside the zip carry the version (`veetee-0.8.12-x86_64-windows\bin\...`), so
  they change with every release. That is what the xtask exists to get right.
