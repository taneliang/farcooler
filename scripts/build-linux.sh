#!/bin/bash
# Build the Linux daemon and CLI that `farcooler host install` uploads.
#
# Linux hosts get static musl binaries. A glibc build made on one distribution
# refuses to start on an older one, and "works on my machine, GLIBC_2.38 not
# found on yours" is the single most common way a self-installed daemon fails.
# musl links everything in, so one binary runs on Debian, Ubuntu, Alpine, Arch
# and a Synology NAS alike.
#
#   ./scripts/build-linux.sh            # for this Mac's architecture
#   ./scripts/build-linux.sh x86_64     # for an Intel/AMD host
#   ./scripts/build-linux.sh aarch64    # for an ARM host
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

ARCH="${1:-$(uname -m)}"
case "$ARCH" in
  arm64|aarch64) ARCH=aarch64; TARGET=aarch64-unknown-linux-musl; TAILCAT_TARGET=linux-musl-aarch64 ;;
  x86_64|amd64)  ARCH=x86_64;  TARGET=x86_64-unknown-linux-musl; TAILCAT_TARGET=linux-musl-x86_64 ;;
  *) echo "unknown architecture: $ARCH (try x86_64 or aarch64)"; exit 1 ;;
esac

OUT="dist/$ARCH-linux"

if ! command -v cross >/dev/null 2>&1 && ! rustup target list --installed | grep -q "^$TARGET$"; then
  cat <<EOF
Cannot build $TARGET yet. Pick one:

  1. cross (needs Docker or Podman, no other setup):
         cargo install cross
         ./scripts/build-linux.sh $ARCH

  2. A native toolchain:
         rustup target add $TARGET
         brew install FiloSottile/musl-cross/musl-cross
     then set the linker in .cargo/config.toml.

  3. Build on the host itself:
         ssh HOST 'git clone <repo> && cd farcooler && cargo build --release'
     and install with:  farcooler host install HOST --from <that>/target/release
EOF
  exit 1
fi

# This is a shipping build, unlike `cargo build`/`cargo test --workspace` —
# those stay on the stub in `farcooler-tailcat` and need no Go at all.
# `build-tailcat.sh` refuses with its own actionable message if Go is
# missing, so this script does not repeat the check; it just needs the
# archive it produces.
./scripts/build-tailcat.sh "$TAILCAT_TARGET"
# Absolute, not relative: a relative `-L` silently fails to resolve. The
# script `cd`s to the repo root at the top, so `$PWD` is that root.
TAILCAT_DIR="$PWD/dist/tailcat/$TAILCAT_TARGET"

echo "==> Building $TARGET"
if command -v cross >/dev/null 2>&1; then
  BUILD=(cross)
else
  BUILD=(cargo)
fi

# `farcooler-cli` does not depend on `farcooler-tailcat` yet — dialing the
# tunnel from the CLI is a later task — so it builds plain.
"${BUILD[@]}" build --release --target "$TARGET" -p farcooler-cli

# `-p farcooler-daemon` alone, through `rustc` rather than `build`, and
# scoped to exactly one target with `--bin farcoolerd`: the package carries
# both a lib and a bin, and `cargo rustc`/`cross rustc` refuse trailing `--`
# args unless exactly one target is selected.
#
# NOT a global `RUSTFLAGS`. That applies to every crate rustc compiles for
# this target, and `-l static=` bundles the named archive's objects into
# whichever crate it is attached to at compile time — a plain `RUSTFLAGS`
# duplicated a 47.6 MB archive into all 142 rlibs in the dependency graph and
# produced a 6.4 GB artifact, found the hard way while wiring the iOS build
# (see `scripts/build-ios-frameworks.sh`'s comment on the same trap). Scoping
# the flags to this one `rustc` invocation is what keeps it to one copy.
"${BUILD[@]}" rustc --release --target "$TARGET" -p farcooler-daemon --bin farcoolerd \
  --features tailcat -- -L "$TAILCAT_DIR" -l static=tailcat

mkdir -p "$OUT"
cp "target/$TARGET/release/farcoolerd" "$OUT/"
cp "target/$TARGET/release/farcooler" "$OUT/"

echo
echo "Built $OUT"
ls -lh "$OUT"
echo
echo "Install with:  farcooler host install user@host"
