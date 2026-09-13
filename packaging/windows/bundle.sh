#!/usr/bin/env bash
# Builds target/dist/veetee-VERSION-windows-x86_64.zip: veetee.exe and
# vt-headless.exe with the GTK 4 and libadwaita runtime they need.
#
# Run from the repository root in an MSYS2 UCRT64 shell, after
#   cargo build --release -p veetee -p vt-headless
set -euo pipefail

prefix=${MINGW_PREFIX:-/ucrt64}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
name="veetee-$version-windows-x86_64"
dist="target/dist/$name"

rm -rf "$dist" "target/dist/$name.zip"
mkdir -p "$dist/bin" "$dist/share/glib-2.0/schemas" "$dist/share/icons"

cp target/release/veetee.exe target/release/vt-headless.exe "$dist/bin/"

# Every DLL from the MSYS2 prefix that the given binaries load, recursively
# (ldd lists the whole tree).
copy_dlls() {
  for binary in "$@"; do
    ldd "$binary"
  done | awk '{ print $3 }' | grep -i "^$prefix/bin/" | sort -u | while read -r dll; do
    cp -n "$dll" "$dist/bin/"
  done
}
copy_dlls "$dist/bin/veetee.exe"
# Loaded at run time rather than linked: veetee binds GL through libepoxy.
cp -n "$prefix/bin/libepoxy-0.dll" "$dist/bin/"

# Image loaders, for the SVG icons GTK draws.
pixbuf=lib/gdk-pixbuf-2.0/2.10.0
mkdir -p "$dist/$pixbuf/loaders"
cp "$prefix/$pixbuf/loaders/"*.dll "$dist/$pixbuf/loaders/"
copy_dlls "$dist/$pixbuf/loaders/"*.dll
# The cache must name the loaders relative to the installation.
root=$(cygpath -m "$(pwd)/$dist")
GDK_PIXBUF_MODULEDIR="$dist/$pixbuf/loaders" gdk-pixbuf-query-loaders \
  | sed -e "s|$root/||g" -e "s|$(pwd)/$dist/||g" > "$dist/$pixbuf/loaders.cache"

# GSettings schemas GTK reads, and the icon themes.
cp "$prefix"/share/glib-2.0/schemas/org.gtk.gtk4.*.gschema.xml "$dist/share/glib-2.0/schemas/"
glib-compile-schemas "$dist/share/glib-2.0/schemas"
cp -r "$prefix/share/icons/hicolor" "$prefix/share/icons/Adwaita" "$dist/share/icons/"

cp README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE THIRD-PARTY.md "$dist/"
cp crates/vt-fonts/fonts/OFL.txt "$dist/fonts-OFL.txt"
cp packaging/windows/README-Windows.txt "$dist/"

# Licences of the bundled libraries (GTK and libadwaita are LGPL-2.1+).
mkdir -p "$dist/licenses"
for dll in "$dist"/bin/*.dll "$dist/$pixbuf"/loaders/*.dll; do
  pacman -Qqo "$prefix/bin/$(basename "$dll")" "$prefix/$pixbuf/loaders/$(basename "$dll")" 2>/dev/null || true
done | sort -u | while read -r package; do
  if [ -d "$prefix/share/licenses/${package#mingw-w64-ucrt-x86_64-}" ]; then
    cp -r "$prefix/share/licenses/${package#mingw-w64-ucrt-x86_64-}" "$dist/licenses/"
  fi
  pacman -Qi "$package" | sed -n 's/^\(Name\|Version\|Licenses\|URL\) *: /\1: /p' >> "$dist/licenses/PACKAGES.txt"
  echo >> "$dist/licenses/PACKAGES.txt"
done

(cd target/dist && zip -qr "$name.zip" "$name")
echo "built target/dist/$name.zip ($(du -h "target/dist/$name.zip" | cut -f1))"
