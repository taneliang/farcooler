#!/bin/bash
# Build the Rust terminal core and stage it for SwiftPM.
#
# SwiftPM cannot build a Rust crate, so this bridges the two: it compiles the
# static library and copies the header next to the module map. crates/vt/include
# is the single source of truth — the copy here is generated and gitignored, so
# the two cannot drift into disagreeing about the ABI.
set -euo pipefail

# A double-clicked build, or one from an IDE, inherits no shell profile.
export PATH="$HOME/.cargo/bin:$PATH"

cd "$(dirname "$0")"
ROOT="$(cd ../.. && pwd)"
CONFIG="${1:-release}"

echo "==> Building the terminal core ($CONFIG)"
if [ "$CONFIG" = "debug" ]; then
  (cd "$ROOT" && cargo build -p farcooler-vt)
else
  (cd "$ROOT" && cargo build --release -p farcooler-vt)
fi

LIB="$ROOT/target/$CONFIG/libfarcooler_vt.a"
[ -f "$LIB" ] || { echo "no static library at $LIB"; exit 1; }

# Package.swift links from target/release. A debug build is staged there too so
# `build-vt.sh debug` produces something the app can actually link against,
# rather than silently linking a stale release library.
if [ "$CONFIG" = "debug" ]; then
  mkdir -p "$ROOT/target/release"
  cp "$LIB" "$ROOT/target/release/libfarcooler_vt.a"
fi

echo "==> Staging the header"
cp "$ROOT/crates/vt/include/farcooler_vt.h" Sources/CFarCoolerVT/farcooler_vt.h

echo "    $(du -h "$LIB" | cut -f1)  $LIB"
