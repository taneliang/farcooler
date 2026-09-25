#!/bin/bash
# Write the manager skill exactly as claude's pane gets it, with every command
# pointing at a fake CLI instead of the real one.
#
#   render-skill.sh <fake-cli-path> <out-file>
#
# The render is the daemon's own (`skill_install::render`), reached through an
# ignored test, so the text a scenario tests is the text an agent reads and
# nothing here re-implements the template.
set -euo pipefail
fake=$1
out=$2
repo=$(cd "$(dirname "$0")/../.." && pwd)
export PATH="/opt/homebrew/bin:$HOME/.cargo/bin:$PATH"
cd "$repo"
FAKE_CLI="$fake" SKILL_OUT="$out" CARGO_BUILD_JOBS=6 \
  cargo test -q -p farcooler-daemon --lib render_for_the_pressure_harness -- --ignored >/dev/null
test -s "$out" || { echo "render-skill: nothing was written to $out" >&2; exit 1; }
grep -q '{{' "$out" && { echo "render-skill: a placeholder was left open in $out" >&2; exit 1; }
echo "$out"
