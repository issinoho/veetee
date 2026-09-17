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

# One orig tarball serves every series, and it must be byte-identical between
# them: Launchpad keeps a single veetee_VERSION.orig.tar.xz per archive and
# rejects a second upload that carries the same name with different contents.
# cargo vendor is not reproducible enough to rely on running it twice, so it
# runs once and the result is cached here.
ORIG="$ROOT/target/ppa/veetee_$VERSION.orig.tar.xz"

git -C "$ROOT" rev-parse -q --verify "refs/tags/$TAG" >/dev/null || {
    echo "no such tag: $TAG" >&2; exit 1; }

if [ ! -f "$ORIG" ]; then
    BUILD="$ROOT/target/ppa/.orig-build"
    echo "==> preparing the shared orig tarball from $TAG"
    rm -rf "$BUILD"; mkdir -p "$BUILD/veetee-$VERSION"
    git -C "$ROOT" archive --format=tar "$TAG" | tar -x -C "$BUILD/veetee-$VERSION"

    echo "==> vendoring crates (needs network here; the builder has none)"
    ( cd "$BUILD/veetee-$VERSION" && cargo vendor --locked --versioned-dirs vendor >/dev/null )
    mkdir -p "$BUILD/veetee-$VERSION/.cargo"
    cat > "$BUILD/veetee-$VERSION/.cargo/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

    mkdir -p "$(dirname "$ORIG")"
    tar --create --xz --directory "$BUILD" \
        --exclude-vcs --file "$ORIG" "veetee-$VERSION"
    rm -rf "$BUILD"
    echo "    $ORIG"
else
    echo "==> reusing the shared orig tarball"
    echo "    $ORIG"
fi

# Unpack the source from that exact tarball, so the tree and the orig always
# agree no matter which series is built first.
echo "==> preparing $SRC"
rm -rf "$OUT"; mkdir -p "$OUT"
tar -xf "$ORIG" -C "$OUT"
cp "$ORIG" "$OUT/"

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
