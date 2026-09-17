#!/usr/bin/env bash
# Build a signed Ubuntu source package of veetee for a PPA.
#
#   ./build-source.sh 1.0.0 resolute [KEYID]
#
# Launchpad builds with no network, so this vendors every crate in Cargo.lock
# into the orig tarball and points .cargo/config.toml at it. The source comes
# from the release tag, not the working tree, so what is uploaded is what was
# released.
set -euo pipefail

VERSION=${1:?usage: build-source.sh VERSION SERIES [KEYID]}
SERIES=${2:?usage: build-source.sh VERSION SERIES [KEYID]}
KEYID=${3:-}

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
OUT="$ROOT/target/ppa/$SERIES"
SRC="$OUT/veetee-$VERSION"
TAG="v$VERSION"

git -C "$ROOT" rev-parse -q --verify "refs/tags/$TAG" >/dev/null || {
    echo "no such tag: $TAG" >&2; exit 1; }

echo "==> preparing $SRC from $TAG"
rm -rf "$OUT"; mkdir -p "$SRC"
git -C "$ROOT" archive --format=tar "$TAG" | tar -x -C "$SRC"

echo "==> vendoring crates (needs network here; the builder has none)"
( cd "$SRC" && cargo vendor --locked --versioned-dirs vendor >/dev/null )
mkdir -p "$SRC/.cargo"
cat > "$SRC/.cargo/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

# The packaging itself is not in the orig tarball: it is the Debian diff.
echo "==> orig tarball"
tar --create --xz --directory "$OUT" \
    --exclude-vcs --file "$OUT/veetee_$VERSION.orig.tar.xz" "veetee-$VERSION"

echo "==> debian/ for $SERIES"
cp -a "$HERE/debian" "$SRC/debian"
cat > "$SRC/debian/changelog" <<EOF
veetee ($VERSION-0ubuntu1~${SERIES}1) $SERIES; urgency=medium

  * veetee $VERSION for $SERIES. See the release notes at
    https://github.com/issinoho/veetee/releases/tag/$TAG

 -- Iain Smith <iain@issinoho.com>  $(date -R)
EOF

echo "==> building source package"
cd "$SRC"
if [ -n "$KEYID" ]; then
    dpkg-buildpackage -S -sa -d -k"$KEYID"
else
    echo "    (unsigned: pass a key id as the third argument to sign)"
    dpkg-buildpackage -S -sa -d -us -uc
fi

echo
echo "==> built in $OUT"
ls -1 "$OUT" | sed 's/^/    /'
echo
echo "Check it, then upload:"
echo "    dput ppa:issinoho/veetee $OUT/veetee_$VERSION-0ubuntu1~${SERIES}1_source.changes"
