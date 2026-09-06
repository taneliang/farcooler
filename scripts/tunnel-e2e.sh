#!/bin/bash
# Run the one end-to-end tunnel test: a real DERP relay, a real tailcat server
# and client, and a real sshd.
#
#   ./scripts/tunnel-e2e.sh
#   ./scripts/tunnel-e2e.sh a_revoked_device        # one test by name
#
# CI runs this from the `tunnel-e2e` job in `.github/workflows/ci.yml`. Nothing
# else runs it: `cargo test --workspace`, the `rust` job's last step, builds
# default features, and `crates/daemon/tests/a_real_tunnel_carries_the_scope.rs`
# is `cfg`'d away without `tailcat`.
#
# ## The three things this supplies that a plain `cargo test` cannot
#
# 1. **The Go archive, linked into this test binary and into `farcoolerd`.**
#    `farcooler-tailcat`'s default is the stub, on purpose, so a checkout with
#    no Go toolchain still builds — which also means the only backend that runs
#    in production is not the one `cargo test` exercises.
#
# 2. **A DERP relay on loopback.** DERP is the rendezvous for every tunneled
#    connection, so two peers in ONE process still cannot find each other with
#    no relay running.
#
# 3. **`TS_DEBUG_USE_DERP_HTTP=1`, in the environment before this runs.** The
#    relay speaks plain HTTP and `InsecureForTests` in the DERP map is not
#    enough — see `crates/daemon/tests/support/derper.rs` for the whole trap.
#    It has to be exported here rather than set from inside the test because Go
#    copies the environment at runtime startup, which in a `c-archive` build
#    happens before `main`.
#
# ## Why `RUSTFLAGS` and not `cargo rustc -- -l static=tailcat`
#
# `cargo rustc`'s trailing arguments reach the FINAL crate only, and cargo
# builds this package's `farcoolerd` binary as part of any integration test —
# so the test target would link and the binary would fail with four undefined
# `fc_tailcat_*` symbols. Measured, not assumed.
#
# `-C link-arg` is what makes a global `RUSTFLAGS` safe here where a global
# `-l static=tailcat` was not: link arguments are used only when rustc actually
# runs a linker, which is for executables and never for an rlib. The 6.4 GB
# XCFramework that `-l static=` in `RUSTFLAGS` once produced — 142 rlibs each
# bundling a 47.6 MB archive — cannot happen this way, and the check is one
# command:
#
#   for f in target/debug/deps/*.rlib; do ar t "$f" | grep -c '^go\.o'; done
#
# A dedicated `CARGO_TARGET_DIR` because `RUSTFLAGS` is part of every unit's
# fingerprint: sharing `target/` with ordinary builds would make each of them
# rebuild the whole graph after the other.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

export PATH="$PATH:/opt/homebrew/bin"

# macOS only, and stated rather than discovered. `build-tailcat.sh` has no
# native-glibc Linux target — its two Linux targets are musl, where a linked Go
# c-archive segfaults in Go's runtime startup before it can print anything
# (`docs/releasing.md`, and `crates/tailcat/src/helper.rs` for why Linux ships a
# spawned helper instead). The helper backend cannot dial at all: a descriptor
# cannot cross its pipe as a word. So the dialing half of this test has nowhere
# to run on Linux today, and saying so beats a mystery.
case "$(uname -s)" in
  Darwin) TAILCAT_TARGET=darwin-arm64 ;;
  *)
    echo "This test runs on macOS only today; see the comment above this line." >&2
    exit 1
    ;;
esac

[ "$(uname -m)" = "arm64" ] || {
  echo "no tailcat archive target for $(uname -m); this expects an Apple silicon Mac" >&2
  exit 1
}

echo "==> the Go archive"
./scripts/build-tailcat.sh "$TAILCAT_TARGET" >/dev/null
ARCHIVE="$ROOT/dist/tailcat/$TAILCAT_TARGET/libtailcat.a"
[ -f "$ARCHIVE" ] || { echo "no archive at $ARCHIVE" >&2; exit 1; }

echo "==> the DERP relay"
if [ ! -x "$ROOT/dist/derper/derper" ]; then
  ./scripts/build-derper.sh >/dev/null
fi
[ -x "$ROOT/dist/derper/derper" ] || { echo "no derper at dist/derper/derper" >&2; exit 1; }

# The three frameworks Go's own runtime and `crypto/x509` need on darwin, which
# Xcode supplies to an app build and nothing supplies to a bare cargo test
# binary. Without them the link fails with undefined `_SCDynamicStore*` and
# `_SecCertificate*` symbols, which is almost certainly why the linked backend
# had never been executed by a test before.
export RUSTFLAGS="-Clink-arg=$ARCHIVE \
  -Clink-arg=-framework -Clink-arg=Security \
  -Clink-arg=-framework -Clink-arg=CoreFoundation \
  -Clink-arg=-framework -Clink-arg=SystemConfiguration"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/tunnel-e2e}"

# See the header. Both peers are this one process, so one export covers both.
export TS_DEBUG_USE_DERP_HTTP=1

echo "==> the test"
# `--nocapture` because the interesting failure is a dial that spends thirty
# seconds arriving at the wrong reason, and the harness prints what it measured
# as it goes.
exec cargo test -p farcooler-daemon --features tailcat \
  --test a_real_tunnel_carries_the_scope -- --nocapture "$@"
