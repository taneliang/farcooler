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
  arm64|aarch64) ARCH=aarch64; TARGET=aarch64-unknown-linux-musl ;;
  x86_64|amd64)  ARCH=x86_64;  TARGET=x86_64-unknown-linux-musl ;;
  *) echo "unknown architecture: $ARCH (try x86_64 or aarch64)"; exit 1 ;;
esac

./scripts/build-tailcat.sh "linux-musl-$ARCH"
export RUSTFLAGS="${RUSTFLAGS:-} -L $PWD/dist/tailcat/linux-musl-$ARCH -l static=tailcat"

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

echo "==> Building $TARGET"
if command -v cross >/dev/null 2>&1; then
  cross build --release --target "$TARGET" -p farcooler-daemon -p farcooler-cli --features farcooler-tailcat/linked
else
  cargo build --release --target "$TARGET" -p farcooler-daemon -p farcooler-cli --features farcooler-tailcat/linked
fi

mkdir -p "$OUT"
cp "target/$TARGET/release/farcoolerd" "$OUT/"
cp "target/$TARGET/release/farcooler" "$OUT/"

echo
echo "Built $OUT"
ls -lh "$OUT"
echo
echo "Install with:  farcooler host install user@host"
