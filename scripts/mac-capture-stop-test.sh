#!/bin/bash
# Prove `scripts/mac-capture.sh stop` ends the scratch daemon and its tmux
# server when there's no CLI to ask (target/ already deleted, review M3).
#
# A stand-in daemon holds the home's lock, a stand-in server has a pane
# opened under the home, and a stand-in `tmux` records what it's told. Run
# against the stop that needed the CLI, it fails.
set -euo pipefail

cd "$(dirname "$0")/.."
home="$(mktemp -d /tmp/fc-stop-XXXXXX)"
bin="$home/bin"
mkdir -p "$bin"
cleanup() {
    kill "${daemon:-}" "${server:-}" 2>/dev/null || true
    rm -rf "$home"
}
trap cleanup EXIT

cat >"$bin/tmux" <<STUB
#!/bin/bash
echo "\$*" >>"$home/tmux.log"
STUB
chmod +x "$bin/tmux"

(exec 9>"$home/farcoolerd.lock"; exec sleep 300) &
daemon=$!
disown "$daemon"
(exec -a "tmux -L farcooler-0abc -f x new-session -c $home/worktrees/demo/wt" sleep 300) &
server=$!
disown "$server"
for _ in $(seq 50); do lsof -t "$home/farcoolerd.lock" >/dev/null 2>&1 && break; sleep 0.1; done

PATH="$bin:$PATH" FARCOOLER_HOME="$home" FARCOOLER_BIN="$home/no-cli" ./scripts/mac-capture.sh stop

fail() { echo "FAIL: $1" >&2; exit 1; }
sleep 0.2
kill -0 "$daemon" 2>/dev/null && fail "the daemon holding the lock is still running"
grep -qx -- "-L farcooler-0abc kill-server" "$home/tmux.log" 2>/dev/null \
    || fail "the server under the home wasn't ended: $(cat "$home/tmux.log" 2>/dev/null)"
echo "ok: stop ends the daemon and its tmux server with no CLI"
