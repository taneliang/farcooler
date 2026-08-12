#!/bin/bash
# What version.sh must answer, checked against real git repositories.
#
# These exist because the channel decides WHERE A DAEMON INSTALLS — its runtime
# directory, its database, its tmux server, its binary name — so a wrong answer
# is not a cosmetic version string. A preview build that reports `stable`
# installs over the stable daemon and writes its database.
#
#   ./scripts/version-test.sh
set -euo pipefail

cd "$(dirname "$0")/.."
SCRIPT="$PWD/scripts/version.sh"
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

# A scratch repository with one commit, so tags have something to point at.
#
# version.sh is COPIED in rather than invoked from here, because its first act
# is `cd "$(dirname "$0")/.."` — it answers about its own repository, which is
# the right behaviour and the reason it cannot be pointed at another one. So
# each case gets a repository shaped like this one: a script in scripts/ and a
# Cargo.toml for `marketing` to read.
scratch() {
  local dir
  dir="$(mktemp -d)"
  mkdir -p "$dir/scripts"
  cp "$SCRIPT" "$dir/scripts/version.sh"
  printf '[workspace.package]\nversion = "0.2.0"\n' > "$dir/Cargo.toml"
  git -C "$dir" init -q -b main
  git -C "$dir" config user.email t@example.com
  git -C "$dir" config user.name t
  git -C "$dir" config commit.gpgsign false
  git -C "$dir" add -A
  git -C "$dir" commit -q -m base
  printf '%s' "$dir"
}

# Run the copied script inside a scratch repository.
at() {
  local dir="$1"
  shift
  (cd "$dir" && ./scripts/version.sh "$@")
}

# --- an untagged commit is dev -------------------------------------------
dir="$(scratch)"
check "untagged is local" "local" "$(at "$dir" channel)"
rm -rf "$dir"

# --- a preview tag is preview --------------------------------------------
dir="$(scratch)"
git -C "$dir" tag -a v0.2.0-preview.3 -m b
check "a preview tag is preview" "preview" "$(at "$dir" channel)"
rm -rf "$dir"

# --- a release tag is release --------------------------------------------
dir="$(scratch)"
git -C "$dir" tag -a v0.2.0 -m r
check "a release tag is stable" "stable" "$(at "$dir" channel)"
rm -rf "$dir"

# --- THE PROMOTION CASE ---------------------------------------------------
#
# Promoting a preview puts a stable tag on the same commit, because the two have
# different bundle identifiers and are therefore different artifacts built from
# one source. `git tag --points-at` then returns both, sorted, so `head -1`
# picks v0.2.0 — and that commit would report `release` forever, including to
# anyone rebuilding the preview from it.
dir="$(scratch)"
git -C "$dir" tag -a v0.2.0-preview.3 -m b
git -C "$dir" tag -a v0.2.0 -m r
check "a promoted commit built as its preview tag is preview" \
  "preview" "$(at "$dir" channel v0.2.0-preview.3)"
check "a promoted commit built as its release tag is stable" \
  "stable" "$(at "$dir" channel v0.2.0)"
check "a promoted preview still says which preview it was" \
  "0.2.0 (preview 3)" "$(at "$dir" display v0.2.0-preview.3)"
rm -rf "$dir"

# --- a dirty tree is dev, whatever it is tagged --------------------------
dir="$(scratch)"
git -C "$dir" tag -a v0.2.0 -m r
touch "$dir/uncommitted"
git -C "$dir" add uncommitted
check "a dirty tree is local even at a release tag" "local" "$(at "$dir" channel)"
rm -rf "$dir"

# --- an unreadable tag name is dev, never release ------------------------
#
# The safe direction, and the same one an unstamped bundle takes: a name we
# cannot read must not be able to promote itself to release.
dir="$(scratch)"
check "a name that is not a version tag is local" "local" "$(at "$dir" channel nonsense)"
rm -rf "$dir"

# --- FARCOOLER_TAG reaches the build scripts ------------------------------
#
# build-app.sh, generate-project.py and build.gradle.kts each ask this script
# independently, so the promotion workflow sets one variable rather than
# threading an argument through four callers — four places to forget it.
dir="$(scratch)"
git -C "$dir" tag -a v0.2.0-preview.3 -m b
git -C "$dir" tag -a v0.2.0 -m r
check "FARCOOLER_TAG decides the channel" \
  "preview" "$(cd "$dir" && FARCOOLER_TAG=v0.2.0-preview.3 ./scripts/version.sh channel)"
check "an explicit argument still wins over the variable" \
  "stable" "$(cd "$dir" && FARCOOLER_TAG=v0.2.0-preview.3 ./scripts/version.sh channel v0.2.0)"
rm -rf "$dir"

# --- an unset variable changes nothing -----------------------------------
dir="$(scratch)"
git -C "$dir" tag -a v0.2.0-preview.1 -m b
check "no variable and no argument reads the tag on HEAD" \
  "preview" "$(at "$dir" channel)"
rm -rf "$dir"

# --- canary has no tag, so CI has to say so ------------------------------
#
# Every push to main is canary, and a commit on main carries no tag — so unlike
# preview and stable this cannot be derived and must be stated. Forgetting to
# state it yields `local`, which is the safe direction: a build that cannot
# prove what it is stays isolated from every other channel's data.
dir="$(scratch)"
check "FARCOOLER_CHANNEL names the channel outright" \
  "canary" "$(cd "$dir" && FARCOOLER_CHANNEL=canary ./scripts/version.sh channel)"
check "a canary build names its commit, since it has no number" \
  "0.2.0 (canary" "$(cd "$dir" && FARCOOLER_CHANNEL=canary ./scripts/version.sh display | cut -c1-13)"
check "an untagged commit with nothing set is local, not canary" \
  "local" "$(at "$dir" channel)"
rm -rf "$dir"

# --- an unreadable channel name is local, never stable -------------------
dir="$(scratch)"
check "a channel name this script does not know is local" \
  "local" "$(cd "$dir" && FARCOOLER_CHANNEL=production ./scripts/version.sh channel)"
check "the names this replaced do not resolve either" \
  "local" "$(cd "$dir" && FARCOOLER_CHANNEL=release ./scripts/version.sh channel)"
rm -rf "$dir"

# --- a dirty tree beats an explicit channel ------------------------------
#
# A dirty tree is never a shippable build, whatever CI says about it.
dir="$(scratch)"
touch "$dir/uncommitted"
git -C "$dir" add uncommitted
check "a dirty tree is local even when CI names a channel" \
  "local" "$(cd "$dir" && FARCOOLER_CHANNEL=stable ./scripts/version.sh channel)"
rm -rf "$dir"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
