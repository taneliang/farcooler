#!/bin/bash
# Prove apps/macos/test.sh builds the Rust cores and every binary (the CLI and
# farcoolerd, ov-288) before it runs the Mac tests
# (ov-134), runs no tests when they don't build, and runs them under the tmux
# leak check, with a private TMUX_TMPDIR.
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
cp scripts/tmux-leak-check.py scripts/swift-test-guard.sh "$scratch/scripts/"
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
echo "swift $* ${TMUX_TMPDIR:+(private tmux)}" >>"$LOG"
echo "Test run with 2 tests in 1 suite passed after 1 second."
STUB
chmod +x "$scratch/apps/macos/build-vt.sh" "$scratch/bin/swift" "$scratch/home/.cargo/bin/cargo" "$scratch/apps/macos/test.sh"

fail() { echo "FAIL: $1" >&2; exit 1; }

export LOG="$scratch/log"
export PATH="$scratch/bin:$PATH"
export HOME="$scratch/home"

: >"$LOG"
env -u TMUX_TMPDIR "$scratch/apps/macos/test.sh" --filter CeremonyTests -j 3 >/dev/null
[ "$(cat "$LOG")" = $'build-vt\ncargo build --bins\nswift test --filter CeremonyTests -j 3 (private tmux)' ] \
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
