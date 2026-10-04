#!/bin/bash
# Run the Mac app's tests against freshly built Rust cores.
#
#   apps/macos/test.sh [swift test arguments]     e.g.  --filter CeremonyTests
#
# Package.swift links `target/release/libfarcooler_vt.a` and
# `libfarcooler_client.a`, and SwiftPM cannot build a Rust crate. A bare
# `swift test` therefore links whatever is sitting there: nothing in a fresh
# worktree, and in an old one a library from before the last change to
# crates/vt or crates/client, which fails to link on a missing symbol and
# sends you off to borrow another checkout's build (ov-134). build-vt.sh runs
# `cargo build`, which is a no-op when nothing changed and a rebuild when
# anything did, so running it first makes the library the tests link the one
# the sources describe. The `swift test` that follows relinks, because
# SwiftPM tracks the library file it links.
#
# build-app.sh does the same for the bundle, so nothing else needs the step.
set -euo pipefail

cd "$(dirname "$0")"

log="$(mktemp -t farcooler-test-vt)"
trap 'rm -f "$log"' EXIT
./build-vt.sh >"$log" 2>&1 || { cat "$log" >&2; echo "test.sh: the Rust cores did not build" >&2; exit 1; }

exec swift test "$@"
