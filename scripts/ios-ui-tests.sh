#!/bin/bash
# Run the iOS UI suite against the demo runner, and go RED when nothing ran.
#
# Two defects made this script necessary, and both of them printed
# `** TEST SUCCEEDED **`.
#
# The first is the invocation. The suite reads the Mac's user and host out of
# `DEMO_USER` and `DEMO_HOST`, which xcodebuild forwards into the test runner
# from any variable named `TEST_RUNNER_<VAR>`. That forwarding reads the
# variable from XCODEBUILD'S OWN ENVIRONMENT — `TEST_RUNNER_DEMO_USER=… \
# xcodebuild test …`, the assignment BEFORE the command, exactly as
# xcodebuild(1) shows it. Written after the command instead it is not an
# environment variable at all, it is a command-line BUILD SETTING: xcodebuild
# accepts it, `-showBuildSettings` shows it set to the right value, and nothing
# is ever forwarded anywhere. The runner then launched the app with an empty
# user, sshd logged `Invalid user`, the app sat on "Not Authorized Yet", and
# every test that needs a live pane skipped waiting for a terminal that could
# not appear.
#
# The second is that a fully-skipped run is a passing run. `Executed 8 tests,
# with 8 tests skipped and 0 failures` exits 0, and that is what let a broken
# terminal scroll ship past eight green scroll tests. So this script reads the
# count back and fails when no test actually executed. A suite that cannot fail
# is worse than no suite, and the only way to tell the two apart is to count.
#
# Partial skips are still allowed on purpose — some tests want hardware this
# machine does not have (KeyboardTabStripTests wants a real iPhone), and going
# red for those would teach everyone to ignore the result, which is the same
# disease. What is refused is a run where NOTHING ran.
#
#   ./scripts/demo-host.sh          # first: stand up the runner
#   ./scripts/ios-ui-tests.sh       # the whole suite
#   ./scripts/ios-ui-tests.sh FarCoolerUITests/TerminalScrollTests
#
# Any argument is passed to -only-testing. DEMO_HOST and SIMULATOR override the
# defaults if you have moved the runner or want another device.
set -euo pipefail

cd "$(dirname "$0")/.."

SIMULATOR="${SIMULATOR:-iPhone 17}"
DEMO_HOST="${DEMO_HOST:-127.0.0.1:2222}"
DEMO_USER="${DEMO_USER:-$(whoami)}"

# Expanded below as `${ONLY[@]+"${ONLY[@]}"}` rather than `"${ONLY[@]}"`,
# because macOS ships bash 3.2, where an empty array is an unbound variable
# under `set -u` — the plain form kills the script before it reaches xcodebuild
# in exactly the case where no argument was given, which is the whole suite.
ONLY=()
for target in "$@"; do
    ONLY+=("-only-testing:$target")
done

# Said out loud, because an empty user here is the whole first defect and it is
# invisible in xcodebuild's output — it surfaces four screens away as an app
# that will not connect.
echo "runner:    $DEMO_USER@$DEMO_HOST"
echo "simulator: $SIMULATOR"
echo

LOG="$(mktemp -t ios-ui-tests)"
trap 'rm -f "$LOG"' EXIT

# The assignments go BEFORE xcodebuild. See the note at the top: this position
# is not a style choice, it is the difference between the suite running and the
# suite skipping itself into a green.
set +e
env \
    TEST_RUNNER_DEMO_USER="$DEMO_USER" \
    TEST_RUNNER_DEMO_HOST="$DEMO_HOST" \
    NSUnbufferedIO=YES \
    xcodebuild test \
    -project apps/ios/FarCooler.xcodeproj \
    -scheme FarCooler \
    -destination "platform=iOS Simulator,name=$SIMULATOR" \
    ${ONLY[@]+"${ONLY[@]}"} 2>&1 | tee "$LOG"
STATUS=${PIPESTATUS[0]}
set -e

# xcodebuild prints its summary once per suite and once for the run, so the
# largest count is the run's. `tests` is singular when there is one of them, and
# the skip clause is absent from the line entirely when nothing skipped — hence
# two patterns rather than one, and a default of 0 for the skips.
#
# `|| true` on both, and it is load-bearing under `set -e`: a grep that matches
# nothing exits 1, and a command substitution IS the assignment's exit status,
# so without it the script dies here — silently, before printing anything, on
# the one run where every test passed and nothing skipped. Which is to say the
# guard against a suite that cannot fail had, briefly, a way to not run at all.
EXECUTED=$(grep -oE 'Executed [0-9]+ tests?' "$LOG" | grep -oE '[0-9]+' | sort -n | tail -1 || true)
SKIPPED=$(grep -oE 'with [0-9]+ tests? skipped' "$LOG" | grep -oE '[0-9]+' | sort -n | tail -1 || true)
EXECUTED=${EXECUTED:-0}
SKIPPED=${SKIPPED:-0}
RAN=$((EXECUTED - SKIPPED))

echo
echo "─────────────────────────────────────────────"
echo "  $EXECUTED collected, $SKIPPED skipped, $RAN actually ran"
echo "─────────────────────────────────────────────"

if [ "$STATUS" -ne 0 ]; then
    echo "FAILED: xcodebuild reported a failure."
    exit "$STATUS"
fi

# The guard. Everything above may have printed ** TEST SUCCEEDED ** already.
if [ "$RAN" -eq 0 ]; then
    echo
    echo "FAILED: not one test executed — $SKIPPED of $EXECUTED skipped."
    echo
    echo "xcodebuild called that a success. It is not one: a suite where every"
    echo "test skips proves nothing about the app, and reads exactly like a"
    echo "suite where everything passed."
    echo
    echo "Usually the runner is not up or not reachable. Start it with"
    echo "  ./scripts/demo-host.sh"
    echo "and check that $DEMO_USER is the account its sshd authenticates."
    exit 1
fi

echo "OK: $RAN test(s) executed."
