#!/bin/bash
# Prove a built `farcoolerd` actually has the tunnel linked into it.
#
# This is the only check on the tunnel that is not a proxy. A daemon built
# without the Go archive still builds, still starts, still serves every runner
# it can reach by address, and answers `no_tailcat` to every tunnel it is asked
# to serve — silently, in a log line nobody reads. Every Linux and macOS
# release shipped exactly that daemon until `linux-binaries.yml` and
# `apps/macos/build-app.sh` were made to link the archive, and nothing anywhere
# went red for it. This is what would have.
#
#   ./scripts/tunnel-smoke.sh dist/x86_64-linux/farcoolerd
#   ./scripts/tunnel-smoke.sh "apps/macos/build/Far Cooler.app/Contents/Resources/farcoolerd"
#
# The binary must be one this machine can execute — the check RUNS it. There is
# no cross-architecture form of this; a runner that builds aarch64 on an x86_64
# host can only assert the weaker things (`file` reports a Go build id, the
# binary is the archive's size larger than a stub) and must say so.
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

mkdir -p "$ROOT/home/.ssh"

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

# `start_tunnel` is spawned before the daemon binds its socket and can block for
# 30-45 seconds on DERP region picking. The stub, by contrast, refuses
# instantly: `farcooler_tailcat::serve` returns `Err(NoTailcatLinked)` without
# touching the network. So the window below does not need to be long enough for
# a real tunnel to come up — it only needs to be long enough for a stub to have
# failed, which is what makes this check reliable rather than dependent on CI
# having working UDP to a DERP relay.
ALIVE=1
for _ in $(seq 1 20); do
  grep -q "no_tailcat" "$LOG" && break
  kill -0 "$PID" 2>/dev/null || { ALIVE=0; break; }
  sleep 1
done

if grep -q "no_tailcat" "$LOG"; then
  echo
  echo "FAILED: this farcoolerd has no tunnel linked — it answered no_tailcat."
  echo "It was built without the Go archive; see scripts/build-linux.sh and"
  echo "apps/macos/build-app.sh for how a shipping build links one."
  echo
  grep -n "tunnel" "$LOG" || true
  exit 1
fi

# Absence of `no_tailcat` is only half an answer: a daemon that died before
# `start_tunnel` ran would also have failed to say it, and this check would
# have proved nothing at all. So one of three things must be true, and each is
# something a stub cannot produce:
#
#   - it reported a tunnel outcome that is not `no_tailcat`, or
#   - it is still running after the window, which means the Go side is still
#     inside `serve` — the stub returns from `serve` without touching the
#     network, in microseconds.
#
# Anything else is a daemon that fell over, and this script says so rather than
# reporting a pass it did not earn.
if grep -qE "the tunnel did not start|serving the tunnel|the tunnel is serving" "$LOG"; then
  echo "    reached the Go side and reported an outcome that is not no_tailcat:"
  grep -nE "tunnel" "$LOG"
elif [ "$ALIVE" = 1 ]; then
  echo "    still inside serve() after the window, which a stub never is"
else
  echo
  echo "FAILED: the daemon exited before it said anything about a tunnel, so"
  echo "this check proved nothing. Its whole log follows."
  echo
  cat "$LOG"
  exit 1
fi
