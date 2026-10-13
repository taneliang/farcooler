#!/bin/bash
# Refresh the website's "Far Cooler builds Far Cooler" section, unattended (ov-499).
#
#   scripts/site-proof-refresh.sh             regenerate, commit, push if main is green
#   scripts/site-proof-refresh.sh --dry-run   regenerate and show the diff; no commit, no push
#   scripts/site-proof-refresh.sh --install   install and load the daily launchd agent
#
# launchd runs it at 4:17 am local time (scripts/com.farcooler.site-proof.plist), from a copy that
# --install makes in ~/Library/Application Support/farcooler-site-proof/ (works from any checkout).
# Each run works in a fresh detached worktree of origin/main under the system
# temp dir, removed at the end, so it never touches a working checkout. It runs
# the generator that is on main (scripts/site-proof.py), and if the page
# changed it commits the page and the frozen token counts (scripts/site-proof-tokens.json) with a message that says what changed.
#
# The commit is made through GitHub's API (`gh api graphql`, createCommitOnBranch)
# rather than locally, so GitHub signs it and no GPG key or pinentry is needed
# (a local signed commit failed under launchd with "No pinentry", Oct 12). The
# mutation carries the file's whole contents in base64 and an expectedHeadOid:
# GitHub refuses it if main has moved since, which is the "main moved" rule
# enforced by the server instead of a race we could lose.
#
# The push rule is .claude/agent/bin/push-guard.sh's, kept here because that
# file is local and untracked: commit to main only when the worktree's HEAD
# still is origin/main and the newest commit on main that isn't from
# github-actions[bot] has a green CI run. When main is red, or moved while this
# ran, it logs why and leaves the change for the next day's run (the generator
# rebuilds from scratch each time).
#
# SITE_PROOF_BASE (default origin/main) names the commit the worktree starts from;
# it exists so a dry run can test a branch that has not landed.
#
# SITE_PROOF_BRANCH (default main) names the branch the commit goes to; it exists
# so the API call can be tried against a branch that does not exist.
#
# Never prompts: no terminal, no git credential prompt, gh never asks.
# Log: ~/Library/Logs/farcooler-site-proof.log.
set -u

LABEL=com.farcooler.site-proof
SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
LOG="$HOME/Library/Logs/farcooler-site-proof.log"
PAGE=services/site/public/index.html
TOKENS=scripts/site-proof-tokens.json   # the frozen weekly token counts; the generator updates it

if [ "${1:-}" = "--install" ]; then
  dest="$HOME/Library/LaunchAgents/$LABEL.plist"
  mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
  # A stable copy outside every checkout, so no branch switch or dirty tree matters. The copy
  # finds the repository through the `repo` file beside it; the generator it runs is always the
  # one in the fresh origin/main worktree, never the copy's.
  common="$(git -C "$(dirname "$SCRIPT")" rev-parse --path-format=absolute --git-common-dir)" || exit 1
  support="$HOME/Library/Application Support/farcooler-site-proof"
  mkdir -p "$support" || exit 1
  cp "$SCRIPT" "$support/site-proof-refresh.sh" && chmod +x "$support/site-proof-refresh.sh" || exit 1
  dirname "$common" > "$support/repo"
  main_script="$support/site-proof-refresh.sh"
  sed -e "s|__SCRIPT__|$main_script|g" -e "s|__HOME__|$HOME|g" \
    "$(dirname "$SCRIPT")/$LABEL.plist" > "$dest" || exit 1
  plutil -lint "$dest" >/dev/null || exit 1
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null
  launchctl bootstrap "gui/$(id -u)" "$dest" || exit 1
  echo "installed $dest; runs daily at 4:17 am"
  exit 0
fi

dry=0
[ "${1:-}" = "--dry-run" ] && dry=1

export PATH="/opt/homebrew/bin:$HOME/.local/bin:/usr/local/bin:/usr/bin:/bin"
export GIT_TERMINAL_PROMPT=0 GH_PROMPT_DISABLED=1 GIT_EDITOR=true
unset GPG_TTY
exec < /dev/null
mkdir -p "$(dirname "$LOG")"
exec > >(tee -a "$LOG") 2>&1
say() { echo "$(date '+%Y-%m-%d %H:%M:%S') site-proof: $*"; }

if [ -f "$(dirname "$SCRIPT")/repo" ]; then
  repo="$(cat "$(dirname "$SCRIPT")/repo")"   # the installed copy
else
  common="$(git -C "$(dirname "$SCRIPT")" rev-parse --path-format=absolute --git-common-dir)" || { say "not in a git checkout"; exit 1; }
  repo="$(dirname "$common")"
fi
tmp="$(mktemp -d "${TMPDIR:-/tmp}/site-proof.XXXXXX")" || exit 1
wt="$tmp/wt"
cleanup() {
  git -C "$repo" worktree remove --force "$wt" >/dev/null 2>&1
  rm -rf "$tmp"
}
trap cleanup EXIT

say "start$([ $dry = 1 ] && echo ' (dry run)')"
git -C "$repo" fetch -q origin main || { say "fetch failed"; exit 1; }
git -C "$repo" worktree add -q --detach "$wt" "${SITE_PROOF_BASE:-origin/main}" || { say "worktree add failed"; exit 1; }
cd "$wt" || exit 1

if ! out="$(python3 scripts/site-proof.py)"; then
  say "generator failed: $out"
  exit 1
fi
say "generator: $(echo "$out" | head -1)"
if [ -z "$(git status --porcelain -- "$PAGE" "$TOKENS")" ]; then
  say "nothing changed"
  exit 0
fi

changes="$(echo "$out" | tail -n +2)"

# The mutation, as a JSON file: the page can be too big for an argument list.
git fetch -q origin main || { say "fetch failed"; exit 1; }
head_oid="$(git rev-parse HEAD)"
nwo="$(git remote get-url origin | sed -E 's#^(git@github\.com:|https://github\.com/)##; s#\.git$##')"
branch="${SITE_PROOF_BRANCH:-main}"
msg_body="$(printf '%s\n\nGenerated by scripts/site-proof-refresh.sh.' "$changes")"
base64 < "$PAGE" | tr -d '\n' > "$tmp/page.b64" || exit 1
base64 < "$TOKENS" | tr -d '\n' > "$tmp/tokens.b64" || exit 1
jq -n --arg nwo "$nwo" --arg branch "$branch" --arg oid "$head_oid" --arg path "$PAGE" --arg tpath "$TOKENS" \
  --arg headline 'site: the proof section is refreshed from the board and git (ov-499)' \
  --arg body "$msg_body" --rawfile contents "$tmp/page.b64" --rawfile tcontents "$tmp/tokens.b64" \
  '{query: "mutation($input: CreateCommitOnBranchInput!) { createCommitOnBranch(input: $input) { commit { oid url signature { isValid state } } } }",
    variables: {input: {
      branch: {repositoryNameWithOwner: $nwo, branchName: $branch},
      expectedHeadOid: $oid,
      message: {headline: $headline, body: $body},
      fileChanges: {additions: [{path: $path, contents: $contents}, {path: $tpath, contents: $tcontents}]}}}}' > "$tmp/mutation.json" \
  || { say "could not build the commit request"; exit 1; }

if [ $dry = 1 ]; then
  git diff --stat -- "$PAGE" "$TOKENS"
  echo "$changes"
  say "dry run: would commit $PAGE ($(wc -c < "$tmp/mutation.json" | tr -d ' ') byte request) to $nwo $branch on top of ${head_oid:0:8}; no commit, no push"
  exit 0
fi

if [ "$head_oid" != "$(git rev-parse origin/main)" ]; then
  say "main moved while this ran; will retry on the next run"
  exit 0
fi
sha="$(git log --format='%H %an' origin/main | grep -v 'github-actions\[bot\]' | head -1 | cut -d' ' -f1)"
ci="$(gh run list --commit "$sha" --workflow CI --json conclusion -q '.[0].conclusion' 2>&1)"
if [ "$ci" != success ]; then
  say "main's newest code commit ${sha:0:8} has CI '$ci'; not committing, will retry on the next run"
  exit 0
fi
if ! res="$(gh api graphql --input "$tmp/mutation.json" 2>&1)"; then
  say "GitHub refused the commit; nothing pushed: $res"
  exit 1
fi
done_oid="$(echo "$res" | jq -r '.data.createCommitOnBranch.commit.oid // empty')"
if [ -z "$done_oid" ]; then
  say "GitHub did not return a commit; nothing pushed: $res"
  exit 1
fi
say "committed ${done_oid:0:8} to $nwo $branch through the API (signature: $(echo "$res" | jq -r '.data.createCommitOnBranch.commit.signature | "\(.state), valid=\(.isValid)"'))"
