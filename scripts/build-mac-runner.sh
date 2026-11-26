#!/bin/bash
# Build the native Mac runner and CLI with tunnel support.
set -euo pipefail

cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=26.0
target="darwin-$(uname -m)"
archive="${FARCOOLER_TAILCAT_OUTPUT_DIR:-$PWD/dist/tailcat/$target}"
FARCOOLER_TAILCAT_OUTPUT_DIR="$archive" ./scripts/build-tailcat.sh "$target"
FARCOOLER_BUNDLE_TAILCAT_DIR="$archive" \
    cargo build --release --locked -p farcooler-cli -p farcooler-daemon --bins \
    --features farcooler-cli/tailcat,farcooler-daemon/tailcat
