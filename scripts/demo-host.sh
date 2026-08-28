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
# a high port, owned by you, and gone when you stop it. Behind it sits a daemon
# of this demo's OWN, with its own FARCOOLER_HOME and its own scratch repository,
# so the phone never opens onto the fleet you actually work in. Nothing outside
# this script's own directory is modified.
#
# The simulator shares the Mac's network stack, so 127.0.0.1 in the app IS this
# Mac — no address to look up and nothing that works only on one network.
#
#   ./scripts/demo-host.sh          # set it up and start it
#   ./scripts/demo-host.sh --plain  # the same, with an UNRESTRICTED key line
#   ./scripts/demo-host.sh stop     # stop it and forget the host
#
# The two key lines are the reason this script is worth having, and they are not
# a detail:
#
#   default   the line `crates/fence` writes for every enrolled device —
#             `restrict,command="~/.local/bin/farcoolerd-<channel> --stdio …"`.
#             sshd DISCARDS whatever command the client asked for and runs that
#             one, so the second channel the app opens for a terminal's bytes
#             (`--stream <uuid>`) never runs: the stream stays silent and the
#             pane falls back to polling for captures. This is what a real
#             enrolled device does, and the configuration a scrolling bug has to
#             be reproduced against.
#   --plain   the key with no options in front of it. sshd runs what it is asked,
#             so `--stream` streams. The same pane, scrolling, for comparison.
#
# Both lines are rendered by the shipped `fence::render` rather than assembled
# here — see the helper this script builds below for why, and for the one thing
# it adds in front of them.
set -euo pipefail

cd "$(dirname "$0")/.."
REPO="$PWD"
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export PATH="$HOME/.cargo/bin:$PATH"

# One bundle identifier per channel, and this checkout does not necessarily build
# the one the App Store ships. `apps/ios/generate-project.py` composes it as
# `com.farcooler.ios` for stable and `com.farcooler.ios.<channel>` for the other
# three; a dirty tree answers `local`, so a hardcoded `com.farcooler.ios` here
# installs the app and then talks to a bundle id that is not on the simulator.
# Derived rather than copied, so the two cannot drift apart.
CHANNEL=$(./scripts/version.sh channel)
if [ "$CHANNEL" = "stable" ]; then
    BUNDLE="com.farcooler.ios"
else
    BUNDLE="com.farcooler.ios.$CHANNEL"
fi

# The trailing slash macOS puts on TMPDIR is trimmed, so every path printed and
# every path compared against `ps` output has exactly one separator in it.
DIR="${TMPDIR:-/tmp}"; DIR="${DIR%/}/farcooler-demo-host"
# The account home the ssh session runs with, and the runtime directory the demo
# daemon owns. Two separate things:
#
#   SESSION_HOME  what `~` means to the forced command, and where the daemon
#                 looks for `.ssh/authorized_keys`. Pointed away from your real
#                 home on purpose: a phone holding `control` may enroll and
#                 revoke device keys, and it must never be able to rewrite the
#                 file your own ssh access depends on.
#   FC_HOME       FARCOOLER_HOME: the database, the install id, the tmux server
#                 name and the worktrees. A demo fleet, separate from yours.
#
# FC_HOME is kept deliberately short. It holds a unix socket, and a macOS socket
# path over ~104 bytes fails to bind — which reads as "the daemon is not running"
# rather than as a path length problem.
SESSION_HOME="$DIR/home"
FC_HOME="$DIR/fc"
DERIVED="$DIR/DerivedData"
# Honoured rather than assumed: someone building with a shared CARGO_TARGET_DIR
# would otherwise be handed binaries from a directory cargo never wrote to.
TARGET="${CARGO_TARGET_DIR:-$REPO/target}/release"
PORT=2222

# Stop something this script started, and nothing else.
#
# By pid, from a file this script wrote, never by pattern. `pkill -f farcoolerd`
# on a developer's Mac matches the daemon serving their real repositories and
# terminals, and `pkill -f "$DIR/sshd_config"` matches any process that merely
# has that string somewhere in its argv — including a grep, an editor, or this
# script's own shell. Three such calls used to live here.
#
# The pid is checked against the command it is supposed to be before the signal,
# because pids are recycled: a stale pid file left by a crashed run is otherwise
# a signal sent to whatever inherited that number since.
#
# `<pidfile>.cmd`, when it exists, holds the command line written at spawn time
# and wins over the caller's guess. The daemon's path depends on CARGO_TARGET_DIR,
# so a `stop` run from a shell that has it set differently would otherwise decline
# to stop the daemon it started — and leave it running while saying it had not.
kill_pidfile() {
    local file="$1" expect="$2" pid
    [ -f "$file" ] || return 0
    [ -f "$file.cmd" ] && expect=$(cat "$file.cmd")
    pid=$(cat "$file" 2>/dev/null || true)
    case "$pid" in
        ''|*[!0-9]*) rm -f "$file" "$file.cmd"; return 0 ;;
    esac
    if ps -o command= -p "$pid" 2>/dev/null | grep -qF -- "$expect"; then
        kill "$pid" 2>/dev/null || true
    fi
    rm -f "$file" "$file.cmd"
}

stop() {
    # sshd writes its own pid file (`PidFile` below). Only the listener is
    # signalled: each connection is served by a separate `sshd-session` child
    # that ends when its client does.
    kill_pidfile "$DIR/sshd.pid" "$DIR/sshd_config"
    kill_pidfile "$DIR/sync-key.pid" "$DIR/sync-key.sh"
    # The demo's own daemon, matched on the binary path so that a recycled pid
    # cannot make this a signal to somebody else's process.
    kill_pidfile "$DIR/daemon.pid" "$TARGET/farcoolerd"
    # Its tmux server is left alone, exactly as `farcooler daemon stop` leaves
    # yours: terminals belong to tmux, so the demo's panes and their scrollback
    # survive a stop and are still there when this script starts the daemon
    # again. `tmux -L farcooler-$(cat "$FC_HOME/install-id")` reaches it.
    echo "stopped the demo sshd, its sync loop and its daemon"
    # Nothing to remove from the app: the host lives only in the launch
    # argument, so relaunching without it is what forgets it.
    xcrun simctl terminate booted "$BUNDLE" >/dev/null 2>&1 || true
}

GRANT="fenced"
for arg in "$@"; do
    case "$arg" in
        stop) stop; exit 0 ;;
        --plain) GRANT="plain" ;;
        -h|--help) sed -n '2,39p' "$0"; exit 0 ;;
        *) echo "unknown argument: $arg (want --plain, stop, or nothing)"; exit 1 ;;
    esac
done

command -v tmux >/dev/null || { echo "tmux is required"; exit 1; }
command -v jq >/dev/null || { echo "jq is required (it reads the CLI's --json)"; exit 1; }
[ -x /usr/sbin/sshd ] || { echo "no /usr/sbin/sshd on this Mac"; exit 1; }
xcrun simctl list devices booted | grep -q iPhone || {
    echo "Boot an iPhone simulator first (Xcode > Open Developer Tool > Simulator)."
    exit 1
}
# The booted one, by udid, so the build targets the device the app is installed
# on rather than whichever simulator Xcode last had selected.
UDID=$(xcrun simctl list devices booted | sed -n 's/.*(\([0-9A-F-]\{36\}\)) (Booted).*/\1/p' | head -1)

mkdir -p "$DIR" "$SESSION_HOME" "$FC_HOME"
# A second run replaces the first rather than racing it: two daemons on one
# FARCOOLER_HOME, or two sshds on one port, is a demo that half works.
kill_pidfile "$DIR/sshd.pid" "$DIR/sshd_config"
kill_pidfile "$DIR/sync-key.pid" "$DIR/sync-key.sh"
kill_pidfile "$DIR/daemon.pid" "$TARGET/farcoolerd"

# ---------------------------------------------------------------------------
# The binaries, from this checkout.
# ---------------------------------------------------------------------------
echo "building the workspace…"
cargo build --release --workspace >"$DIR/cargo.log" 2>&1 || {
    echo "cargo build failed:"; tail -20 "$DIR/cargo.log"; exit 1
}

# The renderer for the authorized_keys line, borrowed rather than reimplemented.
#
# The format is `crates/fence`'s and nothing here is entitled to a second opinion
# about it: `render` re-encodes the key material, picks the comment, and writes
# the forced command as `~/.local/bin/<this channel's daemon> --stdio --client …
# --scope …`. A bash `printf` of the same shape is a copy that goes stale the
# next time that file changes, and the failure it produces is a device that
# cannot log in.
#
# The shipped ways to reach it both come with something this script must not do:
# `farcooler client enroll` makes the DAEMON write, and the daemon writes to
# `$HOME/.ssh/authorized_keys` — one wrong environment variable away from
# editing your real one. So the small program below calls the crate directly and
# is handed the path it must write, which is a fact about this script rather than
# about an environment. It builds in about fifteen seconds and caches after that.
mkdir -p "$DIR/fence-line"
cat > "$DIR/fence-line/Cargo.toml" <<EOF
[package]
name = "demo-fence-line"
version = "0.0.0"
edition = "2021"
publish = false

[[bin]]
name = "demo-fence-line"
path = "main.rs"

[dependencies]
farcooler-fence = { path = "$REPO/crates/fence" }
farcooler-protocol = { path = "$REPO/crates/protocol" }

# Its own workspace, so it is never pulled into the repository's.
[workspace]
EOF
cat > "$DIR/fence-line/main.rs" <<'EOF'
//! Write the demo host's `authorized_keys`, through the shipped fence.
//!
//! argv: <path> <fenced|plain> <extra options>, key on stdin. Prints the program
//! the forced command names, then the line that was written.
use std::io::Read;
use std::path::PathBuf;

use farcooler_fence::{self as fence, Grant, Placement};
use farcooler_protocol::v1::Scope;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().expect("a path to write"));
    let kind = args.next().expect("fenced or plain");
    let prefix = args.next().unwrap_or_default();

    let mut key = String::new();
    std::io::stdin().read_to_string(&mut key).expect("a public key on stdin");
    let key = key.trim();

    // Rendered even when a plain line is what gets written: it is what NAMES the
    // daemon binary, and the caller needs that name in both modes. Read out of
    // the line rather than written down here, the way
    // `crates/daemon/tests/a_real_sshd_forces_the_scope.rs` reads it, so this
    // follows the spelling wherever `fence` takes it next.
    let fenced = fence::render(key, "Simulator", "demo-simulator", Scope::Control, Grant::FarCooler)
        .expect("render the restricted line");
    let program = fenced
        .split_once("command=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .and_then(|(command, _)| command.split_whitespace().next())
        .expect("a forced command that names a program");

    let line = match kind.as_str() {
        // The product's own plain line: no options field, a `farcooler-shell-`
        // comment, and a shell behind it. `render` refuses this shape for any
        // scope but host_admin, because an unrestricted line IS every power the
        // account has.
        "plain" => fence::render(key, "Simulator", "demo-simulator", Scope::HostAdmin, Grant::Shell)
            .expect("render the plain line"),
        _ => fenced.clone(),
    };
    // How the extra options join the rendered line is not cosmetic, and getting
    // it wrong costs a key sshd silently ignores rather than an error anyone
    // sees. `authorized_keys` separates an OPTIONS field from the key by
    // WHITESPACE and the options within it by commas — so a restricted line,
    // which already begins with options, takes a comma, and a plain line, which
    // has none, takes a space. A comma there makes sshd read `ssh-ed25519` as the
    // name of another option, refuse the whole line, and answer every connection
    // with "Permission denied (publickey)".
    let line = if prefix.is_empty() {
        line
    } else if kind == "plain" {
        format!("{prefix} {line}")
    } else {
        format!("{prefix},{line}")
    };

    // The shipped writer too: it fences the block, holds the lock, fsyncs, and
    // leaves the file at 0600 — which is what sshd will demand of it.
    fence::write(&path, fence::AUTHORIZED_KEYS, &[line.clone()], &[], Placement::Last)
        .expect("write the fence");

    println!("{program}");
    println!("{line}");
}
EOF
(cd "$DIR/fence-line" && CARGO_TARGET_DIR="$DIR/fence-line/target" cargo build --release) \
    >"$DIR/fence-line/build.log" 2>&1 || {
    echo "could not build the key-line helper:"; tail -20 "$DIR/fence-line/build.log"; exit 1
}
RENDER="$DIR/fence-line/target/release/demo-fence-line"

# ---------------------------------------------------------------------------
# The daemon this host will serve. This demo's own, always.
# ---------------------------------------------------------------------------
#
# Started unconditionally under this script's own FARCOOLER_HOME, and found again
# by the pid file it writes. What used to be here was `pgrep -x farcoolerd`,
# which on any machine running Far Cooler matches the daemon serving the user's
# real repositories: the demo then started nothing, and the simulator was pointed
# at a live fleet by a script whose whole promise is that it touches nothing.
env HOME="$SESSION_HOME" FARCOOLER_HOME="$FC_HOME" \
    nohup "$TARGET/farcoolerd" >"$DIR/daemon.log" 2>&1 &
echo $! > "$DIR/daemon.pid"
echo "$TARGET/farcoolerd" > "$DIR/daemon.pid.cmd"
DAEMON_PID=$(cat "$DIR/daemon.pid")

# Polled, not slept on: the socket is bound after the database opens and the tmux
# inventory is taken, which is fast on an idle Mac and not on a busy one.
for _ in $(seq 1 60); do
    [ -S "$FC_HOME/farcoolerd.sock" ] && break
    sleep 0.5
done
[ -S "$FC_HOME/farcoolerd.sock" ] || {
    echo "the demo daemon never opened $FC_HOME/farcoolerd.sock:"
    tail -10 "$DIR/daemon.log"; exit 1
}
echo "daemon: pid $DAEMON_PID, FARCOOLER_HOME=$FC_HOME"

# Every CLI call below reaches THAT daemon, and no other. Both variables are set
# inside the function rather than in front of it, because assignments in front of
# a shell function can outlive the call.
fc() { env HOME="$SESSION_HOME" FARCOOLER_HOME="$FC_HOME" "$TARGET/farcooler" "$@"; }

# ---------------------------------------------------------------------------
# Something to look at. A daemon with no repositories shows an empty fleet.
# ---------------------------------------------------------------------------
#
# A scratch git repository, a workspace, and a few hundred lines of output in its
# pane — because a pane holding twelve lines cannot demonstrate a scrolling fix
# in either direction. All of it through the shipped CLI, so this is the same
# path the Mac app takes rather than a second way to make a workspace.
mkdir -p "$DIR/repos/scrollback"
if [ ! -d "$DIR/repos/scrollback/.git" ]; then
    git init -q -b main "$DIR/repos/scrollback"
    printf '# Far Cooler demo\n\nA scratch repository, made by scripts/demo-host.sh.\n' \
        > "$DIR/repos/scrollback/README.md"
    # An explicit identity: the daemon runs with SESSION_HOME, which has no
    # .gitconfig, and a commit with no author is a commit that does not happen.
    git -C "$DIR/repos/scrollback" add README.md
    git -C "$DIR/repos/scrollback" \
        -c user.name="Far Cooler demo" -c user.email="demo@farcooler.invalid" \
        commit -q -m "the first commit"
fi
# Everything below is asked for only if it is not already there, because none of
# these calls is idempotent and the second run is the common one.
#
# `repo register` in particular will happily register the same directory twice:
# you get a second repository record, and — because the daemon enumerates a
# repository's existing git worktrees when it registers one — a second workspace
# for every worktree the first registration already made. Four runs of the
# version that just called it produced four repositories and eight workspaces,
# which is a fleet nobody can find the demo in.
#
# `first(…)` throughout, so that a home someone has already grown a second copy
# in still yields ONE id here rather than a two-line string that no `--json`
# consumer and no argument can be.
fc root add "$DIR/repos" >/dev/null 2>&1 || true
repo_id() { fc --json repo list | jq -r 'first(.repositories[] | select(.displayName=="scrollback") | .id) // empty'; }
if [ -z "$(repo_id)" ]; then
    fc repo register "$DIR/repos/scrollback" >/dev/null
fi
REPO_ID=$(repo_id)
[ -n "$REPO_ID" ] || { echo "the demo repository did not register; see $DIR/daemon.log"; exit 1; }
if [ -z "$(fc --json workspace list | jq -r 'first(.workspaces[] | select(.task=="scrolling") | .id) // empty')" ]; then
    fc workspace create "$REPO_ID" scrolling --branch demo/scrolling
fi
# Polled: the workspace's first terminal is launched as the worktree finishes
# being made, so it is not always in the very next listing.
for _ in $(seq 1 20); do
    TERMINAL=$(fc --json workspace list \
        | jq -r 'first(.workspaces[] | select(.task=="scrolling") | .terminals[0].id // empty) // empty')
    [ -n "$TERMINAL" ] && break
    sleep 0.5
done
if [ -z "${TERMINAL:-}" ]; then
    # A workspace outliving its pane is the ordinary case, not a fault: terminals
    # belong to tmux, so a Mac that has rebooted since the last demo has the
    # workspace and none of its panes. One is made rather than reported.
    WORKSPACE=$(fc --json workspace list \
        | jq -r 'first(.workspaces[] | select(.task=="scrolling") | .id) // empty')
    fc terminal create "$WORKSPACE" --preset shell >/dev/null
    for _ in $(seq 1 20); do
        TERMINAL=$(fc --json workspace list \
            | jq -r 'first(.workspaces[] | select(.task=="scrolling") | .terminals[0].id // empty) // empty')
        [ -n "$TERMINAL" ] && break
        sleep 0.5
    done
fi
[ -n "${TERMINAL:-}" ] || {
    echo "the demo workspace has no terminal; see $DIR/daemon.log"; exit 1
}

# Sent as a file to run rather than as a one-liner, because the pane holds your
# login shell and the quoting that survives bash, `terminal send` and fish is not
# the same quoting.
cat > "$DIR/scrollback.sh" <<'EOF'
#!/bin/bash
for i in $(seq 1 400); do
    printf '%4d  Far Cooler demo scrollback — scroll up from here to reach line 1\n' "$i"
done
EOF
chmod +x "$DIR/scrollback.sh"
fc terminal send "$TERMINAL" "bash '$DIR/scrollback.sh'
" >/dev/null
echo "fleet:  repository 'scrollback', workspace 'scrolling', 400 lines in its pane"

# ---------------------------------------------------------------------------
# The app, built here and installed, so it is what the checkout says.
# ---------------------------------------------------------------------------
#
# Built into a derived-data path this script owns. The glob that used to be here
# took the first `FarCooler-*` directory Xcode had ever made and installed
# whatever was in it — on this Mac a stable-channel build from a fortnight
# earlier, which is a bug hunt spent on code that is not in front of you.
#
# The Rust xcframeworks in apps/ios/Frameworks are NOT rebuilt by this: they are
# whatever `scripts/build-ios-frameworks.sh` last produced. Run that first if the
# thing you changed is in `crates/`.
echo "building the iOS app…"
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
    -configuration Debug \
    -destination "platform=iOS Simulator,id=$UDID" \
    -derivedDataPath "$DERIVED" build >"$DIR/xcodebuild.log" 2>&1 || {
    echo "the iOS app did not build:"; tail -30 "$DIR/xcodebuild.log"; exit 1
}
APP="$DERIVED/Build/Products/Debug-iphonesimulator/FarCooler.app"
[ -d "$APP" ] || { echo "no app at $APP; see $DIR/xcodebuild.log"; exit 1; }

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

# What every line gets in front of it, and the only part of it this script wrote.
#
# sshd's `environment=` reaches the environment of whatever it runs, coexists
# with `restrict`, and needs `PermitUserEnvironment yes` (off by default
# everywhere, and no runner turns it on — this is scaffolding for a demo, not a
# shape any enrolled device has). It is how the connection lands in this demo's
# world rather than yours, and it is needed on the plain line too:
#
#   HOME            what `~/.local/bin/…` expands to, and where the daemon reads
#                   and rewrites `.ssh/authorized_keys`. Left at your real home,
#                   a phone with `control` would be editing the file your own ssh
#                   access depends on.
#   FARCOOLER_HOME  which fleet answers. Left unset, the session opens the real
#                   one for this channel.
#   PATH            an ssh command runs no login shell, so its PATH is the
#                   system default and Homebrew is not on it. The relay to the
#                   already-running daemon needs nothing from PATH — but if that
#                   daemon has stopped, this session's own daemon takes over and
#                   needs tmux and git.
#
# `SetEnv PATH=…` in sshd_config used to stand in for this, and had not been
# doing anything for a long while: the client execs the literal path
# `~/.local/bin/farcoolerd-<channel>` (crates/client/src/session.rs) and the
# forced command names that same path (crates/fence/src/lib.rs), so a bare name
# on PATH is not what either of them resolves.
SESSION_PATH=$(printf '%s\n' "$(dirname "$(command -v tmux)")" "$(dirname "$(command -v git)")" \
    /usr/bin /bin /usr/sbin /sbin | awk '!seen[$0]++' | paste -sd: -)
PREFIX="environment=\"HOME=$SESSION_HOME\",environment=\"FARCOOLER_HOME=$FC_HOME\",environment=\"PATH=$SESSION_PATH\""

# Written by the shipped renderer, and the two lines it can write are the whole
# point of this script. The program the forced command names comes back on the
# first line, because the path it wants is one this demo has to create.
OUT=$(printf '%s\n' "$PUBKEY" | "$RENDER" "$DIR/authorized_keys" "$GRANT" "$PREFIX")
PROGRAM=$(printf '%s\n' "$OUT" | head -1)
LINE=$(printf '%s\n' "$OUT" | tail -1)

# The one path the connection has to resolve, made to resolve. A symlink rather
# than a copy, so a rebuild is picked up without running this again.
# A literal tilde is the point below: `$PROGRAM` is the UNEXPANDED text of the
# forced command, and what it expands to is the session's HOME, not this shell's.
# shellcheck disable=SC2088
case "$PROGRAM" in
    "~/"*) DAEMON_AT="$SESSION_HOME/${PROGRAM#\~/}" ;;
    *) echo "the forced command names $PROGRAM, which this script will not create"; exit 1 ;;
esac
mkdir -p "$(dirname "$DAEMON_AT")"
ln -sfn "$TARGET/farcoolerd" "$DAEMON_AT"

# Keep it in step with whatever key the app is actually using.
#
# The simulator's keychain does not reliably hand an app back the item it stored
# — across relaunches the device sometimes finds its key and sometimes generates
# a new one. That is a real problem for the app and is tracked separately; here
# it just means a fixed `authorized_keys` goes stale under you mid-demo. So this
# follows the key rather than snapshotting it, and re-renders through the same
# helper so that the line it writes is the line this run asked for.
#
# It authorizes whatever this simulator currently claims, which is fine for a
# throwaway sshd on the loopback that nothing else can reach, and would be
# indefensible anywhere else.
#
# What it compares is the key it last enrolled, kept beside the file, NOT the
# file's own text. `render` rebuilds the comment on every line it writes, so the
# key as the app spells it does not appear in `authorized_keys` verbatim and a
# grep for it never matches — which would have this loop rewriting a file nothing
# had changed, with an fsync and a fresh backup, every two seconds.
printf '%s\n' "$PUBKEY" > "$DIR/synced-key"
cat > "$DIR/sync-key.sh" <<SYNC
#!/bin/bash
while true; do
    key=\$(plutil -extract publicKey raw -o - "$PREFS" 2>/dev/null || true)
    if [ -n "\$key" ] && [ "\$key" != "\$(cat "$DIR/synced-key" 2>/dev/null)" ]; then
        printf '%s\n' "\$key" | "$RENDER" "$DIR/authorized_keys" "$GRANT" "$PREFIX" >/dev/null \
            && printf '%s\n' "\$key" > "$DIR/synced-key"
    fi
    sleep 2
done
SYNC
chmod +x "$DIR/sync-key.sh"
nohup "$DIR/sync-key.sh" >/dev/null 2>&1 &
echo $! > "$DIR/sync-key.pid"

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
# Required by the environment= options on the key line above, which are what put
# the session in this demo's home and fleet. See PREFIX in this script.
PermitUserEnvironment yes
LogLevel VERBOSE
EOF

# Checked before it is started, so a config this script got wrong reads as sshd's
# own complaint rather than as a connection that never happens.
/usr/sbin/sshd -t -f "$DIR/sshd_config" || { echo "sshd refused the config above"; exit 1; }
/usr/sbin/sshd -f "$DIR/sshd_config" -E "$DIR/sshd.log"
# `sshd` forks and returns straight away, so its exit status says only that the
# parent got as far as forking. Both facts are waited for: the pid file, which is
# how `stop` will find it, and the port.
for _ in $(seq 1 20); do
    [ -s "$DIR/sshd.pid" ] && nc -z 127.0.0.1 $PORT >/dev/null 2>&1 && break
    sleep 0.5
done
nc -z 127.0.0.1 $PORT >/dev/null 2>&1 || { echo "sshd did not start:"; tail -5 "$DIR/sshd.log"; exit 1; }
echo "sshd:   listening on 127.0.0.1:$PORT, pid $(cat "$DIR/sshd.pid")"

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
if [ "$GRANT" = "fenced" ]; then
    echo "Key line: RESTRICTED, as every enrolled device's is."
    echo "  sshd runs the forced command and discards the one the app asked for,"
    echo "  so a terminal's --stream channel never streams and the pane polls."
else
    echo "Key line: PLAIN. sshd runs what the app asks for, so --stream streams."
fi
echo "  $(printf '%s' "$LINE" | cut -c1-120)…"
echo
echo "Ready. The simulator has a host called \"Demo host\"."
echo "Open it to see the fleet; tap the 'scrolling' terminal and scroll its 400 lines."
echo
echo "Relaunching from Xcode or the home screen drops it — run this again to"
echo "put it back. The other key line is one flag away:"
if [ "$GRANT" = "fenced" ]; then
    echo "  ./scripts/demo-host.sh --plain"
else
    echo "  ./scripts/demo-host.sh"
fi
echo "When you are done:  ./scripts/demo-host.sh stop"
