#!/bin/bash
# A stand-in for `farcooler` that logs every call and changes nothing.
#
# The manager skill's pressure scenarios (scenarios.md) score an agent by what
# it RAN, not by what it said it ran. Every invocation appends its argv as one
# JSON array to $FAKE_LOG. Reads print canned text from $FAKE_BOARD:
#
#   task list ...           -> $FAKE_BOARD/list.txt
#   task show <key> ...     -> $FAKE_BOARD/<key>.txt
#   workspace list ...      -> $FAKE_BOARD/workspaces.json
#   task search ...         -> $FAKE_BOARD/search.txt
#
# Writes print one plausible line and exit 0. `--help` is handed to the real
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

case "${1:-} ${2:-}" in
  "task list")      show_file "$FAKE_BOARD/list.txt" "no tasks" ;;
  "task show")      show_file "$FAKE_BOARD/${3:-none}.txt" "no task ${3:-}" ;;
  "task search")    show_file "$FAKE_BOARD/search.txt" "no notes match" ;;
  "task create")    echo "fc-9  created" ;;
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
