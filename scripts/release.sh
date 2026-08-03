#!/bin/bash
# Cut a release.
#
#   ./scripts/release.sh 0.2.0            a release
#   ./scripts/release.sh 0.2.0 --beta 1   a TestFlight beta of that release
#
# Bumps the one version number, commits it, and tags. CI does the rest — see
# .github/workflows/release.yml, which builds every component from the single
# commit this tag points at. That is the whole reason this is a script and not a
# checklist: four components built from four checkouts is four things that can
# disagree about what version they are.
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:-}"
BETA="${3:-}"
if [ -z "$VERSION" ] || { [ -n "${2:-}" ] && [ "${2:-}" != "--beta" ]; }; then
  echo "usage: release.sh <version> [--beta <n>]    (currently $(scripts/version.sh))" >&2
  exit 1
fi
if [ "${2:-}" = "--beta" ] && ! echo "$BETA" | grep -qE '^[0-9]+$'; then
  echo "--beta needs a number: release.sh $VERSION --beta 1" >&2
  exit 1
fi

# The tag carries the channel; Cargo.toml never does.
#
# A beta of 0.2.0 IS 0.2.0 — same code, same marketing version, same App Store
# entry. What differs is which build someone has, and that is what the tag and
# scripts/version.sh's channel answer. Putting `-beta.1` in the marketing
# version instead would show it to every user in Settings and change the version
# every component compares.
if [ -n "$BETA" ]; then
  TAG="v$VERSION-beta.$BETA"
  WHAT="beta $BETA of $VERSION"
else
  TAG="v$VERSION"
  WHAT="$VERSION"
fi

# Semver, and the middle number is the one that moves.
#
# The apps and the daemon must match exactly within a release — they check each
# other's build stamps — so there is no such thing as an app-only patch. A change
# anywhere is a change to the system. MAJOR is reserved for a break in the ssh
# protocol between an app and a daemon, which is the only break a user has to do
# anything about.
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "not a version: $VERSION (expected MAJOR.MINOR.PATCH)" >&2
  exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "working tree is dirty — a release must name one commit exactly" >&2
  exit 1
fi

if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "$TAG already exists" >&2
  exit 1
fi

# The only edit, and only when the version actually moves. A second beta of a
# version already in Cargo.toml is a tag and nothing else — there is no commit
# to make, and making an empty one would move the build number for no reason.
if [ "$(scripts/version.sh)" != "$VERSION" ]; then
python3 - "$VERSION" <<'PY'
import re
import sys

version = sys.argv[1]
text = open("Cargo.toml").read()
updated, count = re.subn(
    r'(\[workspace\.package\][^\[]*?version = ")[^"]+(")',
    lambda m: m.group(1) + version + m.group(2),
    text,
    count=1,
    flags=re.S,
)
assert count == 1, "could not find [workspace.package] version in Cargo.toml"
open("Cargo.toml", "w").write(updated)
PY

# So the lockfile records the new version too, and the commit is complete.
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null

git add Cargo.toml Cargo.lock
git commit -q -m "release: $VERSION"
fi

git tag -a "$TAG" -m "Far Cooler $WHAT"

cat <<EOF

Tagged $TAG — $WHAT, build $(scripts/version.sh build).
The apps will report it as: $(scripts/version.sh display)

Push it and CI builds everything from this commit:

    git push origin main "$TAG"

EOF
