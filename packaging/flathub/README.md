# Submitting veetee to Flathub

> **Decided against, 17 September 2026.** veetee is not being submitted to Flathub, and this
> directory is kept as the record of why rather than as a plan. Three things stood in the way,
> and none of them is a packaging problem:
>
> - `flatpak-builder-lint` raises `appid-url-not-reachable`, because the app ID
>   `com.issinoho.Veetee` obliges `https://issinoho.com` to answer and it does not — see below.
> - It also raises `finish-args-flatpak-spawn-access` and `finish-args-home-filesystem-access`.
>   Both need exceptions granted by pull request, and Flathub's documentation says such
>   sandbox-escape exceptions "will not be granted if there are signs of LLM usage in the software
>   or in the exception PR". This repository's history carries `Co-Authored-By` lines throughout.
> - Flathub also states that "AI tools or agents must not open or automate Flathub submission pull
>   requests, or generate their commit messages, descriptions, review comments, or replies", so
>   the submission would have to be written and carried entirely by hand.
>
> The permissions the exceptions would cover — host shells, SSH and writing logs where the user
> asks — are the ones that make veetee useful, so narrowing them to suit the store was not worth
> it. `packaging/flatpak/` is unaffected: the Flatpak bundle still builds in CI and ships with
> every release, and `flatpak install` on the built bundle works as it always did.
>
> What follows is the procedure as it stood, should the decision ever be revisited.

`com.issinoho.Veetee.yml` here is the same build as
[`../flatpak/com.issinoho.Veetee.yml`](../flatpak/com.issinoho.Veetee.yml), with the one change
Flathub requires: the source is a `git` source pinned to a tag and the commit that tag names,
rather than the working directory. Keep the two in step — the one that builds in CI is the other.

## Do this part on Linux

There is no flatpak in WSL, and installing one there is not worth the doubt: bubblewrap under
WSL2 is not a configuration anybody tests. Use a Linux desktop.

You do not need a checkout. The manifest builds from the tag, so two downloaded files in an empty
directory are the whole input — which is also the point of the exercise, since a Flathub build
never sees your working tree.

```sh
sudo apt install flatpak flatpak-builder        # or: sudo dnf install flatpak flatpak-builder
flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo

mkdir -p ~/veetee-flathub && cd ~/veetee-flathub
base=https://raw.githubusercontent.com/issinoho/veetee/main
curl -LO $base/packaging/flathub/com.issinoho.Veetee.yml
curl -LO $base/packaging/flatpak/cargo-sources.json
curl -LO $base/data/com.issinoho.Veetee.metainfo.xml
```

### 1. Build it from the tag

`--install-deps-from` pulls the GNOME 50 runtime, its SDK and the Rust extension, which is the
several-gigabyte part and only happens once.

```sh
flatpak-builder --user --install --force-clean \
    --install-deps-from=flathub build com.issinoho.Veetee.yml
flatpak run com.issinoho.Veetee
```

This proves the one thing CI does not: that it builds from the tag alone, offline, against
`cargo-sources.json`.

### 2. Run Flathub's linter

Reviewers go by this. It is a Flatpak of about a gigabyte, so install it deliberately.

```sh
flatpak install flathub org.flatpak.Builder
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest com.issinoho.Veetee.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream com.issinoho.Veetee.metainfo.xml
```

`appstreamcli validate data/com.issinoho.Veetee.metainfo.xml` covers most of the second without
the download, and passes today — with one note about the uppercase in the component ID, which is
not worth breaking every installed copy to silence.

### 3. Before each later release

- **Update the tag and commit** in the manifest: `git rev-list -n 1 vX.Y.Z`.
- **Regenerate `cargo-sources.json`** with flatpak-builder-tools' `flatpak-cargo-generator.py`,
  but only if the dependencies actually changed. Flathub builds offline, so every crate must be
  listed. To tell whether it is needed, look for new external crates in the lock file since the
  file was last written:

  ```sh
  git diff LAST_GENERATED_COMMIT HEAD -- Cargo.lock | grep -E '^\+(source|checksum) '
  ```

  Nothing printed means nothing to do: workspace version bumps and new path dependencies never
  reach it. That was the case for 0.8.12, whose only lock changes were the bump and the new local
  crates `vt-lat` and `vt-lat-helper`.

## Submitting

1. Fork [flathub/flathub](https://github.com/flathub/flathub) and make a branch.
2. Add `com.issinoho.Veetee.yml` and `cargo-sources.json` at the top level of it.
3. Open the pull request **against the `new-pr` branch**, not `master`.
4. A bot builds it and reports. Reviewers ask about permissions; the answers are below.
5. Once accepted, the app gets a repository of its own (`flathub/com.issinoho.Veetee`) and later
   releases are pull requests there, changing the tag and commit.

### The app ID, and the domain behind it

Flathub wants an ID that is the reverse-DNS of a domain the developer controls. `issinoho.com`
resolves (81.129.52.28, no-ip nameservers), so it is owned, but it serves nothing: as of
17 September 2026 nothing answered on port 80 or 443, checked from two networks, and winget's
validation bot found the same from a third — which is what
[winget-pkgs#436670](https://github.com/microsoft/winget-pkgs/pull/436670) failed on. (`www` is a
different host, 90.211.19.100, which answers on 443 but has no certificate for the name.) Worth
settling, because:

- **Acceptance.** `com.issinoho.Veetee` is defensible on ownership alone, but a reviewer may ask,
  the homepage in the metainfo being `issinoho.github.io`.
- **Verified publisher**, later, needs a token served at
  `https://issinoho.com/.well-known/org.flathub.VerifiedApps.txt`. Flathub's other route, proving
  it through the source host, is open only to `io.github.*` IDs — so with this ID the website is
  the only way.

Changing the ID is not a small thing: it is the desktop file, the metainfo, the icon names, the
D-Bus name and every already-installed copy. Ownership of the domain is the better thing to fix.

## The permissions, and why

Reviewers question anything broad, and three of these are. The manifest carries the same reasons
in comments; this is the longer form.

| Permission | Why |
|---|---|
| `--device=all` | Serial lines. A console port is still how a VAX, an Alpha and most DEC hardware is reached, and `--serial /dev/ttyUSB0` is a first-class connection. Flatpak has no narrower permission for serial devices: `--device=dri` does not cover them, and there is no `--device=serial`. |
| `--talk-name=org.freedesktop.Flatpak` | Local shells, `--command` and SSH run on the host through the portal, so the user's own shell, `~/.ssh`, agent and `known_hosts` apply. A sandboxed `ssh` with no access to the user's keys would be worse than not offering SSH. |
| `--filesystem=home` | Session logs and `.vtrec` recordings are written where the user names them, on the command line or in a saved connection, and read back for replay. Reviewers may ask for this to be narrowed; if so, the fallback is the documents portal for files chosen through a dialog, which would mean losing paths in saved connections. |

LAT does not work in the Flatpak at all and no permission changes that: the sandbox refuses
`AF_PACKET` outright. That is documented rather than worked around.
