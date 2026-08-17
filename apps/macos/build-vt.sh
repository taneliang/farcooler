#!/bin/bash
# Build the Rust cores the Mac app links, and stage what SwiftPM needs.
#
# SwiftPM cannot build a Rust crate, so this bridges the two: it compiles the
# static libraries and copies the terminal core's header next to its module map.
# crates/vt/include is the single source of truth — the copy here is generated
# and gitignored, so the two cannot drift into disagreeing about the ABI.
#
# Two libraries now, not one. The terminal core has always been here; the client
# core arrived with device onboarding, because every rule about whether a
# scanned code is acceptable lives in crates/client/src/ceremony.rs and a Mac
# that could not call it would be a third implementation of those rules. The
# name is still build-vt.sh: build-app.sh, the release script and everyone's
# muscle memory call it by that name, and renaming it buys a tidier filename at
# the price of a build that fails for whoever has not pulled.
set -euo pipefail

# A double-clicked build, or one from an IDE, inherits no shell profile.
export PATH="$HOME/.cargo/bin:$PATH"

cd "$(dirname "$0")"
ROOT="$(cd ../.. && pwd)"
CONFIG="${1:-release}"

# The same floor Package.swift declares.
#
# The client core compiles C — BoringSSL, by way of russh — and `cc` targets
# whatever SDK is installed unless told otherwise. Left alone, every one of
# those object files is stamped for a newer macOS than the app is linked for,
# and `ld` says so once per object: two hundred warnings in a build log, which
# is exactly how many it takes for nobody to read the one that matters.
export MACOSX_DEPLOYMENT_TARGET=26.0

echo "==> Building the terminal and client cores ($CONFIG)"
if [ "$CONFIG" = "debug" ]; then
  (cd "$ROOT" && cargo build -p farcooler-vt -p farcooler-client)
else
  (cd "$ROOT" && cargo build --release -p farcooler-vt -p farcooler-client)
fi

for NAME in farcooler_vt farcooler_client; do
  LIB="$ROOT/target/$CONFIG/lib$NAME.a"
  [ -f "$LIB" ] || { echo "no static library at $LIB"; exit 1; }

  # Package.swift links from target/release. A debug build is staged there too
  # so `build-vt.sh debug` produces something the app can actually link against,
  # rather than silently linking a stale release library.
  if [ "$CONFIG" = "debug" ]; then
    mkdir -p "$ROOT/target/release"
    cp "$LIB" "$ROOT/target/release/lib$NAME.a"
  fi

  echo "    $(du -h "$LIB" | cut -f1)  $LIB"
done

# Only the terminal core's header is staged. The client's module map points at
# crates/client/include where it lives, so there is no copy of that one to go
# stale — see Sources/CFarCoolerClient/module.modulemap.
echo "==> Staging the header"
cp "$ROOT/crates/vt/include/farcooler_vt.h" Sources/CFarCoolerVT/farcooler_vt.h
