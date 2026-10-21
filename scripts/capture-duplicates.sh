#!/bin/bash
# Captures of one run that are byte for byte the same file under different
# names (ov-642).
#
#   scripts/capture-duplicates.sh <dir> [--allow <file>]
#
# Hashes every `*.png` under <dir> and reports each group of two or more with
# the same bytes. Four "Done" captures of the sign-in sheet once came out
# identical to the code step's, because the test shot before the model got to
# Done: each was a valid image of the wrong state, and it passed every check
# that looked at one file alone. Two different states drawn identically is the
# one thing only a comparison of the run's own files can see.
#
# A group is excused when every name in it matches a line of the allow file
# (`scripts/capture-duplicates-allow.txt` unless `--allow` names another), a
# shell pattern per line: for states that really do draw alike, like a long
# runner name in a state that never shows it. Say why in a `#` comment above
# the line.
#
# Groups whose names differ only by `-contrast` are reported as notes and do
# not fail the run: that pair is `capture-contrast-check.sh`'s to judge, below
# the title bar, and many sheets draw the same in both because the app's colors
# don't read the setting.
#
# Exits 1 when a group is neither excused nor a contrast pair. Bytes, not
# pixels: the same encoder writes every file of a run, so the same pixels are
# the same bytes, and nothing here needs Pillow.
set -euo pipefail

dir="${1:?usage: capture-duplicates.sh <dir> [--allow <file>]}"
shift
allow="$(cd "$(dirname "$0")" && pwd)/capture-duplicates-allow.txt"
while [ $# -gt 0 ]; do
    case "$1" in
        --allow) allow="${2:?--allow needs a file}"; shift 2 ;;
        *) echo "capture-duplicates: unknown argument $1" >&2; exit 2 ;;
    esac
done
[ -d "$dir" ] || { echo "capture-duplicates: $dir is not a directory" >&2; exit 2; }

exec python3 -I - "$dir" "$allow" <<'PY'
import fnmatch, hashlib, os, sys

folder, allow_path = sys.argv[1], sys.argv[2]
patterns = []
if os.path.exists(allow_path):
    for line in open(allow_path):
        line = line.strip()
        if line and not line.startswith("#"):
            patterns.append(line)

groups = {}
total = 0
for root, _, names in os.walk(folder):
    for name in sorted(names):
        if not name.endswith(".png"):
            continue
        total += 1
        path = os.path.join(root, name)
        groups.setdefault(hashlib.sha256(open(path, "rb").read()).hexdigest(), []).append(os.path.relpath(path, folder))

def stem(path):
    return path.replace("-contrast", "")

failed = notes = 0
for members in sorted(groups.values()):
    if len(members) < 2:
        continue
    if all(any(fnmatch.fnmatch(os.path.basename(m), p) for p in patterns) for m in members):
        continue
    # A contrast twin and its standard file: the contrast check's to judge.
    if len({stem(m) for m in members}) == 1:
        notes += 1
        continue
    failed += 1
    print("capture-duplicates: these are the same bytes under different names, so one of them shows the wrong state:", file=sys.stderr)
    for m in members:
        print(f"  {m}", file=sys.stderr)
print(f"capture-duplicates: {total} captures, {failed} groups the same under different names"
      f" ({notes} contrast pairs left to capture-contrast-check)", file=sys.stderr)
sys.exit(1 if failed else 0)
PY
