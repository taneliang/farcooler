#!/bin/bash
# Prove capture-duplicates.sh fails on the bug it exists for and passes
# otherwise (ov-642): two states saved under different names with the same
# bytes, as the four "Done" sign-in captures were.
set -euo pipefail

cd "$(dirname "$0")/.."
work="$(mktemp -d /tmp/fc-dupes-XXXXXX)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/run" "$work/ok"
none="$work/none.txt"
: >"$none"

printf 'code step' >"$work/run/mac-sheet-code-light.png"
printf 'code step' >"$work/run/mac-sheet-done-light.png"
printf 'other' >"$work/run/mac-sheet-page-light.png"

if scripts/capture-duplicates.sh "$work/run" --allow "$none" 2>"$work/err"; then
    echo "capture-duplicates-test: two names with the same bytes were not reported" >&2
    exit 1
fi
grep -q 'mac-sheet-code-light.png' "$work/err" && grep -q 'mac-sheet-done-light.png' "$work/err" || {
    echo "capture-duplicates-test: the report doesn't name both files" >&2
    cat "$work/err" >&2
    exit 1
}
grep -q 'mac-sheet-page-light.png' "$work/err" && {
    echo "capture-duplicates-test: a file that is not a duplicate was named" >&2
    exit 1
}

# An allow-listed group and a contrast pair pass.
printf 'same' >"$work/ok/a-done-short.png"
printf 'same' >"$work/ok/a-done-long.png"
printf 'twin' >"$work/ok/b-light.png"
printf 'twin' >"$work/ok/b-light-contrast.png"
printf 'a' >"$work/ok/c.png"
printf 'a-done-short.png a-done-long.png\n' >"$work/allow.txt"
scripts/capture-duplicates.sh "$work/ok" --allow "$work/allow.txt" 2>/dev/null || {
    echo "capture-duplicates-test: an allowed group or a contrast pair failed the run" >&2
    exit 1
}
# And without the allow line, the same directory fails.
if scripts/capture-duplicates.sh "$work/ok" --allow "$none" 2>/dev/null; then
    echo "capture-duplicates-test: the allow line did nothing" >&2
    exit 1
fi

# An allowed short-and-long pair does not excuse a light capture that matches its dark twin
# (the polish sweep's bug: every dark shot was light).
mkdir "$work/appearance"
printf 'x' >"$work/appearance/a-done-short-light.png"
printf 'x' >"$work/appearance/a-done-short-dark.png"
printf 'a-done-short-light.png a-done-long-light.png\na-done-short-dark.png a-done-long-dark.png\n' >"$work/pairs.txt"
if scripts/capture-duplicates.sh "$work/appearance" --allow "$work/pairs.txt" 2>"$work/err2"; then
    echo "capture-duplicates-test: a light capture matching its dark twin was excused by a short/long rule" >&2
    exit 1
fi
grep -q 'a-done-short-dark.png' "$work/err2" || { echo "capture-duplicates-test: the light/dark pair isn't named" >&2; exit 1; }
# The same short/long pair in one appearance is excused by that line.
mkdir "$work/pair"
printf 'y' >"$work/pair/a-done-short-light.png"
printf 'y' >"$work/pair/a-done-long-light.png"
scripts/capture-duplicates.sh "$work/pair" --allow "$work/pairs.txt" 2>/dev/null || { echo "capture-duplicates-test: the allowed pair failed" >&2; exit 1; }

# A PNG older than the run's marker is not compared: stale files from a renamed state are not a hit.
mkdir "$work/stale"
printf 'z' >"$work/stale/old-a.png"
printf 'z' >"$work/stale/old-b.png"
sleep 1
touch "$work/marker"
sleep 1
printf 'fresh' >"$work/stale/new.png"
scripts/capture-duplicates.sh "$work/stale" --allow "$none" --since "$work/marker" 2>/dev/null || { echo "capture-duplicates-test: stale files were compared" >&2; exit 1; }
if scripts/capture-duplicates.sh "$work/stale" --allow "$none" 2>/dev/null; then
    echo "capture-duplicates-test: without --since the stale pair should be a hit" >&2
    exit 1
fi
echo "capture-duplicates-test: ok"
