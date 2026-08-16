#!/bin/bash
# That appcast.py produces what Sparkle actually reads, not merely valid XML.
#
# Sparkle ignores an item whose sparkle: attributes do not resolve to its own
# namespace, and it does so SILENTLY — no error, no dialog, just no update
# offered. A test that only greps for the string "sparkle:version" would pass
# on a feed with the wrong xmlns and never catch it, so this asserts the
# namespace resolves, not just that the prefix appears.
#
#   ./scripts/appcast-test.sh
set -euo pipefail

cd "$(dirname "$0")/.."
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

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

VERSION="1.2.3"
BUILD="456"
URL="https://updates.farcooler.com/canary/Far%20Cooler-456.dmg"
LENGTH="98765432"
SIGNATURE="AbCdEf1234567890signature=="
NOTES="https://github.com/farcooler/farcooler/releases/tag/v1.2.3"

./scripts/appcast.py \
  --channel canary --version "$VERSION" --build "$BUILD" \
  --url "$URL" --length "$LENGTH" --signature "$SIGNATURE" --notes "$NOTES" \
  >"$out/appcast.xml"

check "the enclosure URL is present" "present" \
  "$(grep -qF "url=\"$URL\"" "$out/appcast.xml" && echo present || echo missing)"

check "sparkle:edSignature carries the signature" "present" \
  "$(grep -qF "sparkle:edSignature=\"$SIGNATURE\"" "$out/appcast.xml" && echo present || echo missing)"

check "sparkle:version equals the build number" "present" \
  "$(grep -qF "sparkle:version=\"$BUILD\"" "$out/appcast.xml" && echo present || echo missing)"

check "sparkle:shortVersionString equals the marketing version" "present" \
  "$(grep -qF "sparkle:shortVersionString=\"$VERSION\"" "$out/appcast.xml" && echo present || echo missing)"

# The floor is read from Package.swift's own platforms: line, never hardcoded
# here, so a bump to that file cannot leave this test agreeing with a stale
# expectation. See apps/macos/Package.swift's `platforms: [.macOS("...")]`.
FLOOR="$(sed -n 's/.*\.macOS("\([^"]*\)").*/\1/p' apps/macos/Package.swift | head -1)"
[ -n "$FLOOR" ] || { echo "could not read the platform floor from apps/macos/Package.swift" >&2; exit 1; }

check "sparkle:minimumSystemVersion matches Package.swift's platform floor" "present" \
  "$(grep -qF "<sparkle:minimumSystemVersion>$FLOOR</sparkle:minimumSystemVersion>" "$out/appcast.xml" && echo present || echo missing)"

check "the output is well-formed XML" "ok" \
  "$(python3 -c 'import sys, xml.dom.minidom; xml.dom.minidom.parse(sys.argv[1])' "$out/appcast.xml" >/dev/null 2>&1 && echo ok || echo "not well-formed")"

check "exactly one item is present" "1" \
  "$(python3 -c 'import sys, xml.etree.ElementTree as ET; print(len(ET.parse(sys.argv[1]).findall(".//item")))' "$out/appcast.xml")"

# Well-formed is not the same as CORRECT: parse it as a namespaced document and
# confirm every sparkle:-prefixed thing actually lives in Sparkle's namespace,
# rather than trusting that the prefix printed is the prefix that resolves.
check "sparkle: attributes resolve to Sparkle's own namespace" "ok" "$(python3 - "$out/appcast.xml" <<'PYEOF'
import sys
import xml.etree.ElementTree as ET

SPARKLE_NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"
tree = ET.parse(sys.argv[1])
item = tree.find(".//item")
enclosure = item.find("enclosure") if item is not None else None

resolved = (
    item is not None
    and enclosure is not None
    and item.find(f"{{{SPARKLE_NS}}}minimumSystemVersion") is not None
    and f"{{{SPARKLE_NS}}}version" in enclosure.attrib
    and f"{{{SPARKLE_NS}}}shortVersionString" in enclosure.attrib
    and f"{{{SPARKLE_NS}}}edSignature" in enclosure.attrib
)
print("ok" if resolved else "not resolved")
PYEOF
)"

# An unknown channel is refused rather than silently producing a feed, for the
# reason version.sh and icon-label.swift refuse one: a name we cannot read
# must not be able to pass itself off as a real channel.
set +e
./scripts/appcast.py --channel production --version 1.0 --build 1 \
  --url "https://example.com/x.dmg" --length 1 --signature x --notes "https://example.com" \
  >/dev/null 2>&1
code=$?
set -e
check "an unknown channel is refused" "1" "$code"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
