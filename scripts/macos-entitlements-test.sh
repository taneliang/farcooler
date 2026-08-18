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

echo "macOS camera entitlement: OK"
