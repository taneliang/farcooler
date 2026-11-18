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
      # Only this crate depends on farcooler-tailcat, so only its build gets
      # the archive and the feature that asks for it — farcooler-vt and
      # farcooler-review would fail outright: cargo refuses a feature naming
      # a package that is not in that crate's own dependency graph.
      #
      # `cargo rustc -p`, not `cargo build` under a global `RUSTFLAGS`. No
      # linker runs at this step — crate-type is `staticlib, rlib`, so cargo
      # only archives object files — but `-l static=` still BUNDLES the
      # named archive's objects into whichever crate it is attached to,
      # unpacked, at compile time. A plain `RUSTFLAGS` applies to every crate
      # rustc compiles for this target, so it bundled a copy of the 47 MB
      # archive into all 142 rlibs in the dependency graph: a 6.8 GB
      # staticlib and a 6.4 GB XCFramework, found the hard way, by actually
      # measuring one. `cargo rustc`'s trailing `--` args go to the final
      # crate only, so exactly one copy lands, in `farcooler-client` itself,
      # which is what should carry it.
      #
      # The `-L` MUST be absolute: `$PWD` here is the repo root (the script
      # `cd`s there at the top), so this is, but a relative path silently
      # fails to resolve. `-l static=tailcat` MUST be explicit too: nothing
      # in this crate has `#[link(name = "tailcat")]` or a `build.rs`, so
      # without it nothing bundles the archive at all.
      #
      # `--features tailcat`, not `farcooler-tailcat/linked` directly: this
      # crate's own `tailcat` feature (`Cargo.toml`) mirrors the dependency's
      # `linked` feature so `cfg(feature = "tailcat")` reads true inside
      # `farcooler-client` itself. Enabling the dependency's feature straight
      # bypasses that mirror — the archive still links, but the crate cannot
      # tell it did.
      ./scripts/build-tailcat.sh ios-arm64
      SDKROOT="$DEVICE_SDK" cargo rustc -p "$CRATE" --release \
        --target aarch64-apple-ios --features tailcat \
        -- -L "$PWD/dist/tailcat/ios-arm64" -l static=tailcat
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
