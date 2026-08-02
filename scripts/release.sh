#!/bin/bash
# Cut a release.
#
#   ./scripts/release.sh 0.2.0
#
# Bumps the one version number, commits it, and tags. CI does the rest — see
# .github/workflows/release.yml, which builds every component from the single
# commit this tag points at. That is the whole reason this is a script and not a
# checklist: four components built from four checkouts is four things that can
# disagree about what version they are.
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  echo "usage: release.sh <version>    (currently $(scripts/version.sh))" >&2
  exit 1
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

if git rev-parse "v$VERSION" >/dev/null 2>&1; then
  echo "v$VERSION already exists" >&2
  exit 1
fi

# The only edit. Everything else — both Info.plists, the iOS project, the build
# stamp the daemon reports over the wire — derives from this line.
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
git tag -a "v$VERSION" -m "Overnight $VERSION"

cat <<EOF

Tagged v$VERSION (build $(scripts/version.sh build)).

Push it and CI builds everything from this commit:

    git push origin main "v$VERSION"

EOF
