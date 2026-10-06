#!/usr/bin/env bash
# `xcrun notarytool` that retries a dropped network connection, for the
# workflow steps that submit a build to Apple (ov-387).
#
#   notarytool-retry.sh <notarytool arguments...>
#   notarytool-retry.sh --self-test
#
# Canary run 37523131632 failed on a signed, built, finished disk image because
# `notarytool submit` hit NSURLErrorDomain -1005, "The network connection was
# lost", a hiccup between a runner and Apple that a second try would have
# cleared. So a failure that looks like the NETWORK is tried again, up to
# NOTARY_TRIES times in all (default 3), sleeping NOTARY_BACKOFF seconds
# (default 20) after the first and double that after the second.
#
# Anything Apple said is final and fails at once, however many tries are left:
# a notarization that came back Invalid or Rejected, a 401 for a bad password,
# any "HTTP status code". Asking again cannot change that answer, and a retry
# must never be what turns a rejection into an upload that happens to be
# accepted. The two tests below keep the rule honest: the output has to look
# like the network AND carry no word from Apple, or it is not retried. A
# failure that is neither (a bad flag, no such file) also fails at once.
#
# One thing a retry can do: a drop after the upload but before `--wait`
# returned submits the same file a second time. Apple takes that without
# complaint, and it costs minutes, not a wrong answer.
#
# NOTARY_BIN replaces `xcrun notarytool` (the self-test's scripted stand-in).
set -uo pipefail

# What a dropped connection looks like in notarytool's output: Foundation's
# NSURLErrorDomain codes for a timeout (-1001), no host (-1003), a refused or
# lost connection (-1004, -1005), no network (-1009), a secure-connection
# failure (-1200), plus the words those carry.
network() {
  grep -Eqi 'NSURLErrorDomain|network connection was lost|connection appears to be offline|request timed out|could not connect to the server|connection (reset|refused)|timed? ?out|secure connection to the server' "$1"
}

# Anything Apple answered. A reply, not a transport failure.
answered() {
  grep -Eqi 'status: *(invalid|rejected|accepted)|HTTP status code|statusCode|"status" *: *"(invalid|rejected)|not authorized|unauthorized|invalid credentials|Error: *(Unauthorized|Forbidden)' "$1"
}

run() {
  local tries="${NOTARY_TRIES:-3}" delay="${NOTARY_BACKOFF:-20}"
  local -a bin
  # shellcheck disable=SC2206
  bin=(${NOTARY_BIN:-xcrun notarytool})
  local log attempt=0 status
  log="$(mktemp)"
  while :; do
    attempt=$((attempt + 1))
    # tee keeps the log streaming, which matters for a --wait that takes minutes.
    "${bin[@]}" "$@" 2>&1 | tee "$log"
    status=${PIPESTATUS[0]}
    if [ "$status" -eq 0 ]; then
      rm -f "$log"
      return 0
    fi
    if answered "$log" || ! network "$log"; then
      echo "::error::notarytool failed and retrying cannot help (exit $status)" >&2
      rm -f "$log"
      return "$status"
    fi
    if [ "$attempt" -ge "$tries" ]; then
      echo "::error::notarytool kept losing its network connection after $attempt tries; re-run this job" >&2
      rm -f "$log"
      return "$status"
    fi
    echo "notarytool lost its network connection (try $attempt of $tries), retrying in ${delay}s" >&2
    sleep "$delay"
    delay=$((delay * 2))
  done
}

self_test() {
  local dir fails=0
  dir="$(mktemp -d)"
  # A stand-in notarytool: prints $FAKE_ERR and exits 1 for the first
  # $FAKE_FAILS calls, then says Accepted. Each call is counted in $dir/calls.
  cat > "$dir/notarytool" <<'FAKE'
#!/usr/bin/env bash
n=$(( $(cat "$FAKE_DIR/calls" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$FAKE_DIR/calls"
if [ "$n" -le "${FAKE_FAILS:-0}" ]; then printf '%s\n' "$FAKE_ERR" >&2; exit 1; fi
echo "status: Accepted"
FAKE
  chmod +x "$dir/notarytool"
  export FAKE_DIR="$dir" NOTARY_BIN="$dir/notarytool" NOTARY_BACKOFF=0

  case_() { # name, fails, error, want-exit, want-calls, want-stderr-pattern
    echo 0 > "$dir/calls"
    local code calls
    FAKE_FAILS="$2" FAKE_ERR="$3" run submit x.zip >"$dir/stdout" 2>"$dir/stderr"; code=$?
    calls="$(cat "$dir/calls")"
    if [ "$code" -ne "$4" ] || [ "$calls" -ne "$5" ] || { [ -n "$6" ] && ! grep -q -- "$6" "$dir/stderr"; }; then
      echo "FAIL $1: exit $code (want $4), calls $calls (want $5), stderr: $(cat "$dir/stderr")" >&2
      fails=$((fails + 1))
    fi
  }
  local lost='Error: Error Domain=NSURLErrorDomain Code=-1005 "The network connection was lost."'
  case_ "a clean submit is made once"                 0 ""                                0 1 ""
  case_ "a dropped connection is retried, then passes" 1 "$lost"                           0 2 "retrying"
  case_ "two drops are retried"                       2 "$lost"                           0 3 "retrying"
  case_ "a timeout is retried"                        1 'Error: The request timed out.'   0 2 "retrying"
  case_ "a connection that keeps dropping fails after 3 tries" 99 "$lost"                 1 3 "::error::notarytool kept losing"
  case_ "an Invalid notarization fails at once"       99 'status: Invalid'                1 1 "retrying cannot help"
  case_ "a Rejected notarization fails at once"       99 'status: Rejected'               1 1 "retrying cannot help"
  case_ "an Apple reply is final even if it mentions the network" 99 \
    "Error: HTTP status code: 401. NSURLErrorDomain network connection was lost earlier"    1 1 "retrying cannot help"
  case_ "an Invalid result that quotes a timeout is final" 99 \
    "status: Invalid
The binary timed out of signing checks"                                                     1 1 "retrying cannot help"
  case_ "a failure that is not the network fails at once" 99 'Error: no such file x.zip'  1 1 "retrying cannot help"
  rm -rf "$dir"
  if [ "$fails" -ne 0 ]; then echo "$fails self-test case(s) failed" >&2; return 1; fi
  echo "notarytool-retry self-test: ok"
}

if [ "${1:-}" = "--self-test" ]; then self_test; else run "$@"; fi
