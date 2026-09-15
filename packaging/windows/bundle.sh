#!/usr/bin/env bash
# Builds target/dist/veetee-VERSION-x86_64-windows.zip: veetee.exe and
# vt-headless.exe with the GTK 4 and libadwaita runtime they need.
#
# Run from the repository root in an MSYS2 UCRT64 shell, after
#   cargo build --release -p veetee -p vt-headless
set -euo pipefail

prefix=${MINGW_PREFIX:-/ucrt64}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
name="veetee-$version-x86_64-windows"
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
# GTK imports the Vulkan loader, which ldd resolves to the copy in System32
# rather than the MSYS2 one, so copy_dlls never sees it. Windows searches the
# program's own folder first, so shipping it keeps GTK off whatever loader the
# machine happens to have: an old graphics driver leaves a Vulkan 1.0 one in
# System32, and GTK then fails to load at all ("The procedure entry point
# vkBindImageMemory2 could not be located"). veetee draws with GL and never
# calls into Vulkan itself.
cp -n "$prefix/bin/vulkan-1.dll" "$dist/bin/"

# Image loaders, for the SVG icons GTK draws.
pixbuf=lib/gdk-pixbuf-2.0/2.10.0
mkdir -p "$dist/$pixbuf/loaders"
cp "$prefix/$pixbuf/loaders/"*.dll "$dist/$pixbuf/loaders/"
copy_dlls "$dist/$pixbuf/loaders/"*.dll
# The cache must name the loaders relative to the installation, which
# gdk-pixbuf resolves against the folder above bin on Windows.
(cd "$dist" && GDK_PIXBUF_MODULEDIR="$pixbuf/loaders" gdk-pixbuf-query-loaders) > "$dist/$pixbuf/loaders.cache"
grep -q '^"lib/' "$dist/$pixbuf/loaders.cache"

# Every other DLL the bundle imports that MSYS2 also ships must come from bin
# as well, or the machine's own copy would be loaded instead; opengl32.dll is
# the exception, since that one has to be the graphics driver's.
imported=$(objdump -p "$dist"/bin/*.exe "$dist"/bin/*.dll "$dist/$pixbuf"/loaders/*.dll | sed -n 's/^	DLL Name: //p' | tr 'A-Z' 'a-z' | sort -u)
missing=
for dll in $imported; do
  case $dll in opengl32.dll) continue ;; esac
  if [ -e "$prefix/bin/$dll" ] && [ ! -e "$dist/bin/$dll" ]; then
    missing="$missing $dll"
  fi
done
if [ -n "$missing" ]; then
  echo "bundle.sh: imported from $prefix/bin but not bundled:$missing" >&2
  exit 1
fi

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
