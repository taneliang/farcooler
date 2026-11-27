#!/usr/bin/env bash
# Builds the one wasm module services/web's browser checks load, for a host
# without CI's pinned Linux tools (scripts/lane-gate.py runs this; ov-1499).
#
#   crates/web/test-module.sh <out-dir>
#   WEB_WASM_DIR=<out-dir> node test/browser/chat-check.mjs   (in services/web)
#
# Uses $WASM_BINDGEN when set. Otherwise installs exactly the wasm-bindgen-cli
# Cargo.lock names, from crates.io, under this crate's target directory.
set -euo pipefail
if [ $# -ne 1 ]; then
  echo "usage: $0 <out-dir>" >&2
  exit 2
fi
out=$1
case "$out" in /*) ;; *) out="$PWD/$out" ;; esac
cd "$(dirname "$0")"

if [ -z "${WASM_BINDGEN:-}" ]; then
  locked=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
  root="${CARGO_TARGET_DIR:-$PWD/target}/tools/wasm-bindgen-$locked"
  WASM_BINDGEN="$root/bin/wasm-bindgen"
  if [ ! -x "$WASM_BINDGEN" ]; then
    # Its own target directory: the CLI is a host build with its own lockfile.
    "${CARGO:-cargo}" install wasm-bindgen-cli --version "$locked" --locked --root "$root" --target-dir "$root/build"
    rm -rf "$root/build"
  fi
fi
WASM_BINDGEN="$WASM_BINDGEN" ./build.sh --channel canary --module-only --out "$out"
