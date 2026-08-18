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

for signing_path in apps/macos/build-app.sh .github/workflows/canary.yml .github/workflows/release.yml; do
  if ! grep -q -- '--entitlements .*FarCooler.entitlements' "$signing_path"; then
    echo "$signing_path does not apply the Mac app entitlements" >&2
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
