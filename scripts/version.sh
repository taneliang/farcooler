#!/bin/bash
# The version of the whole system, from one place.
#
# Overnight is five things that must agree — the CLI, the daemon, the Mac app,
# the phone app, and the relay — and the failure they produce when they do not
# is not a build error. It is a bug you already fixed still happening, because
# the fix shipped in one of them. This project has hit that twice: once when
# `build-app.sh` copied a stale binary, and once when a Mac drove a Linux host
# running different source over ssh.
#
# So there is exactly one number, `[workspace.package] version` in Cargo.toml,
# and everything else derives from it. Nothing is hand-stamped, because a
# hand-stamped version is one someone forgets on the release that mattered.
#
#   ./scripts/version.sh            0.1.0
#   ./scripts/version.sh build      1284          (commits — monotonic, for the stores)
#   ./scripts/version.sh full       0.1.0+a1b2c3  (what every component reports)
set -euo pipefail

cd "$(dirname "$0")/.."

# The first `version = "…"` under [workspace.package]. Deliberately not a TOML
# parser: this file is ours, its shape is stable, and a release script should not
# need a dependency to answer what version it is releasing.
marketing() {
  awk '/^\[workspace.package\]/{f=1} f && /^version *=/{gsub(/[",]/,"",$3); print $3; exit}' Cargo.toml
}

# App Store build numbers must increase forever and are separate from the
# version people read. Commit count is the one monotonic integer this repo
# already has, and it needs no state, no counter file, and no coordination
# between a laptop and CI.
build() {
  git rev-list --count HEAD 2>/dev/null || echo 0
}

case "${1:-marketing}" in
  marketing) marketing ;;
  build) build ;;
  full)
    sha=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
    dirty=""
    [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ] && dirty="-dirty"
    echo "$(marketing)+${sha}${dirty}"
    ;;
  *)
    echo "usage: version.sh [marketing|build|full]" >&2
    exit 1
    ;;
esac
