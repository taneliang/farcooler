#!/bin/bash
# A stand-in for `farcooler` that logs every call and changes nothing.
#
# The manager skill's pressure scenarios (scenarios.md) score an agent by what
# it RAN, not by what it said it ran. Every invocation appends its argv as one
# JSON array to $FAKE_LOG. Reads print canned text from $FAKE_BOARD:
#
#   task list ...           -> $FAKE_BOARD/list.txt
#   task show <key> ...     -> $FAKE_BOARD/<key>.txt, else the task's row in
#                              list.txt, else a task this world created
#   workspace list ...      -> $FAKE_BOARD/workspaces.json
#   task search ...         -> $FAKE_BOARD/search.txt
#
# Writes print one plausible line and exit 0. `task create` hands out a new key
# each time -- fc-9, fc-10, ... -- from the counter $FAKE_BOARD/next-key, and
# records it in $FAKE_BOARD/created.txt so `task show` answers for it. `--help` is handed to the real
# CLI named by $REAL_FARCOOLER when there is one, so a baseline agent sees the
# real command tree; clap answers `--help` without reaching a daemon.
set -u
: "${FAKE_LOG:?FAKE_LOG must name the log file}"
: "${FAKE_BOARD:?FAKE_BOARD must name the board fixture directory}"

python3 -c 'import json, sys; print(json.dumps(sys.argv[1:]))' "$@" >> "$FAKE_LOG"

for a in "$@"; do
  if [ "$a" = "--help" ] || [ "$a" = "-h" ] || [ "$a" = "help" ]; then
    if [ -n "${REAL_FARCOOLER:-}" ] && [ -x "$REAL_FARCOOLER" ]; then
      exec "$REAL_FARCOOLER" "$@"
    fi
    echo "farcooler task {list,show,create,set,note,ask,block,search,dispatch}; workspace {create,list}; see the skill"
    exit 0
  fi
done

# Global flags may come before the subcommand (`--json workspace list`).
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

case "${1:-} ${2:-}" in
  "task list")      show_file "$FAKE_BOARD/list.txt" "no tasks" ;;
  "task show")      show_task "$(shown_key "$@")" ;;
  "task search")    show_file "$FAKE_BOARD/search.txt" "no notes match" ;;
  "task create")    create_task "$@" ;;
  "task set")       echo "${3:-fc-?}  updated" ;;
  "task note")      echo "${3:-fc-?}  noted" ;;
  "task ask")       echo "${3:-fc-?}  needs decision" ;;
  "task block")     echo "${3:-fc-?}  blocked" ;;
  # What the real `task dispatch` prints, word for word but for the ids.
  "task dispatch")  echo "${3:-fc-?} is in progress in the new lane, terminal 0000abcd"
                    echo "  it won't report back by itself: check the board or \`workspace list --json\`" ;;
  "workspace list") show_file "$FAKE_BOARD/workspaces.json" '{"workspaces":[]}' ;;
  "workspace create") echo "created workspace ${4:-}" ;;
  *)                echo "ok" ;;
esac
exit 0
