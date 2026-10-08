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

"$@" 2>&1 | tee "$log"
rc="${PIPESTATUS[0]}"
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
