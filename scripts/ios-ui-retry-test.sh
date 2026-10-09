#!/bin/bash
# Prove scripts/ios-ui-tests.sh retries a run once, and only once, when it
# failed on iOS infrastructure (ov-449): an app that would not launch, a test
# runner that crashed while bootstrapping, a simulator that would not boot.
#
# It runs a copy of the script beside a stand-in `xcodebuild` that prints the
# next canned log from a queue and counts its calls. Asserted: an infrastructure
# log is retried once and a green second attempt is green; an assertion failure
# is not retried; an assertion failure beside a launch failure is not retried;
# a second infrastructure failure is red; and with IOS_UI_RETRY_INFRA unset
# nothing retries. Run on a mutated script (the signature check removed, or the
# retry made unconditional) it fails.
set -euo pipefail

cd "$(dirname "$0")/.."

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/scripts" "$scratch/bin"
cp scripts/ios-ui-tests.sh "$scratch/scripts/"
touch "$scratch/x.xctestrun"

# Attempt N prints $scratch/log.N, exiting with the status in $scratch/status.N.
cat >"$scratch/bin/xcodebuild" <<'STUB'
#!/bin/bash
n=$(($(cat "$SCRATCH/calls" 2>/dev/null || echo 0) + 1))
echo "$n" >"$SCRATCH/calls"
cat "$SCRATCH/log.$n"
exit "$(cat "$SCRATCH/status.$n")"
STUB
chmod +x "$scratch/bin/xcodebuild"

PASS="Test Suite 'All tests' passed at 2026-10-09.
	 Executed 8 tests, with 0 failures (0 unexpected) in 4.0 (4.0) seconds
** TEST SUCCEEDED **"
LAUNCH="/x/Tests.swift:10: error: -[FarCoolerUITests.A testA] : Failed to launch <XCUIApplicationImpl: 0x1>
Test Suite 'All tests' failed at 2026-10-09.
	 Executed 8 tests, with 1 failure (0 unexpected) in 4.0 (4.0) seconds
** TEST FAILED **"
BOOTSTRAP="Early unexpected exit, operation never finished bootstrapping - no restart will be attempted
** TEST FAILED **"
ASSERT="/x/Tests.swift:20: error: -[FarCoolerUITests.B testB] : XCTAssertTrue failed - the pane never drew
Test Suite 'All tests' failed at 2026-10-09.
	 Executed 8 tests, with 1 failure (0 unexpected) in 4.0 (4.0) seconds
** TEST FAILED **"
MIXED="$LAUNCH
$ASSERT"

fail() { echo "FAIL: $1" >&2; exit 1; }

# run <retry-env> <log1> <status1> [<log2> <status2>]; sets CODE, OUT, CALLS, SUMMARY.
run() {
    local retry="$1"; shift
    rm -f "$scratch"/calls "$scratch"/log.* "$scratch"/status.* "$scratch/summary"
    local n=1
    while [ $# -gt 0 ]; do
        printf '%s\n' "$1" >"$scratch/log.$n"; echo "$2" >"$scratch/status.$n"; shift 2; n=$((n + 1))
    done
    CODE=0
    OUT="$(env SCRATCH="$scratch" PATH="$scratch/bin:$PATH" XCTESTRUN="$scratch/x.xctestrun" \
        DEMO_USER=u DEMO_HOST=h GITHUB_STEP_SUMMARY="$scratch/summary" DIAGNOSE_GRACE=1000 \
        IOS_UI_RETRY_INFRA="$retry" \
        "$scratch/scripts/ios-ui-tests.sh" 2>&1)" || CODE=$?
    CALLS="$(cat "$scratch/calls")"
    SUMMARY="$(cat "$scratch/summary" 2>/dev/null || true)"
}

for infra in "$LAUNCH" "$BOOTSTRAP"; do
    run 1 "$infra" 65 "$PASS" 0
    [ "$CODE" -eq 0 ] || fail "an infrastructure failure then a pass was not green: $OUT"
    [ "$CALLS" -eq 2 ] || fail "an infrastructure failure ran xcodebuild $CALLS time(s), wanted 2"
    grep -q '::warning' <<<"$OUT" || fail "the retry left no warning annotation"
    grep -q 'RETRYING' <<<"$OUT" || fail "the retry was silent in the log"
    grep -q 'retried once' <<<"$SUMMARY" || fail "the retry is not in the job summary"
done

run 1 "$ASSERT" 65 "$PASS" 0
[ "$CALLS" -eq 1 ] || fail "an assertion failure was retried ($CALLS calls)"
[ "$CODE" -eq 65 ] || fail "an assertion failure exited $CODE, wanted 65"
[ -z "$SUMMARY" ] || fail "an assertion failure wrote a retry summary"

run 1 "$MIXED" 65 "$PASS" 0
[ "$CALLS" -eq 1 ] || fail "an assertion failure beside a launch failure was retried ($CALLS calls)"
[ "$CODE" -eq 65 ] || fail "a mixed failure exited $CODE, wanted 65"

run 1 "$LAUNCH" 65 "$BOOTSTRAP" 65 "$PASS" 0
[ "$CALLS" -eq 2 ] || fail "a second infrastructure failure ran xcodebuild $CALLS time(s), wanted 2"
[ "$CODE" -eq 65 ] || fail "a second infrastructure failure exited $CODE, wanted 65"

run "" "$LAUNCH" 65 "$PASS" 0
[ "$CALLS" -eq 1 ] || fail "retried with IOS_UI_RETRY_INFRA unset ($CALLS calls)"
[ "$CODE" -eq 65 ] || fail "unset exited $CODE, wanted 65"

echo "ok: ios-ui-tests.sh retries once on an infrastructure signature, never on an assertion, and goes red when the retry fails too"
