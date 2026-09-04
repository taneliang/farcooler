#!/bin/bash
# Build the tunnel helper a Linux runner spawns.
#
# This is the same Go package `build-tailcat.sh` turns into a C archive for iOS
# and macOS, built the other way: as a standalone program, with cgo off. See
# `crates/tailcat/go/helper.go` for what it is and `crates/tailcat/src/helper.rs`
# for how the daemon talks to it.
#
# Why Linux does not link the archive: a Go c-archive inside a musl binary
# segfaults inside Go's runtime startup, before it can print a word even under
# GOTRACEBACK=system. Measured in a Linux arm64 VM with a five-line Go program
# — static and dynamic alike, while both glibc forms work — and raising the
# per-thread stack through PT_GNU_STACK does not help: 8, 16, 64 and 256 MB
# each segfault identically, with `readelf -l` confirming the size took.
#
# `CGO_ENABLED=0` is the whole fix and it is not a workaround. A Go program
# built this way links no C library at all: it makes raw syscalls, so there is
# no musl and no glibc in it, and one binary runs on Debian, Alpine, and a NAS.
# That is the property musl was chosen for in the first place, arrived at more
# directly.
#
# It also means this script needs no C cross-compiler, which is the whole
# difference between it and `build-tailcat.sh`: `GOOS`/`GOARCH` alone
# cross-compile a cgo-free Go program to anywhere from anywhere.
#
#   ./scripts/build-tunnel-helper.sh x86_64
#   ./scripts/build-tunnel-helper.sh aarch64
set -euo pipefail

cd "$(dirname "$0")/.."

# Homebrew's Go is not on every shell's PATH on this machine. Appended, not
# prepended and not exported alone, so nothing already on PATH (npm included)
# is shadowed or lost. Same rule as `build-tailcat.sh`.
export PATH="$PATH:/opt/homebrew/bin"

ARCH="${1:-$(uname -m)}"
case "$ARCH" in
  arm64|aarch64) ARCH=aarch64; GOARCH=arm64 ;;
  x86_64|amd64)  ARCH=x86_64;  GOARCH=amd64 ;;
  *) echo "unknown architecture: $ARCH (try x86_64 or aarch64)" >&2; exit 1 ;;
esac

OUT="dist/tunnel/$ARCH-linux"
# The name cargo would never give this, because cargo does not build it. It is
# the bare, stable-channel spelling; `farcooler host install` renames it to
# this channel's own on the destination, exactly as it does the daemon and the
# CLI — see `Channel::tunnel_binary_name`.
BIN=farcooler-tunnel

command -v go >/dev/null 2>&1 || {
  cat <<EOF
No Go toolchain, so the tunnel helper cannot be built.

  brew install go

This is only needed for shipping builds. \`cargo build\` and
\`cargo test --workspace\` work without it and produce a farcooler that reaches
runners by address; only tunneled runners are unavailable.
EOF
  exit 1
}

mkdir -p "$OUT"

# Removed up front and moved into place only once complete, for the reason
# `build-tailcat.sh` gives at its own copy of this: a build that fails must
# leave nothing rather than something stale, and a caller that does not check
# an exit code must not be able to ship whatever an old commit last put here.
rm -f "$OUT/$BIN"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# CGO_ENABLED=0 stated rather than left to Go's default. Cross-compiling turns
# it off anyway, but a native x86_64-on-x86_64 CI build would default it ON,
# and an accidentally-cgo build here is precisely the binary that segfaults.
# `-trimpath` for a reproducible one.
(cd crates/tailcat/go && CGO_ENABLED=0 GOOS=linux GOARCH="$GOARCH" \
  go build -trimpath -o "$TMP/$BIN" .)

# Not a proxy for "it works", but it does catch the two ways this file can be
# wrong in a way that only shows up on a runner: built for the host instead of
# the target, or built with cgo after all. `go version -m` reads the build
# settings Go stamps into the binary, so this is the binary's own answer.
go version -m "$TMP/$BIN" | grep -q 'GOARCH='"$GOARCH" || {
  echo "$BIN is not $GOARCH; the cross-compile did not take" >&2; exit 1; }
go version -m "$TMP/$BIN" | grep -q 'CGO_ENABLED=0' || {
  echo "$BIN was built with cgo, which is the thing that segfaults on musl" >&2; exit 1; }

mv "$TMP/$BIN" "$OUT/$BIN"
ls -l "$OUT/$BIN"
