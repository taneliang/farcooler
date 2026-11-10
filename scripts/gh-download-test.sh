#!/usr/bin/env bash
# Tests scripts/gh-download.sh with a fake `gh` on PATH (ov-1030): a failed
# fetch is retried, a wrong checksum is retried then fails, a good one passes.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "${work:?}"' EXIT
mkdir "$work/bin"
cat > "$work/bin/gh" <<'F'
#!/usr/bin/env bash
# Fails $FAKE_FAILS times, then writes "payload" to the --output file.
n=$(cat "$FAKE_COUNT" 2>/dev/null || echo 0)
echo $((n + 1)) > "$FAKE_COUNT"
[ "$n" -lt "$FAKE_FAILS" ] && exit 1
while [ "$#" -gt 0 ]; do [ "$1" = --output ] && out=$2; shift; done
printf payload > "$out"
F
chmod +x "$work/bin/gh"
export PATH="$work/bin:$PATH" GH_DOWNLOAD_WAITS="0 0 0" FAKE_COUNT="$work/count"
good=$(printf payload | shasum -a 256 | cut -d' ' -f1)
run() { rm -f "$FAKE_COUNT"; FAKE_FAILS=$1 "$here/gh-download.sh" o/r v1 a.tgz "$work/out" "${2:-}" 2>/dev/null; }

run 0 "$good" || { echo "FAIL: a good download failed"; exit 1; }
run 2 "$good" || { echo "FAIL: two failures were not retried"; exit 1; }
[ "$(cat "$FAKE_COUNT")" = 3 ] || { echo "FAIL: expected 3 attempts"; exit 1; }
run 3 "$good" && { echo "FAIL: three failures should exhaust the retries"; exit 1; }
run 0 "$(printf 0%.0s {1..64})" && { echo "FAIL: a wrong checksum passed"; exit 1; }
[ "$(cat "$FAKE_COUNT")" = 3 ] || { echo "FAIL: a wrong checksum should be retried"; exit 1; }
run 0 || { echo "FAIL: no checksum should skip the check"; exit 1; }
echo "gh-download-test: ok"
