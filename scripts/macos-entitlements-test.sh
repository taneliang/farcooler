#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

entitlements="apps/macos/Resources/FarCooler.entitlements"
plutil -lint "$entitlements" >/dev/null

camera=$(/usr/libexec/PlistBuddy -c 'Print :com.apple.security.device.camera' "$entitlements")
if [[ "$camera" != "true" ]]; then
  echo "camera entitlement is missing or false" >&2
  exit 1
fi

# The release and canary signatures can't be read back here, so each signing
# path is read as shell instead: every codesign whose target is the app bundle
# ("$app" or "$APP") has to carry the entitlements flag in that same command,
# continuation lines joined and comments ignored. A grep for the flag anywhere
# in the file passed with it moved onto the nested-binary loop, or left behind
# in a comment.
for signing_path in apps/macos/build-app.sh .github/workflows/canary.yml .github/workflows/release.yml; do
  if ! problem=$(python3 - "$signing_path" 2>&1 <<'PY'
import re, sys
commands, pending = [], ""
for raw in open(sys.argv[1]):
    line = raw.strip()
    if not pending and (not line or line.startswith("#")):
        continue
    if line.endswith("\\"):
        pending += line[:-1] + " "
        continue
    commands.append(pending + line)
    pending = ""
app_signs = [c for c in commands
             if re.match(r"codesign\s", c) and re.search(r'"\$(app|APP)"(\s*(\|\||;|&&|$))', c)]
if not app_signs:
    sys.exit("no codesign of the app bundle")
for c in app_signs:
    if not re.search(r"\s--entitlements\s+\S*FarCooler\.entitlements(\s|$)", c):
        sys.exit("the app's codesign has no --entitlements FarCooler.entitlements: " + c)
PY
  ); then
    echo "$signing_path does not apply the Mac app entitlements ($problem)" >&2
    exit 1
  fi
done

if [[ -n "${1:-}" ]]; then
  app="$1"
  if [[ ! -d "$app" ]]; then
    echo "signed app not found: $app" >&2
    exit 1
  fi

  signed_entitlements=$(mktemp -t farcooler-entitlements)
  trap 'rm -f "$signed_entitlements"' EXIT
  if ! codesign -d --entitlements :- "$app" >"$signed_entitlements" 2>/dev/null; then
    echo "could not read entitlements from $app" >&2
    exit 1
  fi
  signed_camera=$(
    /usr/libexec/PlistBuddy -c 'Print :com.apple.security.device.camera' "$signed_entitlements"
  )
  if [[ "$signed_camera" != "true" ]]; then
    echo "$app is not signed with camera access" >&2
    exit 1
  fi
fi

echo "macOS camera entitlement: OK"
