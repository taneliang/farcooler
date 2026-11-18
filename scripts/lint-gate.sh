#!/bin/sh
# Run every repository lint before agent review; keep proto's CI channel checks.
set -u
status=0
log=$(mktemp "${TMPDIR:-/tmp}/lint-gate.XXXXXX") || exit 1
trap 'rm -f "$log"' EXIT HUP INT TERM
run_lint() {
    if python3 "$@" >"$log" 2>&1; then
        printf 'PASS %s\n' "$*"
    else
        printf 'FAILED %s\n' "$*"
        cat "$log"
        status=1
    fi
}
for lint in scripts/*-lint.py; do
    if [ "$lint" = scripts/proto-lint.py ]; then
        run_lint "$lint" --channel preview
        run_lint "$lint" --channel stable
        run_lint "$lint" --channel canary --since-baseline
    else
        run_lint "$lint"
    fi
done
exit "$status"
