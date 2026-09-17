# Submitting veetee to Flathub

`com.issinoho.Veetee.yml` here is the same build as
[`../flatpak/com.issinoho.Veetee.yml`](../flatpak/com.issinoho.Veetee.yml), with the one change
Flathub requires: the source is a `git` source pinned to a tag and the commit that tag names,
rather than the working directory. Keep the two in step — the one that builds in CI is the other.

## Before submitting

1. **Update the tag and commit** in the manifest to the release being submitted:

   ```sh
   git rev-list -n 1 v0.8.12
   ```

2. **Regenerate `cargo-sources.json`** if the dependencies have changed since the last time, with
   flatpak-builder-tools' `flatpak-cargo-generator.py`, and copy it in beside the manifest. Flathub
   builds offline, so every crate has to be listed.

3. **Check it builds from the tag alone**, not from the working tree:

   ```sh
   flatpak-builder --user --install --force-clean build packaging/flathub/com.issinoho.Veetee.yml
   ```

4. **Run Flathub's linter.** It is a Flatpak of about a gigabyte, so install it deliberately:

   ```sh
   flatpak install flathub org.flatpak.Builder
   flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest \
       packaging/flathub/com.issinoho.Veetee.yml
   flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream \
       data/com.issinoho.Veetee.metainfo.xml
   ```

   `appstreamcli validate data/com.issinoho.Veetee.metainfo.xml` covers most of the second one
   without the download, and passes today.

## Submitting

1. Fork [flathub/flathub](https://github.com/flathub/flathub) and make a branch.
2. Add `com.issinoho.Veetee.yml` and `cargo-sources.json` at the top level of it.
3. Open the pull request **against the `new-pr` branch**, not `master`.
4. A bot builds it and reports. Reviewers ask about permissions; the answers are below.
5. Once accepted, the app gets a repository of its own (`flathub/com.issinoho.Veetee`) and later
   releases are pull requests there, changing the tag and commit.

To show as a verified publisher afterwards, Flathub asks for a token to be published at
`https://issinoho.com/.well-known/org.flathub.VerifiedApps.txt`.

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
