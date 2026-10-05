#!/bin/bash
# A stand-in for `farcooler` that logs every call and changes nothing.
#
# The manager skill's pressure scenarios (scenarios.md) score an agent by what
# it RAN, not by what it said it ran. Every invocation appends its argv as one
# JSON array to $FAKE_LOG. Reads print canned text from $FAKE_BOARD:
#
#   task list ...           -> $FAKE_BOARD/list.txt
#   task show <key> ...     -> $FAKE_BOARD/<key>.txt, else the task's row in
#                              list.txt, else a task this world created; the
#                              whole file, whatever --fields asks for
#   task show <key> --json  -> $FAKE_BOARD/<key>.json, else the row as JSON
#                              with no constraints; refused with --fields, as
#                              the real CLI refuses it
#   worktree list ...       -> $FAKE_BOARD/worktrees.json
#   task search ...         -> $FAKE_BOARD/search.txt
#   workspace show <ws> ... -> the workspace in $FAKE_BOARD/workspaces.tsv
#                              (id, name, prefix, home) named by its id, name
#                              or prefix, as the real `workspace show` prints
#                              it, or under --json as its JSON object
#   workspace list ...      -> every workspace in workspaces.tsv
#   plan, plan lane|theme|ruling list|show
#                           -> $FAKE_BOARD/plan.txt
#
# Writes print one plausible line and exit 0. `task create` hands out a new key
# each time -- fc-9, fc-10, ... -- from the counter $FAKE_BOARD/next-key, and
# records it in $FAKE_BOARD/created.txt so `task show` answers for it.
# `workspace create` adds a row to workspaces.tsv and gives the new workspace a
# home under $FAKE_HOMES holding a copy of Main's charter, as the real one does,
# so a split can be scored on whether that charter was edited. `--help` is
# handed to the real CLI named by $REAL_FARCOOLER when there is one, so a
# baseline agent sees the real command tree; clap answers `--help` without
# reaching a daemon.
set -u
: "${FAKE_LOG:?FAKE_LOG must name the log file}"
: "${FAKE_BOARD:?FAKE_BOARD must name the board fixture directory}"

# Flags a shell didn't split (`"$FLAGS"` holding `--repo x --actor manager`)
# reach the real CLI as one argument, and clap refuses it. Refuse it here too,
# so an agent sees the error it would see for real. `--body=two words` is one
# flag, and clap takes it. A refused call goes to $FAKE_LOG.refused, not the
# log: it read nothing and set nothing.
for a in "$@"; do
  case "$a" in
    --*" "*)
      case "${a%%" "*}" in
        *=*) ;;
        *)
          python3 -c 'import json, sys; print(json.dumps(sys.argv[1:]))' "$@" >> "$FAKE_LOG.refused"
          echo "error: unexpected argument '$a' found" >&2
          exit 2
          ;;
      esac
      ;;
  esac
done

python3 -c 'import json, sys; print(json.dumps(sys.argv[1:]))' "$@" >> "$FAKE_LOG"

for a in "$@"; do
  if [ "$a" = "--help" ] || [ "$a" = "-h" ] || [ "$a" = "help" ]; then
    if [ -n "${REAL_FARCOOLER:-}" ] && [ -x "$REAL_FARCOOLER" ]; then
      exec "$REAL_FARCOOLER" "$@"
    fi
    echo "farcooler task {list,show,create,set,note,ask,block,search,dispatch,move}; worktree {create,list,assign,remove}; workspace {create,list,show,start-orchestrator}; plan {set,lane,theme}; page {set,list}; see the skill"
    exit 0
  fi
done

# Global flags may come before the subcommand (`--json worktree list`).
json=0
for a in "$@"; do [ "$a" = "--json" ] && json=1; done
args=("$@")
while [ ${#args[@]} -gt 0 ]; do
  case "${args[0]}" in
    --json) args=("${args[@]:1}") ;;
    --runner|--host) args=("${args[@]:2}") ;;
    *) break ;;
  esac
done
set -- ${args[@]+"${args[@]}"}

show_file() { if [ -f "$1" ]; then cat "$1"; else echo "$2"; fi; }

# `task show <key>`: the world's own file for the key when it has one, then the
# key's row in list.txt (KEY STATUS AGE TITLE), then a task created here.
show_task() {
  local key=$1
  if [ -n "$key" ] && [ -f "$FAKE_BOARD/$key.txt" ]; then cat "$FAKE_BOARD/$key.txt"; return; fi
  local row
  row=$(awk -v k="$key" '$1 == k && k != "KEY" { print; exit }' "$FAKE_BOARD/list.txt" 2>/dev/null)
  if [ -n "$key" ] && [ -n "$row" ]; then
    echo "$row" | awk '{ t = $4; for (i = 5; i <= NF; i++) t = t " " $i; print $1 "  " t; print "status: " $2 }'
    return
  fi
  row=$(awk -F '\t' -v k="$key" '$1 == k { print; exit }' "$FAKE_BOARD/created.txt" 2>/dev/null)
  if [ -n "$key" ] && [ -n "$row" ]; then
    printf '%s  %s\nstatus: backlog\n' "$key" "$(echo "$row" | cut -f2)"
    return
  fi
  echo "no task $key"
}

# `task show --json`: the world's JSON card for the key, else the row
# `task show` would print, in the real JSON's shape with nothing on it. The
# real CLI refuses `--json` with `--fields`, and so does this.
show_task_json() {
  local key=$1 a
  for a in "${@:2}"; do
    case "$a" in --fields|--fields=*)
      echo "error: --fields chooses what a person reads. --json answers with the whole task, so a parser gets one shape every time. use --notes to narrow the history in either" >&2
      exit 1 ;;
    esac
  done
  if [ -n "$key" ] && [ -f "$FAKE_BOARD/$key.json" ]; then cat "$FAKE_BOARD/$key.json"; return; fi
  local row
  row=$(awk -v k="$key" '$1 == k && k != "KEY" { print; exit }' "$FAKE_BOARD/list.txt" 2>/dev/null)
  if [ -z "$key" ] || [ -z "$row" ]; then echo "error: no task $key" >&2; exit 1; fi
  python3 -c 'import json, sys
k, s, *t = sys.argv[1].split()[:1] + sys.argv[1].split()[1:2] + sys.argv[1].split()[3:]
print(json.dumps({"task": {"key": k, "title": " ".join(t), "status": s, "intent": "",
                           "acceptance": [], "constraints": [], "labels": []},
                  "notes": [], "blocks": []}))' "$row"
}

# The key `task show` was asked about: the first word after `show` that is not
# a flag or a flag's value. `--json` and `--runner` are global in the real CLI
# and may come anywhere, so `task show --json fc-1` names fc-1.
shown_key() {
  shift 2
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo|--fields|--notes|--runner|--host) shift; [ $# -gt 0 ] && shift ;;
      -*) shift ;;
      *) echo "$1"; return ;;
    esac
  done
}

# `task create`: the next key from the world's counter, starting at fc-9.
# Under a `mkdir` lock -- atomic, and macOS has no `flock` -- so two creates at
# once can't read the same number, or catch the file mid-write and read
# nothing.
create_task() {
  local n tries=0
  until mkdir "$FAKE_BOARD/.next-key.lock" 2>/dev/null; do
    tries=$((tries + 1))
    if [ $tries -gt 500 ]; then echo "fake farcooler: the key counter's lock is stuck" >&2; exit 1; fi
    sleep 0.01
  done
  # Released however this exits: a fake killed mid-create must not leave the
  # lock behind for every later create in this world to spin on.
  trap 'rmdir "$FAKE_BOARD/.next-key.lock" 2>/dev/null' EXIT
  n=$(cat "$FAKE_BOARD/next-key" 2>/dev/null)
  case "$n" in ''|*[!0-9]*) n=9 ;; esac
  echo $((n + 1)) > "$FAKE_BOARD/next-key"
  rmdir "$FAKE_BOARD/.next-key.lock"
  # Dropped once released, or it would remove the NEXT holder's lock at exit.
  trap - EXIT
  local title="" prev="" a
  for a in "$@"; do
    case "$a" in --title=*) title=${a#--title=} ;; esac
    [ "$prev" = "--title" ] && title=$a
    prev=$a
  done
  printf 'fc-%s\t%s\n' "$n" "$title" >> "$FAKE_BOARD/created.txt"
  echo "fc-$n  created"
}

# The workspace a word names, as its workspaces.tsv line: by id (whole, or
# its end), by name or by prefix, ignoring case. The first line, Main, for an
# empty word: an agent that ran `workspace show "$FARCOOLER_WORKSPACE"` without
# sourcing pane.env asked about nothing, and the log shows the empty word.
find_workspace() {
  local word
  word=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
  if [ -z "$word" ]; then head -n 1 "$FAKE_BOARD/workspaces.tsv"; return; fi
  awk -F '\t' -v w="$word" '
    { id = tolower($1); gsub("-", "", id); ww = w; gsub("-", "", ww)
      if (tolower($2) == w || tolower($3) == w || (length(ww) >= 4 && substr(id, length(id) - length(ww) + 1) == ww)) { print; exit } }
  ' "$FAKE_BOARD/workspaces.tsv"
}

# One workspace as `workspace show` prints it, or its JSON object.
print_workspace() {
  local id name prefix home main=false
  IFS=$'\t' read -r id name prefix home <<<"$1"
  [ "$name" = "Main" ] && main=true
  if [ "$json" = 1 ]; then
    printf '{"id":"%s","short":"%s","repository":"00000000-0000-0000-0000-0000000000ee","name":"%s","task_prefix":"%s","is_main":%s,"ordinal":0,"orchestrator":null,"home":"%s","charter":"%s/charter.md"}\n' \
      "$id" "${id: -8}" "$name" "$prefix" "$main" "$home" "$home"
  else
    printf '%s  %s%s\n' "${id: -8}" "$name" "$([ "$main" = true ] && echo '  (main)')"
    printf '  repository    scratch\n  task prefix   %s\n  orchestrator  none running\n  charter       %s/charter.md\n' "$prefix" "$home"
    printf '  worktrees     none listed by this fake: see `worktree list --json`\n'
  fi
}

# The first word after `<noun> <verb>` that isn't a flag or a flag's value.
first_word() {
  shift 2
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo|--name|--prefix|--harness|--read|--to|--actor|--runner|--host|--file|--theme|--confirm) shift; [ $# -gt 0 ] && shift ;;
      -*) shift ;;
      *) echo "$1"; return ;;
    esac
  done
}

# Every such word: the keys of `task move`.
positional_words() {
  shift 2
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo|--name|--prefix|--harness|--read|--to|--actor|--runner|--host) shift; [ $# -gt 0 ] && shift ;;
      -*) shift ;;
      *) echo "$1"; shift ;;
    esac
  done
}

# A flag's value, as `--flag value` or `--flag=value`.
flag_value() {
  local flag=$1 prev="" a
  shift
  for a in "$@"; do
    case "$a" in "$flag="*) echo "${a#"$flag="}"; return ;; esac
    [ "$prev" = "$flag" ] && { echo "$a"; return; }
    prev=$a
  done
}

# `workspace create`: a new row, and a home holding a copy of Main's charter.
create_workspace() {
  local name prefix n home main_home
  name=$(flag_value --name "$@")
  prefix=$(flag_value --prefix "$@")
  if [ -z "$name" ] || [ -z "$prefix" ]; then
    echo "error: workspace create needs --name and --prefix" >&2
    exit 2
  fi
  n=$(($(wc -l < "$FAKE_BOARD/workspaces.tsv") + 1))
  home="$FAKE_HOMES/$n"
  mkdir -p "$home"
  main_home=$(head -n 1 "$FAKE_BOARD/workspaces.tsv" | cut -f4)
  if [ -f "$main_home/charter.md" ]; then
    cp "$main_home/charter.md" "$home/charter.md"
    # What the copy said, for score.py's "edited down": Main's charter may
    # have changed before the split, so charter.orig isn't the baseline.
    cp "$main_home/charter.md" "$home/charter.at-create"
  fi
  printf '00000000-0000-0000-0000-00000000b%03d\t%s\t%s\t%s\n' "$n" "$name" "$prefix" "$home" \
    >> "$FAKE_BOARD/workspaces.tsv"
  print_workspace "$(tail -n 1 "$FAKE_BOARD/workspaces.tsv")"
}

# The plan and pages (ov-217). Reads print $FAKE_BOARD/plan.txt; writes
# print one line and change nothing, like every other write here.
if [ "${1:-}" = plan ]; then
  case "${2:-}" in
    lane|theme|ruling)
      case "${3:-}" in
        list|show) show_file "$FAKE_BOARD/plan.txt" "no plan yet" ;;
        *) echo "plan ${2} ${3:-?} recorded" ;;
      esac ;;
    set) echo "next up replaced" ;;
    *) show_file "$FAKE_BOARD/plan.txt" "no plan yet" ;;
  esac
  exit 0
fi
if [ "${1:-}" = page ]; then
  case "${2:-}" in
    set) echo "page $(first_word "$@") published" ;;
    rm) echo "page removed" ;;
    *) echo "no pages" ;;
  esac
  exit 0
fi

case "${1:-} ${2:-}" in
  "task list")      show_file "$FAKE_BOARD/list.txt" "no tasks" ;;
  "task show")      if [ "$json" = 1 ]; then show_task_json "$(shown_key "$@")" "$@"
                    else show_task "$(shown_key "$@")"; fi ;;
  "task search")    show_file "$FAKE_BOARD/search.txt" "no notes match" ;;
  "task create")    create_task "$@" ;;
  "task set")       echo "${3:-fc-?}  updated" ;;
  "task note")      echo "${3:-fc-?}  noted" ;;
  "task ask")       echo "${3:-fc-?}  needs decision" ;;
  "task block")     echo "${3:-fc-?}  blocked" ;;
  "task wait")      echo "${3:-fc-?}  waiting" ;;
  "task line")      echo "line updated" ;;
  "task worker")    echo "${3:-fc-?}  subagent recorded" ;;
  # What the real `task dispatch` prints, word for word but for the ids.
  "task dispatch")  echo "${3:-fc-?} is in progress in the new lane, terminal 0000abcd"
                    echo "  it won't report back by itself: check the board or \`worktree list --json\`" ;;
  "task move")      for k in $(positional_words "$@"); do echo "$k  moved to $(flag_value --to "$@")"; done ;;
  "worktree list") show_file "$FAKE_BOARD/worktrees.json" '{"worktrees":[]}' ;;
  "worktree create") echo "created worktree ${4:-}" ;;
  "worktree remove") echo "removed worktree $(first_word "$@"); its branch is kept" ;;
  "worktree assign") echo "$(first_word "$@") now belongs to $(flag_value --to "$@")" ;;
  "workspace show"|"workspace start-orchestrator")
                    ws=$(find_workspace "$(first_word "$@")")
                    if [ -z "$ws" ]; then echo "error: no workspace matching \"$(first_word "$@")\"" >&2; exit 1; fi
                    if [ "$2" = "show" ]; then print_workspace "$ws"
                    else echo "started the orchestrator of $(echo "$ws" | cut -f2), terminal 0000orch"; fi ;;
  "workspace list") while IFS= read -r ws; do print_workspace "$ws"; done < "$FAKE_BOARD/workspaces.tsv" ;;
  "workspace create") create_workspace "$@" ;;
  *)                echo "ok" ;;
esac
exit 0
