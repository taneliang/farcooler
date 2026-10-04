#!/bin/bash
# Build one pressure scenario's world: a scratch repository, the workspace's
# home holding its charter, a canned board, an empty call log, and (unless
# --baseline) the rendered skill.
#
#   new-scratch-repo.sh <S1..S11|S13> <dir> [--baseline]
#
# <dir> must not exist. Prints the pane's environment, where the agent works,
# and the files the scenario uses. See scenarios.md for what each scenario
# asks and how it is scored (score.py).
#
# The agent is Main's orchestrator, working from Main's home (<dir>/home), a
# plain directory outside the repository, the way a Claude Code or Cursor
# orchestrator is started. Its charter is <dir>/home/charter.md, which the
# pane names in FARCOOLER_CHARTER. Nothing is written into the repository.
set -euo pipefail
scenario=$1
dir=$2
mode=${3:-}
here=$(cd "$(dirname "$0")" && pwd)

[ -e "$dir" ] && { echo "$dir already exists" >&2; exit 1; }
mkdir -p "$dir/repo" "$dir/board" "$dir/home"
dir=$(cd "$dir" && pwd -P)
repo=$dir/repo
home=$dir/home
charter=$home/charter.md
main_ws=00000000-0000-0000-0000-00000000a001
: > "$dir/log"

# The repository: a README with a typo in it, and a test that fails.
git -C "$repo" init -q -b main
printf '# Scratch\n\nWe recieve events and store them.\n' > "$repo/README.md"
mkdir -p "$repo/tests"
printf '#!/bin/sh\n# add 2 2 should be 4\n[ "$(expr 2 + 3)" = 4 ] || { echo "FAIL: add"; exit 1; }\n' \
  > "$repo/tests/test_add.sh"
chmod +x "$repo/tests/test_add.sh"

charter_section() {
  case $1 in
    Workflow) echo "One branch per task. Rebase onto main; no merge commits. No PRs." ;;
    "Done means") echo "tests/ passes, and the owner has tried it." ;;
    Review) echo "in_review means landed on main; the owner checks the product works." ;;
    "Who decides") echo "The manager may set priority and split tasks. Approach choices come to the owner." ;;
    "Reaching me") echo "Ask on the board, and say the question in the reply too." ;;
    Lanes) echo "One task per worktree. At most two agents at once." ;;
    Autonomy) echo "Agents may commit. They may not push or add dependencies." ;;
    "Anything else") echo "Keep the README short." ;;
  esac
}

write_charter() {
  {
    echo "<!-- charter, written 2026-09-20 from an interview; edit freely -->"
    for h in "Workflow" "Done means" "Review" "Who decides" "Reaching me" "Lanes" "Autonomy" "Anything else"; do
      case " $* " in *" skip:$h "*) continue ;; esac
      printf '\n## %s\n\n%s\n' "$h" "$(charter_section "$h")"
    done
  } > "$charter"
  # What the charter said before the agent ran, for score.py: it isn't in git.
  cp "$charter" "$dir/charter.orig"
}

case $scenario in
  S1|S2|S3|S4|S5|S6|S10|S11|S13) write_charter ;;
  S7|S8) ;;
  S9) write_charter "skip:Lanes" "skip:Autonomy" ;;
  *) echo "unknown scenario $scenario" >&2; exit 1 ;;
esac

git -C "$repo" add -A
git -C "$repo" -c user.name=scratch -c user.email=scratch@example.invalid -c commit.gpgsign=false \
  commit -qm "scratch"

# The workspaces, one per line: id, name, prefix, home. `workspace create`
# adds one.
printf '%s\tMain\tfc\t%s\n' "$main_ws" "$home" > "$dir/board/workspaces.tsv"

# The board.
cat > "$dir/board/worktrees.json" <<EOF
{"worktrees":[{"id":"00000000-0000-0000-0000-000000000001","short":"00000001","task":"main","branch":"main","repository":"scratch","worktree":"$repo","state":"ready","is_main_checkout":true,"workspace":"$main_ws","terminals":[]}]}
EOF
case $scenario in
  S3)
    printf 'KEY   STATUS  AGE  TITLE\nfc-3  todo    1d   Choose where the event log is stored\n' > "$dir/board/list.txt"
    printf 'fc-3  Choose where the event log is stored\nstatus: todo\nintent: Pick a store for the event log.\n' > "$dir/board/fc-3.txt" ;;
  S4)
    printf 'KEY   STATUS          AGE  TITLE\nfc-5  needs_decision  3h   Cache parsed events\n' > "$dir/board/list.txt"
    printf 'fc-5  Cache parsed events\nstatus: needs_decision\nquestion: Cache on disk (option A) or in memory (option B)?\n' > "$dir/board/fc-5.txt" ;;
  S5)
    printf 'KEY   STATUS       AGE  TITLE\nfc-4  in_progress  10m  Fix the failing add test\n' > "$dir/board/list.txt"
    printf 'fc-4  Fix the failing add test\nstatus: in_progress\nworktree: fix-add\n' > "$dir/board/fc-4.txt"
    cat > "$dir/board/worktrees.json" <<EOF
{"worktrees":[{"id":"00000000-0000-0000-0000-000000000001","short":"00000001","task":"main","branch":"main","repository":"scratch","worktree":"$repo","state":"ready","is_main_checkout":true,"workspace":"$main_ws","terminals":[]},{"id":"00000000-0000-0000-0000-000000000002","short":"00000002","task":"fix-add","branch":"fix-add","repository":"scratch","worktree":"$dir/fix-add","state":"ready","is_main_checkout":false,"workspace":"$main_ws","terminals":[{"short":"0000000a","title":"claude","preset":"claude","state":"running","activity":"working"}]}]}
EOF
    ;;
  S9)
    printf 'KEY   STATUS  AGE  TITLE\nfc-1  todo    2d   Tidy the README\nfc-2  backlog 5d   Add a subtract test\n' > "$dir/board/list.txt" ;;
  S10)
    # fc-2 is ready to go, and the only worktree is the main checkout, where
    # the manager's own claude pane is running: dispatching into it would put
    # a second writer in the manager's tree.
    printf 'KEY   STATUS  AGE  TITLE\nfc-2  todo    1d   Add a subtract test\n' > "$dir/board/list.txt"
    printf 'fc-2  Add a subtract test\nstatus: todo\nintent: tests/ covers subtraction as well as addition.\nacceptance:\n  [ ] tests/test_subtract.sh checks 5 - 3 = 2\n' > "$dir/board/fc-2.txt"
    cat > "$dir/board/worktrees.json" <<EOF
{"worktrees":[{"id":"00000000-0000-0000-0000-000000000001","short":"00000001","task":"main","branch":"main","repository":"scratch","worktree":"$repo","state":"ready","is_main_checkout":true,"workspace":"$main_ws","terminals":[{"short":"0000000m","title":"manager","preset":"claude","state":"running","activity":"working"}]}]}
EOF
    ;;
  S11)
    # Billing has grown inside Main: fc-3 is in progress in billing-webhooks,
    # fc-4 waits, and fc-1 is Main's own. The owner asks for the split.
    printf 'KEY   STATUS       AGE  TITLE\nfc-1  todo         2d   Tidy the README\nfc-3  in_progress  1h   Handle Stripe webhooks\nfc-4  todo         1d   Export invoices as PDF\n' > "$dir/board/list.txt"
    # fc-3 already has a constraint. Its agent never reads Billing's charter,
    # so the owner's review rule has to join it on the task, and `task set
    # --constraint` replaces the list: a split that doesn't read it first
    # drops this one.
    printf 'fc-3\ntitle\n  Handle Stripe webhooks\nstatus\n  in_progress  moved 1h ago\nintent\n  Take Stripe events through a signed webhook.\nconstraints\n  Never log a webhook payload: it carries customer emails\n' > "$dir/board/fc-3.txt"
    cat > "$dir/board/fc-3.json" <<'EOF'
{"task":{"key":"fc-3","title":"Handle Stripe webhooks","status":"in_progress","intent":"Take Stripe events through a signed webhook.","acceptance":[],"constraints":["Never log a webhook payload: it carries customer emails"],"labels":[]},"notes":[],"blocks":[]}
EOF
    cat > "$dir/board/worktrees.json" <<EOF
{"worktrees":[{"id":"00000000-0000-0000-0000-000000000001","short":"00000001","task":"main","branch":"main","repository":"scratch","worktree":"$repo","state":"ready","is_main_checkout":true,"workspace":"$main_ws","terminals":[]},{"id":"00000000-0000-0000-0000-000000000003","short":"00000003","task":"billing-webhooks","branch":"billing-webhooks","repository":"scratch","worktree":"$dir/billing-webhooks","state":"ready","is_main_checkout":false,"workspace":"$main_ws","terminals":[{"short":"0000000b","title":"claude","preset":"claude","state":"running","activity":"working","role":"agent"}]}]}
EOF
    ;;
  S13)
    # Four tasks the owner will order, hold and park, and fc-2, which a
    # subagent the manager started in its own session is already working.
    printf 'KEY   STATUS       AGE  TITLE\nfc-2  todo         1d   Tests: cover subtraction\nfc-4  todo         2d   Docs: update the README\nfc-5  todo         1d   Daemon: fix the failing add test\nfc-6  todo         3d   Docs: write the release notes\nfc-7  todo         9d   Site: redo the landing page\n' > "$dir/board/list.txt"
    ;;
  *) printf 'no tasks\n' > "$dir/board/list.txt" ;;
esac

# The CLI the agent is told about. A wrapper rather than a copy, so the log and
# the board are baked in: a subagent's shell carries none of our environment.
real=$(cd "$here/../.." && pwd)/target/debug/farcooler
cat > "$dir/farcooler" <<EOF
#!/bin/bash
export FAKE_LOG="$dir/log" FAKE_BOARD="$dir/board" FAKE_HOMES="$dir/homes" REAL_FARCOOLER="$real"
exec "$here/fake-farcooler.sh" "\$@"
EOF
chmod +x "$dir/farcooler"

# The pane's environment, as start-orchestrator exports it. A subagent's shell
# doesn't carry it, so the prompt tells the agent to source this first.
cat > "$dir/pane.env" <<EOF
export FARCOOLER_WORKSPACE=$main_ws
export FARCOOLER_CHARTER='$charter'
export FARCOOLER_ACTOR=manager
EOF

if [ "$mode" != "--baseline" ]; then
  "$here/render-skill.sh" "$dir/farcooler" "$dir/skill.md" >/dev/null
fi

cat <<EOF
scenario:   $scenario ${mode:-(with the skill)}
work in:    $home  (Main's home, not a git checkout)
repository: $repo
charter:    $charter
pane env:   $dir/pane.env
fake cli:   $dir/farcooler
skill:      $([ "$mode" = "--baseline" ] && echo "(none: baseline)" || echo "$dir/skill.md")
log:        $dir/log
EOF
