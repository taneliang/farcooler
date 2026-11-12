#!/usr/bin/env bash
# Prove scripts/wait-for-unlock waits while the (faked) lock state says locked,
# says so, goes on once it clears, and never touches the real lock (ov-1089).
# Run on a wait-for-unlock that doesn't loop, it fails.
set -euo pipefail
cd "$(dirname "$0")/.."
scratch="$(mktemp -d)"; trap 'rm -rf "$scratch"' EXIT
export FARCOOLER_LOCK_POLL_SECONDS=1

# Locked for the first two reads, then unlocked.
echo 2 >"$scratch/left"
export FARCOOLER_LOCK_STATE_CMD="n=\$(cat $scratch/left); if [ \$n -gt 0 ]; then echo \$((n-1)) >$scratch/left; echo true; else echo false; fi"
out="$(scripts/wait-for-unlock)"
grep -q "the screen is locked" <<<"$out" || { echo "FAIL: did not say it was waiting: $out"; exit 1; }
grep -q "the screen is unlocked" <<<"$out" || { echo "FAIL: did not go on: $out"; exit 1; }
[ "$(cat "$scratch/left")" = 0 ] || { echo "FAIL: did not poll until unlocked"; exit 1; }

# Unlocked from the start: silent and immediate.
export FARCOOLER_LOCK_STATE_CMD="echo false"
out="$(scripts/wait-for-unlock)"; [ -z "$out" ] || { echo "FAIL: spoke while unlocked: $out"; exit 1; }

# Locked for good: gives up at the limit with exit 3.
export FARCOOLER_LOCK_STATE_CMD="echo true" FARCOOLER_LOCK_WAIT_LIMIT_SECONDS=2
rc=0; scripts/wait-for-unlock >/dev/null 2>&1 || rc=$?
[ "$rc" = 3 ] || { echo "FAIL: expected exit 3, got $rc"; exit 1; }

# The script only reads the lock state.
if grep -Eq 'pmset|caffeinate|loginwindow|osascript|CGSession( |$)|defaults write' scripts/wait-for-unlock; then
  echo "FAIL: wait-for-unlock touches the lock or the display"; exit 1
fi
echo "wait-for-unlock-test: PASS"
