#!/usr/bin/env bash
# `gh api` that retries a transient failure, for the workflow steps that list
# CI runs (ov-344).
#
#   gh-api-retry.sh <gh api arguments...>      stdout is gh's stdout
#   gh-api-retry.sh --self-test
#
# A transient failure is an HTTP 5xx, a timeout or a dropped connection. It is
# retried GH_RETRIES times (default 3) after sleeping GH_BACKOFF seconds
# (default 5), doubling each time. Anything else (a 401, a 404, a bad flag)
# fails at once, since asking again cannot change the answer. After the last
# retry the script prints one `::error::` line and exits 1. Canary must never
# guess what to ship, so there is no fallback: a Plan that cannot read the
# green runs fails, and re-running it is the owner's call.
#
# GH_BIN replaces `gh` (the self-test's scripted stand-in).
set -uo pipefail

transient() {
  grep -Eqi 'HTTP 5[0-9][0-9]|timed? ?out|timeout|connection (reset|refused)|EOF|temporar|TLS handshake|no such host' "$1"
}

run() {
  local gh="${GH_BIN:-gh}" retries="${GH_RETRIES:-3}" delay="${GH_BACKOFF:-5}"
  local out err attempt=0 status
  out="$(mktemp)"; err="$(mktemp)"
  trap 'rm -f "$out" "$err"' RETURN
  while :; do
    "$gh" api "$@" > "$out" 2> "$err"
    status=$?
    if [ "$status" -eq 0 ]; then
      cat "$out"
      return 0
    fi
    if ! transient "$err"; then
      echo "::error::gh api failed and retrying cannot help: $(head -c 200 "$err" | tr '\n' ' ')" >&2
      return 1
    fi
    if [ "$attempt" -ge "$retries" ]; then
      echo "::error::GitHub's API kept failing after $retries retries, so Canary cannot tell which commits CI passed and will not guess. Re-run this workflow once GitHub recovers. Last error: $(head -c 200 "$err" | tr '\n' ' ')" >&2
      return 1
    fi
    attempt=$((attempt + 1))
    echo "gh api failed (attempt $attempt of $((retries + 1))), retrying in ${delay}s: $(head -c 120 "$err" | tr '\n' ' ')" >&2
    sleep "$delay"
    delay=$((delay * 2))
  done
}

self_test() {
  local dir fails=0
  dir="$(mktemp -d)"
  # A stand-in gh: fails with $FAKE_ERR for the first $FAKE_FAILS calls, then
  # prints "ok". Each call is counted in $dir/calls.
  cat > "$dir/gh" <<'FAKE'
#!/usr/bin/env bash
n=$(( $(cat "$FAKE_DIR/calls" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$FAKE_DIR/calls"
if [ "$n" -le "${FAKE_FAILS:-0}" ]; then echo "$FAKE_ERR" >&2; exit 1; fi
echo ok
FAKE
  chmod +x "$dir/gh"
  export FAKE_DIR="$dir" GH_BIN="$dir/gh" GH_BACKOFF=0

  case_() { # name, fails, error, want-exit, want-calls, want-stderr-pattern
    echo 0 > "$dir/calls"
    local out code calls
    out="$(FAKE_FAILS="$2" FAKE_ERR="$3" run x 2>"$dir/stderr")"; code=$?
    calls="$(cat "$dir/calls")"
    if [ "$code" -ne "$4" ] || [ "$calls" -ne "$5" ] || { [ -n "$6" ] && ! grep -q -- "$6" "$dir/stderr"; }; then
      echo "FAIL $1: exit $code (want $4), calls $calls (want $5), stderr: $(cat "$dir/stderr")" >&2
      fails=$((fails + 1))
    fi
    if [ "$4" -eq 0 ] && [ "$out" != ok ]; then
      echo "FAIL $1: stdout was '$out'" >&2; fails=$((fails + 1))
    fi
  }
  case_ "a clean call is made once"           0 ""                              0 1 ""
  case_ "a 502 is retried and then succeeds"  2 "gh: Bad Gateway (HTTP 502)"    0 3 "retrying"
  case_ "a timeout is retried"                1 "Get x: i/o timeout"            0 2 "retrying"
  case_ "a persistent 503 fails after 3 retries" 99 "gh: Unavailable (HTTP 503)" 1 4 "::error::GitHub's API kept failing"
  case_ "a 404 fails at once"                 99 "gh: Not Found (HTTP 404)"     1 1 "::error::gh api failed"
  case_ "a 401 fails at once"                 99 "gh: Bad credentials (HTTP 401)" 1 1 "::error::gh api failed"
  rm -rf "$dir"
  if [ "$fails" -ne 0 ]; then echo "$fails self-test case(s) failed" >&2; return 1; fi
  echo "gh-api-retry self-test: ok"
}

if [ "${1:-}" = "--self-test" ]; then self_test; else run "$@"; fi
