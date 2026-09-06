#!/bin/bash
# Build a DERP relay for `crates/daemon/tests/a_real_tunnel_carries_the_scope.rs`.
#
# DERP is the rendezvous for every tunneled connection, so two tailcat peers on
# ONE machine still cannot find each other without a relay running. This builds
# one, into `dist/derper/derper`, which is gitignored like every other artifact
# under `dist/`.
#
#   ./scripts/build-derper.sh
#
# **This is a test dependency and nothing else.** No shipped binary contains a
# derper, no release path calls this, and `cargo build` and
# `cargo test --workspace` keep working with no Go toolchain anywhere — the
# default build of `farcooler-tailcat` is the stub, and that is deliberate (see
# that crate's `Cargo.toml`). The only thing that needs what this produces is
# the end-to-end tunnel test, which is off by default too.
#
# The version is READ from `crates/tailcat/go/go.mod` rather than written here
# or taken as `@latest`. A relay from a different tailscale.com than the one the
# tunnel is built against is a second variable in the one test whose whole
# purpose is that three real components agree; and `@latest` would make this
# script's output depend on the day it ran.
set -euo pipefail

cd "$(dirname "$0")/.."

# Homebrew's Go is not on every shell's PATH on this machine. Appended, not
# prepended and not exported alone, so nothing already on PATH is shadowed or
# lost — the same line, for the same reason, as `build-tailcat.sh`.
export PATH="$PATH:/opt/homebrew/bin"

OUT="dist/derper"

command -v go >/dev/null 2>&1 || {
  cat <<'EOF'
No Go toolchain, so a local DERP relay cannot be built.

  brew install go

This is only needed to RUN the end-to-end tunnel test. `cargo build` and
`cargo test --workspace` work without it.
EOF
  exit 1
}

# The exact tailscale.com the tunnel is built against. `go.mod`'s require block
# indents with a tab; the grep is anchored on that so a comment mentioning the
# module elsewhere in the file cannot be mistaken for the requirement.
VERSION="$(awk '/^\ttailscale\.com v/ { print $2; exit }' crates/tailcat/go/go.mod)"
[ -n "$VERSION" ] || {
  echo "no tailscale.com requirement found in crates/tailcat/go/go.mod" >&2
  exit 1
}
echo "building derper from tailscale.com $VERSION"

# A module of its own, in a scratch directory, and NOT `crates/tailcat/go`.
# `cmd/derper` pulls in setec and prometheus, which the tunnel does not use and
# which would otherwise land in the shipped module's `go.mod` and `go.sum` — a
# test dependency quietly becoming a dependency of the thing under test.
BUILD="$(mktemp -d)"
trap 'rm -rf "$BUILD"' EXIT

(
  cd "$BUILD"
  go mod init farcooler/derper-for-tests >/dev/null
  go get "tailscale.com/cmd/derper@$VERSION" >/dev/null
  go build -o "$BUILD/derper" tailscale.com/cmd/derper
)

# Removed up front so a failed build leaves nothing linkable rather than
# something stale, and `mv`'d into place so a successful one is atomic. The
# same discipline as `build-tailcat.sh`, arrived at there by reproducing the
# failure it prevents: a build that exits non-zero while leaving an artifact
# from an older commit at the path its caller reads.
mkdir -p "$OUT"
rm -f "$OUT/derper"
mv "$BUILD/derper" "$OUT/derper"
ls -l "$OUT/derper"
