#!/bin/bash
# That four channels produce four icons, and that stable produces none at all.
#
# The banner is what tells a canary from the app someone depends on at a glance,
# so "did it draw anything" is not a question to answer by looking once and
# trusting it afterwards. Stable must come through BYTE-IDENTICAL: it is not
# labeled, and re-encoding it would churn the one asset every channel shares.
#
#   ./scripts/icon-test.sh
set -euo pipefail

cd "$(dirname "$0")/.."
SRC="apps/shared/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
PASS=0
FAIL=0

check() {
  local what="$1" want="$2" got="$3"
  if [ "$want" = "$got" ]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    echo "FAIL: $what" >&2
    echo "  want: $want" >&2
    echo "  got:  $got" >&2
  fi
}

sha() { shasum -a 256 < "$1" | cut -d' ' -f1; }

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

for c in stable canary preview local; do
  swift scripts/icon-label.swift "$c" "$SRC" "$out/$c.png"
done

check "stable is passed through untouched" "$(sha "$SRC")" "$(sha "$out/stable.png")"

for c in canary preview local; do
  if [ "$(sha "$SRC")" = "$(sha "$out/$c.png")" ]; then
    check "$c is labeled" "different from the source" "identical to the source"
  else
    check "$c is labeled" "different from the source" "different from the source"
  fi
done

# The point of the exercise: four channels, four distinguishable icons.
distinct="$(shasum -a 256 "$out"/*.png | awk '{print $1}' | sort -u | wc -l | tr -d ' ')"
check "no two channels share an icon" "4" "$distinct"

# An unknown channel is refused rather than silently unlabeled, for the reason
# version.sh refuses one: a name we cannot read must not pass itself off as
# stable.
set +e
swift scripts/icon-label.swift production "$SRC" "$out/x.png" >/dev/null 2>&1
code=$?
set -e
check "an unknown channel is refused" "1" "$code"

# Verify banner placement: for canary, pixels at the bottom-right corner must
# differ (due to the banner), while center pixels must match (the bear's face is
# unobscured). This catches rotations with the wrong sign.
#
# The banner is painted in the region 850-900 (verified via colorAt sampling
# that shows the canary amber color #E8A21C at these coordinates).
cat > "$out/verify.swift" << 'EOF'
import AppKit
import Foundation

guard CommandLine.arguments.count == 3 else { exit(1) }

let srcPath = CommandLine.arguments[1]
let outPath = CommandLine.arguments[2]

guard let srcData = FileManager.default.contents(atPath: srcPath),
      let outData = FileManager.default.contents(atPath: outPath),
      let srcBitmap = NSBitmapImageRep(data: srcData),
      let outBitmap = NSBitmapImageRep(data: outData) else { exit(1) }

// Sample bottom-right corner region (850-900, 850-900) using colorAt which
// properly handles stride and color space. This region sits on the painted band.
var cornerDiffer = 0
for x in 850...900 {
  for y in 850...900 {
    guard let srcColor = srcBitmap.colorAt(x: x, y: y),
          let outColor = outBitmap.colorAt(x: x, y: y) else { continue }
    if srcColor != outColor {
      cornerDiffer += 1
    }
  }
}

// Sample center of face (510-530, 510-530) must be identical (unobstructed)
var centerDiffer = 0
for x in 510...530 {
  for y in 510...530 {
    guard let srcColor = srcBitmap.colorAt(x: x, y: y),
          let outColor = outBitmap.colorAt(x: x, y: y) else { continue }
    if srcColor != outColor {
      centerDiffer += 1
    }
  }
}

// Bottom-right should have many differences due to banner, center should be identical
if cornerDiffer > 500 && centerDiffer == 0 {
  exit(0)
} else {
  print("corner=\(cornerDiffer) center=\(centerDiffer)")
  exit(1)
}
EOF

if swift "$out/verify.swift" "$SRC" "$out/canary.png" >/dev/null 2>&1; then
  PASS=$((PASS + 1))
else
  FAIL=$((FAIL + 1))
  echo "FAIL: canary banner is in bottom-right corner" >&2
  echo "  banner should differ at bottom-right (850-900) but match at center (510-530)" >&2
fi

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
