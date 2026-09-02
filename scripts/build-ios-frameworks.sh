#!/bin/bash
# Build the Rust cores as XCFrameworks for the iOS app.
#
# An XCFramework is the only packaging Xcode accepts that carries several
# architectures without them colliding: a device slice and a simulator slice are
# both arm64 on Apple silicon, so a fat archive cannot hold them and Xcode picks
# the wrong one. This is why `lipo` is not the answer here.
#
#   ./scripts/build-ios-frameworks.sh            # simulator only, fast
#   ./scripts/build-ios-frameworks.sh --device   # both slices
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

# Xcode's own toolchain, without needing `sudo xcode-select`. DEVELOPER_DIR is
# the documented override and it is per-process, so it changes nothing globally.
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
[ -d "$DEVELOPER_DIR" ] || { echo "Xcode not found at $DEVELOPER_DIR"; exit 1; }

WITH_DEVICE=0
[ "${1:-}" = "--device" ] && WITH_DEVICE=1

SIM_SDK="$DEVELOPER_DIR/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk"
DEVICE_SDK="$DEVELOPER_DIR/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"

OUT="apps/ios/Frameworks"
rm -rf "$OUT"
mkdir -p "$OUT"

for CRATE in farcooler-vt farcooler-client farcooler-review; do
  LIB="lib$(echo "$CRATE" | tr '-' '_').a"
  NAME="$(echo "$CRATE" | sed 's/farcooler-//')"

  echo "==> $CRATE for the simulator"
  SDKROOT="$SIM_SDK" cargo build --release -p "$CRATE" --target aarch64-apple-ios-sim

  # Headers are staged into a subdirectory named for the module.
  #
  # Every framework's headers land in the same `include/` in the build
  # products, so two crates each shipping a `module.modulemap` at the top
  # level collide and the build fails with "multiple commands produce".
  # A subdirectory per module keeps them apart.
  STAGE="target/ios-headers/$NAME"
  rm -rf "$STAGE"
  mkdir -p "$STAGE/$NAME"
  cp "crates/$NAME/include/"* "$STAGE/$NAME/"

  ARGS=(-library "target/aarch64-apple-ios-sim/release/$LIB" -headers "$STAGE")

  if [ "$WITH_DEVICE" = "1" ]; then
    echo "==> $CRATE for the device"
    if [ "$CRATE" = "farcooler-client" ]; then
      # The tunnel is only linked into the device slice. A device archive in a
      # simulator build fails at link (see the module comment above and the
      # standing note on iOS framework slices), so the simulator build just
      # above this stays on the stub, unconditionally, until someone builds a
      # simulator archive too.
      ./scripts/build-tailcat.sh ios-arm64
      SDKROOT="$DEVICE_SDK" RUSTFLAGS="${RUSTFLAGS:-} -L $PWD/dist/tailcat/ios-arm64 -l static=tailcat" \
        cargo build --release -p "$CRATE" --target aarch64-apple-ios --features farcooler-tailcat/linked
      cp dist/tailcat/ios-arm64/libtailcat.a "$OUT/"
    else
      SDKROOT="$DEVICE_SDK" cargo build --release -p "$CRATE" --target aarch64-apple-ios
    fi
    ARGS+=(-library "target/aarch64-apple-ios/release/$LIB" -headers "$STAGE")
  fi

  echo "==> Packaging FarCooler$(tr '[:lower:]' '[:upper:]' <<< "${NAME:0:1}")${NAME:1}.xcframework"
  FRAMEWORK="$OUT/farcooler_$NAME.xcframework"
  xcodebuild -create-xcframework "${ARGS[@]}" -output "$FRAMEWORK" >/dev/null
done

echo
echo "Built:"
ls -1 "$OUT"
