#!/bin/bash
# Prove scripts/ios-ui-tests.sh leases a simulator per run (ov-679): two runs
# started together never share a device, a killed holder's lease is freed, a
# run waits (and times out with a message) when every booted device is in use,
# no new device is booted by default, and two runs with one SIMULATOR_ID never overlap, and CI=true takes no lease.
#
# A copy of the script runs beside stand-in `xcrun` and `xcodebuild` on PATH, as
# ios-ui-retry-test.sh does; no real simulator is touched. The stand-in
# xcodebuild logs `start <udid>` / `end <udid>` and holds the device until
# the test touches its release file, so an overlap on one device is visible
# in the log.
set -euo pipefail

cd "$(dirname "$0")/.."

scratch="$(mktemp -d)"
KILL=""
trap '[ -z "$KILL" ] || kill $KILL 2>/dev/null || true; rm -rf "$scratch"' EXIT
mkdir -p "$scratch/scripts" "$scratch/bin"
cp scripts/ios-ui-tests.sh "$scratch/scripts/"
touch "$scratch/x.xctestrun"

cat >"$scratch/bin/xcodebuild" <<'STUB'
#!/bin/bash
id="$(grep -oE 'id=[^ ]+' <<<"$*" | cut -d= -f2 | head -1)"
echo "start ${id:-none}" >>"$SCRATCH/events"
echo $$ >"$SCRATCH/xcodebuild.pid.${id:-none}"
if [ -n "${RELEASE:-}" ]; then
    # Held until the test touches $RELEASE (the 120s is only a backstop).
    for _ in $(seq 1200); do [ -e "$RELEASE" ] && break; sleep 0.1; done
else
    sleep "${HOLD:-1}"
fi
echo "end ${id:-none}" >>"$SCRATCH/events"
echo "Test Suite 'All tests' passed at 2026-10-21."
echo "	 Executed 8 tests, with 0 failures (0 unexpected) in 4.0 (4.0) seconds"
echo "** TEST SUCCEEDED **"
STUB
cat >"$scratch/bin/xcrun" <<'STUB'
#!/bin/bash
echo "$*" >>"$SCRATCH/xcrun.calls"
if [ "$1 $2" = "simctl list" ]; then cat "$SCRATCH/devices.json"
elif [ "$1 $2" = "simctl clone" ]; then echo CLONE-UDID
fi
exit 0
STUB
chmod +x "$scratch/bin/xcodebuild" "$scratch/bin/xcrun"

# devices <udid:state>...: iPhone 17 devices on one runtime.
devices() {
    local out="" d
    for d in "$@"; do out="$out${out:+,}{\"name\":\"iPhone 17\",\"udid\":\"${d%%:*}\",\"state\":\"${d##*:}\"}"; done
    echo "{\"devices\":{\"com.apple.CoreSimulator.SimRuntime.iOS-27-0\":[$out]}}" >"$scratch/devices.json"
}

fail() { echo "FAIL: $1" >&2; exit 1; }

# Ordering waits on events, never on time: a loaded Mac can stall a run for
# seconds, so each case waits for a file or a line to appear, and the bounds
# here are only backstops.
# until_file <path> / until_grep <pattern> <file>: wait up to 60s.
until_file() { local _; for _ in $(seq 600); do [ -e "$1" ] && return 0; sleep 0.1; done; return 1; }
until_grep() { local _; for _ in $(seq 600); do grep -q "$1" "$2" 2>/dev/null && return 0; sleep 0.1; done; return 1; }
# holding <udid>: the run that took the lease on it has its stand-in started.
holding() { until_file "$scratch/xcodebuild.pid.$1" || fail "no run ever held $1"; }

# launch <outfile> [env...]: one run in the background; its pid in $PID.
launch() {
    local out="$1"; shift
    env -u CI SCRATCH="$scratch" PATH="$scratch/bin:$PATH" XCTESTRUN="$scratch/x.xctestrun" DEMO_USER=u DEMO_HOST=h \
        DIAGNOSE_GRACE=1000 IOS_UI_LEASE_DIR="$scratch/leases" IOS_UI_LEASE_POLL=0.2 "$@" \
        "$scratch/scripts/ios-ui-tests.sh" >"$out" 2>&1 &
    PID=$!
}
reset() { rm -rf "$scratch/release" "$scratch/events" "$scratch/leases" "$scratch/xcrun.calls" "$scratch"/xcodebuild.pid.* "$scratch"/out.*; }

# No two overlapping runs on one device: every `start X` is followed by `end X`
# before the next `start X`.
no_overlap() {
    local ids id
    ids="$(awk '$1=="start"{print $2}' "$scratch/events" | sort -u)"
    for id in $ids; do
        grep -F " $id" "$scratch/events" | awk -v id="$id" '
            $1=="start"{ if (open) bad=1; open=1 } $1=="end"{ open=0 } END{ exit bad }' \
            || fail "two runs held $id at once: $(tr '\n' ';' <"$scratch/events")"
    done
}

# 1. Two runs started together, two booted devices: one each. Three runs: the
# third waits for a release and never shares.
reset; devices AAA:Booted BBB:Booted
# All three hold their device until the third has said it is waiting.
launch "$scratch/out.1" RELEASE="$scratch/release"; p1=$PID
launch "$scratch/out.2" RELEASE="$scratch/release"; p2=$PID
launch "$scratch/out.3" RELEASE="$scratch/release"; p3=$PID
for _ in $(seq 600); do grep -q 'waiting for a free iPhone 17' "$scratch"/out.* 2>/dev/null && break; sleep 0.1; done
touch "$scratch/release"
for p in $p1 $p2 $p3; do wait "$p" || fail "a run did not finish green: $(cat "$scratch"/out.*)"; done
[ "$(grep -c '^start' "$scratch/events")" -eq 3 ] || fail "wanted 3 runs: $(cat "$scratch/events")"
no_overlap
[ "$(sed -n '1,2p' "$scratch/events" | sort -u | wc -l)" -eq 2 ] || fail "the first two runs shared a device: $(cat "$scratch/events")"
grep -q 'waiting for a free iPhone 17' "$scratch"/out.* || fail "the third run did not say it was waiting"
if grep '^simctl boot ' "$scratch/xcrun.calls" | grep -qvE '^simctl boot (AAA|BBB)$'; then
    fail "a device beyond the booted ones was booted"
fi
[ -z "$(ls "$scratch/leases")" ] || fail "leases left behind: $(ls "$scratch/leases")"

# 2. A killed holder's lease is freed. SIGKILL gives the script no chance to
# clean up, so the next run must see the dead pid and take the device.
reset; devices AAA:Booted
launch "$scratch/out.1" HOLD=30; holder=$PID
holding AAA
KILL="$(cat "$scratch/xcodebuild.pid.AAA")"
kill -9 "$holder"; wait "$holder" 2>/dev/null || true; { kill $KILL 2>/dev/null || true; }; KILL=""
[ -L "$scratch/leases/AAA" ] || fail "the killed holder's lease should still be on disk"
launch "$scratch/out.2" HOLD=0 IOS_UI_LEASE_TIMEOUT=10; wait "$PID" || fail "a stale lease blocked the next run: $(cat "$scratch/out.2")"
grep -q 'start AAA' <(tail -2 "$scratch/events") || fail "the next run did not get AAA"

# 2b. A holder that is terminated (not killed) releases its lease itself.
reset; devices AAA:Booted
launch "$scratch/out.1" HOLD=30; holder=$PID
holding AAA
KILL="$(cat "$scratch/xcodebuild.pid.AAA")"
kill -TERM "$holder"; wait "$holder" 2>/dev/null || true; { kill $KILL 2>/dev/null || true; }; KILL=""
[ ! -e "$scratch/leases/AAA" ] || fail "a terminated run left its lease"

# 3. A device held by a live process: the run waits, says so, and gives up with
# a message rather than sharing.
reset; devices AAA:Booted
launch "$scratch/out.1" HOLD=30; holder=$PID
holding AAA
KILL="$(cat "$scratch/xcodebuild.pid.AAA") $holder"
launch "$scratch/out.2" IOS_UI_LEASE_TIMEOUT=2; code=0; wait "$PID" || code=$?
[ "$code" -eq 1 ] || fail "a run with no free device exited $code, wanted 1"
grep -q 'no free iPhone 17 after 2s' "$scratch/out.2" || fail "the timeout said nothing: $(cat "$scratch/out.2")"
grep -q "held by pid $holder" "$scratch/out.2" || fail "the timeout did not name the holder"
[ "$(grep -c '^start' "$scratch/events")" -eq 1 ] || fail "the waiting run started anyway"
{ kill $KILL 2>/dev/null || true; }; KILL=""; wait "$holder" 2>/dev/null || true

# 4. Opt-in cloning: with every booted device in use and a shut-down one to
# clone from, IOS_UI_CLONE_MAX=2 makes a clone; the default (above) never does.
reset; devices AAA:Booted SPARE:Shutdown
launch "$scratch/out.1" RELEASE="$scratch/release"; p1=$PID; holding AAA
launch "$scratch/out.2" HOLD=0 IOS_UI_CLONE_MAX=2; wait "$PID" || fail "the clone run failed: $(cat "$scratch/out.2")"
touch "$scratch/release"; wait "$p1" || fail "the holder failed"
grep -q 'start CLONE-UDID' "$scratch/events" || fail "no clone was used: $(cat "$scratch/events")"
grep -q 'simctl delete CLONE-UDID' "$scratch/xcrun.calls" || fail "the clone was not deleted"
reset; devices AAA:Booted SPARE:Shutdown
launch "$scratch/out.1" RELEASE="$scratch/release"; p1=$PID; holding AAA
launch "$scratch/out.2" HOLD=0 IOS_UI_LEASE_TIMEOUT=1; code=0; wait "$PID" || code=$?
touch "$scratch/release"; wait "$p1" || fail "the holder failed"
[ "$code" -eq 1 ] || fail "without IOS_UI_CLONE_MAX a busy device should time out, exit $code"
! grep -q 'simctl clone' "$scratch/xcrun.calls" || fail "cloned without being asked"

# 5. Two runs with the same SIMULATOR_ID (what lanes and the gate pass) never
# overlap: the second waits.
reset; devices AAA:Booted
launch "$scratch/out.1" RELEASE="$scratch/release" SIMULATOR_ID=AAA; p1=$PID; holding AAA
launch "$scratch/out.2" HOLD=0 SIMULATOR_ID=AAA; p2=$PID
until_grep 'waiting for a free AAA' "$scratch/out.2" || fail "the second SIMULATOR_ID run never said it was waiting: $(cat "$scratch/out.2")"
touch "$scratch/release"
for p in $p1 $p2; do wait "$p" || fail "a SIMULATOR_ID run failed: $(cat "$scratch"/out.*)"; done
no_overlap
grep -q 'waiting for a free AAA' "$scratch/out.2" || fail "the second SIMULATOR_ID run did not say it was waiting"

# 6. CI=true is the per-shard path: no lease, even for the same id twice.
reset; devices AAA:Booted
launch "$scratch/out.1" HOLD=1 SIMULATOR_ID=AAA CI=true; p1=$PID
launch "$scratch/out.2" HOLD=1 SIMULATOR_ID=AAA CI=true; p2=$PID
for p in $p1 $p2; do wait "$p" || fail "a CI run failed"; done
[ ! -e "$scratch/leases" ] || fail "a CI=true run took a lease"
grep -qx 'start AAA' "$scratch/events" || fail "SIMULATOR_ID was not the destination"

echo "ok: ios-ui-tests.sh gives each run its own booted simulator (SIMULATOR_ID too, except under CI=true), frees a killed run's lease, waits and times out when none is free, and clones only when asked"
