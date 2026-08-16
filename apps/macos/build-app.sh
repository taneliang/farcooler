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

# Every per-channel value this script needs, resolved ONCE, here, before
# anything below can change what `version.sh` would answer.
#
# `cargo build --release`, run further down to bundle the CLI and daemon,
# rewrites the tracked `Cargo.lock` whenever it and `Cargo.toml` have drifted —
# a workspace version bump does exactly that — and `version.sh channel` answers
# `local` for a dirty tree. A script that re-asked version.sh after that build
# would name the bundle for one channel while its login agent and icon named it
# for another: a canary run whose first lines saw a clean tree and whose last
# lines did not would ship `Far Cooler Canary.app` carrying the LOCAL launchd
# label and a grey LOCAL banner, ad-hoc signed and handed to someone as a
# canary — the exact failure this whole channel-identity mechanism exists to
# prevent, reintroduced by asking twice inside the script that implements it.
# Resolving everything up front, before the tree can move, means there is only
# ever one answer for a single run of this script to disagree with itself
# about. Do not "simplify" this back to inline `$(../../scripts/version.sh …)`
# calls scattered below — that is what put the bug here the first time.
CHANNEL="$(../../scripts/version.sh channel)"
[ -n "$CHANNEL" ] || { echo "version.sh channel returned nothing; refusing to build"; exit 1; }
# Named for the channel, so a canary installs BESIDE the build someone depends
# on rather than over it. Stable is still `Far Cooler.app`, which is what every
# existing install is called.
#
# Checked before it becomes $APP: everything below — `rm -rf`, `mkdir -p`, the
# LaunchAgent copy, the icon render, the Rust bundling — trusts this path, and
# an empty name would silently turn it into `build/.app`.
APP_NAME="$(../../scripts/version.sh app-name)"
[ -n "$APP_NAME" ] || { echo "version.sh app-name returned nothing; refusing to build build/.app"; exit 1; }
APP_SUFFIX="$(../../scripts/version.sh app-suffix)"
VERSION="$(../../scripts/version.sh)"
BUILD_NUMBER="$(../../scripts/version.sh build)"
DISPLAY_VERSION="$(../../scripts/version.sh display)"
SCHEME="$(../../scripts/version.sh scheme)"
FEED_URL="$(../../scripts/version.sh feed-url)"
APP="build/$APP_NAME.app"

# SwiftPM cannot build a Rust crate, and the app will not link without it.
./build-vt.sh >/dev/null

echo "==> Building ($CONFIG)"
# Held rather than discarded, and printed when the build fails.
#
# `swift build` writes compiler diagnostics to STDOUT, so sending stdout to
# /dev/null to keep the log quiet throws the errors away with it. The script
# then exited on the missing binary saying nothing useful, while the last
# bundle that DID build sat in place looking current — a long way to go to
# find a one-line compile error.
BUILD_LOG="$(mktemp -t farcooler-build)"
trap 'rm -f "$BUILD_LOG"' EXIT
if ! swift build -c "$CONFIG" >"$BUILD_LOG" 2>&1; then
  cat "$BUILD_LOG"
  exit 1
fi

BIN=".build/$CONFIG/Far Cooler"
[ -x "$BIN" ] || { echo "build produced no binary at $BIN"; exit 1; }

echo "==> Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Library/LaunchAgents"

cp "$BIN" "$APP/Contents/MacOS/FarCooler"

# Sparkle, copied in by hand because this script assembles the bundle by hand.
#
# Xcode would embed a framework for you; `swift build` produces a bare
# executable and an artifact directory, so the framework has to be found, copied
# and signed here. The macOS slice is selected explicitly: an XCFramework holds
# several, and copying the wrong one produces a bundle that launches on nothing.
#
# `ditto` rather than `cp -R`: a framework is a bundle of symlinks and extended
# attributes, and `cp` flattens both, which breaks the signature.
echo "==> Embedding Sparkle"
SPARKLE_SRC="$(find .build -path '*Sparkle.xcframework/macos-*/Sparkle.framework' -maxdepth 7 | head -1)"
[ -n "$SPARKLE_SRC" ] || { echo "no Sparkle.framework in .build — did swift build run?"; exit 1; }
mkdir -p "$APP/Contents/Frameworks"
ditto "$SPARKLE_SRC" "$APP/Contents/Frameworks/Sparkle.framework"
echo "    embedded from $SPARKLE_SRC"

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

stamp CFBundleShortVersionString "$VERSION"
stamp CFBundleVersion "$BUILD_NUMBER"
stamp FarCoolerChannel "$CHANNEL"
stamp FarCoolerDisplayVersion "$DISPLAY_VERSION"

# The identity that decides whether two channels are two apps or one.
#
# `UserDefaults` keys off the bundle identifier, so this partitions preferences
# with no code — a canary starts from defaults rather than rewriting the
# settings of the app someone works in.
stamp CFBundleIdentifier "com.farcooler.FarCooler$APP_SUFFIX"
stamp CFBundleName "$APP_NAME"
stamp CFBundleDisplayName "$APP_NAME"

# The URL scheme AuthKit comes back to, per channel.
#
# A scheme that two installed apps both claim is resolved by the OS rather than
# by either of them, so a canary and a stable sharing `farcooler://` means a
# sign-in can be delivered to the wrong app — and the code fails there, because
# it was issued against a different WorkOS environment. Stable still answers
# `farcooler`, so nothing already registered has to change.
#
# Nested, hence the index path: the plist holds one URL type with one scheme in
# it. PlistBuddy's `Set` fails loudly if that shape ever changes, which is the
# behavior wanted — a silently unstamped scheme is an app that cannot receive
# its own callback.
stamp CFBundleURLTypes:0:CFBundleURLSchemes:0 "$SCHEME"

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

# Which feed this build watches, and whose signature it will accept.
#
# The key is read from the committed table rather than written here: it is the
# trust anchor for the whole update path, and a second copy is a second thing to
# disagree. Local gets neither — an empty feed is how Updates.swift knows not to
# start.
stamp SUFeedURL "$FEED_URL"
if [ -n "$FEED_URL" ]; then
  SPARKLE_KEY="$(awk -v c="$CHANNEL" '$1 == c { print $2 }' sparkle-public-keys.txt)"
  [ -n "$SPARKLE_KEY" ] || {
    echo "no public key for $CHANNEL in sparkle-public-keys.txt"; exit 1; }
  stamp SUPublicEDKey "$SPARKLE_KEY"
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
#
# One launchd label per channel. Two apps registering `com.farcooler.daemon`
# means the second registration replaces the first and which daemon starts at
# login becomes a question of install order — silently, since SMAppService
# reports success either way.
AGENT_LABEL="com.farcooler.daemon$APP_SUFFIX"
AGENT_PLIST="$APP/Contents/Library/LaunchAgents/$AGENT_LABEL.plist"
cp Resources/com.farcooler.daemon.plist "$AGENT_PLIST"
/usr/libexec/PlistBuddy -c "Set :Label $AGENT_LABEL" "$AGENT_PLIST" >/dev/null
echo "    login agent $AGENT_LABEL"

# Stamped into the bundle so ServiceRegistration.swift can read the same value
# back rather than recomputing the suffix rule itself — see the plist's own
# comment and ServiceRegistration.swift for why a second copy of that rule is
# the bug this avoids.
stamp FarCoolerAgentLabel "$AGENT_LABEL"

echo "==> Rendering icon"
# The channel's banner, drawn onto a copy in build/ — never onto the source.
#
# Writing a generated icon into the tracked asset catalog would dirty the tree,
# and `version.sh channel` answers `local` for a dirty tree: every step after
# that point would believe it was building local, and a canary would install at
# local's identifier with no error anywhere.
LABELED="build/AppIcon-$CHANNEL.png"
swift ../../scripts/icon-label.swift \
  "$CHANNEL" \
  ../shared/Assets.xcassets/AppIcon.appiconset/AppIcon.png \
  "$LABELED"
ICONSET="build/AppIcon.iconset"
ICON="$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"
if swift Tools/make-icon.swift "$ICONSET" "$ICON" "$LABELED" >/dev/null 2>&1; then
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
