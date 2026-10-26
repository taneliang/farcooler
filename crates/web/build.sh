#!/usr/bin/env bash
# Builds the browser's two wasm modules for services/web (ov-764, plan-763 §1
# "Building and versioning the WASM"):
#
#   <out>/farcooler_web.wasm  this crate: SSH, the protocol, the ceremony
#   <out>/farcooler_web.js    its wasm-bindgen glue (ES module, --target web)
#   <out>/tailcat.wasm        crates/tailcat/go built for GOOS=js
#   <out>/wasm_exec.js        Go's runtime support, from the same toolchain
#
#   crates/web/build.sh --channel canary|stable [--out DIR]
#
# Then `WEB_WASM_DIR=<out> services/web/build.sh --channel <same>`, which
# stores each .wasm as brotli only and bundles both .js files into the page.
#
# Tools, each overridable by environment:
#   CARGO          cargo with the wasm32-unknown-unknown target
#   WASM_BINDGEN   wasm-bindgen-cli, EXACTLY the version Cargo.lock holds for
#                  the wasm-bindgen crate; this script refuses any other,
#                  because a mismatched CLI writes glue the module disagrees
#                  with at runtime
#   WASM_OPT       binaryen's wasm-opt
#   GO             the toolchain crates/tailcat/go/go.mod names
# On macOS, ring's C must be archived by LLVM's ar (Apple's writes archives
# rust-lld cannot index for wasm; spike-754 §1): AR_wasm32_unknown_unknown is
# set to Homebrew's llvm-ar when it is there and nothing else was given.
set -euo pipefail
cd "$(dirname "$0")"
here=$PWD

channel=""
out="$here/dist"
while [ $# -gt 0 ]; do
  case "$1" in
    --channel) channel=${2:-}; shift 2 ;;
    --out) out=${2:-}; shift 2 ;;
    *) echo "crates/web/build.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done
case "$channel" in
  canary|stable) ;;
  *) echo "crates/web/build.sh: --channel is canary or stable" >&2; exit 2 ;;
esac

# The channel the protocol crate will stamp, asked of the one script that
# decides it. A tree with uncommitted changes stamps `local` whatever was
# asked, and a canary page holding a local module would offer codes no canary
# phone accepts and run a daemon no runner has. Refused here, not discovered
# on a phone.
stamped=$(FARCOOLER_CHANNEL=$channel ../../scripts/version.sh channel)
if [ "$stamped" != "$channel" ]; then
  echo "crates/web/build.sh: this tree stamps the $stamped channel, not $channel (uncommitted changes build as local); commit them first" >&2
  exit 1
fi

CARGO=${CARGO:-cargo}
WASM_BINDGEN=${WASM_BINDGEN:-wasm-bindgen}
WASM_OPT=${WASM_OPT:-wasm-opt}
GO=${GO:-go}
if [ -z "${AR_wasm32_unknown_unknown:-}" ] && [ "$(uname)" = Darwin ]; then
  for ar in /opt/homebrew/opt/llvm*/bin/llvm-ar /usr/local/opt/llvm*/bin/llvm-ar; do
    if [ -x "$ar" ]; then export AR_wasm32_unknown_unknown=$ar; break; fi
  done
fi

# The CLI must match the crate it post-processes, exactly.
locked=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
cli=$("$WASM_BINDGEN" --version | awk '{ print $2 }')
if [ -z "$locked" ] || [ "$cli" != "$locked" ]; then
  echo "crates/web/build.sh: wasm-bindgen CLI is ${cli:-missing}, Cargo.lock has ${locked:-none}; install wasm-bindgen-cli $locked" >&2
  exit 1
fi

mkdir -p "$out"
out=$(cd "$out" && pwd)
rm -f "$out"/farcooler_web* "$out"/tailcat.wasm "$out"/wasm_exec.js

# --- Rust: the channel is stamped by farcooler-protocol's build.rs, which is
# what makes a canary page refuse a stable phone's codes and run a canary
# daemon. --locked: the browser's dependency set is the one committed.
FARCOOLER_CHANNEL=$channel "$CARGO" build --locked --release --target wasm32-unknown-unknown
"$WASM_BINDGEN" --target web --no-typescript --out-dir "$out" --out-name farcooler_web \
  target/wasm32-unknown-unknown/release/farcooler_web.wasm
# The features rustc's wasm32 target emits by default, named so wasm-opt
# neither refuses the module nor introduces anything a browser lacks.
"$WASM_OPT" -Oz \
  --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
  --enable-mutable-globals --enable-reference-types --enable-multivalue \
  -o "$out/farcooler_web.wasm" "$out/farcooler_web_bg.wasm"
rm "$out/farcooler_web_bg.wasm"

# --- Go: the runner's own module, so the browser's Tailcat and the runner's
# are the same version by construction. wasm-opt gains nothing on Go's output
# (spike-761 §4), so it is not run.
(
  cd ../tailcat/go
  tags=$("$GO" run ./webtags)
  GOOS=js GOARCH=wasm CGO_ENABLED=0 "$GO" build -trimpath -ldflags="-s -w" -tags "$tags" -o "$out/tailcat.wasm" .
  cp "$("$GO" env GOROOT)/lib/wasm/wasm_exec.js" "$out/wasm_exec.js"
)

ls -l "$out"
