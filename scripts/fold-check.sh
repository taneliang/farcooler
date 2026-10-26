#!/bin/bash
# Re-run, on the combined tree, the cheap lints that folding gated lanes can break (ov-817).
#
#   scripts/fold-check.sh [base]        # base defaults to origin/main
#
# Each lane passes its own gate on its own tree. Folding several into one train
# builds a tree no gate saw, and some checks read the whole tree: a file one
# lane grew and another grew past its budget, a test floor each lane raised, a
# migration number two lanes took. A lane gate run with --only skips the lints
# job altogether, so even a single lane can arrive here unchecked.
#
# These are CI's lints for the same things, with their own tests left out (CI
# runs those). Every check runs even when an earlier one fails, and a summary
# says which failed. Exit 1 if any did. No builds, no tests: seconds.
set -u
cd "$(dirname "$0")/.." || exit 2
export PYTHONDONTWRITEBYTECODE=1
base="${1:-origin/main}"
git rev-parse --verify -q "$base^{commit}" >/dev/null || { echo "fold-check: no base $base (git fetch origin?)" >&2; exit 2; }

names=(); results=(); secs=(); failed=0
check() {  # check <name> <command...>
  local name="$1" t0 out rc; shift
  t0=$SECONDS
  out="$("$@" 2>&1)"; rc=$?
  names+=("$name"); secs+=("$((SECONDS - t0))")
  if [ "$rc" -eq 0 ]; then results+=(ok); else
    results+=(FAIL); failed=$((failed + 1))
    echo "--- $name failed (exit $rc): $*"; printf '%s\n' "$out" | tail -25
  fi
}

check "file size budget"   ./scripts/file-size-budget.py
check "test floors"        ./scripts/test-floor-check.py --base "$base"
check "proto lint preview" ./scripts/proto-lint.py --channel preview
check "proto lint stable"  ./scripts/proto-lint.py --channel stable
check "proto lint canary"  ./scripts/proto-lint.py --channel canary
check "migration lint"     ./scripts/migration-lint.py
check "capability names"   ./scripts/capability-names.py --check
check "copy lint"          ./scripts/copy-lint.py
check "doc comments"       ./scripts/doc-comment-check.py --base "$base"

echo
echo "## fold-check summary (against $base, HEAD $(git rev-parse --short HEAD))"
echo
echo "| Check | Result | Secs |"
echo "| --- | --- | --- |"
for i in "${!names[@]}"; do echo "| ${names[$i]} | ${results[$i]} | ${secs[$i]} |"; done
echo
if [ "$failed" -eq 0 ]; then echo "fold-check: all ${#names[@]} checks passed"; else echo "fold-check: FAILED ($failed of ${#names[@]})"; exit 1; fi
