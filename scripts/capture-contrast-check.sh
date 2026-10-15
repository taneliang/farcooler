#!/bin/bash
# Whether each Increase Contrast capture really differs from its standard twin,
# below the title bar (ov-519 review R1).
#
#   scripts/capture-contrast-check.sh <dir> [stage]
#
# Compares the pixels of every `<stage>-*-contrast.png` in <dir> with its
# twin's, with the top 120 pixels (the title bar at 2x, which changes with the
# window's appearance alone) left out. A pair whose content is the same means
# Increase Contrast never reached the views: the capture proves nothing, and
# this exits 1. Pixels, not bytes: two PNGs of the same pixels can differ byte
# for byte, which is how a `cmp` of the files passed while every pixel below
# the title bar matched.
set -euo pipefail

dir="${1:?usage: capture-contrast-check.sh <dir> [stage]}"
stage="${2:-}"
exec python3 - "$dir" "$stage" <<'PY'
import pathlib, sys
try:
    from PIL import Image, ImageChops
except ImportError:
    sys.exit("capture-contrast-check: needs Pillow (python3 -m pip install --user pillow)")

TOP = 120
folder, stage = pathlib.Path(sys.argv[1]), sys.argv[2]
pattern = f"{stage}-*-contrast.png" if stage else "*-contrast.png"
same = checked = 0
for contrast in sorted(folder.glob(pattern)):
    twin = contrast.with_name(contrast.name.replace("-contrast.png", ".png"))
    if not twin.exists():
        continue
    checked += 1
    a, b = (Image.open(p).convert("RGB") for p in (contrast, twin))
    w, h = a.size
    if a.size == b.size and ImageChops.difference(a.crop((0, TOP, w, h)), b.crop((0, TOP, w, h))).getbbox() is None:
        print(f"capture-contrast-check: {contrast.name} draws the same as {twin.name} below the title bar: "
              "Increase Contrast didn't apply", file=sys.stderr)
        same += 1
print(f"capture-contrast-check: {checked} pairs, {same} the same below the title bar", file=sys.stderr)
sys.exit(1 if same else 0)
PY
