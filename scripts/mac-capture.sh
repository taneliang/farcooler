#!/bin/bash
# Capture the real Mac window, offscreen, against a seeded scratch daemon (ov-278).
#
#   scripts/mac-capture.sh <out-dir> [stage]       every seeded place, light and dark
#   scripts/mac-capture.sh stop                    stop the scratch daemon and its tmux
#
# What it does, in order:
#
#   1. Starts a daemon of its own under FARCOOLER_HOME (default /tmp/fc-t/<lane>,
#      short because the socket lives under it), from this checkout's CLI.
#   2. Seeds it once: a scratch repository, a workspace with tasks on its board,
#      a worktree with a shell, a second shell and a committed change, and a
#      task waiting on a decision. The ids land in $FARCOOLER_HOME/seed.json.
#   3. Runs `RealWindowCaptures` (apps/macos/Tests/CeremonyTests), which opens
#      the real `ContentView` in a real titled window off every screen, sends
#      it to each place and writes `<stage>-<place>-<variant>.png` into
#      <out-dir>. Variants: light, dark, and both with Increase Contrast.
#
# Nothing here sends input, and nothing touches your own daemon: the test
# refuses to run unless FARCOOLER_HOME and FARCOOLER_BIN point at the scratch
# one. Off in CI: the test is enabled only by FARCOOLER_CAPTURE_OUT.
#
# Environment, all optional:
#   FARCOOLER_CAPTURE_LANE    names the scratch home, /tmp/fc-t/<lane>  (capture)
#   FARCOOLER_CAPTURE_ONLY    one place's name, to capture just that one
#   FARCOOLER_CAPTURE_PLACES  extra places, "name=<selection>" per line, where
#                             <selection> is what `SelectionMemory.key` stores
#   FARCOOLER_CAPTURE_HEIGHT  the window's height in points (860)
#   FARCOOLER_CAPTURE_HOVER   jump bar pieces drawn hovered, by VoiceOver name, one
#                             per line ("Go to Billing", "Show other workspaces")
#   FARCOOLER_CAPTURE_WAIT    seconds a window settles before it's drawn (5)
#   FARCOOLER_CAPTURE_WIDTH   the window's width in points (1360)
#   FARCOOLER_CAPTURE_PEEK    set: the plan peeked over the chat, as ⌥⌘P does
#   FARCOOLER_BIN             the CLI; unset, target/debug/farcooler and farcoolerd are built
#   FARCOOLER_CAPTURE_APP_BIN the CLI the window runs, when it isn't FARCOOLER_BIN:
#                             a wrapper that stalls draws a runner still connecting
#   SWIFT_JOBS                swift test's -j (3)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
lane="${FARCOOLER_CAPTURE_LANE:-capture}"
home="${FARCOOLER_HOME:-/tmp/fc-t/$lane}"
bin="${FARCOOLER_BIN:-$root/target/debug/farcooler}"

fc() { FARCOOLER_HOME="$home" "$bin" "$@"; }

if [ "${1:-}" = "stop" ]; then
    # The daemon, then its tmux server, which `daemon stop` leaves running.
    # With the CLI: `status` names the socket in its recovery line. Without
    # one (target/ already deleted): the daemon is whoever holds the home's
    # lock, and the server is the one whose panes open under the home.
    socket=""
    if [ -x "$bin" ]; then
        socket=$(fc status 2>/dev/null | sed -n -E 's/.*tmux -L (farcooler-[0-9a-f]+).*/\1/p' | head -1)
        fc daemon stop >/dev/null 2>&1 || true
    fi
    for pid in $(lsof -t "$home/farcoolerd.lock" 2>/dev/null); do kill "$pid" 2>/dev/null || true; done
    if [ -z "$socket" ]; then
        socket=$(ps -axo command= | grep -F -- "-c $home/" | grep -v grep \
            | sed -n -E 's/.*tmux -L (farcooler-[0-9a-f]+).*/\1/p' | head -1)
    fi
    if [ -n "$socket" ]; then
        tmux -L "$socket" kill-server 2>/dev/null || true
    else
        echo "mac-capture: no tmux server found for $home" >&2
    fi
    exit 0
fi

out="${1:?usage: scripts/mac-capture.sh <out-dir> [stage] | stop}"
stage="${2:-capture}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"

# The CLI and the daemon beside it: a CLI with no `farcoolerd` next to it
# starts whichever one is installed, and that one is a different build.
if [ -z "${FARCOOLER_BIN:-}" ]; then
    echo "mac-capture: building the CLI and the daemon" >&2
    (cd "$root" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" "$HOME/.cargo/bin/cargo" build -q -p farcooler-cli -p farcooler-daemon)
fi

mkdir -p "$home"
# A first start can outlast ensure's own five seconds; one more ask is enough.
fc --json daemon ensure >/dev/null 2>&1 || fc --json daemon ensure >/dev/null

seed="$home/seed.json"
if [ ! -f "$seed" ]; then
    repos="$home/repos"
    demo="$repos/demo"
    mkdir -p "$demo"
    g() { git -C "$1" -c user.name=Capture -c user.email=capture@example.com "${@:2}"; }
    g "$demo" init -q -b main
    printf 'demo\n' >"$demo/README.md"
    g "$demo" add README.md
    g "$demo" commit -q -m "Start the demo"
    fc root add "$repos" >/dev/null
    fc repo register "$demo" >/dev/null

    ws=$(fc --json workspace create demo --name "Billing" --prefix bil | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
    wt_json=$(fc --json worktree create demo bil-1-invoices --branch bil-1-invoices --fork-only)
    wt=$(printf '%s' "$wt_json" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
    wt_path=$(printf '%s' "$wt_json" | python3 -c 'import json,sys; print(json.load(sys.stdin)["worktree"])')
    fc worktree assign "$wt" --to "$ws" >/dev/null

    # A committed change and an uncommitted one, so Changes has a diff.
    mkdir -p "$wt_path/src"
    printf 'func total(_ lines: [Int]) -> Int {\n    lines.reduce(0, +)\n}\n' >"$wt_path/src/Invoice.swift"
    g "$wt_path" add src/Invoice.swift
    g "$wt_path" commit -q -m "Add invoice totals"
    printf 'demo\n\nInvoices add up their lines.\n' >"$wt_path/README.md"

    shell=$(fc --json terminal create "$wt" --title "Terminal 1" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
    second=$(fc --json terminal create "$wt" --tile --title "Terminal 2" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')

    started_json=$(fc --json task create --workspace "$ws" --title "Total each invoice" \
        --intent "Invoices show the sum of their lines.")
    started=$(printf '%s' "$started_json" | python3 -c 'import json,sys; print(json.load(sys.stdin)["key"])')
    task=$(printf '%s' "$started_json" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
    fc task set "$started" --repo demo --status in_progress >/dev/null
    asked=$(fc --json task create --workspace "$ws" --title "Round half-cents" --intent "Pick a rounding rule." \
        | python3 -c 'import json,sys; print(json.load(sys.stdin)["key"])')
    fc task ask "$asked" --repo demo --actor manager --body "Banker's rounding, or half up?" >/dev/null
    done_key=$(fc --json task create --workspace "$ws" --title "Name the currency field" \
        | python3 -c 'import json,sys; print(json.load(sys.stdin)["key"])')
    fc task set "$done_key" --repo demo --status done >/dev/null

    python3 - "$seed" "$ws" "$wt" "$shell" "$second" "$task" <<'EOF'
import json, sys
path, ws, wt, shell, second, task = sys.argv[1:]
json.dump({"workspace": ws, "worktree": wt, "shell": shell, "second": second, "task": task}, open(path, "w"), indent=2)
EOF
fi

cd "$root/apps/macos"
# Built Rust cores first, as test.sh does: a stale library fails to link.
./build-vt.sh >/dev/null
env -u FARCOOLER_WORKSPACE \
    FARCOOLER_HOME="$home" FARCOOLER_BIN="${FARCOOLER_CAPTURE_APP_BIN:-$bin}" \
    FARCOOLER_CAPTURE_OUT="$out" FARCOOLER_CAPTURE_STAGE="$stage" FARCOOLER_CAPTURE_SEED="$seed" \
    swift test -j "${SWIFT_JOBS:-3}" --filter RealWindowCaptures
echo "mac-capture: wrote $(ls "$out" | grep -c "^$stage-") images to $out" >&2
echo "mac-capture: the daemon is still up for the next run; scripts/mac-capture.sh stop ends it" >&2
