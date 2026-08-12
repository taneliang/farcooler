#!/bin/bash
# The version of the whole system, from one place.
#
# Far Cooler is five things that must agree — the CLI, the daemon, the Mac app,
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
#   ./scripts/version.sh channel    dev|beta|release
#   ./scripts/version.sh display    0.1.0 (beta 3)   what a human is shown
#
# There is deliberately no `full` here. The stamp components report to each
# other — `0.1.0+a1b2c3` — is computed once, in crates/protocol/build.rs, and
# asked for with `farcooler --version`. A second implementation in this file
# would be exactly the drift the file exists to prevent.
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
  # A shallow clone is the trap here, and it fails SILENTLY: `git rev-list
  # --count HEAD` succeeds on a depth-1 checkout and returns 1. Ship from one
  # and the App Store build number goes backwards, which is rejected at upload
  # and unfixable afterwards — build numbers can never be reused. So say so
  # rather than emit a plausible wrong answer.
  if [ "$(git rev-parse --is-shallow-repository 2>/dev/null)" = "true" ]; then
    echo "shallow clone: build numbers would restart at 1 (use fetch-depth: 0)" >&2
    exit 1
  fi
  git rev-list --count HEAD 2>/dev/null || echo 0
}

# Which kind of build this is: dev, beta, or release.
#
# Derived from the tag on HEAD rather than a flag someone passes, because a flag
# is a thing to forget on the build that mattered — and the failure mode here is
# specific and nasty: a beta and a release that call themselves the same version.
# Someone reports a bug against "0.2.0" and there is no way to know which 0.2.0
# they had.
#
#   clean tag v0.2.0         → release
#   clean tag v0.2.0-beta.3  → beta
#   anything else            → dev  (untagged, or a dirty tree, which is not
#                                    something anyone should be shipping)
#
# Takes an OPTIONAL tag, which is the tag being built. It is needed because
# promotion puts two tags on one commit: beta and release have different bundle
# identifiers, so they are genuinely different artifacts, and promoting means
# rebuilding the beta's source at the release channel rather than re-uploading
# its binary. Without the argument, `git tag --points-at HEAD` returns both and
# `head -1` picks whichever sorts first — `v0.2.0` before `v0.2.0-beta.3` — so
# after a promotion that commit would report `release` forever, and anyone
# rebuilding the beta from it would get a binary that installs at the release
# path and writes the release runtime directory.
#
# Not a flag someone remembers: the workflow created the tag one job earlier and
# passes it through. With no argument the old behaviour stands, so a local build
# is unaffected and still defaults to dev.
channel() {
  if [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]; then
    echo local
    return
  fi

  # `canary` is every push to main, which carries NO TAG — so unlike preview and
  # stable it cannot be derived from one, and CI has to say so outright.
  #
  # This is not the flag the comment above argues against. Forgetting it yields
  # `local`, which is the safe direction: a build that cannot prove what it is
  # stays isolated from every other channel's data. Forgetting a flag that
  # defaulted to `stable` would be the opposite.
  case "${FARCOOLER_CHANNEL:-}" in
    canary|preview|stable|local) echo "$FARCOOLER_CHANNEL"; return ;;
    "") ;;
    # A name this script does not know. Local, for the same reason an unstamped
    # bundle is: something we cannot read must not promote itself.
    *) echo local; return ;;
  esac
  # An argument, else `FARCOOLER_TAG`, else the tag on HEAD.
  #
  # The environment variable is how the promotion workflow reaches the build
  # scripts. `build-app.sh`, `generate-project.py` and `build.gradle.kts` each
  # ask this script independently, so threading an argument to all of them would
  # mean four places to forget it — and the whole point is that a promoted
  # commit must not be able to report the wrong channel.
  #
  # This is not the flag the comment above argues against. Nobody types it: the
  # workflow created the tag one job earlier and passes what it made. Unset — a
  # local build, always — the behaviour is exactly what it was.
  #
  # `|| true` because `grep` exits 1 when a commit carries no tags, which is the
  # ordinary case for every dev build. Harmless inside the `case` this used to
  # be, but fatal in an assignment under `set -e`: the script would abort and
  # print nothing, so a caller reading an empty channel gets neither an answer
  # nor an error.
  tag="${1:-${FARCOOLER_TAG:-$(git tag --points-at HEAD 2>/dev/null | grep '^v' | head -1 || true)}}"
  case "$tag" in
    "") echo local ;;
    *-preview.*) echo preview ;;
    v*) echo stable ;;
    # Something that is not a version tag at all. Local, for the same reason an
    # unstamped bundle is: a name we cannot read must not be able to promote
    # itself to stable.
    *) echo local ;;
  esac
}

# The version a person is shown, which is not the version a machine compares.
#
# A release says "0.2.0" and nothing else — the channel is the default and
# saying so is noise. A beta and a dev build must announce themselves, because
# the whole point is that someone looking at a bug report can tell.
display() {
  case "$(channel "$1")" in
    stable) marketing ;;
    preview)
      # From the named tag when there is one, so a promoted commit still says
      # which preview it was rather than reading the stable tag beside it.
      n=$(printf '%s' "${1:-$(git tag --points-at HEAD 2>/dev/null | grep '^v.*-preview\.' | head -1)}" | sed 's/.*-preview\.//')
      echo "$(marketing) (preview ${n:-?})"
      ;;
    # Canary and local both name the commit, because neither has a number and
    # the commit is the only thing that tells two of them apart in a bug report.
    *) echo "$(marketing) ($(channel "$1") $(git rev-parse --short HEAD 2>/dev/null || echo unknown))" ;;
  esac
}

case "${1:-marketing}" in
  marketing) marketing ;;
  build) build ;;
  channel) channel "${2:-}" ;;
  display) display "${2:-}" ;;
  *)
    echo "usage: version.sh [marketing|build|channel [tag]|display [tag]]" >&2
    exit 1
    ;;
esac
