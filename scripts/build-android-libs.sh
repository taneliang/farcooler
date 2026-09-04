#!/bin/bash
# Build the Rust cores as one shared object per Android ABI.
#
# The counterpart to build-ios-frameworks.sh, and simpler for one reason: an
# APK holds a directory per ABI, so several architectures coexist without the
# packaging gymnastics an XCFramework exists to solve. Each `.so` goes straight
# into `jniLibs/<abi>/` and Gradle picks them up with no configuration.
#
# TWO `.so` per ABI, since the tunnel arrived: `libfarcooler_jni.so` and the Go
# tunnel it links against, `libtailcat.so`. Android is the one platform that
# links the tunnel dynamically — `go build -buildmode=c-archive` refuses
# GOOS=android — so the archive every other platform bundles at Rust build time
# is a loaded library here. `scripts/build-tailcat.sh android-arm64` builds it
# and this script links and stages it; `Native.kt` loads it first.
#
#   ./scripts/build-android-libs.sh              # arm64 only — the phone
#   ./scripts/build-android-libs.sh --emulator   # plus x86_64 for the emulator
#
# The NDK is found through ANDROID_NDK_HOME, ANDROID_HOME/ndk/*, or the default
# SDK location, in that order. Say so rather than failing inside cargo with a
# linker error nobody can read.
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

# The NDK lookup, and the `minSdk` floor it honors, live in `android-ndk.sh`
# because `build-tailcat.sh` needs the very same answer: the Go tunnel and the
# Rust core are linked into one app and must be compiled against one NDK, at
# one API level. Two copies of that discovery is the divergence this file used
# to be half of.
. scripts/android-ndk.sh

WITH_EMULATOR=0
[ "${1:-}" = "--emulator" ] && WITH_EMULATOR=1

# --- Find the NDK -----------------------------------------------------------

android_toolchain

OUT="apps/android/app/src/main/jniLibs"

# Where cargo puts its output. Cargo honors CARGO_TARGET_DIR on its own; the
# copies at the end of `build` have to be told, or a build with one set stages
# nothing — or worse, stages whatever an older build left in `target/`.
TARGET_DIR="${CARGO_TARGET_DIR:-target}"

# --- Build ------------------------------------------------------------------

# One target per ABI, named the way both cargo and Android know them. The two
# vocabularies do not match — `aarch64-linux-android` is `arm64-v8a` to Gradle —
# so the mapping is written down once here rather than inferred anywhere.
build() {
  local target="$1" abi="$2" prefix="$3" tailcat="$4"

  # The same wrapper `build-tailcat.sh` compiles the Go half with, from the
  # same lookup, so the two objects that end up linked together were built
  # against one NDK at one API level.
  local cc
  cc="$(ndk_cc "$prefix")"

  echo "==> $abi ($(basename "$cc"))"

  # The tunnel first: cargo needs the `.so` on disk to link against.
  #
  # `build-tailcat.sh` refuses with its own actionable message if Go is
  # missing, so this script does not repeat the check. It does refuse to go on
  # without it: a core that cannot reach a Tailcat runner is not a smaller
  # feature, it is a runner this app cannot connect to at all — `Reach` is one
  # per runner with no fallback (`crates/client/src/ssh.rs`), and the choice is
  # made when the runner is set up, where this app cannot see it.
  ./scripts/build-tailcat.sh "$tailcat" || {
    echo "    the tunnel did not build; refusing to stage a core that cannot dial one" >&2
    exit 1
  }

  # Cargo's per-target linker variable, upper-cased with dashes as underscores.
  # Set for this invocation only: a global `.cargo/config.toml` entry would make
  # every build on this machine depend on one NDK version being installed.
  local var
  var="CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_LINKER"

  # `cargo rustc`, and `-l dylib=` rather than `-l static=`.
  #
  # Two departures from the other platform scripts, one shared reason and one
  # of Android's own:
  #
  #   - `cargo rustc`'s trailing `--` args reach the FINAL crate only, which is
  #     what should carry the link. A plain `RUSTFLAGS` reaches every crate in
  #     the graph, and with `-l static=` that once bundled a copy of the
  #     archive into all 142 rlibs and produced a 6.4 GB artifact (see
  #     `scripts/build-ios-frameworks.sh`). A dynamic link cannot repeat that
  #     trap, but the scoping is kept anyway: one rule, not two.
  #   - `dylib`, because Go cannot produce an archive for Android at all. The
  #     `.so` is linked against here and LOADED at runtime by `Native.kt`, and
  #     it is `libtailcat.so` in `jniLibs/<abi>/` that satisfies the
  #     DT_NEEDED this puts on `libfarcooler_jni.so`.
  #
  # `--features tailcat` is this crate's mirror of
  # `farcooler-tailcat/linked`, the same indirection `farcooler-client` and
  # `farcooler-daemon` use, so `cfg(feature = "tailcat")` reads true inside the
  # crates that ask. The `-L` MUST be absolute: `$PWD` is the repo root, since
  # this script `cd`s there at the top.
  env \
    "$var=$cc" \
    "CC_${target//-/_}=$cc" \
    "AR_${target//-/_}=$TOOLCHAIN/bin/llvm-ar" \
    cargo rustc --release -p farcooler-jni --target "$target" --features tailcat \
    -- -L "$PWD/dist/tailcat/$tailcat" -l dylib=tailcat

  mkdir -p "$OUT/$abi"
  cp "$TARGET_DIR/$target/release/libfarcooler_jni.so" "$OUT/$abi/"
  cp "dist/tailcat/$tailcat/libtailcat.so" "$OUT/$abi/"
}

build aarch64-linux-android arm64-v8a aarch64-linux-android android-arm64

if [ "$WITH_EMULATOR" = "1" ]; then
  build x86_64-linux-android x86_64 x86_64-linux-android android-x86_64
fi

echo
echo "Built:"
find "$OUT" -name '*.so' -exec ls -lh {} \; | awk '{print "  " $NF " (" $5 ")"}'
