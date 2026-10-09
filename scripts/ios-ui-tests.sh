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
#
# SIMULATOR IS A NAME, NOT A RUNTIME. With no OS named, xcodebuild fills in
# `OS:latest`, so `SIMULATOR="iPhone 17"` on a Mac with both the 26.5 and 27.0
# runtimes installed runs on 27.0 — quietly, and the log only says so in the
# destination line nobody reads. A run meant to prove the 26 path (the app's
# deployment target) that does not name the OS has proved the newest one.
# Name it with SIMULATOR_OS (not a bare OS, which other tools set):
#
#   SIMULATOR_OS=26.5 ./scripts/ios-ui-tests.sh FarCoolerUITests/ShellGestureTests
#
# Unset, the destination is exactly what it always was.
#
# SIMULATOR_ID names one device by UDID instead, and wins over both. CI uses it
# because a runner image's simulator names change without warning, so it picks
# an iPhone that exists rather than naming one that might not.
#
# SKIP_TESTING is a space-separated list passed to -skip-testing, for a method
# inside a class that is otherwise runnable — CI runs the runnerless classes
# and has to leave out the one live test some of them carry.
#
# XCTESTRUN names a `.xctestrun` file from an earlier `build-for-testing`, and
# then the script runs `test-without-building` against those products instead
# of building the scheme. CI builds the app and its UI test bundle once, in the
# `ios` job, and hands the products to every shard (ov-301); each shard used to
# spend about five minutes compiling the same thing. Unset, the script builds
# exactly as it always did.
set -euo pipefail

cd "$(dirname "$0")/.."

SIMULATOR="${SIMULATOR:-iPhone 17}"
SIMULATOR_OS="${SIMULATOR_OS:-}"
SIMULATOR_ID="${SIMULATOR_ID:-}"
DEMO_HOST="${DEMO_HOST:-127.0.0.1:2222}"
DEMO_USER="${DEMO_USER:-$(whoami)}"
XCTESTRUN="${XCTESTRUN:-}"

if [ -n "$XCTESTRUN" ]; then
    [ -f "$XCTESTRUN" ] || { echo "ios-ui-tests: XCTESTRUN names $XCTESTRUN, which does not exist" >&2; exit 1; }
    ACTION=(test-without-building -xctestrun "$XCTESTRUN")
else
    # The project file is generated and not tracked (ov-338), so a checkout
    # that has never generated one, or generated one before the last source
    # was added, would otherwise build a project that is missing or stale.
    ./apps/ios/generate-project.py >/dev/null
    ACTION=(test -project apps/ios/FarCooler.xcodeproj -scheme FarCooler)
fi

# Expanded below as `${ONLY[@]+"${ONLY[@]}"}` rather than `"${ONLY[@]}"`,
# because macOS ships bash 3.2, where an empty array is an unbound variable
# under `set -u` — the plain form kills the script before it reaches xcodebuild
# in exactly the case where no argument was given, which is the whole suite.
ONLY=()
for target in "$@"; do
    ONLY+=("-only-testing:$target")
done
for target in ${SKIP_TESTING:-}; do
    ONLY+=("-skip-testing:$target")
done

if [ -n "$SIMULATOR_ID" ]; then
    DESTINATION="platform=iOS Simulator,id=$SIMULATOR_ID"
else
    DESTINATION="platform=iOS Simulator,name=$SIMULATOR${SIMULATOR_OS:+,OS=$SIMULATOR_OS}"
fi

# Said out loud, because an empty user here is the whole first defect and it is
# invisible in xcodebuild's output — it surfaces four screens away as an app
# that will not connect.
echo "runner:    $DEMO_USER@$DEMO_HOST"
[ -z "$XCTESTRUN" ] || echo "products:  $XCTESTRUN (test-without-building)"
if [ -n "$SIMULATOR_ID" ]; then
    echo "simulator: $SIMULATOR_ID"
else
    echo "simulator: $SIMULATOR (OS: ${SIMULATOR_OS:-latest installed})"
fi
echo

LOG="$(mktemp -t ios-ui-tests)"
WATCHDOG=""
trap 'rm -f "$LOG"; [ -z "$WATCHDOG" ] || kill "$WATCHDOG" 2>/dev/null; true' EXIT

# THE WAIT AFTER THE LAST TEST (ov-13). Once the last test has finished, and by
# default only when something failed, xcodebuild runs
# `simctl diagnose --timeout=600` as its own child and waits for it before it
# prints `** TEST … **`. That normally takes 10 to 15 seconds. Sometimes one of
# its steps never gets a reply from the simulator, and xcodebuild then sits at
# 0% CPU for the full 600 seconds with every result already printed and no
# verdict. It reads exactly like a hang, and anything that wraps this script in
# a timeout shorter than ten minutes after the last test kills it first and
# loses the verdict.
#
# `-collect-test-diagnostics never` below turns that step off; measured on a
# failing run, no `simctl diagnose` starts and the verdict follows the last
# suite line within a second. Nothing here reads those archives.
#
# The watchdog is the backstop, because diagnose has also been seen after runs
# where every test passed, for a reason nobody has reproduced, so the flag is
# not proven to cover every way in. Once the run's top-level suite has reported
# and DIAGNOSE_GRACE seconds (default 120) have passed, it kills a
# `simctl diagnose` whose parent is THIS script's xcodebuild, by pid, and
# nothing else, with SIGKILL so no handler in simctl can hold it up (xcodebuild
# leaves the child running when its own timeout fires, so nothing waits on a
# clean exit). xcodebuild takes that as the end of the step and prints its real
# verdict and exit status at once. So at worst the verdict comes DIAGNOSE_GRACE
# plus 5 seconds after the last test; a caller's timeout needs that much room
# past the tests, not ten minutes.
DIAGNOSE_GRACE="${DIAGNOSE_GRACE:-120}"
(
    armed=""
    while kill -0 $$ 2>/dev/null; do
        if [ -z "$armed" ]; then
            if grep -qE "^Test Suite '(All tests|Selected tests)' (passed|failed)" "$LOG"; then
                armed=$SECONDS
            fi
        elif [ $((SECONDS - armed)) -ge "$DIAGNOSE_GRACE" ]; then
            for xcb in $(pgrep -P $$ -x xcodebuild); do
                for diag in $(pgrep -P "$xcb" -f 'simctl diagnose'); do
                    echo "ios-ui-tests: xcodebuild's simctl diagnose (pid $diag) is still running ${DIAGNOSE_GRACE}s after the last test; killing it so the verdict can print" >&2
                    kill -KILL "$diag" 2>/dev/null
                done
            done
            exit 0
        fi
        sleep 5
    done
) &
WATCHDOG=$!

# The assignments go BEFORE xcodebuild. See the note at the top: this position
# is not a style choice, it is the difference between the suite running and the
# suite skipping itself into a green.
# ONE RETRY, FOR INFRASTRUCTURE ONLY (ov-449). With IOS_UI_RETRY_INFRA=1 (CI
# sets it; a local run does not) a failed run is run once more when its output
# carries a known infrastructure signature and no failure of any other kind.
# Three of four main runs on 2026-10-09 needed a manual shard rerun for
# "Failed to launch <XCUIApplicationImpl…>" or "Early unexpected exit, operation
# never finished bootstrapping", about 25 to 35 minutes each.
#
# THE WHOLE SHARD, not only the failed classes. A runner crash kills the run
# mid-flight, so the classes that never started have no result at all and a
# "retry what failed" list would silently drop them: a green that skipped half
# a shard, the failure mode this script exists to prevent. A shard is the unit
# CI already schedules, and the retry costs one shard, not the whole suite.
#
# NEVER AN ASSERTION. Every `error:` line a test reports must match a signature;
# one that does not (an XCTAssert, a timeout, a crash in the app) means a real
# failure, and it is not retried however many launch errors sit beside it. The
# second attempt is final: a second infrastructure failure goes red.
INFRA_SIGNATURES='Failed to launch|Early unexpected exit|never finished bootstrapping|Unable to boot|Failed to boot|Simulator device failed to boot'

# 0 (true) when the log shows an infrastructure failure and nothing else failed.
infra_only_failure() {
    grep -qE "$INFRA_SIGNATURES" "$1" || return 1
    # Test failures are `file:line: error: -[Class test] : message`.
    ! grep -E ': error: ' "$1" | grep -vE "$INFRA_SIGNATURES" | grep -q .
}

run_xcodebuild() {
    env \
        TEST_RUNNER_DEMO_USER="$DEMO_USER" \
        TEST_RUNNER_DEMO_HOST="$DEMO_HOST" \
        NSUnbufferedIO=YES \
        xcodebuild "${ACTION[@]}" \
        -destination "$DESTINATION" \
        -collect-test-diagnostics never \
        ${ONLY[@]+"${ONLY[@]}"} 2>&1 | tee "$LOG"
    STATUS=${PIPESTATUS[0]}
}

# Errexit stays off until the watchdog is reaped below: `wait` on a killed job
# returns 143.
set +e
run_xcodebuild
if [ "$STATUS" -ne 0 ] && [ "${IOS_UI_RETRY_INFRA:-}" = 1 ] && infra_only_failure "$LOG"; then
    SIGNATURE=$(grep -E "$INFRA_SIGNATURES" "$LOG" | head -1 | cut -c1-200)
    echo
    echo "::warning title=iOS UI shard retried::Infrastructure failure, not a test failure: $SIGNATURE"
    echo "ios-ui-tests: RETRYING THE WHOLE RUN ONCE. Infrastructure failure (exit $STATUS): $SIGNATURE"
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        {
            echo "### iOS UI shard retried once"
            echo
            echo "The first attempt failed on infrastructure, not on a test (exit $STATUS):"
            echo
            echo '```'
            echo "$SIGNATURE"
            echo '```'
            echo
            echo "The whole shard ran again. A second failure of any kind fails the job."
        } >>"$GITHUB_STEP_SUMMARY"
    fi
    echo
    : >"$LOG"
    run_xcodebuild
fi
# The wait is what keeps bash from printing the whole watchdog as a
# "Terminated" job notice into the output.
kill "$WATCHDOG" 2>/dev/null
wait "$WATCHDOG" 2>/dev/null
WATCHDOG=""
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

# The second guard. A live-runner test that could not find the runner skips
# with `LiveRunner.missing` ("NO LIVE RUNNER:") at the front of its message, and
# that is never a pass: the harness suites still ran, so the count above is
# satisfied, and the whole live half of the suite — scrollback, streaming, an
# ask answered from the phone — would otherwise be reported green while
# testing nothing (ov-55 4A review). Other skips (hardware this Mac lacks, a
# runner without a capability) stay allowed.
MISSING=$(grep -E 'Test skipped.*NO LIVE RUNNER:' "$LOG" | sort -u || true)
if [ -n "$MISSING" ]; then
    echo
    echo "FAILED: live-runner tests skipped because they could not reach the runner:"
    echo "$MISSING" | sed -E 's/^.*-\[([^]]*)\].*$/  \1/'
    echo
    echo "Start it with ./scripts/demo-host.sh (DEMO_PORT and DEMO_UDID pick a"
    echo "port and a simulator), then run this again with DEMO_HOST to match."
    exit 1
fi

echo "OK: $RAN test(s) executed."
