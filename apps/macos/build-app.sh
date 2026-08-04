#!/bin/bash
# Assemble a real Far Cooler.app bundle.
#
# `swift build` alone produces a bare Unix executable. macOS gives such a process
# no activation policy, so its window cannot become key, it never receives
# keystrokes, and it gets no Dock tile or menu bar. A bundle with an Info.plist
# declaring CFBundlePackageType APPL and NSPrincipalClass NSApplication is what
# makes it an actual application.
set -euo pipefail

cd "$(dirname "$0")"

CONFIG="${1:-release}"
APP="build/Far Cooler.app"

# SwiftPM cannot build a Rust crate, and the app will not link without it.
./build-vt.sh >/dev/null

echo "==> Building ($CONFIG)"
swift build -c "$CONFIG" >/dev/null

BIN=".build/$CONFIG/Far Cooler"
[ -x "$BIN" ] || { echo "build produced no binary at $BIN"; exit 1; }

echo "==> Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Library/LaunchAgents"

cp "$BIN" "$APP/Contents/MacOS/FarCooler"
cp Resources/Info.plist "$APP/Contents/Info.plist"

# The version is stamped into the BUNDLED plist, not the source one.
#
# Stamping the source would make every build dirty the working tree, which is
# how a version ends up committed by accident and then hand-edited later. There
# is one number — see scripts/version.sh — and this is where it lands.
# `Set` on a key the plist does not have prints an error and exits 0, which is
# how FarCoolerChannel silently shipped empty. Every stamp goes through here so
# a missing key is a failed build rather than a bundle that lies about itself.
stamp() {
  /usr/libexec/PlistBuddy -c "Set :$1 $2" "$APP/Contents/Info.plist" 2>&1 | grep -q . && {
    echo "    Info.plist has no :$1 to stamp"; exit 1; }
  return 0
}

VERSION="$(../../scripts/version.sh)"
BUILD_NUMBER="$(../../scripts/version.sh build)"
stamp CFBundleShortVersionString "$VERSION"
stamp CFBundleVersion "$BUILD_NUMBER"
stamp FarCoolerChannel "$(../../scripts/version.sh channel)"
stamp FarCoolerDisplayVersion "$(../../scripts/version.sh display)"

# The WorkOS client id, if this build has one.
#
# Public — it names the app to WorkOS rather than authenticating as anything —
# but still not committed, so that a fork points at its own WorkOS project by
# setting one environment variable instead of editing source. A build without it
# works completely; the only thing missing is the sign-in button, and sign-in
# buys notifications and nothing else.
if [ -n "${FARCOOLER_WORKOS_CLIENT_ID:-}" ]; then
  stamp FarCoolerWorkOSClientID "$FARCOOLER_WORKOS_CLIENT_ID"
fi
printf 'APPL????' > "$APP/Contents/PkgInfo"

# The app is the distribution unit: it carries the CLI it drives.
#
# This is also what makes launching from the Dock work at all. A double-clicked
# app inherits no shell environment, so it cannot find a binary that only exists
# on your PATH.
# Both binaries, side by side.
#
# The CLI starts the daemon by looking for `farcoolerd` next to itself, so a
# bundle carrying only the CLI would auto-start nothing and every command would
# time out waiting. Shipping them together also guarantees the pair match, which
# is the mismatch the version handshake exists to catch.
#
# BUILT here, never merely copied. This script used to bundle whatever happened
# to be sitting in `target/release` and print a warning only if it was missing
# entirely — so a Rust fix, compiled and unit-tested minutes earlier, shipped as
# the binary from whenever someone last ran `cargo build --release`. The app
# launched, the tests passed, and the bug under investigation was still there,
# because the code that fixed it was never in the bundle. A stale binary must
# not be a thing this script is capable of producing.
echo "==> Building the CLI and daemon (release)"
(cd ../.. && cargo build --release --bin farcooler --bin farcoolerd) || {
  echo "    the Rust build failed; refusing to bundle a stale binary"
  exit 1
}

CLI="../../target/release/farcooler"
DAEMON="../../target/release/farcoolerd"
[ -x "$CLI" ] && [ -x "$DAEMON" ] || { echo "missing $CLI or $DAEMON after a successful build"; exit 1; }
cp "$CLI" "$APP/Contents/Resources/farcooler"
cp "$DAEMON" "$APP/Contents/Resources/farcoolerd"
echo "    bundled $("$CLI" --version 2>/dev/null || echo cli) + farcoolerd"

# The Linux binaries `farcooler host install` uploads to a remote host, if this
# machine has built any.
#
# An end user only ever has the app — no checkout, no `scripts/build-linux.sh`,
# no Rust toolchain — so "Install" on a Linux or WSL machine can only work if
# its own copy of `farcooler`/`farcoolerd` already travelled inside this
# bundle. `host_install.rs` looks for them at `Resources/dist/<arch>-linux`
# next to itself before it looks anywhere else.
#
# Building a musl target needs `cross` (Docker) or a musl cross-compiler this
# script does not assume every machine has, so this bundles whatever already
# exists in `dist/` and says so rather than failing the whole app build over
# it — the WorkOS client id a few lines up makes the same trade for the same
# reason. Run `./scripts/build-linux.sh <arch>` first to have something here.
echo "==> Bundling Linux binaries, if built"
BUNDLED_LINUX=0
for SLUG in x86_64-linux aarch64-linux; do
  SRC="../../dist/$SLUG"
  if [ -f "$SRC/farcoolerd" ] && [ -f "$SRC/farcooler" ]; then
    mkdir -p "$APP/Contents/Resources/dist/$SLUG"
    cp "$SRC/farcoolerd" "$SRC/farcooler" "$APP/Contents/Resources/dist/$SLUG/"
    echo "    bundled $SLUG"
    BUNDLED_LINUX=$((BUNDLED_LINUX + 1))
  fi
done
[ "$BUNDLED_LINUX" -eq 0 ] && echo "    (none found — run ./scripts/build-linux.sh to bundle remote-host installs)"

# SMAppService looks for the agent plist at exactly this path inside the bundle.
# Anywhere else and registration reports notFound.
cp Resources/com.farcooler.daemon.plist "$APP/Contents/Library/LaunchAgents/"

echo "==> Rendering icon"
ICONSET="build/AppIcon.iconset"
ICON="$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"
if swift Tools/make-icon.swift "$ICONSET" "$ICON" >/dev/null 2>&1; then
  rm -rf "$ICONSET"
else
  echo "    (icon render failed, continuing without one)"
fi

# Ad-hoc sign so macOS treats it as a stable identity across launches. This is
# not notarization; public distribution needs a Developer ID.
echo "==> Signing (ad-hoc)"
codesign --force --deep --sign - "$APP" 2>/dev/null || echo "    (ad-hoc signing skipped)"

# Refresh Launch Services so the Dock picks up the icon and identifier.
LSREG="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
[ -x "$LSREG" ] && "$LSREG" -f "$APP" 2>/dev/null || true

echo
echo "Built $APP"
echo
echo "Run it with the CLI it drives:"
echo "  FARCOOLER_BIN=\$PWD/../../target/release/farcooler open $APP"
