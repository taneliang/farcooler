#!/bin/bash
# Package the Developer ID-signed binaries from the canary runner build.
# Usage: CANARY_SPARKLE_KEY=... scripts/package-mac-runner.sh BIN_DIR OUTPUT_DIR
set -euo pipefail

resources="$1"
out="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"
build="$("$root/scripts/version.sh" build)"
arch="$(/usr/bin/uname -m)"
expected="$(awk '$1 == "canary" { print $2 }' "$root/apps/macos/sparkle-public-keys.txt")"
mkdir -p "$out"
stage="$(/usr/bin/mktemp -d "$out/.package.XXXXXXXX")"
trap 'rm -r "$stage"' EXIT

cp "$resources/farcooler" "$stage/farcooler-canary"
cp "$resources/farcoolerd" "$stage/farcoolerd-canary"
cp "$resources/farcooler-lfs-filter" "$stage/"
cp "$root/scripts/install-mac-runner.sh" "$stage/"
for binary in farcooler-canary farcoolerd-canary farcooler-lfs-filter; do
    # Developer ID Application from Apple's Developer ID hierarchy.
    /usr/bin/codesign --verify --strict --test-requirement \
        'anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists' \
        "$stage/$binary"
done

/usr/bin/tar -czf "$stage/runner.tar.gz" -C "$stage" \
    farcooler-canary farcoolerd-canary farcooler-lfs-filter install-mac-runner.sh
checksum="$(/usr/bin/shasum -a 256 "$stage/runner.tar.gz")"
checksum="${checksum%% *}"
printf 'runner-v1 canary %s %s %s\n' "$arch" "$build" "$checksum" > "$stage/manifest"
printf '%s' "${CANARY_SPARKLE_KEY:?CANARY_SPARKLE_KEY is required}" |
    /usr/bin/swift "$root/scripts/sign-mac-runner-manifest.swift" "$stage/manifest" "$expected" > "$out/latest"
# Exercise the shipped pin and system verifier on the same index clients get.
awk '/^printf.*canary ssh-ed25519/ {split($0, fields, "\047"); print fields[4]}' \
    "$root/scripts/install-mac-runner.sh" > "$stage/allowed"
tail -n +2 "$out/latest" > "$stage/manifest.sig"
/usr/bin/ssh-keygen -Y verify -I canary -n runner@farcooler.com \
    -f "$stage/allowed" -s "$stage/manifest.sig" < "$stage/manifest"
mv "$stage/runner.tar.gz" "$out/$build-$checksum.tar.gz"
