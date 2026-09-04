#!/bin/bash
# Prove a built `farcoolerd` actually has the tunnel linked into it.
#
# This is the only check on the tunnel that is not a proxy. A daemon built
# without the Go archive still builds, still starts, still serves every runner
# it can reach by address, and answers `no_tailcat` to every tunnel it is asked
# to serve — silently, in a log line nobody reads. Every Mac release shipped
# exactly that daemon until `apps/macos/build-app.sh` was made to link the
# archive, and nothing anywhere went red for it. This is what would have.
#
#   ./scripts/tunnel-smoke.sh "apps/macos/build/Far Cooler.app/Contents/Resources/farcoolerd"
#   ./scripts/tunnel-smoke.sh dist/x86_64-linux/farcoolerd
#
# It is not only a linkage check. It RUNS the daemon, so it also catches a
# binary that linked and then cannot start — which is exactly what a Linux
# daemon with the archive linked used to do (see `docs/releasing.md`): it
# segfaulted before it logged a line, and a check that only looked for a Go
# build id would have called that a pass and shipped it. That is why Linux now
# ships a separate helper process instead.
#
# Which is also why this check still earns its place on Linux, in a new way. A
# `farcoolerd` built with `tailcat-helper` finds `farcooler-tunnel` beside
# itself and spawns it; a helper that is missing, unexecutable, or built for
# the wrong architecture makes `serve` answer `no_tailcat` — deliberately, so
# that the one grep below covers the new failure mode as well as the old one.
# See `crates/tailcat/src/helper.rs`. A tarball that forgot to pack the helper
# therefore fails here rather than on somebody's runner.
#
# The binary must be one this machine can execute. There is no
# cross-architecture form of this; a host that builds aarch64 on x86_64 can
# only assert the weaker things and must say so.
set -euo pipefail

DAEMON="${1:?usage: tunnel-smoke.sh <path to farcoolerd>}"
[ -x "$DAEMON" ] || { echo "not an executable: $DAEMON" >&2; exit 1; }
DAEMON="$(cd "$(dirname "$DAEMON")" && pwd)/$(basename "$DAEMON")"

# `/tmp`, not `$TMPDIR`: the daemon binds a Unix socket under this root and
# `sun_path` holds 104 bytes on darwin, where `$TMPDIR` alone is already sixty
# of them. `crates/daemon/tests/an_empty_allowlist_starts_no_tunnel.rs` keeps
# its own root short for the same reason, and names it.
ROOT="$(mktemp -d /tmp/fcsmoke.XXXXXX)"
LOG="$ROOT/daemon.log"
PID=""

# Kill by PID, never by name. A developer machine is very likely running a real
# Far Cooler daemon, and a pattern kill would take it down.
cleanup() {
  [ -n "$PID" ] && kill "$PID" 2>/dev/null || true
  [ -n "$PID" ] && wait "$PID" 2>/dev/null || true
  rm -rf "$ROOT"
}
trap cleanup EXIT

# 0700 on both, explicitly, and not left to the umask.
#
# `farcooler-fence` applies sshd's own `StrictModes` rule to the directory
# `authorized_keys` lives in and refuses a group- or world-writable one — a
# directory somebody else can write is a directory in which they can replace
# that file. Ubuntu's default umask is 0002, which makes `mkdir -p` produce
# 0775 and the fence refuse it; macOS's 0022 does not, which is exactly how
# this went unnoticed until the check was first run on Linux. The daemon then
# answers `FenceUnreadable` and never reaches the tunnel at all, and this
# script would have been asserting nothing.
mkdir -p "$ROOT/home/.ssh"
chmod 700 "$ROOT/home" "$ROOT/home/.ssh"

# Its existence is the whole feature flag — `Service::tailcat_key`. The contents
# are not asserted on here: whatever the Go side makes of them, the outcome it
# reports is not `no_tailcat`, which is the one thing this script is about.
printf 'scratch tailcat identity\n' > "$ROOT/tailcat.key"
chmod 600 "$ROOT/tailcat.key"

# One enrolled device carrying a node key, which is what `tunnel_plan` requires
# before `start_tunnel` reaches `farcooler_tailcat::serve` at all — without it
# the daemon answers `NobodyAdmitted` and never calls the FFI, and this check
# would pass on a stub.
#
# Written by hand rather than through `farcooler-fence`, because the point is to
# exercise the SHIPPED binary and nothing else. The markers are matched
# literally (`crates/fence/src/lib.rs`), and the parser only asks that the
# options field carry a forced command with `--client` and a 43-character
# `--node-key`. The key material below is the same obviously-synthetic pair
# `crates/daemon/tests/an_empty_allowlist_starts_no_tunnel.rs` uses.
cat > "$ROOT/home/.ssh/authorized_keys" <<'EOF'
# BEGIN FAR COOLER — do not edit inside this block
restrict,command="~/.local/bin/farcoolerd --stdio --client smoke --scope control --node-key 3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA smoke
# END FAR COOLER
EOF
chmod 600 "$ROOT/home/.ssh/authorized_keys"

echo "==> Starting $DAEMON against $ROOT"
# HOME as well as FARCOOLER_HOME: the daemon finds `authorized_keys` through
# the user's home directory, and a check that wrote into the real one would be
# editing the file that holds this machine's SSH access.
HOME="$ROOT/home" FARCOOLER_HOME="$ROOT" RUST_LOG=info "$DAEMON" > "$LOG" 2>&1 &
PID=$!

# What this waits for is a line saying what happened to the tunnel — any of
# them, including the refusals. The identity above is deliberately not a usable
# key, so `serve` fails at the file rather than on the network: the answer
# arrives in milliseconds and this never depends on CI having working UDP to a
# DERP relay.
#
# REACHED is every outcome that proves the tunnel code ran. REFUSED is every
# outcome that proves it did not — a broken fixture, not a broken daemon, and
# each one is a way this script can assert nothing while looking like a pass.
REACHED="the tunnel did not start|serving the tunnel|the tunnel is serving"
REFUSED="serves no tunnel"

for _ in $(seq 1 30); do
  grep -qE "no_tailcat|$REACHED|$REFUSED" "$LOG" && break
  kill -0 "$PID" 2>/dev/null || break
  sleep 1
done

if grep -q "no_tailcat" "$LOG"; then
  echo
  echo "FAILED: this farcoolerd has no tunnel — it answered no_tailcat."
  echo "Either it was built without the tunnel at all, or (on Linux) there is no"
  echo "\`farcooler-tunnel\` beside it that this machine can execute. See"
  echo "scripts/build-linux.sh and apps/macos/build-app.sh for what a shipping"
  echo "build produces."
  echo
  grep -n "tunnel" "$LOG" || true
  exit 1
fi

# Absence of `no_tailcat` is only half an answer: a daemon that never reached
# the tunnel would also have failed to say it. So a positive line is required,
# and "the daemon is still running" is deliberately NOT one of the things that
# can produce a pass — a daemon stays running whatever happens to its tunnel,
# which made that branch a check incapable of failing. It reported one on
# Linux for a daemon that answered `FenceUnreadable` and never called `serve`
# at all.
if grep -qE "$REFUSED" "$LOG"; then
  echo
  echo "FAILED: the daemon refused before it reached the tunnel, so this check"
  echo "proved nothing. That is this script's own fixture being wrong, not the"
  echo "daemon: it means the scratch identity or authorized_keys below was not"
  echo "accepted. Its whole log follows."
  echo
  cat "$LOG"
  exit 1
fi

if grep -qE "$REACHED" "$LOG"; then
  echo "    reached the tunnel and reported an outcome that is not no_tailcat:"
  grep -nE "tunnel" "$LOG"
else
  echo
  echo "FAILED: the daemon said nothing about a tunnel within the window, so"
  echo "this check proved nothing. Its whole log follows."
  echo
  cat "$LOG"
  exit 1
fi
