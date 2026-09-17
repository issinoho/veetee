# Submitting veetee to winget

The Windows zip already holds the GTK runtime beside the program, so winget takes it as a
**portable** package: it unpacks the zip and makes aliases to `bin\veetee.exe` and
`bin\vt-headless.exe`. Windows resolves the DLLs from the directory the executable sits in, so
nothing else is needed and nothing is installed into the system.

That means no new artifact to build or sign — the manifests here point at the zip a release
already publishes.

This is the chosen route for Windows: **there is no Inno Setup or WiX installer**, and none is
planned. `winget uninstall` removes veetee and it registers an Add/Remove Programs entry, so the
only thing given up is a Start menu shortcut — judged not worth a second artifact to build and
code-sign every release.

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
cargo xtask winget 1.0.0
```

It fetches `SHA256SUMS` from that release and rewrites the version, the checksum, the paths inside
the zip (which carry the version) and the release date. Check the result:

```powershell
winget validate --manifest packaging\winget\manifests
```

To try it before submitting anything — this installs veetee on the machine you run it on.
Installing from a local manifest is an admin setting, so enable it once from an *elevated*
shell:

```powershell
winget settings --enable LocalManifestFiles
```

Then, unelevated:

```powershell
winget install --manifest packaging\winget\manifests
veetee                     # the alias is on the path in a new shell, not this one
```

To remove it again, use the identifier `winget list veetee` reports rather than the package
identifier: an install from a local manifest has no source to match against, so
`winget uninstall issinoho.veetee` answers "No installed package found".

```powershell
winget uninstall --id 'ARP\User\X64\issinoho.veetee__DefaultSource'
```

Where winget cannot create symlinks — Developer Mode off, as on an ordinary desktop — it puts the
package's `bin` directory on the user's `PATH` instead of making aliases in `WinGet\Links`. Both
work, but only the second is what the validation VM exercises.

## Submitting

Use Microsoft's own tool rather than forking by hand: `winget-pkgs` is a multi-gigabyte repository
to clone for three small files, and `wingetcreate` forks, branches, commits and opens the pull
request through the API instead.

```powershell
winget install Microsoft.WingetCreate
wingetcreate submit --token (gh auth token) packaging\winget\manifests
```

It lands in `manifests/i/issinoho/veetee/VERSION/`. A bot then validates the manifests and
installs the package on a clean virtual machine; a moderator reviews after that. The first
submission takes longer than later ones, the publisher identity being new — after that each
release is the same three files with a new version and checksum.

`wingetcreate` warns that `--token` can end up in a log, and it is right: the expansion keeps it
out of shell history but not out of the process command line. `wingetcreate token --store` runs
GitHub's device flow once and caches its own token, after which `submit` needs no token at all.

0.8.12 was submitted this way as
[winget-pkgs#436670](https://github.com/microsoft/winget-pkgs/pull/436670).

### Every URL in the manifests has to answer

The validation bot fetches each one, and the first attempt failed on exactly that:

```
Url Validation Error
  https://issinoho.com
    Error Message: A connection attempt failed because the connected party did not
    properly respond after a period of time (issinoho.com:443)
```

`PublisherUrl` was `https://issinoho.com`, which resolves but serves nothing on 80 or 443 —
checked from two networks. It is now `https://github.com/issinoho`, which is the usual answer for
a publisher with no separate site. If the domain is ever stood up, this is the line to change
back; the Flathub README wants the same domain for a verified badge, so the two would be settled
together.

## Notes on the manifests

- `MinimumOSVersion: 10.0.17763.0` is Windows 10 1809, which is what the GTK runtime in the zip
  needs and what the README claims.
- `PortableCommandAlias` puts `veetee` and `vt-headless` on the path. Removing the package removes
  the aliases and the unpacked directory.
- The relative paths inside the zip carry the version (`veetee-1.0.0-x86_64-windows\bin\...`), so
  they change with every release. That is what the xtask exists to get right.
