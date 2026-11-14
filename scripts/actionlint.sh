#!/usr/bin/env bash
# Lint .github/workflows with the repo's actionlint config (ov-1240).
#
# No actionlint release (latest 1.7.12) knows GitHub's documented
# `concurrency.queue: single|max` key (upstream rhysd/actionlint#654, #657, #746).
# Until one does, this builds upstream at a pinned revision plus the reviewed
# patch scripts/actionlint/queue.patch, which accepts `queue` only with the
# literal values `single` or `max`. Nothing is ignored or disabled.
#
#   scripts/actionlint.sh              lint .github/workflows/*.yml
#   scripts/actionlint.sh --self-test  prove the patch accepts/rejects the right values
#   scripts/actionlint.sh --print-bin  build if needed, print the binary path
# Needs git and Go (>= 1.24). Binary cache: ${ACTIONLINT_CACHE:-~/.cache/farcooler-actionlint}.
set -euo pipefail
REV=011a6d15e749bb3f2d771eed9c7aa0e7e3e10ee7
root=$(cd "$(dirname "$0")/.." && pwd)
patch_file=$root/scripts/actionlint/queue.patch
cache=${ACTIONLINT_CACHE:-$HOME/.cache/farcooler-actionlint}
key=$REV-$(shasum -a 256 "$patch_file" | cut -c1-12)
bin=$cache/$key/actionlint

build() {
  [ -x "$bin" ] && return
  local src
  src=$(mktemp -d)
  git clone -q https://github.com/rhysd/actionlint "$src/s"
  git -C "$src/s" checkout -q "$REV"
  git -C "$src/s" apply "$patch_file"
  mkdir -p "$cache/$key"
  (cd "$src/s" && go build -o "$bin" ./cmd/actionlint)
  rm -rf "$src"
}

self_test() {
  local d out
  d=$(mktemp -d)
  mk() { # name, concurrency body
    printf 'on: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n    concurrency:\n      group: g\n%s\n    steps:\n      - run: echo hi\n' "$2" >"$d/$1.yml"
  }
  mk max '      queue: max'; mk single '      queue: single'
  mk bad '      queue: all'; mk typo '      queu: max'
  for ok in max single; do
    "$bin" "$d/$ok.yml" || { echo "FAIL: queue: $ok rejected"; exit 1; }
  done
  for bad in bad typo; do
    if "$bin" "$d/$bad.yml" >/dev/null 2>&1; then echo "FAIL: $bad accepted"; exit 1; fi
  done
  out=$("$bin" "$d/bad.yml" 2>&1 || true)
  grep -q 'must be "single" or "max"' <<<"$out" || { echo "FAIL: wrong message: $out"; exit 1; }
  rm -rf "$d"
  echo "actionlint.sh self-test: ok"
}

build
case "${1:-}" in
  --print-bin) echo "$bin" ;;
  --self-test) self_test ;;
  "") cd "$root" && "$bin" -version | head -1 && "$bin" -config-file .github/actionlint.yaml .github/workflows/*.yml && echo "actionlint: ok" ;;
  *) echo "usage: $0 [--self-test|--print-bin]" >&2; exit 2 ;;
esac
