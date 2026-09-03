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

# `cross` cannot build what this script ships, so its presence does not
# satisfy this check on its own — a NATIVE `$TARGET` is required regardless.
# Two independent reasons, both fatal: `cross` builds inside a Docker/Podman
# container that mounts this repo at a path of ITS choosing, so the absolute
# `-L "$TAILCAT_DIR"` this script hands `farcooler-daemon`'s link step below
# cannot resolve inside it — and the Go archive itself needs a Go toolchain
# plus a musl cross-compiler ON THIS HOST regardless of any container, which
# is exactly the setup `cross` exists so a contributor does not need. An
# earlier version of this script offered `cross` as a fix for a missing
# native target; it was not one, and the help text below no longer says it is.
HAVE_NATIVE_TARGET=0
rustup target list --installed 2>/dev/null | grep -q "^$TARGET$" && HAVE_NATIVE_TARGET=1

if [ "$HAVE_NATIVE_TARGET" -ne 1 ]; then
  cat <<EOF
Cannot build $TARGET with the tunnel linked — this script needs a NATIVE musl
target on this machine; \`cross\` cannot supply one (see the comment above this
message in the script for why). Pick one:

  1. A native toolchain:
         rustup target add $TARGET
         brew install FiloSottile/musl-cross/musl-cross
     then set the linker in .cargo/config.toml.

  2. Build on the host itself:
         ssh HOST 'git clone <repo> && cd farcooler && cargo build --release'
     and install with:  farcooler host install HOST --from <that>/target/release

  3. \`cross\` alone can still build $TARGET WITHOUT the tunnel, outside this
     script:
         cargo install cross
         cross build --release --target $TARGET -p farcooler-cli -p farcooler-daemon
     The resulting \`farcoolerd\` fails every tunnel with \`no_tailcat\` rather
     than serving one.
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

# Plain `cargo`, never `cross` — see the comment above the toolchain check:
# `cross`'s container can neither resolve this script's absolute `-L` nor
# supply the archive itself, and the check above already refused unless a
# native `$TARGET` is present.

# `farcooler-cli` does not depend on `farcooler-tailcat` yet — dialing the
# tunnel from the CLI is a later task — so it builds plain.
cargo build --release --target "$TARGET" -p farcooler-cli

# `-p farcooler-daemon` alone, through `rustc` rather than `build`, and
# scoped to exactly one target with `--bin farcoolerd`: the package carries
# both a lib and a bin, and `cargo rustc` refuses trailing `--` args unless
# exactly one target is selected.
#
# NOT a global `RUSTFLAGS`. That applies to every crate rustc compiles for
# this target, and `-l static=` bundles the named archive's objects into
# whichever crate it is attached to at compile time — a plain `RUSTFLAGS`
# duplicated a 47.6 MB archive into all 142 rlibs in the dependency graph and
# produced a 6.4 GB artifact, found the hard way while wiring the iOS build
# (see `scripts/build-ios-frameworks.sh`'s comment on the same trap). Scoping
# the flags to this one `rustc` invocation is what keeps it to one copy.
cargo rustc --release --target "$TARGET" -p farcooler-daemon --bin farcoolerd \
  --features tailcat -- -L "$TAILCAT_DIR" -l static=tailcat

mkdir -p "$OUT"
cp "target/$TARGET/release/farcoolerd" "$OUT/"
cp "target/$TARGET/release/farcooler" "$OUT/"

echo
echo "Built $OUT"
ls -lh "$OUT"
echo
echo "Install with:  farcooler host install user@host"
