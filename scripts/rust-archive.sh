#!/usr/bin/env bash
# The one producer of the Rust test archive (ov-1210, extending ov-1182).
#
# CI's two archive jobs (Linux, Mac) and the local lane gate all run this, so a
# change to what goes into the archive is made once. It builds the runtime
# binaries the tests spawn (farcooler, farcoolerd, the LFS filter), then packs
# every workspace test target with them into one nextest archive. Consumers
# run it in partitions with
#   cargo nextest run --profile ci --archive-file <archive> --extract-to . \
#     --extract-overwrite --workspace-remap "$PWD"
# and compile nothing. The caller owns the environment (CI sets
# CARGO_PROFILE_*_DEBUG=0 on the job), so the archive is the same wherever the
# producer runs.
#
# Usage: scripts/rust-archive.sh <archive-file>
set -euo pipefail

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo "usage: scripts/rust-archive.sh <archive-file>" >&2
  exit 2
fi
archive="$1"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
mkdir -p "$(dirname "$archive")"
cargo build --locked --workspace --bins
cargo nextest archive --locked --profile ci --workspace --archive-file "$archive"
