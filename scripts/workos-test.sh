#!/bin/bash
# That the four channels really are four WorkOS projects.
#
# The failure this guards against does not look like a failure. A preview build
# handed the stable client id signs people into the stable environment's
# accounts — `accounts.id` in the relay IS the WorkOS user id — and everything
# about that build looks right: it installs beside the stable app, reports
# `preview`, and talks to the preview relay. Only the identities are wrong.
#
# So the case that matters most here is the third one: a repository that holds
# ONLY the stable secret must build a preview with no client id at all, rather
# than falling back to the one it happens to have.
#
#   ./scripts/workos-test.sh
set -euo pipefail

cd "$(dirname "$0")/.."
PASS=0
FAIL=0

check() {
  local what="$1" want="$2" got="$3"
  if [ "$want" = "$got" ]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    echo "FAIL: $what" >&2
    echo "  want: $want" >&2
    echo "  got:  $got" >&2
  fi
}

# A scratch repository, for the reason version-test.sh explains at length: both
# scripts begin by `cd`-ing to their own repository root, which is what makes
# them answer about the tree they live in and what stops them being pointed at
# another one. Each case therefore gets a repository shaped like this one.
scratch() {
  local dir
  dir="$(mktemp -d)"
  mkdir -p "$dir/scripts"
  cp scripts/version.sh scripts/workos-client-id.sh "$dir/scripts/"
  printf '[workspace.package]\nversion = "0.2.0"\n' > "$dir/Cargo.toml"
  git -C "$dir" init -q -b main
  git -C "$dir" config user.email t@example.com
  git -C "$dir" config user.name t
  git -C "$dir" config commit.gpgsign false
  git -C "$dir" add -A
  git -C "$dir" commit -q -m base
  printf '%s' "$dir"
}

dir="$(scratch)"

# All four present: each channel takes its own and nothing else.
# `env -u GITHUB_ENV`, and it is not decoration.
#
# The script has two output modes: under Actions it writes the id to $GITHUB_ENV
# and prints a human line, and everywhere else it prints the id itself. A test
# that merely does not SET GITHUB_ENV still inherits the runner's, so every one
# of these cases took the Actions branch in CI and compared an id against "The
# preview channel's WorkOS project". Green on a laptop, six failures on a
# runner. Ambient environment is an input; the cases that care must name it.
run() {
  (cd "$dir" && env -u GITHUB_ENV \
    LOCAL_WORKOS_CLIENT_ID=id-local \
    CANARY_WORKOS_CLIENT_ID=id-canary \
    PREVIEW_WORKOS_CLIENT_ID=id-preview \
    STABLE_WORKOS_CLIENT_ID=id-stable \
    "$@" ./scripts/workos-client-id.sh)
}

check "untagged is the local project" "id-local" "$(run)"
check "FARCOOLER_CHANNEL picks canary" "id-canary" "$(run FARCOOLER_CHANNEL=canary)"
check "a preview tag picks preview" "id-preview" "$(run FARCOOLER_TAG=v0.2.0-preview.3)"
check "a release tag picks stable" "id-stable" "$(run FARCOOLER_TAG=v0.2.0)"

# The same, with nothing set. Separate from `run` because these cases are about
# what happens when a secret is ABSENT, and `run` supplies all four.
bare() {
  (cd "$dir" && env -u GITHUB_ENV "$@" ./scripts/workos-client-id.sh)
}

# THE ONE THAT MATTERS. Only the stable secret exists — the state a repository
# is in halfway through being set up — and a preview build must come out with
# nothing rather than with the stable environment's identities.
only_stable="$(bare STABLE_WORKOS_CLIENT_ID=id-stable FARCOOLER_TAG=v0.2.0-preview.3 2>/dev/null)"
check "preview never falls back to stable" "" "$only_stable"

# Absent is not fatal: a fork has none of these and still builds an app that
# works, minus the sign-in button. Exercised for stable, the channel where the
# temptation to fail hard is strongest.
set +e
bare FARCOOLER_TAG=v0.2.0 >/dev/null 2>&1
check "a missing secret does not fail the build" "0" "$?"
set -e

# But it says so, and only where someone will install the result. A local build
# without one is the ordinary case and must stay quiet, or the warning becomes
# something people learn to read past.
noisy="$(bare FARCOOLER_TAG=v0.2.0 2>&1 >/dev/null | grep -c 'stable' || true)"
check "a missing stable id warns" "1" "$noisy"
quiet="$(bare 2>&1 >/dev/null | wc -l | tr -d ' ')"
check "a missing local id is silent" "0" "$quiet"

# Under Actions the id reaches later steps through GITHUB_ENV — build-app.sh and
# generate-project.py both read it from the environment — so the file it writes
# is part of the contract, not an implementation detail.
env_file="$(mktemp)"
(cd "$dir" && GITHUB_ENV="$env_file" PREVIEW_WORKOS_CLIENT_ID=id-preview \
  FARCOOLER_TAG=v0.2.0-preview.3 ./scripts/workos-client-id.sh >/dev/null)
check "GITHUB_ENV carries the id" "FARCOOLER_WORKOS_CLIENT_ID=id-preview" "$(cat "$env_file")"

# And under Actions the warning has to be an ANNOTATION on stdout rather than a
# line on stderr, or it never reaches the run summary — which is the only place
# anyone would see it. This is the branch the runner actually takes, and the one
# that went unexercised while the suite was passing.
annotation="$(cd "$dir" && GITHUB_ENV="$env_file" FARCOOLER_TAG=v0.2.0 \
  ./scripts/workos-client-id.sh 2>/dev/null | grep -c '^::warning::' || true)"
check "under Actions the warning is an annotation" "1" "$annotation"

rm -rf "$dir" "$env_file"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
