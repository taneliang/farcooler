#!/bin/bash
# Build the Rust cores as one shared object per Android ABI.
#
# The counterpart to build-ios-frameworks.sh, and simpler for one reason: an
# APK holds a directory per ABI, so several architectures coexist without the
# packaging gymnastics an XCFramework exists to solve. Each `.so` goes straight
# into `jniLibs/<abi>/` and Gradle picks them up with no configuration.
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

# The oldest Android the app supports — `minSdk` in the Gradle build.
#
# The NDK names a linker wrapper after the API level it targets, and that level
# is a FLOOR, not a ceiling: it decides which platform symbols the `.so` may
# reference, so anything at or below `minSdk` is correct and anything above it
# would link against symbols a supported device might not have. An NDK release
# only ships wrappers up to the platform it was cut alongside, so asking for
# exactly `minSdk` fails whenever the NDK is a release or two behind the OS —
# which it usually is. `wrapper_api` below picks the highest one that is safe.
MIN_SDK=37

WITH_EMULATOR=0
[ "${1:-}" = "--emulator" ] && WITH_EMULATOR=1

# --- Find the NDK -----------------------------------------------------------

find_ndk() {
  if [ -n "${ANDROID_NDK_HOME:-}" ] && [ -d "$ANDROID_NDK_HOME" ]; then
    echo "$ANDROID_NDK_HOME"
    return
  fi
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
  # Highest version, so a machine with several installed uses the newest rather
  # than whichever the shell happened to glob first.
  local newest
  newest="$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1 || true)"
  [ -n "$newest" ] && echo "$newest"
}

NDK="$(find_ndk)"
if [ -z "$NDK" ]; then
  echo "No Android NDK found." >&2
  echo "Set ANDROID_NDK_HOME, or install one:" >&2
  echo "  sdkmanager 'ndk;29.0.14206865'" >&2
  exit 1
fi

HOST="darwin-x86_64"
case "$(uname -s)" in
  Linux) HOST="linux-x86_64" ;;
esac
# The NDK ships one prebuilt toolchain directory whose name is the HOST it runs
# on, not the host it targets — `darwin-x86_64` is correct on Apple silicon too,
# because the binaries inside are universal.
TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/$HOST"
[ -d "$TOOLCHAIN" ] || { echo "NDK toolchain missing at $TOOLCHAIN" >&2; exit 1; }

OUT="apps/android/app/src/main/jniLibs"

# The highest API wrapper this NDK ships that is not newer than MIN_SDK.
#
# Discovered rather than hard-coded, because the two versions move
# independently: a newer NDK adds wrappers, a newer Android raises MIN_SDK, and
# pinning either one means the build breaks on somebody else's machine for a
# reason that has nothing to do with the change they made.
wrapper_api() {
  local prefix="$1" best=""
  for candidate in "$TOOLCHAIN/bin/$prefix"[0-9]*-clang; do
    [ -x "$candidate" ] || continue
    local level="${candidate##*/$prefix}"
    level="${level%-clang}"
    [ "$level" -le "$MIN_SDK" ] || continue
    [ -z "$best" ] || [ "$level" -gt "$best" ] || continue
    best="$level"
  done
  echo "$best"
}

# --- Build ------------------------------------------------------------------

# One target per ABI, named the way both cargo and Android know them. The two
# vocabularies do not match — `aarch64-linux-android` is `arm64-v8a` to Gradle —
# so the mapping is written down once here rather than inferred anywhere.
build() {
  local target="$1" abi="$2" prefix="$3"

  local api
  api="$(wrapper_api "$prefix")"
  if [ -z "$api" ]; then
    echo "This NDK has no $prefix wrapper at or below API $MIN_SDK." >&2
    exit 1
  fi
  local cc="$TOOLCHAIN/bin/${prefix}${api}-clang"

  echo "==> $abi (API $api)"
  # Cargo's per-target linker variable, upper-cased with dashes as underscores.
  # Set for this invocation only: a global `.cargo/config.toml` entry would make
  # every build on this machine depend on one NDK version being installed.
  local var
  var="CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_LINKER"
  env \
    "$var=$cc" \
    "CC_${target//-/_}=$cc" \
    "AR_${target//-/_}=$TOOLCHAIN/bin/llvm-ar" \
    cargo build --release -p farcooler-jni --target "$target"

  mkdir -p "$OUT/$abi"
  cp "target/$target/release/libfarcooler_jni.so" "$OUT/$abi/"
}

build aarch64-linux-android arm64-v8a aarch64-linux-android

if [ "$WITH_EMULATOR" = "1" ]; then
  build x86_64-linux-android x86_64 x86_64-linux-android
fi

echo
echo "Built:"
find "$OUT" -name '*.so' -exec ls -lh {} \; | awk '{print "  " $NF " (" $5 ")"}'
