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
#   - NEVER OVER A BREAK. The proto that shipped is checked against the one
#     recorded, by proto-lint's own rules, before it replaces it. Copied
#     unchecked, a break that shipped became the baseline, and every later lint
#     passed against it. canary.yml's `wire` job should stop a break before it
#     ships; this is the lock that holds if that gate is ever lost, and it fails
#     the job rather than warning, because a break in the field needs a person.
#   - COMPARED WITHOUT IT. Only the proto below that line decides whether there
#     is anything to commit, so a push that leaves the wire alone commits
#     nothing — the header keeps naming the first commit to ship this wire.
set -euo pipefail

shipped="${1:?usage: canary-baseline.sh <shipped commit>}"
baseline=proto/baseline/canary.proto
# Resolved now: the checkout below moves the tree, and this path with it.
lint="$(cd "$(dirname "$0")" && pwd)/proto-lint.py"
# No lint, no baseline: an unchecked proto is exactly what this must not record.
if [ ! -x "$lint" ]; then
  echo "::error::the wire lint is unavailable ($lint), so the shipped proto cannot be checked and the canary baseline was not advanced" >&2
  exit 1
fi

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

  # Only a commit on main. Canary can be dispatched from any branch, and a
  # branch's proto recorded here would be compared against fields main may
  # never get — and, since no later main commit descends from it, would stop
  # the baseline advancing for good. canary.yml gates on the ref as well; this
  # is the lock that holds if that gate is ever lost.
  if ! git merge-base --is-ancestor "$shipped" HEAD; then
    echo "::warning::$shipped is not on main, so it is not recorded as the canary wire baseline"
    exit 0
  fi

  if [ -f "$baseline" ]; then
    recorded="$(sed -n '1s#^// Shipped by Canary at \([0-9a-f]\{40\}\)\..*#\1#p' "$baseline")"
    # A recorded commit this checkout cannot find (history rewritten) cannot be
    # compared, so it does not block; one it can find must be an ancestor.
    if [ -n "$recorded" ] && git cat-file -e "$recorded^{commit}" 2>/dev/null \
      && ! git merge-base --is-ancestor "$recorded" "$shipped"; then
      echo "::warning::the canary wire baseline records $recorded, which $shipped does not descend from, so it was not advanced"
      exit 0
    fi
    # Staged or not is beside the point here: the file is compared directly,
    # so a FIRST baseline (untracked, invisible to `git diff`) still counts as
    # a change. That trap is the one promote.yml's baseline job fell into.
    if cmp -s <(tail -n +2 "$baseline") <(tail -n +2 "$new"); then
      echo "canary baseline unchanged"
      exit 0
    fi
    # The same rules CI's and Canary's `wire` jobs apply, so what is refused
    # here is exactly what the lint would have refused before the ship.
    # A break, or a lint that could not run (a crash prints no verdict), and
    # either way nothing is recorded. Each says which it was.
    if ! problems="$("$lint" --compare "$baseline" "$new" 2>&1)"; then
      case "$problems" in
        *"wire compatibility problem"*)
          echo "::error::$shipped breaks the wire Canary already shipped, so the baseline was not advanced. Ship a fix that restores compatibility." >&2 ;;
        *)
          echo "::error::the wire lint failed to run, so the shipped proto cannot be checked and the canary baseline was not advanced" >&2 ;;
      esac
      echo "$problems" >&2
      exit 1
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

  # Never forced. A rejected push means main moved; forcing would erase
  # whatever moved it.
  if git push --quiet origin HEAD:main; then
    echo "canary baseline recorded from $shipped"
    exit 0
  fi
  echo "::warning::push of the canary baseline was rejected (attempt $attempt); retrying from the new main"
done

echo "::error::could not push the canary baseline after three attempts" >&2
exit 1
