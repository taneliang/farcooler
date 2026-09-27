#!/bin/bash
# Opt this clone, and every worktree of it, into the repo's git hooks.
#
# Writes a two-line stub for each hook in scripts/git-hooks/ into the clone's
# shared hooks directory. The stub runs the committing worktree's own copy, so
# a lane on an older branch runs that branch's checks, and removing the
# worktree this was installed from strands nothing. Leaves git config alone (no
# core.hooksPath), and won't replace a hook that isn't one of these stubs.
#
#   ./scripts/install-git-hooks.sh
#   ./scripts/install-git-hooks.sh --uninstall
set -euo pipefail

cd "$(dirname "$0")/.."
hooks="$(git rev-parse --path-format=absolute --git-common-dir)/hooks"
mark="# installed by scripts/install-git-hooks.sh"
mkdir -p "$hooks"

# With core.hooksPath set, git never reads the hooks directory at all, so a
# stub written there would sit inert and say nothing.
if hooks_path="$(git config --get core.hooksPath)" && [ "${1:-}" != "--uninstall" ]; then
  echo "warning: core.hooksPath is set to '$hooks_path', so git won't run hooks from $hooks." >&2
  echo "         Link or copy scripts/git-hooks/* into '$hooks_path' instead." >&2
fi

# The stub this script writes for a hook of that name, byte for byte.
stub() {
  cat <<STUB
#!/bin/bash
$mark
hook="\$(git rev-parse --show-toplevel)/scripts/git-hooks/$1"
if [ -x "\$hook" ]; then exec "\$hook" "\$@"; fi
STUB
}

# Hooks this repo once shipped under another name. A stub for one runs nothing
# (its target is gone) but would sit there looking installed, so both install
# and uninstall take it out. Only when it is exactly the stub written here: a
# hook someone else wrote, or one edited since, is left alone.
#   pre-commit  became commit-msg, so the check can read the message's trailer
for name in pre-commit; do
  dest="$hooks/$name"
  if [ -f "$dest" ] && [ ! -e "scripts/git-hooks/$name" ] \
    && [ "$(cat "$dest")" = "$(stub "$name")" ]; then
    rm "$dest"
    echo "removed the retired $name stub"
  fi
done

for src in scripts/git-hooks/*; do
  name="$(basename "$src")"
  dest="$hooks/$name"
  ours=false
  if [ -f "$dest" ] && grep -qxF "$mark" "$dest"; then ours=true; fi
  if [ "${1:-}" = "--uninstall" ]; then
    if $ours; then rm "$dest"; echo "removed $name"; fi
    continue
  fi
  if [ -e "$dest" ] && ! $ours; then
    echo "$dest already exists and isn't one of this repo's stubs; leaving it" >&2
    exit 1
  fi
  stub "$name" > "$dest"
  chmod +x "$dest"
  echo "installed $name"
done
