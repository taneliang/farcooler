# Workspaces are workstreams

Date: 2026-09-27
Status: design approved in conversation; spec awaiting review
Builds on: [`2026-09-08-agent-factory-design.md`](2026-09-08-agent-factory-design.md)

## Problem

The board sits outside the thing it is about. The Mac sidebar reads
*repository → Board → worktrees*, one board per repository, and the orchestrator
that drives it is an ordinary terminal in the main checkout that nothing records
as the orchestrator. The intended way of working is the opposite: an
orchestrator, its board, and the worktrees its agents are using are one unit.

And one per repository is too few. A large repository at work carries several
long-running workstreams at once. Trying to run all of them through one
orchestrator muddles the board and, worse, the **conversation**: the agent and
the user both lose track when one session context-switches between large,
unrelated things. The way this happens in practice is gradual — everything
starts in one orchestrator, a thread grows until it is crowding the rest, and it
gets split off into an orchestrator of its own. Splitting has to be cheap.

## The model

**A workspace is a workstream:** a name, a task prefix, a board, a charter, at
most one live orchestrator, and the worktrees it owns. Every repository has one
workspace called Main, from which others are split off. A workspace lives in one
repository for now.

**A worktree is the directory and branch**, and everything done to one: its
diff, committed and uncommitted, is how you watch an agent's progress. Each
worktree belongs to at most one workspace. Worktrees are disposable; agents make
and remove them freely.

**The orchestrator lives outside the repository**, in the workspace's home under
`$FARCOOLER_HOME`. So the main checkout is not special: it is Main's worktree,
like any other. Other workspaces' orchestrators may still run in it when their
harness requires (see "Launch recipes"), which is why a terminal records both
where it runs and whose work it is doing.

The word "workspace" moves up a level. What the product called a workspace
until now — one worktree plus branch — is renamed *worktree* everywhere that is
what it means.

## Data model

### `workspaces` (new)

```
id              uuid
repository_id   uuid, cascades from repositories
name            text, the user's; "Main" for the first
task_prefix     text, the user's; unique per runner
is_main         bool; exactly one per repository; cannot be deleted
ordinal         u32, order within the repository
resource_version
```

- **The home** is `$FARCOOLER_HOME/workspaces/<id>/`, keyed by id so a rename
  never moves files. It holds `charter.md` and serves as the orchestrator's
  working directory. Nothing else is put there.
- **Changing the prefix** affects only tasks created afterwards. Keys are stored
  whole on the task row (`tasks.key`, as today), so every key already written
  down or spoken keeps resolving. A new number is one more than the highest
  existing number under that prefix anywhere on the runner, so `bil-3` is never
  issued twice, even after renaming a prefix away and back.
- **Prefix uniqueness** is per runner and checked when set, because a moved
  task keeps its key and keys must resolve without naming a workspace.
- **Deleting** a workspace is refused while it holds tasks or worktrees; move
  them first. Main cannot be deleted. Deleting a repository still cascades
  through everything, as it does today.

### `worktrees` (renamed from `workspaces`, same rows)

- New `workspace_id`, **nullable**: the owner. Null means unclaimed.
- `ordinal` now orders a worktree within its workspace.
- Everything else (`hidden`, `is_main_checkout`, `worktree_missing`, …) is
  unchanged.

### `terminals`

- `workspace_id` is renamed `worktree_id`: where it runs. Same meaning as now.
- New `workspace_id`: whose work it is. Usually the owner of its worktree, but
  not always — a Codex orchestrator for Billing runs in the main checkout,
  which Main owns, and a quick investigation Billing starts there is still
  Billing's.
- New `role`: `orchestrator | agent | shell`. At most one live orchestrator per
  workspace.

### `tasks`

- New `workspace_id`, not null: the board it is on.
- The old `workspace_id` (the lane) is renamed `worktree_id`, still optional.
- `repository_id` stays; it is derivable, but keeps the cascade simple.
- `repositories.task_key_prefix` retires; the prefix lives on the workspace.

### Migration

Kept small: there is one user, and every app updates together. It must preserve
existing boards — tasks, keys, notes — and nothing more.

- Each repository gets a Main workspace whose prefix is the repository's
  current `task_key_prefix`.
- Every worktree, task, and terminal is assigned to its repository's Main.
  Every terminal's role starts as `agent`; an orchestrator running today is
  re-tagged with `farcooler terminal set-role` or by restarting it through
  `start-orchestrator`.
- A `.farcooler/manager.md` found in a repository is copied to Main's
  `charter.md`. The file in the repository is left alone; removing it is the
  user's commit to make.

## Claiming worktrees

`reconcile.rs` already adopts every worktree git lists. Claiming decides the
owner of an adopted, unclaimed worktree. Signals, strongest first; the first
claim wins and sticks:

1. **Explicit.** `farcooler worktree create`, `farcooler worktree assign`, and
   dispatch name the workspace. Every pane Far Cooler launches carries
   `FARCOOLER_WORKSPACE`, so an agent running `farcooler worktree create` claims
   for its own workspace without a flag.
2. **The agent's hooks.** Far Cooler already installs hooks in all three
   harnesses, and every hook payload carries the `cwd` the agent is working
   in. `hook_ingress.rs` (`announced_terminal`) already matches a hook's `cwd`
   to a worktree for Codex and Cursor. A hook from a terminal whose `cwd` is
   inside an unclaimed worktree claims it for that terminal's workspace. This
   signal does not depend on whether the harness moves its own process.
   (Parsing session logs was considered first; hooks are the same fact,
   already ingested, and arrive per tool call.)
3. **The process tree.** For harnesses with no readable log: walk the pane's
   processes (as `foreground.rs` already reads the process table per tty) and
   read each working directory — `proc_pidinfo` on macOS, `/proc/<pid>/cwd` on
   Linux. Weakest, because a short-lived subshell can fall between scans.
4. **Nothing matched:** the worktree stays unclaimed.

A `cwd` is matched to the worktree with the **longest** path containing it.
Worktrees are often nested inside the main checkout (`.worktrees/x`), and a
shortest or first match would hand every nested worktree's activity to Main.

Rules on top:

- **Orchestrators never claim.** Terminals with `role = orchestrator` are
  skipped by signals 2 and 3; otherwise a Codex orchestrator for Billing would
  claim the main checkout.
- **No stealing.** A terminal of workspace B seen working in a worktree A owns
  does not move ownership. It is reported instead: in `farcooler worktree list`,
  and as a warning on dispatch ("a terminal from Billing is working in a
  worktree Main owns"). This is the two-writers hazard the agent-factory design
  recorded, now visible across workstreams. Moving ownership is
  `farcooler worktree assign`.
- `worktree list` shows which signal made each claim, so a wrong claim is
  diagnosable.
- Removing a worktree removes its claim. The existing rule — never drop a row
  that still holds terminals — is unchanged.

## The orchestrator

### Charter

`$FARCOOLER_HOME/workspaces/<id>/charter.md`: the user's instructions for this
workstream, the same open-ended prose the agent-factory design described. **Not
committed to the repository.** It is personal: coworkers are not running this
orchestrator, and the user syncs or shares the file however they like. A new
workspace's charter starts as a copy of Main's.

Every orchestrator pane carries `FARCOOLER_WORKSPACE=<id>` and
`FARCOOLER_CHARTER=<path>`. The manager skill's first step is to read
`$FARCOOLER_CHARTER`, and every wake-up prompt tells it to re-read it. One
mechanism for every harness, and one that survives compaction, rather than
relying on three harnesses' instruction-file rules.

### Launch recipes

`farcooler workspace start-orchestrator <ws> --harness claude|codex|cursor`
launches the orchestrator with `role = orchestrator`, installs the manager skill
the way `skill_install.rs` already does per harness, and applies:

| | Working directory | Repository context | Memory |
|---|---|---|---|
| Claude Code | the home | `--add-dir <main checkout>` and `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` | `autoMemoryDirectory` pointed at the repository's memory directory, through the `--settings` file Far Cooler already writes |
| Cursor | the home | `--workspace <main checkout>` | none documented |
| Codex | the main checkout (`--cd`) | native | none |

Why they differ, measured against the documentation and source on 2026-09-27:

- **Claude Code** loads `CLAUDE.md` from an added directory only with that
  environment variable, and discovers skills from it. Its auto-memory
  directory is chosen from the git root of the working directory, which the
  home does not have, hence the override.
- **Cursor**'s `--workspace` "sets an explicit repository root", from which it
  reads `AGENTS.md`, `CLAUDE.md`, and `.cursor/rules`.
- **Codex** anchors `AGENTS.md`, skills, and `.codex/config.toml` to the
  working directory's walk up to `.git`. `--add-dir` is only a sandbox write
  grant. So Codex must run inside the repository.

**Memory is shared with the repository by default**: an orchestrator reads and
writes the same memory directory as every session in the repository, because
almost everything in it is about the repository. Focus comes from the
conversation and the board, not from memory. Per-workspace memory can be an
option later.

**At most one live orchestrator per workspace.** A second `start-orchestrator`
is refused; `--replace` closes the old pane first.

### Wake-ups and events

- Board change events carry the task's workspace, so a board view re-reads
  only when its own board moved.
- **The wake loop does not exist yet** (`skill_install.rs`:
  `WAKE_LOOP_EXISTS = false`), and this work does not build it. What it does is
  record what the wake loop will need: which terminal is a workspace's
  orchestrator (`role`), and which workspace every terminal and task belongs
  to. When the loop is built it delivers a change on workspace W to W's live
  orchestrator, under its two existing rules, and a workspace with no live
  orchestrator is not woken.
- `NeedsDecision` is not pushed today either, and no push or Live Activity
  payload names a repository (the "workspace" in a notification's subtitle is
  the worktree's name). Nothing here changes push.

### Splitting is done by agents

The first cut ships the operations, not a split command. The manager skill gains
a section on splitting a workstream off, which an orchestrator follows when the
user asks:

1. `farcooler workspace create --name … --prefix …`
2. Edit the new charter (copied from Main's) down to this workstream.
3. `farcooler task move <key>… --to <ws>` and
   `farcooler worktree assign <worktree> --to <ws>`.
4. Write the handoff: a `Decision` note on each moved task stating why it moved,
   and one handoff note carrying what the old conversation knew that the board
   does not.
5. `farcooler workspace start-orchestrator <ws>`, pointing it at the handoff.

Step 4 is required, not optional: a thin handoff strands context in the old
conversation, which defeats the split.

## The rename

Old *workspace* becomes *worktree* wherever that is what it means, in one
behavior-free lane that lands before any new behavior:

- Store: the `workspaces` table, `Workspace` struct, and `workspace_id` columns
  that mean the worktree.
- Proto: message and field names only. **Field numbers are frozen.** Protobuf
  never puts a name on the wire, so `WorkspaceCreate workspace_create = 23`
  becoming `WorktreeCreate worktree_create = 23` is byte-identical. The new
  workspace messages take fresh tags.
- The method labels in `crates/protocol/src/lib.rs` (`"workspace.create"` and
  its kin), which gate capabilities. The existing `worktree.list` (worktrees
  git has that Far Cooler has not adopted) and its `WorktreeList` /
  `ExistingWorktree` messages would collide, so they become
  `worktree.discover` and `DiscoveredWorktreeList` / `DiscoveredWorktree`.
- Capability **values** advertised on the wire stay as they are; only the Rust
  constant names change.
- Rust, Swift, and Kotlin types and identifiers.
- App copy: a worktree row, "New Workspace", "Find Workspace or Agent", and the
  rest become *worktree* where they mean the directory.
- `docs/workspaces.md` is rewritten: *workspace* is the workstream; *worktree*
  is the directory and branch. Its "what does not rename" section goes: the
  reason for it — shell history and scripts — does not outweigh one vocabulary
  for one user.

## CLI

```
farcooler worktree   create | list | hide | unhide | reorder | remove | assign
                     (plus the existing file search)
farcooler workspace  create --name … --prefix …
                     list | show | rename | set-prefix | delete
                     start-orchestrator <ws> --harness … [--replace]
farcooler task       move <key>… --to <ws>
                     list: --workspace <ws> names a board; otherwise
                     $FARCOOLER_WORKSPACE's; otherwise the whole repository,
                     each row naming its workspace
farcooler terminal   set-role <terminal> orchestrator|agent|shell
```

`farcooler workspace create --branch …`, the old spelling, fails with a pointer
to `farcooler worktree create`; the `--branch` flag makes it detectable, so a
stale script errors instead of creating a workspace named after a branch.

The manager skill moves in the same release: `$FARCOOLER_CHARTER` in place of
`.farcooler/manager.md`, the splitting section, and *worktree* wherever it
means the directory.

## Apps

No new actions in the first cut. Creating, moving, and splitting happen through
the CLI.

- **Mac sidebar:** *repository → workspace → Board row, orchestrator row,
  worktrees*, plus one collapsed "Unclaimed" group per repository. The
  workspace level is **always shown**, even when Main is the only one, so the
  model is visible before the first split. Main is drawn like any other
  workspace.
- The command palette and the attention cycle search and step across
  workspaces as they do now.
- **iOS and Android:** the same grouping in the fleet list, and a board per
  workspace. No workspace management.
- **Push and the Live Activity** are unchanged: none of their payloads names a
  repository, and the worktree name they carry is still a worktree name.
- The phones group by runner today, and Android's fleet is a flat list, so
  grouping by workspace there is new layout, not a regrouping.

## Sequencing

1. **Spike** (throwaway, on a scratch runner, scratch daemons killed by PID).
   For Claude Code, Codex, and Cursor:
   - which claiming signal sees an agent enter a worktree (Claude Code's
     `EnterWorktree`, a `cd` in Codex and in Cursor), and how quickly;
   - whether Cursor's `--workspace` loads `AGENTS.md` and `CLAUDE.md`;
   - the directory name Claude Code derives for a repository's auto-memory, so
     Far Cooler points at it instead of guessing;
   - whether Claude Code honors `.claude/settings.json` from an added directory.

   Findings are written into this spec before step 3 is planned.
2. **Rename**, behavior-free.
3. **Workspace model**: migration, `workspace` and `task move` CLI, home and
   charter, launch recipes, role, wake routing.
4. **Claiming**: explicit, hooks, process tree, and the cross-workspace
   warning.
5. **Apps**: Mac sidebar, then iOS and Android.
6. **Skill**: charter from the environment, and the splitting section.

## Testing

- **Migration**: one fixture store with two repositories, worktrees, terminals,
  tasks with notes, and a `manager.md`. Everything lands in its Main, keys and
  notes unchanged, and the charter is copied.
- **Invariants**: prefix uniqueness; a key never issued twice across prefix
  renames; delete refused while a workspace holds tasks or worktrees; Main
  undeletable; one live orchestrator per workspace; orchestrators never claim;
  claims are sticky.
- **Wire**: field numbers of renamed messages are unchanged (a test over the
  descriptor, broken once on purpose to watch it go red).
- **Claiming**: hook payload fixtures for all three harnesses, and a
  process-table fixture in `foreground.rs`'s style.
- **Event routing**: a task change carries its workspace, and a board view for
  Billing re-reads on Billing's changes and not on Main's.
- **Apps**: AgentKit model tests for the grouping; Kotlin equivalents.

## Risks

- **The rename is large.** It is mechanical but touches every platform. Landing
  it alone, with no behavior change, keeps a missed reference a build failure
  rather than a behavior bug.
- **Claiming can be wrong.** An agent that passes through someone else's
  unclaimed worktree claims it. Claims stick, so recovery is one
  `worktree assign`, and `worktree list` shows which signal made the claim.
- **Claude Code settings may not reach an orchestrator.** If added directories
  do not contribute `.claude/settings.json`, a Claude Code orchestrator runs
  without the repository's hooks and permission allowlist. Tolerable for an
  agent that does not write code; the spike settles it.
- **A harness may trigger neither claiming signal** — for example one whose hooks omit `cwd`. Its agents'
  worktrees then stay unclaimed until assigned: degraded, but honest.
- **Splitting is only as good as the handoff.** The skill makes the handoff
  note a required step.

## Later: across repositories

Not built now, and not blocked. Ownership sits on the worktree, and the
workspace's `repository_id` is the one constraint to lift. What it would take:

- **The orchestrator's home** — already outside every repository.
- **Task prefixes** — already per workspace.
- **Launch recipes** with several repositories: several `--add-dir`s for Claude
  Code, whose root `CLAUDE.md` files would then all load at once and apply
  everywhere, with no scoping to their own repository; Codex, which anchors to
  one working directory, would need another answer.
- **Memory**: `autoMemoryDirectory` names one directory, so only one
  repository's memory loads natively.

Diffs are not a cost: a diff is per worktree, and every worktree is in exactly
one repository.

## Spike findings (2026-09-27)

Measured on this runner with Claude Code 2.1.283, codex-cli 0.153.4 and
cursor-agent 2026.09.23 (which updated itself to 2026.09.26 mid-spike), through
a scratch daemon (Canary CLI `0.1.0+f805fb8d`, `FARCOOLER_HOME=/tmp/fc-ws/home`)
over a scratch repository at `/tmp/fc-ws/spike/repo` with a nested worktree
(`.worktrees/nested`) and a sibling one (`../sibling`). Every agent was the
real one.

**How hooks were read.** The daemon does not log a hook's `cwd` at any level,
and `daemon ensure` sends its stderr to `/dev/null`. So a recording hook was
added next to Far Cooler's in each harness's project hook file
(`.claude/settings.local.json`, `.codex/hooks.json`, `.cursor/hooks.json`). It
got the same stdin as `farcooler hook`, for Far Cooler's events and for the
tool events Far Cooler does not register (`PreToolUse`/`PostToolUse`, and
Cursor's `preToolUse`/`beforeShellExecution`). Raw payloads, redacted, are in
[`2026-09-27-workspaces-spike-hooks/`](2026-09-27-workspaces-spike-hooks/):
`<harness>-before-move.json` and `<harness>-after-move.json` are events Far
Cooler registers today, and `<harness>-after-move-tool.json` is the tool event
that carries the move, where there is one. Redacted: the home directory became
`/tmp/fc-ws/user`, Claude's `scratchpad_dir` became `/tmp/fc-ws/scratchpad`,
Cursor's `user_email` became `user@example.com`, and every session,
conversation, turn, prompt and tool-use UUID was replaced by a placeholder
(`00000000-0000-4000-8000-…`), consistently across files. No tokens or
credentials were in any payload.

### Hook `cwd` and process `cwd` after a move

| Harness | Move | Hook `cwd` follows? | Latency | Process cwd (`lsof -a -d cwd`) follows? |
|---|---|---|---|---|
| Claude Code | Bash `cd .worktrees/nested && ls` | **Yes.** `PostToolUse` of that same call reports the nested path; every later hook too, including `Stop` | 0 s to `PostToolUse` (not registered today); ~5 s to `Stop` in a one-line turn, so end of turn in general | No. `claude` stays in the launch directory |
| Claude Code | `EnterWorktree` with `path` = the sibling | **Yes.** `PostToolUse` of `EnterWorktree` reports the sibling | Same as above | **Yes**, at once: `claude` itself moves |
| Codex | `cd .worktrees/nested && ls`, and `workdir` = the sibling | **No.** `cwd` is always the launch directory. `tool_input` carries only `command`, never the `workdir` | Never | No. Each command is a short-lived child of `codex` with `cwd` = its `workdir`, and **no controlling tty** (`??`) |
| Cursor | `cd .worktrees/nested && ls` in its shell | **No.** Tool events send `"cwd": ""` although the shell's `cd` persists into later calls; `workspace_roots` stays the launch directory | Never | No. Each command is a new `zsh -c` child (restoring a saved shell state) with the persisted `cwd`, **no controlling tty**, alive only while it runs |
| Cursor | a shell call with its `cwd` parameter = the sibling | **Only on tool events.** `preToolUse` and `beforeShellExecution` carry that `cwd`, spelled as given (`/tmp/…`, not `/private/tmp/…`) | 0 s, before the command runs (not registered today) | As above |

Consequences for "Claiming worktrees":

- **Far Cooler's registered events do not carry a move for Codex or Cursor at
  all.** Codex's `cwd` never changes. Cursor's registered events
  (`sessionStart`, `beforeSubmitPrompt`, `stop`) carry no `cwd`, only
  `workspace_roots`, which does not change. "Every hook payload carries the
  `cwd`" is false for Cursor.
- **Signal 2 works for Claude Code only**, and with today's registrations it
  arrives at the end of the turn (`Stop`), not per tool call. Per-tool-call
  claiming needs `PostToolUse` registered for Claude (priced in
  `hook_install.rs` at ~15-18 ms per call). For Cursor, registering
  `preToolUse` would catch only calls that pass an explicit `cwd`; a `cd` in its
  shell is invisible to hooks. For Codex, no hook sees a move.
- **Signal 3 as written would miss Codex and Cursor.** Their commands run
  without a controlling tty, so a per-tty scan like `foreground.rs` never sees
  them. The walk has to follow descendants of the pane's agent process by parent
  PID. Even then it sees a worktree only while a command is running there. For
  Codex this is the only signal; for Cursor it is the only one for a `cd`.
- `cwd` spelling differs: Claude and Codex report the resolved path
  (`/private/tmp/…`), Cursor reports what the model typed. Matching must
  canonicalize first, as `hook_ingress.rs` already does for Codex.
- A Claude Code pane stopped while inside an `EnterWorktree` leaves
  `activeWorktreeSession` in `~/.claude.json` for the launch directory.
- Codex runs project hooks only after an interactive "Hooks need review" trust.
  That trust is recorded in `~/.codex/config.toml` per hooks file (canonical
  path) and hash, so each new worktree's `.codex/hooks.json` asks again. Launched
  as `codex exec --cd /tmp/…` (not the resolved path), the project hooks were
  skipped with no prompt.
- On this runner the Homebrew `codex` could not run shell commands at all
  ("timed out negotiating with the code-mode host"; `codex-code-mode-host` is
  not on its path), and in a Far Cooler pane it reached for Computer Use
  instead. The Codex rows were measured with the `codex` binary bundled in
  ChatGPT.app, the same version, started from a shell pane.

### Orchestrator context from a home outside the repository

| Harness | Command, run from `/tmp/fc-ws/spike/home` | Answer |
|---|---|---|
| Claude Code | `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1 claude --add-dir <repo> -p …` | PINEAPPLE, told not to read files |
| Claude Code (control) | the same without the variable | Did not know; no `CLAUDE.md` loaded |
| Cursor | `cursor-agent --workspace /tmp/fc-ws/spike/repo -p …` | **Refused**: "Workspace Trust Required", although the pane had trusted the same directory |
| Cursor | `cursor-agent --workspace /private/tmp/fc-ws/spike/repo -p …` | PINEAPPLE, with no tool call |
| Codex | `codex exec --cd <repo> …` | PINEAPPLE |

Cursor keeps trust as `~/.cursor/projects/<slug of the path as given>/.workspace-trusted`.
So trust follows the literal spelling of the path. Claude Code also discovers
skills from the added directory: a `.claude/skills/spike-mango` in the
repository appeared in a home-launched session's skills list.

### Claude Code's memory directory and added-directory settings

| Launched in | Transcripts | Auto-memory directory |
|---|---|---|
| `repo/` | `-private-tmp-fc-ws-spike-repo` | `-private-tmp-fc-ws-spike-repo/memory` |
| `repo/.worktrees/nested` | `-private-tmp-fc-ws-spike-repo--worktrees-nested` | `-private-tmp-fc-ws-spike-repo/memory` |
| `../sibling` | `-private-tmp-fc-ws-spike-sibling` | `-private-tmp-fc-ws-spike-repo/memory` |
| the home (not a repository) | `-private-tmp-fc-ws-spike-home` | `-private-tmp-fc-ws-spike-home/memory` |
| the home, `--settings` with `autoMemoryDirectory` = the repo's | — | `-private-tmp-fc-ws-spike-repo/memory` |

All under `~/.claude/projects/`. The rule observed:
`slug(p)` = `realpath(p)` with every character outside `[A-Za-z0-9]` replaced
by `-` (so `/.worktrees` becomes `--worktrees`). Transcripts go to
`slug(cwd)`, and move with the session: after `EnterWorktree`, the running
session's transcript was under `slug(sibling)`. Memory goes to
`slug(main checkout)` for every worktree of a repository (the git common
directory's parent), and to `slug(cwd)` outside a repository. So the
`autoMemoryDirectory` override in the recipe is needed, and it works through
`--settings`.

**Added-directory settings do not reach a Claude Code orchestrator.** With
`repo/.claude/settings.json` holding a `SessionStart` hook that touches
`SETTINGS_HONORED`, the home-launched run did not create the file; the same
`claude -p` run inside `repo/` did. A hook in `repo/.claude/settings.local.json`
did not fire from the home either. This settles the risk under "Risks": a Claude
Code orchestrator runs without the repository's hooks and permission
allowlist. Far Cooler's own hooks still reach it, since they come through
`--settings`.

### Against the recipe table in "Launch recipes"

No row is contradicted. Each row's context mechanism worked as written. Two
rows need an addition:

- **Cursor**: `--workspace` must get the resolved path of the main checkout,
  or the launch must pass `--trust`. A path spelled differently from the one
  trusted is refused (evidence: the two Cursor rows above).
- **Codex**: nothing to change for context. `--cd` should get the resolved path
  anyway, because Codex keys hook trust by resolved path (see above).

The explanation under the table, "the git root of the working directory", is
more precisely the main checkout: every worktree shares the main checkout's
memory directory.
