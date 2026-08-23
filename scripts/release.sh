#!/bin/bash
# Move the one version number.
#
#   ./scripts/release.sh 0.2.0
#
# This no longer tags, and no longer ships anything. Shipping is the **Promote**
# workflow in the Actions tab: pick a channel, type the version you believe you
# are shipping, and it tags and builds in one run.
#
# The split is deliberate. A tag must point at a commit that has been BUILT and
# RUN as a preview, and a script that bumped the version and tagged in one breath
# would be tagging a commit nobody had tried — `.github/workflows/release.yml`
# builds every component from the single commit a tag points at, so that commit
# had better be one that already worked.
#
# So the version moves here, on `main`, as an ordinary pull request. Then you
# press the button. `crates/protocol/build.rs` stamps FARCOOLER_BUILD from this
# literal, which is why the button can only ever CONFIRM the version and never
# choose one: a tag naming a version Cargo.toml disagreed with would have the
# binary and the plist reporting different releases to each other.
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  echo "usage: release.sh <version>    (currently $(scripts/version.sh))" >&2
  echo "" >&2
  echo "Moves [workspace.package] version in Cargo.toml and commits it." >&2
  echo "To ship, run the Promote workflow from the Actions tab." >&2
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
  echo "working tree is dirty — the version bump must be its own commit" >&2
  exit 1
fi

CURRENT="$(scripts/version.sh)"
if [ "$CURRENT" = "$VERSION" ]; then
  echo "Cargo.toml already says $VERSION. Nothing to do." >&2
  echo "To ship it, run the Promote workflow from the Actions tab." >&2
  exit 1
fi

# A version can only go forward.
#
# App Store Connect will not accept a build below one it already has, and that
# is unfixable afterwards — the same class of mistake scripts/version.sh warns
# about for build numbers. Going 0.3.2 → 0.4.0 mid-beta is ordinary and cheap;
# going back is not recoverable.
LOWER="$(printf '%s\n%s\n' "$CURRENT" "$VERSION" | sort -V | head -1)"
if [ "$LOWER" = "$VERSION" ]; then
  echo "$VERSION is not ahead of $CURRENT." >&2
  echo "A version can only go forward: the app stores will not take a build" >&2
  echo "numbered below one they already have, and that cannot be undone." >&2
  exit 1
fi

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

# The channel named below has to be one the Promote dropdown actually offers.
# It said `beta` until now, which `2ae5cf3` removed when it renamed the channels
# to local/canary/preview/stable — so these instructions were telling whoever ran
# them to pick an option that is not in the list.
cat <<EOF

Moved $CURRENT → $VERSION, build $(scripts/version.sh build).

Push it, land it on main, then ship from the Actions tab:

    git push origin HEAD

    Actions → Promote → Run workflow
      channel:          preview
      confirm_version:  $VERSION

EOF
