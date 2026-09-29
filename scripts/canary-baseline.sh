#!/bin/bash
# Record the proto a Canary build just shipped as proto/baseline/canary.proto,
# committed on main.
#
# Canary puts every push to main on the owner's phones and Macs, so a Canary
# phone and a newer Canary daemon disagree the moment a field is renumbered —
# and proto-lint had no Canary baseline to catch it. This writes one, the way
# promote.yml writes preview's and stable's, from the commit that SHIPPED rather
# than from main (which has moved on by the time a build finishes).
#
#   scripts/canary-baseline.sh <shipped commit>
#
# Run from a checkout of main with full history and push credentials; canary.yml
# does that. It resets that checkout to origin/main, so never run it in a tree
# you care about.
#
# The first line of the baseline names the commit it came from, as a comment
# proto-lint strips. Two things read it:
#
#   - NEVER BACKWARDS. A re-run of an old Canary run (after an upload limit, say)
#     ships an old commit after newer ones already shipped. The newer proto is
#     still in the field, so it stays the baseline: a commit that does not
#     descend from the recorded one records nothing.
#   - COMPARED WITHOUT IT. Only the proto below that line decides whether there
#     is anything to commit, so a push that leaves the wire alone commits
#     nothing — the header keeps naming the first commit to ship this wire.
set -euo pipefail

shipped="${1:?usage: canary-baseline.sh <shipped commit>}"
baseline=proto/baseline/canary.proto

# The shipped commit, by object, not by whatever main is now. Fetched if the
# checkout lacks it, and loudly absent otherwise.
git cat-file -e "$shipped^{commit}" 2>/dev/null \
  || git fetch --quiet --no-tags origin "$shipped"
shipped="$(git rev-parse "$shipped^{commit}")"

new="$(mktemp)"
trap 'rm -f "$new"' EXIT
{
  printf '// Shipped by Canary at %s. Written by canary.yml; see scripts/proto-lint.py.\n' "$shipped"
  git show "$shipped:proto/farcooler.proto"
} > "$new"

# Three tries, because the two shipping jobs of one run finish minutes apart and
# each records; a human push can land in between too. A rejected push starts
# over from the new main rather than merging.
for attempt in 1 2 3; do
  git fetch --quiet --no-tags origin main
  git checkout --quiet --detach FETCH_HEAD

  if [ -f "$baseline" ]; then
    recorded="$(sed -n '1s#^// Shipped by Canary at \([0-9a-f]\{40\}\)\..*#\1#p' "$baseline")"
    # A recorded commit this checkout cannot find (history rewritten) cannot be
    # compared, so it does not block; one it can find must be an ancestor.
    if [ -n "$recorded" ] && git cat-file -e "$recorded^{commit}" 2>/dev/null \
      && ! git merge-base --is-ancestor "$recorded" "$shipped"; then
      echo "canary baseline records $recorded, which $shipped does not descend from; leaving it"
      exit 0
    fi
    # Staged or not is beside the point here: the file is compared directly,
    # so a FIRST baseline (untracked, invisible to `git diff`) still counts as
    # a change. That trap is the one promote.yml's baseline job fell into.
    if cmp -s <(tail -n +2 "$baseline") <(tail -n +2 "$new"); then
      echo "canary baseline unchanged"
      exit 0
    fi
  fi

  mkdir -p proto/baseline
  cp "$new" "$baseline"
  git add "$baseline"
  git -c user.name="github-actions[bot]" \
    -c user.email="github-actions[bot]@users.noreply.github.com" \
    commit --quiet -m "chore: canary wire baseline at ${shipped:0:10}

What the Canary channel now owes compatibility to: the proto of the commit
Canary just shipped. Recorded by canary.yml rather than derived from git
history — see scripts/proto-lint.py for why the file exists."

  if git push --quiet origin HEAD:main; then
    echo "canary baseline recorded from $shipped"
    exit 0
  fi
  echo "::warning::push of the canary baseline was rejected (attempt $attempt); retrying from the new main"
done

echo "::error::could not push the canary baseline after three attempts" >&2
exit 1
