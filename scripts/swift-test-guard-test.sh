#!/bin/bash
# Prove apps/macos/test.sh fails a run that ends early or counts too few tests (ov-324).
#
# It runs a copy of test.sh beside a stand-in `swift` that plays three runs:
# one that exits 0 mid-run with no summary line (the integ-14 NSMenu case), a
# full one, and one that finishes but ran fewer tests than the floor. Run on a
# test.sh without the guard, the first and third cases pass and this fails.
set -euo pipefail

cd "$(dirname "$0")/.."

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/apps/macos" "$scratch/bin" "$scratch/scripts" "$scratch/home/.cargo/bin"
cp apps/macos/test.sh "$scratch/apps/macos/test.sh"
cp scripts/tmux-leak-check.py scripts/test-leak-check.py scripts/swift-test-guard.sh "$scratch/scripts/"
echo 100 >"$scratch/apps/macos/test-floor.txt"
printf '#!/bin/bash\nexit 0\n' | tee "$scratch/apps/macos/build-vt.sh" >"$scratch/home/.cargo/bin/cargo"
chmod +x "$scratch/apps/macos/build-vt.sh" "$scratch/home/.cargo/bin/cargo" "$scratch/apps/macos/test.sh"

cat >"$scratch/bin/swift" <<'STUB'
#!/bin/bash
echo "Test run started."
case "$MODE" in
  early) echo "Test case 'MenuTests.opens' started."; exit 0 ;;
  full) echo "✔ Test run with 150 tests in 20 suites passed after 3.1 seconds." ;;
  few) echo "✔ Test run with 99 tests in 20 suites passed after 3.1 seconds." ;;
  failed) echo "✘ Test run with 150 tests in 20 suites failed after 3.1 seconds."; exit 1 ;;
esac
STUB
chmod +x "$scratch/bin/swift"
# The power log as pmset prints it, from $PMSET_LOG (ov-742).
printf '#!/bin/bash\nprintf "%%b" "$PMSET_LOG"\n' >"$scratch/bin/pmset"
chmod +x "$scratch/bin/pmset"

fail() { echo "FAIL: $1" >&2; exit 1; }
export PATH="$scratch/bin:$PATH" HOME="$scratch/home"
run() { MODE="$1" "$scratch/apps/macos/test.sh" 2>&1; }

if out="$(run early)"; then fail "a run that exited 0 mid-run passed"; fi
grep -q "never printed" <<<"$out" || fail "the early exit's message is missing: $out"

out="$(run full)" || fail "a full run failed: $out"
grep -q "150 tests ran" <<<"$out" || fail "the count was not reported: $out"

if out="$(run few)"; then fail "a run below the floor passed"; fi
grep -q "below the floor of 100" <<<"$out" || fail "the floor's message is missing: $out"
grep -q "raise it" <<<"$out" || fail "the message does not say how to raise the floor: $out"

if run failed >/dev/null; then fail "a failing run passed"; fi

# The display's state, from the power log: off when the run began, or
# turned off during it, is said; on throughout is not (ov-742).
line() { printf '%s -0700 Notification        \\tDisplay is turned %s                    \\n' "$1" "$2"; }
export PMSET_LOG
PMSET_LOG="$(line '2000-01-01 09:00:00' on)$(line '2000-01-01 10:00:00' off)"
out="$(run failed)" || true
grep -q "the display was off" <<<"$out" || fail "a run begun with the display off didn't say so: $out"
PMSET_LOG="$(line '2000-01-01 10:00:00' off)$(line '2000-01-01 11:00:00' on)"
out="$(run full)" || fail "a full run failed: $out"
if grep -q "the display was off" <<<"$out"; then fail "a run with the display on said it was off: $out"; fi
PMSET_LOG="$(line '2000-01-01 11:00:00' on)$(line '2999-01-01 10:00:00' off)"
out="$(run full)" || fail "a full run failed: $out"
grep -q "the display was off" <<<"$out" || fail "a run the display went off during didn't say so: $out"
PMSET_LOG="$(line '2000-01-01 10:00:00' off)$(line '2000-01-01 11:00:00' on)$(line '2999-01-01 10:00:00' on)"
out="$(run full)" || fail "a full run failed: $out"
if grep -q "the display was off" <<<"$out"; then fail "a display turned on during the run said it was off: $out"; fi
PMSET_LOG=""
out="$(run full)" || fail "a full run failed: $out"
if grep -q "the display was off" <<<"$out"; then fail "an empty power log said the display was off: $out"; fi

echo "ok: test.sh fails a run that ends early or counts fewer tests than the floor, passes a full one, and says when the display was off"
