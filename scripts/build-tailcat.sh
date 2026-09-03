#!/bin/bash
# Build the tailcat Go shim as a C archive for one target.
#
# Go lives here rather than in a build.rs on purpose. `crates/protocol/build.rs`
# vendors protoc so that "the build has no system dependency", and the README
# promises Rust 1.85+ is enough to build this repository. A build.rs that shelled
# out to `go` would break both, for every contributor, forever. The platform
# scripts already do per-platform native work; this is more of it.
#
#   ./scripts/build-tailcat.sh ios-arm64
#   ./scripts/build-tailcat.sh linux-musl-x86_64
set -euo pipefail

cd "$(dirname "$0")/.."

# Homebrew's Go is not on every shell's PATH on this machine. Appended, not
# prepended and not exported alone, so nothing already on PATH (npm included)
# is shadowed or lost.
export PATH="$PATH:/opt/homebrew/bin"

TARGET="${1:?usage: build-tailcat.sh <target>}"
OUT="dist/tailcat/$TARGET"

command -v go >/dev/null 2>&1 || {
  cat <<EOF
No Go toolchain, so the tunnel cannot be built.

  brew install go

This is only needed for shipping builds. \`cargo build\` and
\`cargo test --workspace\` work without it and produce a farcooler that reaches
runners by address; only tunneled runners are unavailable.
EOF
  exit 1
}

case "$TARGET" in
  ios-arm64)
    export CGO_ENABLED=1 GOOS=ios GOARCH=arm64
    export CC="$(xcrun --sdk iphoneos -f clang) -isysroot $(xcrun --sdk iphoneos --show-sdk-path) -arch arm64"
    ;;
  linux-musl-x86_64)
    export CGO_ENABLED=1 GOOS=linux GOARCH=amd64 CC=x86_64-linux-musl-gcc
    ;;
  linux-musl-aarch64)
    export CGO_ENABLED=1 GOOS=linux GOARCH=arm64 CC=aarch64-linux-musl-gcc
    ;;
  # No android-* case. `go build -buildmode=c-archive` refuses GOOS=android
  # outright ("not supported on android/arm64", confirmed against this repo's
  # Go 1.27.0) — Android only gets `c-shared`, a dynamically-linked `.so`, not
  # the static archive every case above produces. That is a different
  # packaging model (a second `.so` per ABI, loaded and linked at runtime
  # rather than at Rust's build time) and a real decision for whoever wires
  # Android's tunnel support, not a detail this script can paper over. See
  # `scripts/build-android-libs.sh`, which leaves Android on the stub.
  *) echo "unknown target: $TARGET"; exit 1 ;;
esac

mkdir -p "$OUT"

# A failed or interrupted build must never leave a stale or partial archive
# at $OUT — a caller that does not check this script's exit code (a manual
# run, an IDE build step) would otherwise silently link whatever an old
# commit last put there. Removed up front, so a build that fails leaves
# nothing rather than something stale; built to a scratch directory and
# `mv`'d into place only once complete, so a build that succeeds is atomic
# and a build that is interrupted mid-write never leaves a half-written file
# at the final path either.
rm -f "$OUT/libtailcat.a" "$OUT/libtailcat.h"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

(cd crates/tailcat/go && go build -buildmode=c-archive -o "$TMP/libtailcat.a" .)
mv "$TMP/libtailcat.a" "$OUT/libtailcat.a"
mv "$TMP/libtailcat.h" "$OUT/libtailcat.h"
ls -l "$OUT/libtailcat.a"
