#!/bin/bash
# Give the iOS simulator a host to talk to, without touching your Mac's settings.
#
# The phone reaches a host over ssh and nothing else — there is no Far Cooler
# network listener, by design. So to try the app you need sshd running somewhere.
# Turning on Remote Login in System Settings would do it, and it is a system-wide
# change requiring an admin password for something you only want while playing.
#
# So this starts a throwaway sshd instead: its own host key, its own
# authorized_keys holding only this simulator's device key, bound to 127.0.0.1 on
# a high port, owned by you, and gone when you stop it. Nothing outside its own
# directory is modified.
#
# The simulator shares the Mac's network stack, so 127.0.0.1 in the app IS this
# Mac — no address to look up and nothing that works only on one network.
#
#   ./scripts/demo-host.sh          # set it up and start it
#   ./scripts/demo-host.sh stop     # stop it and forget the host
set -euo pipefail

cd "$(dirname "$0")/.."
REPO="$PWD"
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export PATH="$HOME/.cargo/bin:$PATH"

BUNDLE="com.farcooler.ios"
DIR="${TMPDIR:-/tmp}/farcooler-demo-host"
PORT=2222

stop() {
    [ -f "$DIR/sshd.pid" ] && kill "$(cat "$DIR/sshd.pid")" 2>/dev/null || true
    pkill -f "$DIR/sshd_config" 2>/dev/null || true
    pkill -f "$DIR/sync-key.sh" 2>/dev/null || true
    echo "stopped the demo sshd"
    # Nothing to remove from the app: the host lives only in the launch
    # argument, so relaunching without it is what forgets it.
    xcrun simctl terminate booted "$BUNDLE" >/dev/null 2>&1 || true
}

[ "${1:-}" = "stop" ] && { stop; exit 0; }

command -v tmux >/dev/null || { echo "tmux is required"; exit 1; }
xcrun simctl list devices booted | grep -q iPhone || {
    echo "Boot an iPhone simulator first (Xcode > Open Developer Tool > Simulator)."
    exit 1
}

# ---------------------------------------------------------------------------
# The daemon this host will serve. Started if it is not already running.
# ---------------------------------------------------------------------------
cargo build --release --workspace >/dev/null 2>&1
pgrep -x farcoolerd >/dev/null || {
    nohup "$REPO/target/release/farcoolerd" >"$DIR.daemon.log" 2>&1 &
    sleep 2
}
echo "daemon: $(pgrep -x farcoolerd >/dev/null && echo running || echo 'not running')"

# ---------------------------------------------------------------------------
# The app, installed and launched once so it has generated its device key.
# ---------------------------------------------------------------------------
APP=$(find ~/Library/Developer/Xcode/DerivedData/FarCooler-*/Build/Products/Debug-iphonesimulator \
      -maxdepth 1 -name "FarCooler.app" 2>/dev/null | head -1)
[ -n "$APP" ] || {
    echo "Build the iOS app first:"
    echo "  cd apps/ios && xcodebuild -project FarCooler.xcodeproj -scheme FarCooler \\"
    echo "    -sdk iphonesimulator -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build"
    exit 1
}
xcrun simctl install booted "$APP"
xcrun simctl launch booted "$BUNDLE" >/dev/null
sleep 4
# Stopped before reading, because `cfprefsd` writes the preferences file lazily
# and the app is quitting is when it flushes. Reading while it runs returns
# whatever was on disk from a previous launch — which, since the simulator
# re-scopes the keychain on every re-signed build and the app then generates a
# fresh key, is reliably the wrong one.
xcrun simctl terminate booted "$BUNDLE" >/dev/null 2>&1 || true
sleep 2

# Read straight out of the app's own preferences file.
#
# `simctl spawn booted defaults read` reports the domain as missing even when the
# plist plainly exists — `defaults` inside the simulator does not resolve a
# sandboxed app's domain by name. The file is the thing either way.
PREFS="$(xcrun simctl get_app_container booted "$BUNDLE" data)/Library/Preferences/$BUNDLE.plist"
PUBKEY=$(plutil -extract publicKey raw -o - "$PREFS" 2>/dev/null || true)
[ -n "$PUBKEY" ] || {
    echo "The app has not generated a device key yet. Open it once and try again."
    exit 1
}

# ---------------------------------------------------------------------------
# A private sshd. Only this device's key can reach it.
# ---------------------------------------------------------------------------
mkdir -p "$DIR"
[ -f "$DIR/hostkey" ] || ssh-keygen -q -t ed25519 -f "$DIR/hostkey" -N ""
printf '%s\n' "$PUBKEY" > "$DIR/authorized_keys"
chmod 600 "$DIR/authorized_keys"

# Keep it in step with whatever key the app is actually using.
#
# The simulator's keychain does not reliably hand an app back the item it stored
# — across relaunches the device sometimes finds its key and sometimes generates
# a new one. That is a real problem for the app and is tracked separately; here
# it just means a fixed `authorized_keys` goes stale under you mid-demo. So this
# follows the key rather than snapshotting it.
#
# It authorises whatever this simulator currently claims, which is fine for a
# throwaway sshd on the loopback that nothing else can reach, and would be
# indefensible anywhere else.
cat > "$DIR/sync-key.sh" <<SYNC
#!/bin/bash
while true; do
    key=\$(plutil -extract publicKey raw -o - "$PREFS" 2>/dev/null || true)
    if [ -n "\$key" ] && ! grep -qF "\$key" "$DIR/authorized_keys" 2>/dev/null; then
        printf '%s\n' "\$key" > "$DIR/authorized_keys"
        chmod 600 "$DIR/authorized_keys"
    fi
    sleep 2
done
SYNC
chmod +x "$DIR/sync-key.sh"
pkill -f "$DIR/sync-key.sh" 2>/dev/null || true
nohup "$DIR/sync-key.sh" >/dev/null 2>&1 &

cat > "$DIR/sshd_config" <<EOF
Port $PORT
ListenAddress 127.0.0.1
HostKey $DIR/hostkey
AuthorizedKeysFile $DIR/authorized_keys
PidFile $DIR/sshd.pid
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
# The phone runs \`farcoolerd --stdio\` over this connection, and a
# non-interactive ssh session does not read your shell's PATH. Saying where the
# binary is here beats putting a symlink somewhere on the real PATH.
SetEnv PATH=$REPO/target/release:/usr/bin:/bin:/usr/sbin:/sbin
EOF

pkill -f "$DIR/sshd_config" 2>/dev/null || true
/usr/sbin/sshd -f "$DIR/sshd_config" -D -e >"$DIR/sshd.log" 2>&1 &
sleep 2
nc -z 127.0.0.1 $PORT || { echo "sshd did not start:"; tail -5 "$DIR/sshd.log"; exit 1; }
echo "sshd: listening on 127.0.0.1:$PORT"

# ---------------------------------------------------------------------------
# The host, passed at launch.
# ---------------------------------------------------------------------------
#
# A launch argument rather than a write into the app's preferences file. The
# simulator's `cfprefsd` owns that file while the app is running and flushes its
# own cached copy over anything written underneath it — which silently undid the
# host every time. `UserDefaults` reads `-key value` from the command line in the
# argument domain, above everything on disk, which is the supported way to say
# this and cannot be raced.
#
# It also means nothing is persisted: launch without the argument and the host is
# gone, so there is nothing for `stop` to clean up.
xcrun simctl terminate booted "$BUNDLE" >/dev/null 2>&1 || true
xcrun simctl launch booted "$BUNDLE" -farcoolerDemoHost "$USER@127.0.0.1:$PORT" >/dev/null

echo
echo "Ready. The simulator has a host called \"Demo host\"."
echo "Open it to see the fleet; tap a terminal to watch it live."
echo
echo "Relaunching from Xcode or the home screen drops it — run this again to"
echo "put it back. When you are done:  ./scripts/demo-host.sh stop"
