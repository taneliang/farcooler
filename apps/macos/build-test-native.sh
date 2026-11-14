#!/bin/bash
# One native profile for the Mac tests' static libraries and real CLI/daemon
# (ov-1183). CI uses opt-level 1 and no symbols, retaining dev assertions and
# overflow checks. Building release libraries and then debug binaries compiled
# the same dependency graph twice. All four packages stay in every invocation
# so Cargo's feature union also stays the same when test.sh revalidates it.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
export MACOSX_DEPLOYMENT_TARGET=26.0
cd "$(dirname "$0")/../.."

# Keep FFI/rendering code optimized, while the CLI and daemon retain their
# original unoptimized dev build. They serve functional integration tests;
# spending LLVM optimization time on these two large crates buys no coverage.
cargo build --config profile.dev.opt-level=1 \
  --config profile.dev.package.farcooler-cli.opt-level=0 \
  --config profile.dev.package.farcooler-daemon.opt-level=0 \
  --lib --bins -p farcooler-vt -p farcooler-client -p farcooler-cli -p farcooler-daemon

# Swift packages link from target/release. Stage freshly built test libraries
# there, as build-vt.sh debug already does. The real-CLI tests use target/debug,
# so the CLI and its sibling daemon require no path override or copying.
mkdir -p target/release
for name in farcooler_vt farcooler_client; do
  cp "target/debug/lib$name.a" "target/release/lib$name.a"
done
cp -p crates/vt/include/farcooler_vt.h apps/macos/Sources/CFarCoolerVT/farcooler_vt.h
