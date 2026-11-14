#!/bin/bash
# Prove apps/macos/test.sh builds the Rust cores and every binary (the CLI and
# farcoolerd, ov-288) before it runs the Mac tests
# (ov-134), runs no tests when they don't build, and runs them under the tmux
# leak check, with a private TMUX_TMPDIR, and under the temp-file leak check
# (ov-541), with a private TMPDIR.
#
# It runs a copy of test.sh beside a stand-in build-vt.sh and a stand-in
# `swift`, each of which appends to one log, so the order is what is asserted.
# Run on a mutated test.sh (the build-vt.sh or cargo build --bins line removed)
# it fails.
set -euo pipefail

cd "$(dirname "$0")/.."
real="$PWD/apps/macos/test.sh"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/apps/macos" "$scratch/bin" "$scratch/scripts"
mkdir -p "$scratch/home/.cargo/bin"
cp "$real" "$scratch/apps/macos/test.sh"
cp scripts/tmux-leak-check.py scripts/test-leak-check.py scripts/swift-test-guard.sh "$scratch/scripts/"
echo 1 >"$scratch/apps/macos/test-floor.txt"

cat >"$scratch/apps/macos/build-vt.sh" <<'STUB'
#!/bin/bash
echo "build-vt" >>"$LOG"
[ "${BUILD_FAILS:-}" = 1 ] && { echo "error: the vt crate does not compile"; exit 1; }
exit 0
STUB
# A stand-in cargo, found the way test.sh finds the real one: off $HOME.
cat >"$scratch/home/.cargo/bin/cargo" <<'STUB'
#!/bin/bash
echo "cargo $*" >>"$LOG"
[ "${BINS_FAIL:-}" = 1 ] && { echo "error: farcoolerd does not compile"; exit 1; }
exit 0
STUB
cat >"$scratch/bin/swift" <<'STUB'
#!/bin/bash
case "${TMPDIR:-}" in /tmp/fcl-*/) tmp="(private tmp)" ;; *) tmp="" ;; esac
echo "swift $* ${TMUX_TMPDIR:+(private tmux)} $tmp" >>"$LOG"
echo "Test run with 2 tests in 1 suite passed after 1 second."
STUB
chmod +x "$scratch/apps/macos/build-vt.sh" "$scratch/bin/swift" "$scratch/home/.cargo/bin/cargo" "$scratch/apps/macos/test.sh"

cat >"$scratch/scripts/heavy" <<'STUB'
#!/bin/bash
[ "$1" = -- ] && shift
exec "$@"
STUB
cat >"$scratch/scripts/macos-test-partitions.py" <<'STUB'
#!/bin/bash
echo "partitions $*" >>"$LOG"
STUB
chmod +x "$scratch/scripts/heavy" "$scratch/scripts/macos-test-partitions.py"

fail() { echo "FAIL: $1" >&2; exit 1; }

export LOG="$scratch/log"
export PATH="$scratch/bin:$PATH"
export HOME="$scratch/home"

: >"$LOG"
env -u TMUX_TMPDIR "$scratch/apps/macos/test.sh" --filter CeremonyTests -j 3 >/dev/null
[ "$(cat "$LOG")" = $'build-vt\ncargo build --bins\nswift test --filter CeremonyTests -j 3 (private tmux) (private tmp)' ] \
  || fail "expected build-vt, cargo build --bins, then swift with its arguments under the leak check; got: $(tr '\n' '|' <"$LOG")"

: >"$LOG"
if out="$(BUILD_FAILS=1 "$scratch/apps/macos/test.sh" 2>&1)"; then fail "a failed build still exited 0"; fi
[ "$(cat "$LOG")" = "build-vt" ] || fail "swift ran after a failed build: $(tr '\n' '|' <"$LOG")"
grep -q "the vt crate does not compile" <<<"$out" || fail "the build's own error was swallowed: $out"

: >"$LOG"
if out="$(BINS_FAIL=1 "$scratch/apps/macos/test.sh" 2>&1)"; then fail "a failed binaries build still exited 0"; fi
[ "$(cat "$LOG")" = $'build-vt\ncargo build --bins' ] || fail "swift ran after the binaries failed: $(tr '\n' '|' <"$LOG")"
grep -q "farcoolerd does not compile" <<<"$out" || fail "the binaries' own error was swallowed: $out"

echo "ok: test.sh builds the Rust cores and every binary first, passes its arguments on under the tmux leak check, and stops when they fail"

: >"$LOG"
FARCOOLER_MAC_TEST_PROCESSES=2 "$scratch/apps/macos/test.sh" >/dev/null
head -3 "$LOG" | diff -u <(printf '%s\n' build-vt 'cargo build --bins' 'swift build --build-tests  ') - \
  || fail "parallel mode must build cores, binaries and test bundles once"
grep -q '^partitions --repeat 1 --out ../../target/macos-test-partitions-' "$LOG" \
  || fail "parallel mode did not run the complete inventory in the worktree target"
if FARCOOLER_MAC_TEST_PROCESSES=2 "$scratch/apps/macos/test.sh" --filter CeremonyTests >/dev/null 2>&1; then
  fail "parallel mode silently ignored a filter"
fi
echo "ok: parallel mode builds once, runs the guarded complete inventory, and refuses filters"
