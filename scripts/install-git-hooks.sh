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
  cat > "$dest" <<STUB
#!/bin/bash
$mark
hook="\$(git rev-parse --show-toplevel)/scripts/git-hooks/$name"
if [ -x "\$hook" ]; then exec "\$hook" "\$@"; fi
STUB
  chmod +x "$dest"
  echo "installed $name"
done
