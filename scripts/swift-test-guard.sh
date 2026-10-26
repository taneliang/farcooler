#!/bin/bash
# Run a Swift test command and fail unless the run finished and ran enough.
#
#   scripts/swift-test-guard.sh <floor-file> -- <command...>
#
# A test that exits the process mid-run (a real NSMenu did, in integ-14) ends
# `swift test` with exit 0, and every test after it goes unrun while the run
# looks green (ov-324). swift-testing prints its final line, "Test run with N
# tests ... passed" or "failed", only when the run reaches the end. So this
# fails when that line is missing, and when N is below the number in
# <floor-file>, a checked-in floor that a change adding tests raises.
set -uo pipefail

floor_file="$1"
[ "${2:-}" = "--" ] || { echo "usage: swift-test-guard.sh <floor-file> -- <command...>" >&2; exit 2; }
shift 2

# The test process must not die from SIGPIPE (ov-800). Test runners leave it at
# its default, so a write to a child that has exited (a stand-in CLI that never
# reads its stdin) killed swiftpm-testing-helper with signal 13 and stopped the
# whole suite. The core now refuses the signal itself (crates/client/src/pipe.rs),
# and this is the backstop for any writer that does not: an ignored signal stays
# ignored through exec, so the write fails with EPIPE and that test reports it.
trap '' PIPE

log="$(mktemp -t farcooler-swift-test)"
trap 'rm -f "$log"' EXIT

# A Mac whose display is asleep (15 minutes idle) gives windows no display
# ticks, so an animated scroll in a test window never lands and parks a pool
# thread in NSAnimation. 64 parked threads starve the Swift concurrency pool
# and the real-CLI tests stall behind them, and NavigatorRevealTests and
# AgentScrollTests fail on the animation that never ran (ov-429). CI's virtual
# display never sleeps. So wake the display, and hold it awake for the run.
if command -v caffeinate >/dev/null 2>&1; then
  caffeinate -u -t 1 || true
  set -- caffeinate -d "$@"
fi

# The wake doesn't always take: in Train 267's gate the display went off at
# 22:40 and stayed off through the whole Mac run, which got 0 display ticks
# in 120 s, and the navigator's reveal tests failed as if they were flaky
# under load (ov-742). So say so, from the power log, after the run.
started="$(date '+%Y-%m-%d %H:%M:%S')"
display_note() {
  command -v pmset >/dev/null 2>&1 || return 0
  # "2026-10-24 22:40:54 -0700 Notification  Display is turned off": was it
  # off when the run began (the last such line before), or turned off since?
  local off
  off="$(pmset -g log 2>/dev/null | grep -E 'Display is turned (on|off)' | awk -v start="$started" '
    { at = $1 " " $2; state = ($NF == "off") }
    at < start { before = state; next }
    state { during = 1 }
    END { if (before || during) print "off" }')"
  [ "$off" = "off" ] || return 0
  echo "swift-test-guard: the display was off for some or all of this run (pmset -g log), and caffeinate didn't wake it. A window gets no display ticks then, so the tests that wait on an animated scroll fail: TestWindowTests' tick check, NavigatorRevealTests, NavigatorSplitTests' arrowing test, TranscriptScrollerTests. Wake the display (unlock the screen) and run again before reading those failures as flaky." >&2
}

"$@" 2>&1 | tee "$log"
rc="${PIPESTATUS[0]}"
display_note
[ "$rc" -eq 0 ] || exit "$rc"

floor="$(tr -d '[:space:]' <"$floor_file")"
case "$floor" in '' | *[!0-9]*) echo "swift-test-guard: $floor_file must hold one number" >&2; exit 2 ;; esac

summary="$(grep -E 'Test run with [0-9]+ tests? .*(passed|failed)' "$log" | tail -1 || true)"
if [ -z "$summary" ]; then
  echo "swift-test-guard: the run ended with exit 0 but never printed swift-testing's \"Test run with N tests ... passed\" line, so it stopped early and the tests after that point did not run." >&2
  exit 1
fi
count="$(sed -E 's/.*Test run with ([0-9]+) tests?.*/\1/' <<<"$summary")"
if [ "$count" -lt "$floor" ]; then
  echo "swift-test-guard: the run counted $count tests, below the floor of $floor in $floor_file. Tests went unrun. If tests were deleted on purpose, lower the floor in the same commit; if you added tests, raise it to the new count." >&2
  exit 1
fi
echo "swift-test-guard: $count tests ran (floor $floor)."
