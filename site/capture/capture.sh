#!/bin/sh
# Regenerates the website screenshots from veetee's own renderer.
# Run from the repository root after `cargo build --release -p veetee`.
set -e
here=$(cd "$(dirname "$0")" && pwd)
img="$here/../img"
bin=target/release/veetee
python3 "$here/scenes.py"
for scene in hero colour graphics; do
  printf '#!/bin/sh\ncat %s/%s.bin\nsleep 30\n' "$here" "$scene" > "$here/$scene.sh"
  chmod +x "$here/$scene.sh"
done
# A clean configuration, so saved Set-Up and preferences on this machine do
# not show. The VT420 saves jump scroll, so scenes are complete when captured.
config=$(mktemp -d)
curved=$(mktemp -d)
trap 'rm -rf "$config" "$curved"' EXIT
mkdir -p "$config/veetee" "$curved/veetee"
printf 'scroll=jump\n' > "$config/veetee/setup-vt420-session1.conf"
cp "$config/veetee/setup-vt420-session1.conf" "$curved/veetee/"
printf 'glow=1\nafterglow=1\ncurvature=1\n' > "$curved/veetee/appearance.conf"
shot() { # name model phosphor command [config dir]
  XDG_CONFIG_HOME="${5:-$config}" VEETEE_CAPTURE="$here/$1.ppm" VEETEE_CAPTURE_DELAY_MS=2500 \
    timeout 25 "$bin" --model "$2" --phosphor "$3" --command "$4" || true
}
shot vt420-order-entry vt420 white "$here/hero.sh"
shot vt420-graphics vt420 white "$here/graphics.sh"
shot vt525-colour vt525 white "$here/colour.sh"
shot vt420-132-amber vt420 amber "$here/132-columns.sh"
shot vttest-green vt420 green target/conformance/vttest-20251205/vttest
shot vt420-curved-green vt420 green "$here/hero.sh" "$curved"
VEETEE_STARTUP_ACTION=setup shot vt420-setup vt420 white "$here/hero.sh"
VEETEE_STARTUP_ACTION=setup shot vt520-setup vt520 white "$here/hero.sh"
python3 - "$here" "$img" <<'PY'
import sys, pathlib
from PIL import Image
here, img = map(pathlib.Path, sys.argv[1:])
for ppm in here.glob("*.ppm"):
    Image.open(ppm).convert("RGB").save(img / (ppm.stem + ".png"), optimize=True)
    ppm.unlink()
PY
echo "The dual-sessions picture is a whole-window capture (VEETEE_CAPTURE_WINDOW) with --sessions 2."
