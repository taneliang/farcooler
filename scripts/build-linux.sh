#!/bin/bash
# Build the Linux daemon, CLI and tunnel helper that `farcooler host install`
# uploads.
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

OUT="dist/$ARCH-linux"

# `cross` cannot build what this script ships, so its presence does not
# satisfy this check on its own — a NATIVE `$TARGET` is required regardless.
#
# The reason changed with the tunnel and is worth restating rather than
# inheriting. It used to be the absolute `-L` this script handed the daemon's
# link step, which a container mounting this repo at a path of its own choosing
# could not resolve. There is no `-L` any more: Linux does not link the Go
# archive at all, it ships a separate helper (see the Build step below). What
# remains is simpler and still fatal — this script produces a `dist/` layout
# with THREE binaries in it, and `cross` can produce two. A tarball missing the
# helper installs a runner that answers `no_tailcat` to every tunnel, silently,
# which is exactly the failure this whole path exists to end. So the check
# stays: one command, one complete layout, and the same binaries a laptop
# makes.
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
     than serving one — it is built without \`tailcat-helper\`, and there is no
     helper beside it either way.
EOF
  exit 1
fi

# This is a shipping build, unlike `cargo build`/`cargo test --workspace` —
# those stay on the stub in `farcooler-tailcat` and need no Go at all.
# `build-tunnel-helper.sh` refuses with its own actionable message if Go is
# missing, so this script does not repeat the check; it just needs the binary
# it produces.
#
# Linux serves its tunnel from this separate process rather than from a Go
# c-archive linked into `farcoolerd`, and that is not a preference. An archive
# linked into a musl binary segfaults inside Go's runtime startup before it can
# log a line — measured, reproduced with a five-line Go program, and not fixed
# by any thread stack size. `scripts/build-tunnel-helper.sh` carries the
# numbers. iOS and macOS still link the archive; it works there and it ships.
./scripts/build-tunnel-helper.sh "$ARCH"

echo "==> Building $TARGET"

# Plain `cargo`, never `cross` — see the comment above the toolchain check.

# The CLI builds plain, with no tunnel feature, and on Linux that is not a
# choice being deferred. `farcooler runner pipe` dials, and the helper backend
# this platform uses cannot dial at all — `crates/tailcat/src/helper.rs` says
# why: a dialed tunnel is a file descriptor, and a descriptor cannot cross the
# helper's pipe as a word. Its `dial` returns `NoTailcatLinked` outright. The
# other backend, `linked`, would need the Go archive and cgo here, which is
# what the musl segfault in that same file rules out.
#
# So a Linux runner SERVES a tunnel and does not dial one. `farcooler runner
# pipe` on this binary says "this build of farcooler has no tunnel it can
# dial", which is the truth about it.
cargo build --release --target "$TARGET" -p farcooler-cli

# `--features tailcat-helper` is the whole difference between a daemon that
# serves a tunnel and one that answers `no_tailcat`. It selects
# `farcooler-tailcat`'s helper backend, which links no Go and needs none at
# Rust build time at all — so this is a plain `cargo build` again.
#
# It used to be a `cargo rustc` carrying `-L`/`-l static=tailcat` scoped to
# this one invocation, and the scoping mattered: `-l static=` bundles the named
# archive's objects into whichever crate it is attached to, so a global
# `RUSTFLAGS` once duplicated a 47.6 MB archive into all 142 rlibs in the
# dependency graph and produced a 6.4 GB artifact. That trap is gone from
# Linux with the archive, but it is still live on the two platforms that do
# link one — see `apps/macos/build-app.sh` and
# `scripts/build-ios-frameworks.sh`, which both still carry it.
cargo build --release --target "$TARGET" -p farcooler-daemon --bin farcoolerd \
  --features tailcat-helper

mkdir -p "$OUT"
cp "target/$TARGET/release/farcoolerd" "$OUT/"
cp "target/$TARGET/release/farcooler" "$OUT/"
# Beside the daemon, which is where the daemon looks: `helper_path` reads the
# directory `farcoolerd` itself is in, the same way `farcooler` finds
# `farcoolerd` next to itself. A `dist/` without this file is a runner that
# answers `no_tailcat`.
cp "dist/tunnel/$ARCH-linux/farcooler-tunnel" "$OUT/"

echo
echo "Built $OUT"
ls -lh "$OUT"
echo
echo "Install with:  farcooler host install user@host"
