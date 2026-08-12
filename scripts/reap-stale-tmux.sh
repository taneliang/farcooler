#!/bin/bash
# Kill tmux servers left behind by the test suite.
#
# Every live tmux test creates its own server. They now die with their test
# (see `Reaped` in crates/tmux/tests/live_tmux.rs), but the ones that leaked
# before that will not remove themselves — nothing else knows their socket
# names. On the machine this was written for there were 362 of them, each
# holding a session, a pane and an interactive shell.
#
# They are not free. tmux is single-threaded per server and they compete for the
# same CPU: `capture-pane` against the real fleet was timed at 50ms at rest and
# 740ms with those servers running.
#
#   ./scripts/reap-stale-tmux.sh          # say what would be killed
#   ./scripts/reap-stale-tmux.sh --yes    # actually kill it
#
# THE SERVER BELONGING TO YOUR INSTALL IS NEVER TOUCHED, in either mode. It is
# read from the install-id file and excluded by name, because the whole point of
# a private socket per install is that your running terminals are not test
# debris. Nothing here matches on `pkill`-style patterns for the same reason.
set -euo pipefail

CONFIRM="${1:-}"

TMUX="$(command -v tmux || true)"
[ -n "$TMUX" ] || { echo "tmux is not on PATH"; exit 1; }

SOCKET_DIR="${TMUX_TMPDIR:-/tmp}/tmux-$(id -u)"
[ -d "$SOCKET_DIR" ] || { echo "no tmux socket directory at $SOCKET_DIR"; exit 0; }

# The default install, resolved the way the daemon resolves it. Fatal if it
# cannot be read: a run that does not know which server is yours is a run that
# must not kill anything.
case "$(uname -s)" in
  Darwin) DEFAULT_HOME="$HOME/Library/Application Support/com.farcooler.FarCooler" ;;
  *)      DEFAULT_HOME="${XDG_DATA_HOME:-$HOME/.local/share}/farcooler" ;;
esac
DEFAULT_HOME="${FARCOOLER_HOME:-$DEFAULT_HOME}"

if [ ! -f "$DEFAULT_HOME/install-id" ]; then
  echo "no install-id under $DEFAULT_HOME — refusing to guess which server is yours" >&2
  exit 1
fi

# Every install a LIVE daemon is using, not just the default one.
#
# Sparing only the default was not enough and would have done damage. A daemon
# started with its own `FARCOOLER_HOME` — which is how scratch daemons and
# agent-driven test runs are launched — has a different install id and therefore
# a different socket, and it is every bit as live as yours. Three such daemons
# were running the first time this script was pointed at a real machine.
#
# Their homes come from the processes themselves rather than from a list kept
# somewhere: the running daemon is the only thing that actually knows.
spared=()
add_spared() {
  local home="$1"
  [ -f "$home/install-id" ] || return 0
  spared+=("farcooler-$(tr -d '[:space:]' < "$home/install-id")")
}
add_spared "$DEFAULT_HOME"

while read -r pid; do
  [ -n "$pid" ] || continue
  # `ps eww` prints the process environment after its command line. A daemon
  # with no FARCOOLER_HOME is on the default one, already spared above.
  home="$(ps eww -o command= -p "$pid" 2>/dev/null | tr ' ' '\n' \
          | sed -n 's/^FARCOOLER_HOME=//p' | head -1)"
  [ -n "$home" ] && add_spared "$home"
done < <(pgrep -x farcoolerd 2>/dev/null || true)

# Deduplicate, so the count reported is a count of installs and not of daemons.
IFS=$'\n' read -r -d '' -a spared < <(printf '%s\n' "${spared[@]}" | sort -u; printf '\0')
echo "sparing ${#spared[@]} live install(s):"
printf '  %s\n' "${spared[@]}"

is_spared() {
  local name="$1"
  for s in "${spared[@]}"; do [ "$s" != "$name" ] || return 0; done
  return 1
}

# A server holding work outside a temporary directory is not test debris.
#
# The leaked servers all sit on worktrees under the system temp dir, because
# that is where the fixtures build them. Anything else is somebody's actual
# session, and the fact that no daemon is currently attached to it does not make
# it disposable — `farcooler attach` can still reach it.
#
# Answered by asking the server, which also doubles as the liveness check: a
# socket whose server is gone fails here and is simply removed.
holds_real_work() {
  local name="$1" paths
  paths="$("$TMUX" -L "$name" list-panes -a -F '#{pane_current_path}' 2>/dev/null)" || return 1
  [ -n "$paths" ] || return 1
  grep -qvE "^(/private)?(/var/folders/|/tmp/)" <<<"$paths"
}

stale=()
kept=()
for socket in "$SOCKET_DIR"/farcooler-*; do
  [ -S "$socket" ] || continue
  name="$(basename "$socket")"
  is_spared "$name" && continue
  if holds_real_work "$name"; then
    kept+=("$name")
    continue
  fi
  stale+=("$name")
done

if [ ${#kept[@]} -gt 0 ]; then
  echo "keeping ${#kept[@]} server(s) holding work outside a temp directory:"
  printf '  %s\n' "${kept[@]}"
fi

if [ ${#stale[@]} -eq 0 ]; then
  echo "nothing stale."
  exit 0
fi

echo "${#stale[@]} stale server(s)."

if [ "$CONFIRM" != "--yes" ]; then
  printf '%s\n' "${stale[@]}" | head -20
  [ ${#stale[@]} -gt 20 ] && echo "  … and $(( ${#stale[@]} - 20 )) more"
  echo
  echo "This was a dry run. Re-run with --yes to kill them."
  exit 0
fi

killed=0
for name in "${stale[@]}"; do
  # A socket with no server behind it is the common case — the server exited and
  # left the file. `kill-server` fails on those, which is not an error worth
  # reporting; the socket is removed either way.
  "$TMUX" -L "$name" kill-server >/dev/null 2>&1 && killed=$(( killed + 1 )) || true
  rm -f "$SOCKET_DIR/$name"
done

echo "killed $killed server(s), removed ${#stale[@]} socket(s)."
