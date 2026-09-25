#!/bin/bash
# Build one pressure scenario's world: a scratch repository, a canned board,
# an empty call log, and (unless --baseline) the rendered skill.
#
#   new-scratch-repo.sh <S1..S10> <dir> [--baseline]
#
# <dir> must not exist. Prints the environment to export and the files the
# scenario uses. See scenarios.md for what each scenario asks and how it is
# scored (score.py).
set -euo pipefail
scenario=$1
dir=$2
mode=${3:-}
here=$(cd "$(dirname "$0")" && pwd)

[ -e "$dir" ] && { echo "$dir already exists" >&2; exit 1; }
mkdir -p "$dir/repo" "$dir/board"
dir=$(cd "$dir" && pwd -P)
repo=$dir/repo
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
    "Anything else") echo "Keep the README short. This file is committed." ;;
  esac
}

write_charter() {
  mkdir -p "$repo/.farcooler"
  {
    echo "<!-- charter, written 2026-09-20 from an interview; edit freely -->"
    for h in "Workflow" "Done means" "Review" "Who decides" "Reaching me" "Lanes" "Autonomy" "Anything else"; do
      case " $* " in *" skip:$h "*) continue ;; esac
      printf '\n## %s\n\n%s\n' "$h" "$(charter_section "$h")"
    done
  } > "$repo/.farcooler/manager.md"
}

case $scenario in
  S1|S2|S3|S4|S5|S6|S10) write_charter ;;
  S7|S8) ;;
  S9) write_charter "skip:Lanes" "skip:Autonomy" ;;
  *) echo "unknown scenario $scenario" >&2; exit 1 ;;
esac

git -C "$repo" add -A
git -C "$repo" -c user.name=scratch -c user.email=scratch@example.invalid -c commit.gpgsign=false \
  commit -qm "scratch"

# The board.
cat > "$dir/board/workspaces.json" <<EOF
{"workspaces":[{"id":"00000000-0000-0000-0000-000000000001","short":"00000001","task":"main","branch":"main","repository":"scratch","worktree":"$repo","state":"ready","is_main_checkout":true,"terminals":[]}]}
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
    printf 'fc-4  Fix the failing add test\nstatus: in_progress\nworkspace: fix-add\n' > "$dir/board/fc-4.txt"
    cat > "$dir/board/workspaces.json" <<EOF
{"workspaces":[{"id":"00000000-0000-0000-0000-000000000001","short":"00000001","task":"main","branch":"main","repository":"scratch","worktree":"$repo","state":"ready","is_main_checkout":true,"terminals":[]},{"id":"00000000-0000-0000-0000-000000000002","short":"00000002","task":"fix-add","branch":"fix-add","repository":"scratch","worktree":"$dir/fix-add","state":"ready","is_main_checkout":false,"terminals":[{"short":"0000000a","title":"claude","preset":"claude","state":"running","activity":"working"}]}]}
EOF
    ;;
  S9)
    printf 'KEY   STATUS  AGE  TITLE\nfc-1  todo    2d   Tidy the README\nfc-2  backlog 5d   Add a subtract test\n' > "$dir/board/list.txt" ;;
  S10)
    # fc-2 is ready to go, and the only workspace is the main checkout, where
    # the manager's own claude pane is running: dispatching into it would put
    # a second writer in the manager's tree.
    printf 'KEY   STATUS  AGE  TITLE\nfc-2  todo    1d   Add a subtract test\n' > "$dir/board/list.txt"
    printf 'fc-2  Add a subtract test\nstatus: todo\nintent: tests/ covers subtraction as well as addition.\nacceptance:\n  [ ] tests/test_subtract.sh checks 5 - 3 = 2\n' > "$dir/board/fc-2.txt"
    cat > "$dir/board/workspaces.json" <<EOF
{"workspaces":[{"id":"00000000-0000-0000-0000-000000000001","short":"00000001","task":"main","branch":"main","repository":"scratch","worktree":"$repo","state":"ready","is_main_checkout":true,"terminals":[{"short":"0000000m","title":"manager","preset":"claude","state":"running","activity":"working"}]}]}
EOF
    ;;
  *) printf 'no tasks\n' > "$dir/board/list.txt" ;;
esac

# The CLI the agent is told about. A wrapper rather than a copy, so the log and
# the board are baked in: a subagent's shell carries none of our environment.
real=$(cd "$here/../.." && pwd)/target/debug/farcooler
cat > "$dir/farcooler" <<EOF
#!/bin/bash
export FAKE_LOG="$dir/log" FAKE_BOARD="$dir/board" REAL_FARCOOLER="$real"
exec "$here/fake-farcooler.sh" "\$@"
EOF
chmod +x "$dir/farcooler"

if [ "$mode" != "--baseline" ]; then
  "$here/render-skill.sh" "$dir/farcooler" "$dir/skill.md" >/dev/null
fi

cat <<EOF
scenario:   $scenario ${mode:-(with the skill)}
repository: $repo
fake cli:   $dir/farcooler
skill:      $([ "$mode" = "--baseline" ] && echo "(none: baseline)" || echo "$dir/skill.md")
log:        $dir/log
EOF
