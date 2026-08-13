#!/bin/bash
# The WorkOS client id for the channel this build IS.
#
# Four channels, four WorkOS projects, no sharing. The relay's `accounts.id`
# **is** the WorkOS user id, so two channels pointed at one project do not merely
# look alike — they are the same accounts, and a preview build would be signing
# people into the stable environment's identities. That is the partition four
# relays and four bundle identifiers exist to keep, and a client id is the one
# place it can be undone with a single wrong secret name.
#
# So the secret is never named at a call site. It is derived from
# `version.sh channel`, which is the same answer the bundle identifier and the
# runtime directory come from — meaning the app's identity and the project it
# authenticates against cannot disagree, whatever a workflow author writes.
#
# This exists as a script rather than as YAML because two jobs in release.yml
# need it, and a copy in each is a copy that gets fixed once.
#
#   LOCAL_WORKOS_CLIENT_ID=… ./scripts/workos-client-id.sh
#
# Under Actions it appends FARCOOLER_WORKOS_CLIENT_ID to $GITHUB_ENV, for every
# later step in that job: `build-app.sh` and `generate-project.py` both read it
# from the environment. Run by hand it prints the id, so `export
# FARCOOLER_WORKOS_CLIENT_ID=$(./scripts/workos-client-id.sh)` builds locally
# against whichever project you have.
#
# EMPTY IS NOT AN ERROR. A fork has none of these, and the app it builds works
# completely except for the sign-in button — which is why this warns rather than
# fails. It warns loudly for a channel people install, because a stable release
# that silently cannot sign in is a release with no notifications.
set -euo pipefail

cd "$(dirname "$0")/.."

channel="$(./scripts/version.sh channel)"

# Named literally, one per line, so `grep PREVIEW_WORKOS_CLIENT_ID` finds every
# place it is expected — a computed `${CHANNEL}_WORKOS_CLIENT_ID` would be
# invisible to the person auditing whether the four are really separate.
case "$channel" in
  local)   id="${LOCAL_WORKOS_CLIENT_ID:-}" ;;
  canary)  id="${CANARY_WORKOS_CLIENT_ID:-}" ;;
  preview) id="${PREVIEW_WORKOS_CLIENT_ID:-}" ;;
  stable)  id="${STABLE_WORKOS_CLIENT_ID:-}" ;;
  # version.sh answers with one of the four or with `local`, so this is
  # unreachable — and it stays here because the way it would become reachable is
  # a fifth channel added there and forgotten here, which must not quietly
  # resolve to the empty string.
  *) echo "unknown channel: $channel" >&2; exit 1 ;;
esac

if [ -z "$id" ] && [ "$channel" != local ]; then
  message="No client id for the $channel channel: this build has no sign-in, and sign-in is what buys notifications."
  if [ -n "${GITHUB_ENV:-}" ]; then
    echo "::warning::$message"
  else
    echo "$message" >&2
  fi
fi

if [ -n "${GITHUB_ENV:-}" ]; then
  echo "FARCOOLER_WORKOS_CLIENT_ID=$id" >> "$GITHUB_ENV"
  echo "The $channel channel's WorkOS project"
else
  echo "$id"
fi
