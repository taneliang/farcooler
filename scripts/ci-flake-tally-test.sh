#!/bin/bash
# That ci-flake-tally.py reads the logs CI really produces (ov-697).
#
# The fixtures under scripts/fixtures/ci-flake-tally/ are trimmed from real
# failing runs on main (Oct 22): a crates.io DNS error (39778945810 attempt 1),
# the answer_wake nextest failure (39778945810 attempt 2), the AgentKit row
# budget and the Mac flick test (39773526744 attempts 1 and 2), the route_tests
# flake (39733277521) and the LEAKED farcoolerd (39720153339). Only the
# synthetic-* ones are written by hand: no failing XCTest or Gradle log has
# been kept, so they follow those tools' documented output.
#
# A parser that stops matching does not fail a build, it just finds no tests,
# and the tally then calls every red job "unclassified"; so each format is
# pinned to the exact name it must yield.
#
#   ./scripts/ci-flake-tally-test.sh
set -euo pipefail

cd "$(dirname "$0")"
FX=fixtures/ci-flake-tally
TALLY=./ci-flake-tally.sh
PASS=0
FAIL=0

check() {
  local what="$1" want="$2" got="$3"
  if [ "$want" = "$got" ]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    echo "FAIL: $what" >&2
    echo "  want: $want" >&2
    echo "  got:  $got" >&2
  fi
}

tab=$'\t'
check "nextest FAIL line names crate and test, once" \
  "test${tab}nextest${tab}farcooler-daemon watch::answer_wake::tests::compose_codex_tests::the_screen_is_read_after_the_rollout_check" \
  "$($TALLY --parse-log $FX/nextest-answer-wake.log)"
check "nextest ignores PASS lines whose names say error or leak" \
  "test${tab}nextest${tab}farcooler-daemon watch::answer_wake::tests::route_tests::a_draft_is_pasted_never_entered" \
  "$($TALLY --parse-log $FX/nextest-route-tests.log)"
check "swift-testing reads the failed test, not its suite or the run" \
  "test${tab}swift-testing${tab}An update costs the main thread 2 ms or less with 50,000 rows held" \
  "$($TALLY --parse-log $FX/swift-testing-row-budget.log)"
check "swift-testing counts a test once across its issues" \
  "test${tab}swift-testing${tab}Flicking up, no row on screen moves except by the scroll" \
  "$($TALLY --parse-log $FX/swift-testing-flick.log)"
check "XCTest reads the Failing tests list and the failed cases, once each" \
  "test${tab}XCTest${tab}BoardTests.testColumnCount
test${tab}XCTest${tab}PairingTests.testScanRetry" \
  "$($TALLY --parse-log $FX/synthetic-xctest.log)"
check "Gradle reads a FAILED test line, not a FAILED task" \
  "test${tab}Gradle${tab}com.farcooler.app.PairingViewModelTest > scan retries after a timeout" \
  "$($TALLY --parse-log $FX/synthetic-gradle.log)"
check "a leak check's LEAKED is a failure named without its pid" \
  "test${tab}leak${tab}LEAKED farcoolerd" \
  "$($TALLY --parse-log $FX/leaked-farcoolerd.log)"
leaks="$(mktemp)"
trap 'rm -f "$leaks"' EXIT
printf '%s\n' \
  "Rust (ubuntu-24.04)${tab}UNKNOWN STEP${tab}2026-10-22T06:58:38.2496034Z tmux-leak-check: LEAKED tmux server 5895 (socket under /tmp/fc-leak-ebl_eqik)" > "$leaks"
check "a leaked tmux server is named without its pid or socket" \
  "test${tab}leak${tab}LEAKED tmux server" \
  "$($TALLY --parse-log "$leaks")"
check "a DNS error with no test is infrastructure" \
  "infra${tab}DNS error" \
  "$($TALLY --parse-log $FX/dns-crates-io.log)"
check "a runner never acquired is infrastructure (annotation)" \
  "infra${tab}not acquired by Runner" \
  "$($TALLY --parse-log /dev/null --annotations $FX/annotation-not-acquired.txt)"
check "a cancelled job is infrastructure" \
  "infra${tab}cancelled" \
  "$($TALLY --parse-log /dev/null --conclusion cancelled)"
printf '%s\n' \
  "Rust (ubuntu-24.04)${tab}Run cargo clippy${tab}2026-10-22T02:35:55.3976261Z ^[[1m^[[91merror^[[0m: could not compile \`farcooler-daemon\` (lib test) due to 1 previous error" > "$leaks"
check "a compile error is a build failure, not a test or the runner" \
  "build${tab}build or lint error" \
  "$($TALLY --parse-log "$leaks")"
check "a failure with nothing to read is not dropped" \
  "other${tab}no failing test found in the log" \
  "$($TALLY --parse-log /dev/null)"

report="$($TALLY --report $FX/manifest.tsv)"
first="$(printf '%s\n' "$report" | grep -m1 -E '^ +1\.')"
check "the test that failed twice ranks first, with its count" \
  "  1. 2x  farcooler-daemon watch::answer_wake::tests::compose_codex_tests::the_screen_is_read_after_the_rollout_check  [nextest]" \
  "$first"
check "the DNS failure sits under infrastructure, not in the test ranking" \
  "1" "$(printf '%s\n' "$report" | sed -n '/^Infrastructure/,$p' | grep -c '1x  DNS error')"
check "a test is never listed above the Infrastructure heading as DNS" \
  "0" "$(printf '%s\n' "$report" | sed -n '1,/^Infrastructure/p' | grep -c 'DNS')"
check "the leak is ranked as a test" \
  "1" "$(printf '%s\n' "$report" | grep -c 'LEAKED farcoolerd')"
check "a failure on a run that went green is marked" \
  "1" "$(printf '%s\n' "$report" | grep -c 'run later went green')"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
