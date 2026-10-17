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
#   FARCOOLER_CAPTURE_FOLD    collapse or expand: the navigator's tree folded, as View's items do
#   FARCOOLER_CAPTURE_RUNNERS runners configured before the window opens, one target per
#                             line: "carl.invalid" is never reachable, so the toolbar says it's offline
#   FARCOOLER_CAPTURE_SCALE   the bitmap's scale, 1 or 2; unset, the window's own
#   FARCOOLER_CAPTURE_WAIT    seconds a window settles before it's drawn (5)
#   FARCOOLER_CAPTURE_WIDTH   the window's width in points (1360)
#   FARCOOLER_CAPTURE_PEEK    set: the plan peeked over the chat, as ⌥⌘P does
#   FARCOOLER_CAPTURE_RULING_ACTIONS  set: Keep, Reverse and Discuss drawn on each open
#                             ruling, as when the pointer is on it (a capture can't hover)
#   FARCOOLER_BIN             the CLI; unset, target/debug/farcooler and farcoolerd are built
#   FARCOOLER_CAPTURE_APP_BIN the CLI the window runs, when it isn't FARCOOLER_BIN:
#                             a wrapper that stalls draws a runner still connecting
#   SWIFT_JOBS                swift test's -j (3)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
lane="${FARCOOLER_CAPTURE_LANE:-capture}"
home="${FARCOOLER_HOME:-/tmp/fc-t/$lane}"
bin="${FARCOOLER_BIN:-$root/target/debug/farcooler}"

# Its own config.toml, never this Mac's shared one (ov-394).
fc() { FARCOOLER_HOME="$home" FARCOOLER_CONFIG="$home/config.toml" "$bin" "$@"; }

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
# The agent every launch here runs: a script that prints a few lines and waits,
# named by absolute path through FARCOOLER_STAND_IN_AGENT, so no pane this
# daemon opens can start a real claude, codex or any other agent (ov-203). It
# runs as `claude` by argv, as the demo runner's stand-in does, so the daemon
# names the pane as an agent. Nothing reads or sends anything.
stand_in="$home/stand-in/claude"
mkdir -p "$home/stand-in"
cat >"$stand_in" <<'STANDIN'
#!/bin/bash
# A capture's stand-in agent, never a real one. See scripts/mac-capture.sh.
exec -a claude /bin/bash "$(dirname "$0")/agent-screen.sh" "$@"
STANDIN
cat >"$home/stand-in/agent-screen.sh" <<'SCREEN'
#!/bin/bash
clear
printf '\n'
printf '\033[1m%s\033[0m\n\n' "Total each invoice"
printf '%s\n' "I'll start with how an invoice is built today." ""
printf '\033[32m●\033[0m Read(src/Invoice.swift)\n    Read 3 lines\n'
printf '\033[32m●\033[0m Update(src/Invoice.swift)\n    Added total(_:) to sum the lines of an invoice\n'
printf '\033[32m●\033[0m Bash(swift test --filter InvoiceTests)\n    Test run with 4 tests passed\n\n'
printf '%s\n\n' "Totals are summed in whole cents. Next I'll wire the total into the invoice header."
printf '\033[2m%s\033[0m\n' "────────────────────────────────────────────────────────────"
printf '❯ \n'
exec /bin/sleep 86400
SCREEN
chmod +x "$stand_in" "$home/stand-in/agent-screen.sh"

# A first start can outlast ensure's own five seconds; one more ask is enough.
FARCOOLER_STAND_IN_AGENT="$stand_in" fc --json daemon ensure >/dev/null 2>&1 \
    || FARCOOLER_STAND_IN_AGENT="$stand_in" fc --json daemon ensure >/dev/null

# `ensure` reuses a daemon that is already up, and one started without the
# variable would launch the real claude for the agent pane and the
# orchestrator below. Nothing is seeded until the daemon says it runs ours.
status_text=$(fc status 2>/dev/null || true) # not piped to grep -q: pipefail would read its early exit as a failure
if ! printf '%s\n' "$status_text" | grep -qF "STAND-IN: $stand_in runs instead"; then
    echo "mac-capture: the scratch daemon at $home isn't running the stand-in agent, so it would start a real one." >&2
    echo "mac-capture: run scripts/mac-capture.sh stop (or remove $home) and try again." >&2
    exit 1
fi

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

    # An agent at work: its own worktree with a committed change, a pane running
    # the stand-in above on the started task, and the workspace's orchestrator
    # (the same stand-in, so no login is read). Added after the ones above, so
    # their ids and places are as they were.
    agent_json=$(fc --json worktree create demo bil-4-totals --branch bil-4-totals --fork-only)
    agent_wt=$(printf '%s' "$agent_json" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
    agent_path=$(printf '%s' "$agent_json" | python3 -c 'import json,sys; print(json.load(sys.stdin)["worktree"])')
    fc worktree assign "$agent_wt" --to "$ws" >/dev/null
    mkdir -p "$agent_path/src" "$agent_path/Tests"
    printf 'struct Invoice {\n    var lines: [Int]\n\n    /// The invoice total, in whole cents.\n    var total: Int { lines.reduce(0, +) }\n}\n' >"$agent_path/src/Invoice.swift"
    printf 'func testTotalSumsTheLines() {\n    assert(Invoice(lines: [250, 125]).total == 375)\n}\n' >"$agent_path/Tests/InvoiceTests.swift"
    g "$agent_path" add src/Invoice.swift Tests/InvoiceTests.swift
    g "$agent_path" commit -q -m "Total each invoice from its lines"
    printf 'struct Invoice {\n    var lines: [Int]\n\n    /// The invoice total, in whole cents.\n    var total: Int { lines.reduce(0, +) }\n    var header: String { "Total: \\(total)" }\n}\n' >"$agent_path/src/Invoice.swift"
    agent=$(fc --json terminal create "$agent_wt" --preset claude --task "$started" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
    fc workspace start-orchestrator Billing --harness claude >/dev/null 2>&1 \
        || echo "mac-capture: the orchestrator didn't start; the window will say so" >&2

    python3 - "$seed" "$ws" "$wt" "$shell" "$second" "$task" "$agent_wt" "$agent" <<'EOF'
import json, sys
path, ws, wt, shell, second, task, agent_wt, agent = sys.argv[1:]
json.dump({"workspace": ws, "worktree": wt, "shell": shell, "second": second, "task": task,
           "agentWorktree": agent_wt, "agent": agent}, open(path, "w"), indent=2)
EOF
fi

cd "$root/apps/macos"
# Built Rust cores first, as test.sh does: a stale library fails to link.
./build-vt.sh >/dev/null
env -u FARCOOLER_WORKSPACE \
    FARCOOLER_HOME="$home" FARCOOLER_CONFIG="$home/config.toml" FARCOOLER_BIN="${FARCOOLER_CAPTURE_APP_BIN:-$bin}" \
    FARCOOLER_CAPTURE_OUT="$out" FARCOOLER_CAPTURE_STAGE="$stage" FARCOOLER_CAPTURE_SEED="$seed" \
    swift test -j "${SWIFT_JOBS:-3}" --filter RealWindowCaptures
echo "mac-capture: wrote $(ls "$out" | grep -c "^$stage-") images to $out" >&2
# An Increase Contrast capture that draws the same as its standard twin
# below the title bar proves nothing (ov-519 review R1).
"$root/scripts/capture-contrast-check.sh" "$out" "$stage"
echo "mac-capture: the daemon is still up for the next run; scripts/mac-capture.sh stop ends it" >&2
