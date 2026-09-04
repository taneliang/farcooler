# The Android NDK, found once for every script that needs one.
#
# Sourced, never executed. Both callers `cd` to the repository root first, so
# the path is written from there:
#
#     . scripts/android-ndk.sh
#     android_toolchain                     # sets NDK and TOOLCHAIN
#     cc="$(ndk_cc aarch64-linux-android)"  # the clang wrapper to compile with
#
# Two scripts need this lookup and they must agree. `build-android-libs.sh`
# compiles the Rust cores and `build-tailcat.sh` compiles the Go tunnel, and
# the two halves are linked together into one app: a second copy of this
# discovery is how they would come to disagree — one finding a newer NDK than
# the other, or one pinning an API level the other discovered — and the
# disagreement surfaces as a link error against a platform symbol one half was
# allowed to reference and the other was not. One copy, called from both.
#
# No `set -e` here: both callers set their own, and a sourced file changing
# the caller's shell options is a surprise nobody reads this file to find.

# The oldest Android the app supports — `minSdk` in the Gradle build.
#
# The NDK names a linker wrapper after the API level it targets, and that level
# is a FLOOR, not a ceiling: it decides which platform symbols a `.so` may
# reference, so anything at or below `minSdk` is correct and anything above it
# would link against symbols a supported device might not have. An NDK release
# only ships wrappers up to the platform it was cut alongside, so asking for
# exactly `minSdk` fails whenever the NDK is a release or two behind the OS —
# which it usually is. `wrapper_api` below picks the highest one that is safe.
MIN_SDK=37

# ANDROID_NDK_HOME, then ANDROID_HOME/ndk/*, then the default SDK location.
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

# Sets NDK and TOOLCHAIN, or says how to install one and exits. Called
# explicitly rather than on source, so a script that also builds non-Android
# targets does not demand an NDK to build one of those.
android_toolchain() {
  NDK="$(find_ndk)"
  if [ -z "$NDK" ]; then
    echo "No Android NDK found." >&2
    echo "Set ANDROID_NDK_HOME, or install one:" >&2
    echo "  sdkmanager 'ndk;29.0.14206865'" >&2
    exit 1
  fi

  local host="darwin-x86_64"
  case "$(uname -s)" in
    Linux) host="linux-x86_64" ;;
  esac
  # The NDK ships one prebuilt toolchain directory whose name is the HOST it
  # runs on, not the host it targets — `darwin-x86_64` is correct on Apple
  # silicon too, because the binaries inside are universal.
  TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/$host"
  [ -d "$TOOLCHAIN" ] || { echo "NDK toolchain missing at $TOOLCHAIN" >&2; exit 1; }
}

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

# The compiler for one ABI, by the triple prefix the NDK names its wrappers
# after: `aarch64-linux-android`, `x86_64-linux-android`. Both cargo and cgo
# want this same string.
#
# `return 1` rather than `exit 1`, because this is meant to be called in a
# command substitution and an `exit` inside one only ends the subshell. Callers
# must assign on a line of their own — `local cc; cc="$(ndk_cc ...)"` — since
# `local cc="$(...)"` takes `local`'s exit status, not the substitution's, and
# `set -e` would then let an empty compiler through.
ndk_cc() {
  local prefix="$1" api
  api="$(wrapper_api "$prefix")"
  if [ -z "$api" ]; then
    echo "This NDK has no $prefix wrapper at or below API $MIN_SDK." >&2
    return 1
  fi
  echo "$TOOLCHAIN/bin/${prefix}${api}-clang"
}
