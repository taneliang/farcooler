#!/bin/bash
# Assemble a real Overnight.app bundle.
#
# `swift build` alone produces a bare Unix executable. macOS gives such a process
# no activation policy, so its window cannot become key, it never receives
# keystrokes, and it gets no Dock tile or menu bar. A bundle with an Info.plist
# declaring CFBundlePackageType APPL and NSPrincipalClass NSApplication is what
# makes it an actual application.
set -euo pipefail

cd "$(dirname "$0")"

CONFIG="${1:-release}"
APP="build/Overnight.app"

# SwiftPM cannot build a Rust crate, and the app will not link without it.
./build-vt.sh >/dev/null

echo "==> Building ($CONFIG)"
swift build -c "$CONFIG" >/dev/null

BIN=".build/$CONFIG/Overnight"
[ -x "$BIN" ] || { echo "build produced no binary at $BIN"; exit 1; }

echo "==> Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/Overnight"
cp Resources/Info.plist "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# The app is the distribution unit: it carries the CLI it drives.
#
# This is also what makes launching from the Dock work at all. A double-clicked
# app inherits no shell environment, so it cannot find a binary that only exists
# on your PATH.
# Both binaries, side by side.
#
# The CLI starts the daemon by looking for `overnightd` next to itself, so a
# bundle carrying only the CLI would auto-start nothing and every command would
# time out waiting. Shipping them together also guarantees the pair match, which
# is the mismatch the version handshake exists to catch.
CLI="../../target/release/overnight"
DAEMON="../../target/release/overnightd"
if [ -x "$CLI" ] && [ -x "$DAEMON" ]; then
  cp "$CLI" "$APP/Contents/Resources/overnight"
  cp "$DAEMON" "$APP/Contents/Resources/overnightd"
  echo "    bundled $(cd ../.. && ./target/release/overnight --version 2>/dev/null || echo 'cli') + overnightd"
else
  echo "    WARNING: missing $CLI or $DAEMON"
  echo "    build them first:  (cd ../.. && cargo build --release)"
fi

echo "==> Rendering icon"
ICONSET="build/AppIcon.iconset"
rm -rf "$ICONSET"
if swift Tools/make-icon.swift "$ICONSET" >/dev/null 2>&1; then
  iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
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
echo "  OVERNIGHT_BIN=\$PWD/../../target/release/overnight open $APP"
