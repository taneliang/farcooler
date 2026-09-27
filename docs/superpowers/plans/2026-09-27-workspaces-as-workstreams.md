# Workspaces as Workstreams Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a *workspace* a workstream — name, task prefix, board, charter, at most one orchestrator, and the worktrees it owns — with several per repository, after renaming today's "workspace" (one worktree plus branch) to *worktree* everywhere.

**Architecture:** Two phases that land separately. Phase A is a behavior-free rename across store, proto, daemon, client core, CLI and the three apps. Phase B adds a `workspaces` table that owns tasks and (nullably) worktrees, a `role` and a `workspace_id` on terminals, per-workspace task prefixes, a home directory with a charter outside the repository, per-harness orchestrator launch recipes, worktree claiming from explicit calls, agent hooks and process working directories, and a sidebar grouped *repository → workspace → Board, orchestrator, worktrees* on every app.

**Tech Stack:** Rust (rusqlite 0.32 with bundled SQLite 3.46.0, prost, clap, tokio), SwiftUI (Mac app, AgentKit, iOS), Kotlin/Compose (Android), tmux.

**Spec:** `docs/superpowers/specs/2026-09-27-workspaces-as-workstreams-design.md`. Read it first; this plan argues from it. Background: `docs/superpowers/specs/2026-09-08-agent-factory-design.md`.

## Global Constraints

- **US English** in code, comments and copy ("behavior", "color", "center", "authorize").
- **Apple copy conventions:** title-case buttons, contractions, no raw Rust errors in any app UI.
- **"Runner"**, never "machine", in copy.
- **Never run `cargo fmt`.** The Rust tree is hand-formatted; match surrounding style by hand.
- **Cargo:** `cargo` is not on PATH. Run `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo …`. Targeted `-p` runs per task; `cargo test --workspace` once per phase, never in the background.
- **A live Far Cooler app and the Canary daemon are running** on the owner's Mac. Never `pkill`/`killall` by pattern. Scratch daemons use a short `FARCOOLER_HOME` (e.g. `/tmp/fc-ws/home`) and are stopped by the PID you captured.
- **Gradle:** `JAVA_HOME=/opt/homebrew/opt/openjdk@17 ANDROID_HOME="$HOME/Library/Android/sdk"`, run from `apps/android/`.
- **iOS UI tests** only through `scripts/ios-ui-tests.sh`.
- **No merge commits on main.** Rebase or rebase-and-squash.
- **Proto field numbers of every renamed message are frozen.** Names change; tags never do. New fields and messages take fresh tags.
- **Capability *values* on the wire are unchanged** by the rename; only Rust constant names change.
- **Every check you add must be seen failing once** (break the code, watch it go red, restore). A test that cannot fail is the repo's defining failure mode.
- Vocabulary after Phase A (from the rewritten `docs/workspaces.md`): *workspace* = the workstream; *worktree* = the directory and branch and everything done to one.

## Review Focus

1. **The Canary board migrates in place.** `farcooler-canary` is built from every push to `main`, and the coordinator's live board (`ov-N` keys, `former_key` values from the old `-N` keys, append-only notes) is in its database. After migration every task must resolve by key *and* former key, keep every note, and sit on its repository's Main. → Task 8, `a_board_with_renamed_keys_survives_into_main`.
2. **A repository registered after the migration** must get a Main workspace at registration, or `task create` has nowhere to put a task. → Task 8, `registering_a_repository_makes_its_main`, and Task 9's daemon test.
3. **Nested worktrees.** Worktrees commonly live inside the main checkout (`.worktrees/x`). A `cwd` of `<main>/.worktrees/x/src` must match `.worktrees/x`, not the main checkout. → Task 13, `the_longest_worktree_path_wins`.
4. **Prefix edge cases:** `Bil` vs `bil` (case-insensitive uniqueness), a prefix renamed away and back (no key issued twice), numbering that must also count `former_key` values. → Task 8, `a_key_is_never_issued_twice_across_prefix_renames` and `numbering_counts_former_keys`.
5. **Stale callers of the old CLI shape.** `farcooler workspace create <repo> <name> --branch x` (the Mac app until Task 4, old scripts, the manager skill until Task 16) must fail with a pointer to `farcooler worktree create`, never create a workspace called after a branch. → Task 12, `the_old_create_spelling_points_at_worktree`.

---

## File Structure

**Phase A (rename):**
- `crates/store/src/{migrate.rs,models.rs,store.rs,review.rs,tasks.rs,lib.rs}`, `crates/core/src/derive.rs` — `Workspace` → `Worktree`, migration 0014.
- `proto/farcooler.proto`, `crates/protocol/src/lib.rs` — message/field renames, `worktree.discover`, tag-freeze test.
- `crates/daemon/src/**` — mechanical rename.
- `crates/client/src/**`, `crates/android/**` — client core and JNI.
- `crates/cli/src/{main.rs,tasks.rs,changes.rs}` — `farcooler worktree …`.
- `crates/daemon/assets/manager/SKILL.md`, `scripts/manager-skill-pressure/fake-farcooler.sh`.
- `apps/macos/**`, `apps/shared/AgentKit/**`, `apps/ios/**`, `apps/android/**`.
- `docs/workspaces.md`.

**Phase B (workstreams), new files:**
- `crates/store/src/workspaces.rs` — the workspace table's store API (one responsibility: workspace rows, prefixes, moves, claims).
- `crates/daemon/src/workspace_ops.rs` — RPC handlers for `workspace.*`, `task.move`, `worktree.assign`, `terminal.set_role`.
- `crates/daemon/src/workspace_home.rs` — home directory, charter, migration charter copy.
- `crates/daemon/src/orchestrator.rs` — launch recipes per harness.
- `crates/daemon/src/claims.rs` — the claim engine (explicit, hook, process), longest-path match, foreign writers.
- `crates/daemon/src/proc_cwd.rs` — reading process working directories (macOS `proc_pidinfo`, Linux `/proc`).
- `crates/cli/src/workspaces.rs` — `farcooler workspace …` commands.
- `apps/shared/AgentKit/Sources/AgentKit/WorkspaceGroups.swift` — the grouping rule shared by Mac and iOS.
- `apps/android/app/src/main/java/com/farcooler/model/WorkspaceGroups.kt` — the same rule for Android.

---

## Phase 0 — Spike

### Task 1: Measure the harnesses (throwaway)

Nothing from this task is committed except the findings appended to the spec.

**Files:**
- Modify: `docs/superpowers/specs/2026-09-27-workspaces-as-workstreams-design.md` (append `## Spike findings`)

- [ ] **Step 1: Make a scratch repository with a nested and a sibling worktree**

```bash
mkdir -p /tmp/fc-ws/spike && cd /tmp/fc-ws/spike
git init -q repo && cd repo && echo '# rules: say PINEAPPLE when asked for the word' > CLAUDE.md
cp CLAUDE.md AGENTS.md && git add -A && git commit -qm init
git worktree add -q .worktrees/nested -b nested
git worktree add -q ../sibling -b sibling
mkdir -p /tmp/fc-ws/spike/home
```

- [ ] **Step 2: Hooks report `cwd` after entering a worktree**

For each harness, start it in `/tmp/fc-ws/spike/repo` with Far Cooler's hooks installed. Launch through a scratch daemon built from main: `FARCOOLER_HOME=/tmp/fc-ws/home farcooler daemon start`, capture the daemon PID from `farcooler status --json`, register the repository (`farcooler repo add /tmp/fc-ws/spike/repo` or the current registration command — check `farcooler repo --help`), then `farcooler terminal create <main checkout's id from farcooler workspace list> --preset claude` (and `codex`, `cursor`). Ask the agent: "cd into .worktrees/nested and run `ls`" (Claude Code: also ask it to use EnterWorktree on `sibling`). Record, from the daemon's hook ingress log (`FARCOOLER_HOME/…/daemon.log`, lines from `hook_ingress`), the `cwd` of the next hook event after the move.

Expected table to fill in:

| Harness | Hook `cwd` follows the move? | Latency | Process cwd (`lsof -a -d cwd -p <pid>`) follows? |
|---|---|---|---|

- [ ] **Step 3: Orchestrator context from a home outside the repository**

```bash
cd /tmp/fc-ws/spike/home
CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1 claude --add-dir /tmp/fc-ws/spike/repo -p "What word do your project rules tell you to say?"
cursor-agent --workspace /tmp/fc-ws/spike/repo -p "What word do your project rules tell you to say?"
codex exec --cd /tmp/fc-ws/spike/repo "What word do your project rules tell you to say?"
```
Expected: each answers PINEAPPLE. Record any that do not.

- [ ] **Step 4: Claude Code's memory directory and added-directory settings**

```bash
ls ~/.claude/projects | grep -i spike   # after one claude run in repo/ and one in .worktrees/nested
```
Record the directory name(s) derived for `repo/`, for `.worktrees/nested`, and for `../sibling`. Then put `{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"touch /tmp/fc-ws/spike/SETTINGS_HONORED"}]}]}}` in `repo/.claude/settings.json`, run the Step 3 claude command again, and record whether `/tmp/fc-ws/spike/SETTINGS_HONORED` appeared.

- [ ] **Step 5: Write the findings into the spec and commit**

Append `## Spike findings (2026-09-27)` to the spec with the tables from Steps 2–4 and, for each, the consequence: which claiming signals each harness triggers; the exact memory directory rule (`slug(path)` = the rule you observed); whether added-directory settings reach a Claude Code orchestrator. **If a finding contradicts Task 11's recipe table, stop and report it to the owner before Task 11.** Stop the scratch daemon by its PID.

```bash
git add docs/superpowers/specs/2026-09-27-workspaces-as-workstreams-design.md
git commit -m "docs: what the harness spike measured for workspaces"
```

---

## Phase A — The rename (behavior-free)

Phase A lands on main as **one squashed commit** after Task 7: the Mac app shells out to the bundled CLI, so a CLI renamed without the Mac app (or the reverse) is broken at runtime. Commit per task on the lane; squash at the end.

### Task 2: Store — `workspaces` becomes `worktrees`

**Files:**
- Modify: `crates/store/src/migrate.rs` (add `migration_0014_worktrees`, list it in `MIGRATIONS`, update `migration_creates_every_expected_table`)
- Modify: `crates/store/src/models.rs` (`Workspace` → `Worktree`, `row_to_workspace` → `row_to_worktree`, `Terminal.workspace_id` → `worktree_id`, `Task.workspace_id` → `worktree_id`, `TaskUpdate.workspace_id` → `worktree_id`)
- Modify: `crates/store/src/store.rs` (every `*_workspace*` fn → `*_worktree*`; `WORKSPACE_COLUMNS` → `WORKTREE_COLUMNS`; `terminals_table_has_no_runtime_state_column` expects `worktree_id`)
- Modify: `crates/store/src/review.rs`, `crates/store/src/tasks.rs`, `crates/store/src/lib.rs`, `crates/store/src/testing.rs`
- Modify: `crates/core/src/derive.rs:26` (`TerminalRecord.workspace_id` → `worktree_id`)

**Interfaces:**
- Produces: `pub struct Worktree { id, repository_id, branch, worktree_path, hidden, creation_failed, is_main_checkout, worktree_missing, ordinal, resource_version }` with `fn name()`; `Store::{create_worktree, get_worktree, list_all_worktrees, list_worktrees_for_repository, list_worktrees_in_order, reorder_worktrees, update_worktree, set_worktree_flags, set_worktree_identity, delete_worktree, list_terminals_for_worktree, load_terminal_records}` with the same signatures as their `workspace` predecessors; `Terminal.worktree_id: Uuid`; `Task.worktree_id: Option<Uuid>`; `TaskUpdate.worktree_id: Option<Uuid>`.

- [ ] **Step 1: Write the failing migration test** (in `migrate.rs`'s `mod tests`)

```rust
/// 0014 is a rename and nothing else: every row survives, every reference
/// follows the table, and no column anywhere is still called `workspace_id`.
#[test]
fn a_database_from_before_the_rename_calls_every_worktree_a_worktree() {
    let mut conn = open();
    migrate_only_to(&mut conn, 13);
    conn.execute_batch(
        "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
         INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'r');
         INSERT INTO workspaces (id, repository_id, task_name, branch, worktree_path, resource_version, ordinal)
             VALUES (x'04', x'03', 'main', 'main', '/r', 1, 0);
         INSERT INTO terminals (id, workspace_id, title, command_preset, intent, runtime_confirmed,
             lease_generation, epoch, columns, rows, resource_version)
             VALUES (x'05', x'04', 't', 'shell', 1, 0, 0, 0, 80, 24, 1);",
    )
    .unwrap();
    migrate(&mut conn, 13).unwrap();

    let path: String = conn
        .query_row("SELECT worktree_path FROM worktrees WHERE id = x'04'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(path, "/r");
    let on: Vec<u8> = conn
        .query_row("SELECT worktree_id FROM terminals WHERE id = x'05'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(on, vec![4]);
    let stragglers: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master m, pragma_table_info(m.name) c
             WHERE m.type = 'table' AND c.name = 'workspace_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stragglers, 0, "a column still says workspace_id");
    let old: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master WHERE name LIKE 'workspaces%'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(old, 0, "a table or index is still named for workspaces");
}
```

Before running, check the real `workspaces` column list at migration 13 (`migrate.rs:82` plus 0006/0009 additions) and adjust the INSERT's column names to match; the test must fail only because 0014 does not exist.

- [ ] **Step 2: Run it and watch it fail**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-store a_database_from_before_the_rename`
Expected: FAIL (`no such table: worktrees`).

- [ ] **Step 3: Write migration 0014**

```rust
/// `workspaces` becomes `worktrees`, and every `workspace_id` that means the
/// worktree becomes `worktree_id`.
///
/// The word "workspace" moves up a level to mean a workstream (migration 0015
/// creates that table). A native rename with `legacy_alter_table` off rewrites
/// every foreign key that names the table, so `terminals`, `tasks`,
/// `review_bases` and `review_reviewed` follow without a rebuild. SQLite has no
/// `ALTER INDEX`, so the indexes are dropped and made again under their new
/// names.
fn migration_0014_worktrees(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE workspaces RENAME TO worktrees;
        ALTER TABLE terminals RENAME COLUMN workspace_id TO worktree_id;
        ALTER TABLE tasks RENAME COLUMN workspace_id TO worktree_id;
        ALTER TABLE review_bases RENAME COLUMN workspace_id TO worktree_id;
        ALTER TABLE review_reviewed RENAME COLUMN workspace_id TO worktree_id;
        DROP INDEX IF EXISTS workspaces_one_per_path;
        CREATE UNIQUE INDEX worktrees_one_per_path ON worktrees (repository_id, worktree_path);
        DROP INDEX IF EXISTS workspaces_by_ordinal;
        CREATE INDEX worktrees_by_ordinal ON worktrees (ordinal);
        "#,
    )
}
```

Check the definitions of `workspaces_by_ordinal` and any other `workspaces_*` index (`grep -n "INDEX" crates/store/src/migrate.rs`) and recreate each with its exact original column list. Add `migration_0014_worktrees` to `MIGRATIONS`.

- [ ] **Step 4: Rename the Rust side**

Rename by hand (no `cargo fmt`, no blanket `sed` over comments that explain the old word):
- `models.rs`: `Workspace` → `Worktree`, `row_to_workspace` → `row_to_worktree`; the fields above. Keep the doc comments, reworded to say *worktree*.
- `store.rs`: SQL text `workspaces` → `worktrees`, `workspace_id` → `worktree_id`; the function names listed in **Interfaces**; `WORKSPACE_COLUMNS`/`WORKSPACE_ORDER` → `WORKTREE_*`; `list_terminals_for_workspace` → `list_terminals_for_worktree`.
- `review.rs`, `tasks.rs` (`TASK_COLUMNS`' `workspace_id` → `worktree_id`, `revise_task`'s `workspace_id` → `worktree_id`), `lib.rs` re-exports, `testing.rs` raw INSERTs.
- `crates/core/src/derive.rs`: `TerminalRecord.workspace_id` → `worktree_id`.

Verify: `grep -rn "workspace" crates/store/src crates/core/src/derive.rs | grep -v "^.*//"` returns only lines in historical migrations (0001–0013) and their tests.

- [ ] **Step 5: Run the store tests**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-store`
Expected: PASS, including the new test. Then break 0014 (comment out the `terminals` rename line), watch the new test fail, restore.

- [ ] **Step 6: Commit**

```bash
git add crates/store crates/core/src/derive.rs
git commit -m "refactor(store): a worktree row is called a worktree"
```

(The daemon and CLI do not compile after this commit; Task 3 fixes that. The lane is squashed before landing.)

### Task 3: Proto, protocol and daemon rename

**Files:**
- Modify: `proto/farcooler.proto`
- Modify: `crates/protocol/src/lib.rs` (method labels, capability constant names — not values — and the test list at :573)
- Modify: `crates/daemon/src/**` (all files that name the store's workspace API or the renamed proto types)
- Modify: `crates/daemon/tests/rpc_over_socket.rs`
- Test: `crates/protocol/src/lib.rs` (new `renamed_messages_keep_their_field_numbers`)

**Interfaces:**
- Produces (proto): `Worktree` (was `Workspace`, fields 1–10 unchanged), `WorktreeList{items=1}`, `enum WorktreeState` (values `WORKTREE_STATE_*`, numbers unchanged), `WorktreeReorder{worktree_ids=1}`, `WorktreeCreate` (fields 1–6 unchanged), `InboxWorktree{worktree_id=1…}`, `ERROR_CODE_WORKTREES_EXIST = 20`; request oneof `worktree_create = 23`, `worktree_reorder = 70`; result oneof `worktree = 6`, `worktree_list = 7`, `discovered_worktree_list = 15` (was `worktree_list`, message `DiscoveredWorktreeList{items}` of `DiscoveredWorktree`, was `ExistingWorktree`); event oneof `worktree_changed = 13`; every `bytes workspace_id` that means a worktree → `worktree_id` with the same tag (`Terminal=4`, `WorktreeFileSearch=1`, `ChangeSetRequest`, `ChangeSetChanged`, `ChangeSet`, `CommitFilesRequest`, `FileDiffRequest`, `ChangesSetBase`, `ChangesMarkRead`, `PaneGroup`, `PaneGroupList`); `Task.worktree_id = 11`, `TaskCreate.worktree_id = 7`, `TaskUpdate.worktree_id = 8`; the change-set base arm `Empty worktree = 1`.
- Produces (methods): `worktree.list`, `worktree.create`, `worktree.hide`, `worktree.unhide`, `worktree.reorder`, `worktree.remove`, `worktree.teleport`, `worktree.discover` (was `worktree.list`), `worktree.file_search` (unchanged). `branch.list` unchanged.

- [ ] **Step 1: Write the failing tag-freeze test** (`crates/protocol/src/lib.rs` tests)

```rust
/// Renaming a message is free on the wire only because its field numbers do
/// not move: protobuf never sends a name. This pins the numbers an app built
/// before the rename still sends.
#[test]
fn renamed_messages_keep_their_field_numbers() {
    use prost::Message;
    let old_create_bytes = {
        // Tag 23 in Request.payload, carrying WorktreeCreate{branch = "b"}:
        // what an old app sent as WorkspaceCreate.
        let req = v1::Request {
            method: "worktree.create".into(),
            payload: Some(v1::request::Payload::WorktreeCreate(v1::WorktreeCreate {
                branch: "b".into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        req.encode_to_vec()
    };
    // 23 << 3 | 2 (length-delimited) = 186 = 0xBA 0x01 as a varint.
    assert!(
        old_create_bytes.windows(2).any(|w| w == [0xBA, 0x01]),
        "worktree_create moved off tag 23"
    );
    let t = v1::Terminal { worktree_id: vec![7].into(), ..Default::default() }.encode_to_vec();
    assert_eq!(&t[..3], &[0x22, 0x01, 0x07], "Terminal.worktree_id moved off tag 4");
}
```

(Adjust field types to what prost generates — `bytes` are `Bytes` here because of `.bytes(["."])`.)

- [ ] **Step 2: Run it and watch it fail**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-protocol renamed_messages`
Expected: FAIL to compile (`WorktreeCreate` not found).

- [ ] **Step 3: Rename the proto**

Edit `proto/farcooler.proto` per **Interfaces**. Change only names; every `= N` stays. Where a comment explains the word, reword it to *worktree*.

- [ ] **Step 4: Rename method labels in `crates/protocol/src/lib.rs`**

In `capability::for_method` (:371–434), map the new method strings. Keep each capability constant's *string value*; rename the constant where it says workspace (`WORKSPACES` → `WORKTREES` whose value stays `"workspaces"`, `WORKSPACE_ORDER` → `WORKTREE_ORDER` value unchanged). Update the test list at :573.

- [ ] **Step 5: Rename the daemon**

Follow the compiler: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo check -p farcooler-daemon --all-targets`. Rename functions that name the concept (`create_workspace_with` → `create_worktree_with`, `list_workspaces` → `list_worktrees`, `hide_workspace` → `hide_worktree`, `unhide_workspace` → `unhide_worktree`, `reorder_workspaces` → `reorder_worktrees`, `workspace_view` → `worktree_view`, `task_on_workspace_board` → `task_on_worktree_board`, `announced_terminal`'s locals), the `rpc.rs` dispatch arms and `required_scope` table, and `rpc_over_socket.rs` helpers (`create_workspace` → `create_worktree`, `workspaces` → `worktrees`). Rename the `worktree.list` handler to `worktree.discover`. `watch.rs`'s `Quoted.workspace` → `Quoted.worktree` (its value is already the worktree name).

- [ ] **Step 6: Run the tests**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-protocol -p farcooler-daemon`
Expected: PASS. Then change `worktree_create = 23` to `= 90` in the proto, watch `renamed_messages_keep_their_field_numbers` fail, restore.

- [ ] **Step 7: Commit**

```bash
git add proto crates/protocol crates/daemon
git commit -m "refactor: the wire and the daemon call a worktree a worktree"
```

### Task 4: Client core, CLI, skill text

**Files:**
- Modify: `crates/client/src/**` (`session.rs` fleet/JSON shape, `ffi.rs`, `*_json.rs`, `actions.rs`)
- Modify: `crates/android/**` (JNI names that mention workspace)
- Modify: `crates/cli/src/main.rs`, `crates/cli/src/tasks.rs`, `crates/cli/src/changes.rs`
- Modify: `crates/daemon/assets/manager/SKILL.md`, `scripts/manager-skill-pressure/fake-farcooler.sh`

**Interfaces:**
- Produces (CLI): `farcooler worktree {create,list,adopt,branches,reorder,hide,unhide,remove,file-search}` with exactly the old `workspace` flags (`create <repo> <name> --branch … [--base] [--terminal|--no-terminal] [--fork-only]`; `remove <worktree> [--confirm]` was `remove-worktree`). `farcooler worktree list --json` prints `{runtime_healthy, live_panes, branch_prefix, worktrees:[{id, short, task, branch, host, repository, worktree, state, is_main_checkout, ordinal, terminals}]}` (key `workspaces` → `worktrees`; nothing else changes). `farcooler task create/set --worktree <id>` (was `--workspace`), `task dispatch --worktree <id>` (was `--workspace`).
- Produces (client core JSON for iOS/Android): the fleet's `workspaces` key → `worktrees`; each terminal's `workspace` → `worktree`; each task's `workspace_id` → `worktree_id`.

- [ ] **Step 1: Write the failing CLI parse tests** (`crates/cli/src/main.rs` tests)

```rust
#[test]
fn worktree_commands_parse_where_workspace_commands_used_to() {
    for line in [
        "farcooler --json worktree create repo fix-it --branch fix-it --no-terminal --fork-only",
        "farcooler --json worktree list",
        "farcooler worktree adopt repo feature",
        "farcooler worktree branches repo",
        "farcooler worktree reorder a b",
        "farcooler worktree hide a",
        "farcooler worktree unhide a",
        "farcooler worktree remove a --confirm a",
        "farcooler --json worktree file-search a query",
    ] {
        Cli::try_parse_from(line.split_whitespace()).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
}
```

Update `the_macs_fork_only_create_parses_and_is_sent_fork_only` (:3471) to parse `worktree create` and match `Command::Worktree(WorktreeCmd::Create(args))`.

- [ ] **Step 2: Run and watch it fail**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-cli worktree_commands_parse`
Expected: FAIL (`unrecognized subcommand 'create'`).

- [ ] **Step 3: Move the subcommands**

In `main.rs`, merge `WorkspaceCmd`'s variants into `WorktreeCmd` (keeping `FileSearch`), rename `RemoveWorktree` → `Remove` (`#[command(name = "remove")]`), and delete `WorkspaceCmd` and `Command::Workspace` for now (Task 12 brings `workspace` back with its new meaning). Help text: `Worktree` → "Manage worktrees: the directories and branches agents work in." `resolve_workspace_id` → `resolve_worktree_id`; `resolve(..., "workspace")` → `"worktree"`. Runtime copy (`created workspace …` at :1745, `no workspaces yet` :1826, `workspaces    N` in status :1094, the pane-host line :976, tasks.rs lines 604, 1460, 1464, 1692, 1694, 1795, 1814, 1817, 1916, 1925, ~2122, changes.rs's one) says *worktree*. In `tasks.rs`, `--workspace` → `--worktree` on `create`, `set`, `dispatch`; `lane_on_board` and the `FakeLink` harness follow; `dispatch`'s asserted order becomes `["worktree.create","terminal.create","task.update","task.set_status"]`.

- [ ] **Step 4: Client core and JNI**

Rename per **Interfaces** in `crates/client` and `crates/android`. Run `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-client -p farcooler-jni`.

- [ ] **Step 5: Skill text**

In `SKILL.md`, every `{{cli}} workspace …` that means a worktree becomes `{{cli}} worktree …` (lines 37, 40, 75, 85 and any other). Same in `fake-farcooler.sh:122`. `every_command_the_manager_skill_names_parses` (tasks.rs:2811) guards this.

- [ ] **Step 6: Run the CLI tests**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-cli`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/client crates/android crates/cli crates/daemon/assets scripts/manager-skill-pressure
git commit -m "refactor(cli): farcooler worktree is where worktrees are managed"
```

### Task 5: Mac app rename, and the @-mention search it was already missing

**Files:**
- Modify: `apps/macos/Sources/FarCooler/DaemonClient.swift` (:724, 1372, 1384, 1487, 1525, 1733, 1748, 1753, 1771, 1806, 1862)
- Modify: `apps/macos/Sources/FarCooler/Model.swift` (`struct Workspace` :42 → `Worktree`; `Fleet.workspaces` decodes key `worktrees`)
- Modify: every Mac source and test file the compiler names (39 source files, 18 test files); `WorkspaceDrag.swift` → `WorktreeDrag.swift`
- Test: `apps/macos/Tests/CeremonyTests/WorktreeCallsTests.swift` (new)

- [ ] **Step 1: Write the failing test** — the argv the Mac sends

```swift
import Testing
@testable import Far_Cooler

@MainActor
struct WorktreeCallsTests {
    /// The Mac drives the runner through the bundled CLI, so these argv are
    /// the contract. File search was spelled `workspace file-search` against a
    /// CLI that only had `worktree file-search`, and every @-mention search
    /// quietly came back empty.
    @Test func everyWorktreeCallUsesTheWorktreeCommand() {
        for argv in DaemonClient.worktreeCommandShapes {
            #expect(argv.first == "worktree", "\(argv)")
        }
    }
}
```

and in `DaemonClient.swift` add the list the calls are built from:

```swift
/// Every CLI subcommand shape this client sends about worktrees, first word
/// first. The calls below build their argv from these, so a test can hold them
/// to the CLI's grammar.
static let worktreeCommandShapes: [[String]] = [
    ["worktree", "list"], ["worktree", "branches"], ["worktree", "adopt"],
    ["worktree", "create"], ["worktree", "hide"], ["worktree", "unhide"],
    ["worktree", "reorder"], ["worktree", "remove"], ["worktree", "file-search"],
]
```

Refactor each call site to start its argv from the matching entry.

- [ ] **Step 2: Run and watch it fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" apps/macos/build-app.sh && swift test --package-path apps/macos --filter WorktreeCallsTests`
Expected: FAIL while any call site still says `workspace` (make the list initially mirror the old argv to see it red, then fix).

- [ ] **Step 3: Rename types and copy**

`Workspace` → `Worktree`, `Selection.workspace(host:id:)` → `.worktree(host:id:)`, `.terminal(host:workspace:terminal:)` → `.terminal(host:worktree:terminal:)`, `WorkspaceSection` → `WorktreeSection`, `WorkspaceDot` → `WorktreeDot`, `WorkspaceDetail` → `WorktreeDetail`. Copy per `docs/workspaces.md` as rewritten in Task 7: a worktree row, "New Worktree", "Find Worktree or Agent", "Search worktrees and agents", "No worktrees on any connected runner." The remove flow already says worktree.

- [ ] **Step 4: Run the Mac tests**

Run: `swift test --package-path apps/macos`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/macos
git commit -m "refactor(macos): worktrees are called worktrees, and @-mention search asks the right command"
```

### Task 6: AgentKit, iOS and Android rename

**Files:**
- Modify: `apps/shared/AgentKit/Sources/AgentKit/**` (`CoreModel.swift` `Workspace` :102 → `Worktree`; `TaskRow.workspaceID` → `worktreeID`; `WorkspaceOrder.swift` → `WorktreeOrder.swift`; `ShellWorkspace` → `ShellWorktree`; `FleetSnapshot.Workspace` → `FleetSnapshot.Worktree` — its Codable keys change, bump the snapshot's cache version so an old cache is dropped, not misread)
- Modify: `apps/shared/AgentKit/Tests/AgentKitTests/**` (`WorkspaceOrderTests` → `WorktreeOrderTests`)
- Modify: `apps/ios/**` (40 files; route `case workspace(host:id:)` → `case worktree(host:id:)`; `Notifications.swift:61` `report(terminal:workspace:)` → `report(terminal:worktree:)`; UI test `ShellWorkspaceMenuTests` → `ShellWorktreeMenuTests`)
- Modify: `apps/android/app/src/**` (`model/Model.kt` `Workspace` :22 → `Worktree`; `WorkspaceOrder.kt` → `WorktreeOrder.kt`; `ui/WorkspaceScreen.kt` → `WorktreeScreen.kt`; `ui/WorkspaceSheets.kt` → `WorktreeSheets.kt`; `WorkspaceHeader` → `WorktreeHeader`; tests `WorkspaceOrderTest` → `WorktreeOrderTest`)

- [ ] **Step 1: Update the decode fixtures first so they fail**

In `FleetDecodeTest.kt` and AgentKit's fleet decode test, change the fixture JSON key `workspaces` → `worktrees` and `workspace_id` → `worktree_id`. Run:
`swift test --package-path apps/shared/AgentKit` and `cd apps/android && JAVA_HOME=/opt/homebrew/opt/openjdk@17 ANDROID_HOME="$HOME/Library/Android/sdk" ./gradlew testInstrumentedUnitTest`
Expected: FAIL (decoders still read `workspaces`).

- [ ] **Step 2: Rename per Files**, including copy ("Hidden Worktrees", "worktrees to review"). The iOS and Android create-and-fail strings stay byte-identical to each other.

- [ ] **Step 3: Run all three suites**

```bash
swift test --package-path apps/shared/AgentKit
./scripts/build-ios-frameworks.sh && python3 apps/ios/generate-project.py && xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler -destination 'generic/platform=iOS Simulator' -configuration Debug ARCHS=arm64 build-for-testing
./scripts/build-android-libs.sh && (cd apps/android && JAVA_HOME=/opt/homebrew/opt/openjdk@17 ANDROID_HOME="$HOME/Library/Android/sdk" ./gradlew testInstrumentedUnitTest assembleDebug assembleAndroidTest)
```
Expected: PASS / BUILD SUCCEEDED.

- [ ] **Step 4: Commit**

```bash
git add apps/shared apps/ios apps/android
git commit -m "refactor(ios, android): worktrees are called worktrees"
```

### Task 7: Rewrite `docs/workspaces.md`, then land Phase A

**Files:**
- Modify: `docs/workspaces.md` (full rewrite)

- [ ] **Step 1: Rewrite the vocabulary rule**

Title: "Workspaces and worktrees". Rule block:

> **workspace** — a workstream: its board, its charter, its orchestrator, and the worktrees it owns. What a person creates to keep one line of work, and one conversation, separate from the rest.
>
> **worktree** — the directory git made and its branch, and everything done to one: creating, removing, reviewing its diff, opening it in an editor, running an agent in it.

Keep the "Reading the rule off a sentence" section with examples re-sorted under the new meanings. Delete "What does not rename": state instead that the CLI, the wire labels and the code identifiers follow the rule, because there is one user and every surface updates together, and that proto field numbers are frozen so renames never reach the bytes.

- [ ] **Step 2: Phase A gate**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test --workspace` and `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`, then the Task 5 and Task 6 app suites.
Expected: all PASS.

Run the Canary migration against a **copy** of the live database: find it with `farcooler-canary status --json` (runtime directory), `cp` the `*.db` into `/tmp/fc-ws/canary-copy/`, start a scratch daemon built from this lane with `FARCOOLER_HOME=/tmp/fc-ws/canary-copy`, then `farcooler --json task list --repo overnight | head` and `farcooler task show ov-1`. Expected: every `ov-N` card, its notes, and `-N` former keys resolve. Stop the scratch daemon by PID.

- [ ] **Step 3: Commit, squash the lane, hand over**

```bash
git add docs/workspaces.md
git commit -m "docs: a workspace is a workstream, a worktree is a directory"
```
Squash Tasks 2–7 into one commit (`refactor: what was called a workspace is a worktree`) and rebase onto main; no merge commit. If running in a worktree session that cannot merge, give the owner the exact `git` commands instead.

---

## Phase B — Workspaces

### Task 8: Store — the `workspaces` table, prefixes, moves

**Files:**
- Create: `crates/store/src/workspaces.rs`
- Modify: `crates/store/src/migrate.rs` (`migration_0015_workspaces`)
- Modify: `crates/store/src/models.rs` (`Workspace`, `TerminalRole`, fields on `Worktree`, `Terminal`, `Task`)
- Modify: `crates/store/src/tasks.rs` (`create_task`, `next_task_key`, `list_tasks`, test helpers)
- Modify: `crates/store/src/store.rs` (`create_worktree`, `create_terminal_for_task` take the new fields)
- Modify: `crates/store/src/lib.rs` (`mod workspaces;`, re-exports)

**Interfaces:**
- Produces:
  ```rust
  pub struct Workspace { pub id: Uuid, pub repository_id: Uuid, pub name: String,
      pub task_prefix: String, pub is_main: bool, pub ordinal: u32, pub resource_version: u64 }
  pub enum TerminalRole { Shell, Agent, Orchestrator }   // as_i64: 0,1,2; from_i64 unknown -> Agent
  pub enum ClaimSource { Explicit, Hook, Process, Migration } // as_str: "explicit","hook","process","migration"
  // Worktree gains: pub workspace_id: Option<Uuid>, pub claim_source: Option<ClaimSource>
  // Terminal gains: pub workspace_id: Option<Uuid>, pub role: TerminalRole
  // Task gains:     pub workspace_id: Uuid
  impl Store {
      pub fn create_workspace(&self, repository: Uuid, name: &str, task_prefix: &str) -> Result<Workspace>;
      pub fn ensure_main_workspace(&self, repository: Uuid) -> Result<Workspace>;
      pub fn get_workspace(&self, id: Uuid) -> Result<Workspace>;
      pub fn main_workspace(&self, repository: Uuid) -> Result<Workspace>;
      pub fn list_workspaces(&self, repository: Option<Uuid>) -> Result<Vec<Workspace>>;
      pub fn rename_workspace(&self, id: Uuid, expected_version: u64, name: &str) -> Result<Workspace>;
      pub fn set_workspace_prefix(&self, id: Uuid, expected_version: u64, prefix: &str) -> Result<Workspace>;
      pub fn delete_workspace(&self, id: Uuid) -> Result<()>;
      pub fn move_tasks(&self, tasks: &[Uuid], to: Uuid, actor: Actor) -> Result<Vec<Task>>;
      pub fn claim_worktree(&self, worktree: Uuid, workspace: Uuid, source: ClaimSource) -> Result<Option<Worktree>>; // None if already claimed (sticky)
      pub fn assign_worktree(&self, worktree: Uuid, workspace: Uuid) -> Result<Worktree>;          // explicit, overrides
      pub fn set_terminal_role(&self, terminal: Uuid, role: TerminalRole) -> Result<Terminal>;
      pub fn set_terminal_workspace(&self, terminal: Uuid, workspace: Uuid) -> Result<Terminal>;
      pub fn live_orchestrator(&self, workspace: Uuid) -> Result<Option<Terminal>>;
      pub fn next_task_key(&self, workspace: Uuid) -> Result<String>;
      pub fn create_task(&self, workspace: Uuid, title: &str, actor: Actor) -> Result<Task>;
      pub fn list_tasks(&self, scope: TaskScope, status: Option<TaskStatus>) -> Result<Vec<Task>>;
  }
  pub enum TaskScope { Workspace(Uuid), Repository(Uuid) }
  pub fn valid_prefix(prefix: &str) -> bool; // ^[a-z][a-z0-9]{0,7}$
  ```
  Errors: `StoreError::InvalidArgument { what: "task_prefix" }` for an invalid prefix; `StoreError::Conflict { what: "task_prefix" }` for a taken one; `StoreError::Conflict { what: "workspace_not_empty" }` from `delete_workspace` when tasks, worktrees or terminals remain; `StoreError::InvalidArgument { what: "main_workspace" }` for deleting Main. (Use the crate's existing error variants; if `Conflict` does not exist, use the closest existing variant and name it in the doc comment.)

- [ ] **Step 1: Write the failing migration tests** (`migrate.rs` tests)

```rust
/// The live board this must carry: keys, former keys from the prefixless
/// era, and notes, all ending up on the repository's Main.
#[test]
fn a_board_with_renamed_keys_survives_into_main() {
    let mut conn = open();
    migrate_only_to(&mut conn, 14);
    conn.execute_batch(
        "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
         INSERT INTO repositories VALUES (x'03', x'02', x'01', 'overnight', '/r/.git', '', 1, 'ov');
         INSERT INTO worktrees (id, repository_id, task_name, branch, worktree_path, resource_version, ordinal, is_main_checkout)
             VALUES (x'04', x'03', 'main', 'main', '/r', 1, 0, 1);
         INSERT INTO terminals (id, worktree_id, title, command_preset, intent, runtime_confirmed,
             lease_generation, epoch, columns, rows, resource_version)
             VALUES (x'05', x'04', 't', 'claude', 1, 0, 0, 0, 80, 24, 1),
                    (x'06', x'04', 's', 'shell', 1, 0, 0, 0, 80, 24, 1);
         INSERT INTO tasks (id, repository_id, key, former_key, title, status, status_since, created_at, resource_version)
             VALUES (x'07', x'03', 'ov-1', '-1', 'first', 'todo', 0, 0, 1);
         INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra)
             VALUES (x'08', x'07', 'decision', 'manager', 0, 'why', '{}');",
    )
    .unwrap();
    migrate(&mut conn, 14).unwrap();

    let (ws, name, prefix, main): (Vec<u8>, String, String, bool) = conn
        .query_row("SELECT id, name, task_prefix, is_main FROM workspaces WHERE repository_id = x'03'", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!((name.as_str(), prefix.as_str(), main), ("Main", "ov", true));
    let task_ws: Vec<u8> = conn.query_row("SELECT workspace_id FROM tasks WHERE key = 'ov-1'", [], |r| r.get(0)).unwrap();
    assert_eq!(task_ws, ws);
    let wt: (Vec<u8>, String) = conn
        .query_row("SELECT workspace_id, claim_source FROM worktrees WHERE id = x'04'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(wt, (ws.clone(), "migration".to_string()));
    let roles: Vec<(i64, Vec<u8>)> = conn
        .prepare("SELECT role, workspace_id FROM terminals ORDER BY id").unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
        .collect::<Result<_, _>>().unwrap();
    assert_eq!(roles, vec![(1, ws.clone()), (0, ws.clone())], "claude is an agent, shell is a shell");
    let notes: i64 = conn.query_row("SELECT count(*) FROM task_notes WHERE task_id = x'07'", [], |r| r.get(0)).unwrap();
    assert_eq!(notes, 1);
    let former: String = conn.query_row("SELECT former_key FROM tasks WHERE id = x'07'", [], |r| r.get(0)).unwrap();
    assert_eq!(former, "-1");
}

/// A repository whose prefix was never claimed still gets a Main with one.
#[test]
fn a_repository_without_a_prefix_gets_a_main_with_one() {
    let mut conn = open();
    migrate_only_to(&mut conn, 14);
    conn.execute_batch(
        "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
         INSERT INTO repositories VALUES (x'03', x'02', x'01', 'far cooler', '/r/.git', '', 1, '');",
    )
    .unwrap();
    migrate(&mut conn, 14).unwrap();
    let prefix: String = conn.query_row("SELECT task_prefix FROM workspaces", [], |r| r.get(0)).unwrap();
    assert_eq!(prefix, "fc");
}

/// Deleting a workspace that still holds a task is refused by the schema
/// itself, and deleting the repository still takes everything with it.
#[test]
fn a_workspace_with_tasks_cannot_be_deleted_but_its_repository_can() {
    let mut conn = open();
    migrate_only_to(&mut conn, 14);
    conn.execute_batch(
        "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
         INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'r');
         INSERT INTO tasks (id, repository_id, key, title, status, status_since, created_at, resource_version)
             VALUES (x'07', x'03', 'r-1', 't', 'todo', 0, 0, 1);",
    )
    .unwrap();
    migrate(&mut conn, 14).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    assert!(conn.execute("DELETE FROM workspaces", []).is_err());
    assert!(conn.execute("UPDATE tasks SET workspace_id = NULL", []).is_err(), "a task needs a workspace");
    conn.execute("DELETE FROM repositories", []).unwrap();
    let left: i64 = conn.query_row("SELECT count(*) FROM tasks", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0);
}
```

Match the INSERT column lists to the real schema at version 14 before running (the `task_notes` columns in particular, from `migrate.rs:475`).

- [ ] **Step 2: Run and watch them fail**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-store survives_into_main a_repository_without_a_prefix a_workspace_with_tasks`
Expected: FAIL (`no such table: workspaces`).

- [ ] **Step 3: Write migration 0015**

```rust
/// Workspaces: workstreams that own tasks and, when claimed, worktrees.
///
/// Every repository gets a Main that takes its prefix, its whole board, every
/// worktree and every terminal. A task's workspace is required, but SQLite will
/// not `ADD COLUMN … NOT NULL` without a default and a rebuild of `tasks` inside
/// this transaction would cascade into `task_notes` (foreign keys cannot be
/// turned off mid-transaction). So the column is added nullable, filled, and
/// held non-null by two triggers.
///
/// The references are plain (NO ACTION), which SQLite checks at the end of the
/// statement: deleting a workspace that anything still points at fails, which
/// is the refusal the design wants, while deleting a repository cascades
/// through workspaces, worktrees, terminals and tasks in one statement and
/// leaves nothing pointing anywhere by the time the check runs.
fn migration_0015_workspaces(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE workspaces (
            id BLOB PRIMARY KEY NOT NULL,
            repository_id BLOB NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            task_prefix TEXT NOT NULL,
            is_main INTEGER NOT NULL DEFAULT 0,
            ordinal INTEGER NOT NULL,
            resource_version INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX workspaces_one_prefix ON workspaces (task_prefix COLLATE NOCASE);
        CREATE UNIQUE INDEX workspaces_one_main ON workspaces (repository_id) WHERE is_main = 1;
        ALTER TABLE worktrees ADD COLUMN workspace_id BLOB REFERENCES workspaces(id);
        ALTER TABLE worktrees ADD COLUMN claim_source TEXT;
        ALTER TABLE terminals ADD COLUMN workspace_id BLOB REFERENCES workspaces(id);
        ALTER TABLE terminals ADD COLUMN role INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE tasks ADD COLUMN workspace_id BLOB REFERENCES workspaces(id);
        "#,
    )?;
    let repos: Vec<(Vec<u8>, String, String)> = tx
        .prepare("SELECT id, display_name, task_key_prefix FROM repositories ORDER BY rowid")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let now = crate::tasks::now_ms();
    for (ordinal, (repo, name, prefix)) in repos.iter().enumerate() {
        let prefix = if prefix.is_empty() {
            crate::workspaces::free_prefix(tx, &crate::tasks::derive_prefix(name))?
        } else {
            prefix.clone()
        };
        let ws = uuid::Uuid::new_v4();
        tx.execute(
            "INSERT INTO workspaces (id, repository_id, name, task_prefix, is_main, ordinal, created_at)
             VALUES (?1, ?2, 'Main', ?3, 1, ?4, ?5)",
            rusqlite::params![ws.as_bytes().to_vec(), repo, prefix, ordinal as i64, now],
        )?;
        tx.execute(
            "UPDATE worktrees SET workspace_id = ?1, claim_source = 'migration' WHERE repository_id = ?2",
            rusqlite::params![ws.as_bytes().to_vec(), repo],
        )?;
        tx.execute("UPDATE tasks SET workspace_id = ?1 WHERE repository_id = ?2", rusqlite::params![ws.as_bytes().to_vec(), repo])?;
    }
    tx.execute_batch(
        r#"
        UPDATE terminals SET workspace_id = (SELECT workspace_id FROM worktrees w WHERE w.id = terminals.worktree_id);
        UPDATE terminals SET role = 0 WHERE command_preset = 'shell';
        CREATE TRIGGER tasks_need_a_workspace BEFORE INSERT ON tasks WHEN NEW.workspace_id IS NULL
            BEGIN SELECT RAISE(ABORT, 'a task needs a workspace'); END;
        CREATE TRIGGER tasks_keep_a_workspace BEFORE UPDATE OF workspace_id ON tasks WHEN NEW.workspace_id IS NULL
            BEGIN SELECT RAISE(ABORT, 'a task needs a workspace'); END;
        "#,
    )
}
```

Use the crate's existing time helper instead of `now_ms` if it has a different name (`grep -n "fn now" crates/store/src`). `repositories.task_key_prefix` is left in place (dropping it would break historical migration 0012 and its tests) and is no longer read by new code; say so in a comment on `Repository.task_key_prefix` and remove it from the struct.

- [ ] **Step 4: Write the failing store API tests** (`workspaces.rs` `mod tests`)

```rust
#[test]
fn registering_a_repository_makes_its_main() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("billing service");
    let main = store.ensure_main_workspace(repo).unwrap();
    assert!(main.is_main);
    assert_eq!(main.name, "Main");
    assert_eq!(main.task_prefix, "bs");
    assert_eq!(store.ensure_main_workspace(repo).unwrap().id, main.id, "idempotent");
}

#[test]
fn prefixes_are_unique_on_the_runner_ignoring_case() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("r");
    store.create_workspace(repo, "Billing", "bil").unwrap();
    assert!(store.create_workspace(repo, "Other", "BIL").is_err());
    assert!(store.create_workspace(repo, "Bad", "1x").is_err(), "must start with a letter");
    assert!(store.create_workspace(repo, "Long", "abcdefghi").is_err(), "at most 8");
}

#[test]
fn a_key_is_never_issued_twice_across_prefix_renames() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("r");
    let ws = store.create_workspace(repo, "Billing", "bil").unwrap();
    let a = store.create_task(ws.id, "a", Actor::User).unwrap();
    let ws = store.set_workspace_prefix(ws.id, ws.resource_version, "pay").unwrap();
    let b = store.create_task(ws.id, "b", Actor::User).unwrap();
    let ws = store.set_workspace_prefix(ws.id, ws.resource_version, "bil").unwrap();
    let c = store.create_task(ws.id, "c", Actor::User).unwrap();
    assert_eq!((a.key.as_str(), b.key.as_str(), c.key.as_str()), ("bil-1", "pay-1", "bil-2"));
}

#[test]
fn numbering_counts_former_keys() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("r");
    let main = store.ensure_main_workspace(repo).unwrap();
    store.insert_task_with_former_key_for_test(main.id, "zz-9", "r-7");
    let t = store.create_task(main.id, "next", Actor::User).unwrap();
    assert_eq!(t.key, format!("{}-8", main.task_prefix));
}

#[test]
fn a_moved_task_keeps_its_key_and_leaves_a_note() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("r");
    let main = store.ensure_main_workspace(repo).unwrap();
    let billing = store.create_workspace(repo, "Billing", "bil").unwrap();
    let t = store.create_task(main.id, "t", Actor::User).unwrap();
    let moved = store.move_tasks(&[t.id], billing.id, Actor::Manager).unwrap();
    assert_eq!(moved[0].key, t.key);
    assert_eq!(moved[0].workspace_id, billing.id);
    assert_eq!(store.tasks_with_key(None, &t.key).unwrap().len(), 1);
    let notes = store.list_notes_for_test(t.id);
    assert!(notes.iter().any(|n| n.kind == NoteKind::StatusChange || n.body.contains("Billing")),
        "a move is recorded in the task's history");
}

#[test]
fn a_move_across_repositories_is_refused() {
    let store = Store::open_in_memory().unwrap();
    let a = store.register_repository_for_test("a");
    let b = store.register_repository_for_test("b");
    let t = store.create_task(store.ensure_main_workspace(a).unwrap().id, "t", Actor::User).unwrap();
    assert!(store.move_tasks(&[t.id], store.ensure_main_workspace(b).unwrap().id, Actor::User).is_err());
}

#[test]
fn deleting_refuses_main_and_anything_still_holding_work() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("r");
    let main = store.ensure_main_workspace(repo).unwrap();
    assert!(store.delete_workspace(main.id).is_err());
    let ws = store.create_workspace(repo, "Billing", "bil").unwrap();
    let t = store.create_task(ws.id, "t", Actor::User).unwrap();
    assert!(store.delete_workspace(ws.id).is_err());
    store.move_tasks(&[t.id], main.id, Actor::User).unwrap();
    store.delete_workspace(ws.id).unwrap();
}

#[test]
fn a_claim_sticks_and_an_assignment_overrides_it() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("r");
    let main = store.ensure_main_workspace(repo).unwrap();
    let billing = store.create_workspace(repo, "Billing", "bil").unwrap();
    let wt = store.create_unclaimed_worktree_for_test(repo, "/r/.worktrees/x");
    assert!(store.claim_worktree(wt, billing.id, ClaimSource::Hook).unwrap().is_some());
    assert!(store.claim_worktree(wt, main.id, ClaimSource::Hook).unwrap().is_none(), "sticky");
    let wt = store.assign_worktree(wt, main.id).unwrap();
    assert_eq!((wt.workspace_id, wt.claim_source), (Some(main.id), Some(ClaimSource::Explicit)));
}

#[test]
fn one_orchestrator_per_workspace() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("r");
    let main = store.ensure_main_workspace(repo).unwrap();
    let wt = store.create_unclaimed_worktree_for_test(repo, "/r");
    let a = store.create_terminal_for_test(wt, main.id);
    let b = store.create_terminal_for_test(wt, main.id);
    store.set_terminal_role(a, TerminalRole::Orchestrator).unwrap();
    assert!(store.set_terminal_role(b, TerminalRole::Orchestrator).is_err());
    assert_eq!(store.live_orchestrator(main.id).unwrap().map(|t| t.id), Some(a));
}
```

Add the `*_for_test` helpers to the `#[cfg(test)] impl Store` block in `tasks.rs`: `insert_task_with_former_key_for_test(workspace, key, former)`, `list_notes_for_test(task)`, `create_unclaimed_worktree_for_test(repo, path) -> Uuid`, `create_terminal_for_test(worktree, workspace) -> Uuid`. Update `create_task_for_test` and `seeded()` to set `workspace_id` (they raw-INSERT and would now hit the trigger). `live_orchestrator` counts a terminal as live when `role = Orchestrator` and `intent` is not STOPPED/FAILED and `exit_code IS NULL`; enforce "one" in `set_terminal_role` by that same test.

- [ ] **Step 5: Run and watch them fail, then implement `workspaces.rs`**

Implement per **Interfaces**:
- `valid_prefix`: `^[a-z][a-z0-9]{0,7}$`, checked before touching SQL.
- `free_prefix(conn, base)`: `base`, then `base2`, `base3`… until no `workspaces.task_prefix` matches case-insensitively (same retry rule `claim_task_key_prefix` uses).
- `ensure_main_workspace`: return the `is_main` row, or insert one named "Main" with `free_prefix(derive_prefix(display_name))`.
- `next_task_key(workspace)`: `prefix-N` with `N = 1 + MAX` over **all tasks on the runner** of the number after `prefix-` in `key` **and** in `former_key` (case-insensitive `LIKE prefix || '-%'`, `CAST(SUBSTR(…) AS INTEGER)`), so a deleted top number is not reused only if it was never observed — accepted; the spec's guarantee is about renames, which this covers.
- `create_task(workspace, …)`: repository from the workspace; replaces `create_task(repository, …)`. Update every caller (`grep -rn "create_task(" crates`).
- `move_tasks`: one transaction; refuse when any task's repository differs from the target's; set `workspace_id`; append a `Comment` note by `actor` with body `"Moved from {old name} to {new name}."`.
- `list_tasks(TaskScope, status)`: replaces `list_tasks(repository, status)`; `list_tasks_stale_for` takes `TaskScope` too.
- `delete_workspace`: refuse `is_main`; otherwise `DELETE`, mapping the FK error to `Conflict { what: "workspace_not_empty" }`.

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-store`
Expected: PASS. Break `next_task_key` to ignore `former_key`, watch `numbering_counts_former_keys` fail, restore.

- [ ] **Step 6: Commit**

```bash
git add crates/store
git commit -m "feat(store): workspaces own tasks and worktrees, and every repository starts with Main"
```

### Task 9: Proto and daemon RPCs for workspaces

**Files:**
- Modify: `proto/farcooler.proto`
- Modify: `crates/protocol/src/lib.rs` (capability `WORKSTREAMS = "workstreams"`, method → capability rows, add to `ALL`)
- Create: `crates/daemon/src/workspace_ops.rs`
- Modify: `crates/daemon/src/rpc.rs` (dispatch arms, `required_scope`, the two scope-table tests at :2263 and :2433)
- Modify: `crates/daemon/src/service.rs` (`register_repository` calls `ensure_main_workspace` instead of `assign_task_key_prefix`; `create_worktree_with`/`adopt_branch` take `workspace: Option<Uuid>`)
- Modify: `crates/daemon/src/task_ops.rs` (`create` takes a workspace, `list` a scope; `announce` carries the workspace)
- Modify: `crates/daemon/src/watch.rs` (`task_changed_event` sets `workspace_id`)
- Test: `crates/daemon/tests/rpc_over_socket.rs`

**Interfaces:**
- Consumes: Task 8's `Store` API.
- Produces (proto; take the next free tags — request oneof after 79, result after 42, event after 22; `Terminal`, `Worktree`, `Task`, `TaskCreate`, `TaskChanged`, `TaskListRequest` after their highest):
  ```proto
  message Workspace { bytes id = 1; uint64 resource_version = 2; bytes repository_id = 3;
      string name = 4; string task_prefix = 5; bool is_main = 6; uint32 ordinal = 7;
      optional bytes orchestrator_terminal_id = 8; string home = 9; string charter_path = 10; }
  message WorkspaceList { repeated Workspace items = 1; }
  message WorkspaceCreate { string name = 1; string task_prefix = 2; }          // target_resource_id = repository
  message WorkspaceRename { string name = 1; }                                  // target = workspace
  message WorkspaceSetPrefix { string task_prefix = 1; }                        // target = workspace
  message WorkspaceStartOrchestrator { string harness = 1; bool replace = 2; }  // target = workspace
  message TaskMove { repeated bytes task_ids = 1; bytes workspace_id = 2; }
  message WorktreeAssign { bytes workspace_id = 1; }                            // target = worktree
  message TerminalSetRole { TerminalRole role = 1; }                            // target = terminal
  enum TerminalRole { TERMINAL_ROLE_UNSPECIFIED = 0; TERMINAL_ROLE_SHELL = 1;
      TERMINAL_ROLE_AGENT = 2; TERMINAL_ROLE_ORCHESTRATOR = 3; }
  // Worktree += optional bytes workspace_id; optional string claim_source; repeated bytes foreign_writer_workspace_ids;
  // Terminal += optional bytes workspace_id; TerminalRole role;
  // Task += bytes workspace_id;   TaskCreate += bytes workspace_id;
  // TaskListRequest += optional bytes workspace_id;   TaskChanged += optional bytes workspace_id;
  // WorktreeCreate += optional bytes workspace_id;
  ```
- Produces (methods, all capability `workstreams`): `workspace.list` (Read), `workspace.create`, `workspace.rename`, `workspace.set_prefix`, `workspace.delete`, `task.move`, `worktree.assign`, `terminal.set_role` (Control), `workspace.start_orchestrator` (Control) is **not** added here; Task 11 adds its proto message, dispatch arm and scope row together, so the scope-table test never lists a method that is not dispatched.
- Produces (daemon): `workspace_ops::{list, create, rename, set_prefix, delete, move_tasks, assign_worktree, set_role}`; each mutation calls `watcher.announce_fleet_changed()`; `task.move` also announces `task_changed` per task with the new workspace.

- [ ] **Step 1: Write the failing socket tests** (`rpc_over_socket.rs`)

```rust
#[tokio::test]
async fn a_registered_repository_lists_its_main_workspace() {
    let h = start(Scope::Control).await;
    let mut client = connect(&h).await;
    let repo = registered_repository(&h, &mut client).await;
    let mut r = request("workspace.list");
    r.target_resource_id = Some(repo.as_bytes().to_vec().into());
    let Some(result::Value::WorkspaceList(list)) = client.call(r).await.unwrap().value else { panic!() };
    assert_eq!(list.items.len(), 1);
    assert!(list.items[0].is_main);
    assert_eq!(list.items[0].name, "Main");
}

#[tokio::test]
async fn a_task_created_on_a_workspace_is_announced_with_it() {
    let h = start(Scope::Control).await;
    let mut client = connect(&h).await;
    let repo = registered_repository(&h, &mut client).await;
    let billing = create_workspace(&mut client, repo, "Billing", "bil").await;
    let mut events = subscribe(&h).await;
    let task = create_task_on(&mut client, billing.id.clone(), "t").await;
    assert_eq!(task.key, "bil-1");
    let changed = next_task_changed(&mut events).await;
    assert_eq!(changed.workspace_id.as_deref(), Some(&billing.id[..]));
}

#[tokio::test]
async fn moving_a_task_moves_its_board() {
    let h = start(Scope::Control).await;
    let mut client = connect(&h).await;
    let repo = registered_repository(&h, &mut client).await;
    let main = main_workspace(&mut client, repo).await;
    let billing = create_workspace(&mut client, repo, "Billing", "bil").await;
    let task = create_task_on(&mut client, main.id.clone(), "t").await;
    let mut r = request("task.move");
    r.payload = Some(request::Payload::TaskMove(TaskMove { task_ids: vec![task.id.clone()], workspace_id: billing.id.clone() }));
    client.call(r).await.unwrap();
    assert!(list_tasks_on(&mut client, main.id.clone()).await.is_empty());
    assert_eq!(list_tasks_on(&mut client, billing.id.clone()).await[0].key, task.key);
}

#[tokio::test]
async fn read_scope_cannot_create_a_workspace() {
    let h = start(Scope::Read).await;
    let mut client = connect(&h).await;
    let repo = registered_repository_via_control(&h).await;
    let mut r = request("workspace.create");
    r.target_resource_id = Some(repo.as_bytes().to_vec().into());
    r.payload = Some(request::Payload::WorkspaceCreate(WorkspaceCreate { name: "B".into(), task_prefix: "b".into() }));
    assert!(client.call(r).await.is_err());
}
```

Write the helpers (`create_workspace`, `main_workspace`, `create_task_on`, `list_tasks_on`, `subscribe`, `next_task_changed`, `registered_repository_via_control`) next to the existing ones at :151–215, in the same style; reuse what exists (e.g. `announced_actor` at :3053 shows how events are read).

- [ ] **Step 2: Run and watch them fail**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-daemon --test rpc_over_socket workspace`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** the proto additions, capability rows, `workspace_ops.rs`, the dispatch arms, the `required_scope` rows, and the `task_ops`/`watch` changes per **Interfaces**. `task.create` without a `workspace_id` uses the repository's Main (resolved from `target_resource_id` as today), so an older caller still lands somewhere sane. `task.list` with a `workspace_id` lists that board; with only a repository, lists every workspace in it. Map store errors to the protocol's existing `ErrorCode`s (`INVALID_ARGUMENT`, `CONFLICT` or nearest) with a message an app can say as-is ("That prefix is already used by another workspace.", "Move this workspace's tasks and worktrees first.", "Main can't be deleted.").

- [ ] **Step 4: Run the daemon tests**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-daemon -p farcooler-protocol`
Expected: PASS, including the scope-table tests with the new methods listed.

- [ ] **Step 5: Commit**

```bash
git add proto crates/protocol crates/daemon
git commit -m "feat(daemon): workspace RPCs, and task events say which board moved"
```

### Task 10: Home, charter and pane environment

**Files:**
- Create: `crates/daemon/src/workspace_home.rs`
- Modify: `crates/core/src/lib.rs` (`pane_env::WORKSPACE = "FARCOOLER_WORKSPACE"`, `pane_env::CHARTER = "FARCOOLER_CHARTER"`)
- Modify: `crates/daemon/src/service.rs` (`with_pane_env` :661 exports `FARCOOLER_WORKSPACE` for every agent pane with a workspace; daemon startup calls `workspace_home::adopt_repository_charters`)
- Modify: `crates/daemon/src/workspace_ops.rs` (`create` makes the home and seeds the charter)

**Interfaces:**
- Produces:
  ```rust
  pub fn home(root: &Path, workspace: Uuid) -> PathBuf;          // <FARCOOLER_HOME>/workspaces/<uuid>
  pub fn charter_path(root: &Path, workspace: Uuid) -> PathBuf;  // home/charter.md
  pub fn make_home(root: &Path, workspace: Uuid, seed_from: Option<&Path>) -> io::Result<PathBuf>; // never overwrites an existing charter
  pub fn adopt_repository_charters(root: &Path, mains: &[(Uuid, PathBuf /* main checkout */)]) -> io::Result<usize>;
  ```
  `root` is the directory `Service` already calls `self.root` (the runner's `FARCOOLER_HOME`-derived root; confirm with `paths.rs`).

- [ ] **Step 1: Write the failing tests** (`workspace_home.rs` `mod tests`, using `tempfile`)

```rust
#[test]
fn a_repository_charter_is_copied_into_main_and_left_where_it_was() {
    let root = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join(".farcooler")).unwrap();
    std::fs::write(repo.path().join(".farcooler/manager.md"), "ours").unwrap();
    let main = Uuid::new_v4();
    assert_eq!(adopt_repository_charters(root.path(), &[(main, repo.path().into())]).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(charter_path(root.path(), main)).unwrap(), "ours");
    assert!(repo.path().join(".farcooler/manager.md").exists());
}

#[test]
fn an_existing_charter_is_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let ws = Uuid::new_v4();
    make_home(root.path(), ws, None).unwrap();
    std::fs::write(charter_path(root.path(), ws), "edited").unwrap();
    let seed = root.path().join("seed.md");
    std::fs::write(&seed, "seed").unwrap();
    make_home(root.path(), ws, Some(&seed)).unwrap();
    assert_eq!(std::fs::read_to_string(charter_path(root.path(), ws)).unwrap(), "edited");
}

#[test]
fn an_agent_pane_knows_its_workspace() {
    let ws = Uuid::new_v4();
    let cmd = crate::service::with_pane_env_for_test("claude", Some("ov-3"), Some(ws), "claude");
    assert!(cmd.contains(&format!("FARCOOLER_WORKSPACE={ws}")));
}
```

(`with_pane_env_for_test` is a thin `#[cfg(test)]` wrapper you add over `with_pane_env`'s real signature.)

- [ ] **Step 2: Run, watch fail, implement, run**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-daemon workspace_home an_agent_pane_knows`
Expected: FAIL, then PASS after implementing. `workspace.create` seeds from Main's charter; `adopt_repository_charters` runs at daemon start for every Main whose home has no charter. Fill `Workspace.home` and `.charter_path` in `workspace.list`.

- [ ] **Step 3: Commit**

```bash
git add crates/core crates/daemon
git commit -m "feat(daemon): every workspace has a home and a charter outside the repository"
```

### Task 11: Orchestrator launch recipes

Check Task 1's findings first. The recipes below assume the spike confirmed them; change a row if it did not, and note the change in the commit message.

**Files:**
- Create: `crates/daemon/src/orchestrator.rs`
- Modify: `crates/daemon/src/service.rs` (`preset_command_with_hooks` :293 gains an `OrchestratorLaunch` in `LaunchExtras`; `claude_extra_flags` :264 writes an orchestrator-specific settings file)
- Modify: `crates/daemon/src/workspace_ops.rs` (`start_orchestrator`), `crates/daemon/src/rpc.rs` (dispatch + scope row for `workspace.start_orchestrator`)
- Modify: `crates/daemon/src/skill_install.rs` (manager skill step 1 reads `$FARCOOLER_CHARTER`; drop `.farcooler/manager.md`)

**Interfaces:**
- Produces:
  ```rust
  pub struct OrchestratorLaunch { pub workspace: Uuid, pub home: PathBuf, pub main_checkout: PathBuf,
      pub charter: PathBuf, pub memory_dir: Option<PathBuf> }
  pub fn working_directory(h: Harness, l: &OrchestratorLaunch) -> PathBuf;  // Claude/Cursor: home; Codex: main_checkout
  pub fn extra_args(h: Harness, l: &OrchestratorLaunch) -> Vec<String>;
      // Claude: ["--add-dir", main]; Cursor: ["--workspace", main]; Codex: []
  pub fn extra_env(h: Harness, l: &OrchestratorLaunch) -> Vec<(String, String)>;
      // all: FARCOOLER_WORKSPACE, FARCOOLER_CHARTER, FARCOOLER_ACTOR=manager
      // Claude also: CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1
  pub fn claude_memory_dir(main_checkout: &Path) -> PathBuf; // the rule Task 1 measured
  ```
  `workspace.start_orchestrator` creates a terminal in the workspace's Main-checkout worktree row (Codex) or in the Main-checkout row with the pane's working directory overridden to the home (Claude, Cursor), with `role = Orchestrator` and `workspace_id = <ws>`; refuses when `live_orchestrator` is `Some` unless `replace`, in which case it stops that terminal first.

- [ ] **Step 1: Write the failing tests** (`orchestrator.rs` `mod tests`)

```rust
fn launch() -> OrchestratorLaunch {
    OrchestratorLaunch { workspace: Uuid::nil(), home: "/h/ws".into(), main_checkout: "/r".into(),
        charter: "/h/ws/charter.md".into(), memory_dir: Some("/m".into()) }
}

#[test]
fn claude_and_cursor_live_in_the_home_and_codex_in_the_repository() {
    assert_eq!(working_directory(Harness::Claude, &launch()), PathBuf::from("/h/ws"));
    assert_eq!(working_directory(Harness::Cursor, &launch()), PathBuf::from("/h/ws"));
    assert_eq!(working_directory(Harness::Codex, &launch()), PathBuf::from("/r"));
}

#[test]
fn each_harness_is_pointed_back_at_the_repository() {
    assert_eq!(extra_args(Harness::Claude, &launch()), ["--add-dir", "/r"]);
    assert_eq!(extra_args(Harness::Cursor, &launch()), ["--workspace", "/r"]);
    assert!(extra_args(Harness::Codex, &launch()).is_empty());
    let env = extra_env(Harness::Claude, &launch());
    assert!(env.contains(&("CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD".into(), "1".into())));
    assert!(env.contains(&("FARCOOLER_CHARTER".into(), "/h/ws/charter.md".into())));
}

#[test]
fn claude_memory_follows_the_repository() {
    assert_eq!(
        claude_memory_dir(Path::new("/Users/e-liang/Dev/overnight")),
        dirs_home().join(".claude/projects/-Users-e-liang-Dev-overnight/memory"),
    );
}
// `dirs_home()` is whatever this crate already uses for `$HOME` (grep `home_dir` in
// crates/daemon/src). The expected directory name is the rule Task 1 recorded;
// if the spike found a different rule, this assertion is where it goes.
```

and a socket test in `rpc_over_socket.rs`:

```rust
#[tokio::test]
async fn a_second_orchestrator_is_refused_unless_replacing() {
    let h = start(Scope::Control).await;
    let mut client = connect(&h).await;
    let repo = registered_repository(&h, &mut client).await;
    let main = main_workspace(&mut client, repo).await;
    let first = start_orchestrator(&mut client, main.id.clone(), "shell-as-claude", false).await.unwrap();
    assert!(start_orchestrator(&mut client, main.id.clone(), "shell-as-claude", false).await.is_err());
    let second = start_orchestrator(&mut client, main.id.clone(), "shell-as-claude", true).await.unwrap();
    assert_ne!(first.id, second.id);
}
```

Write the `start_orchestrator(client, workspace_id, harness, replace) -> Result<Terminal, _>` helper beside Task 9's helpers. The socket test cannot launch a real harness; add a test-only harness value that runs `sh` under the orchestrator recipe (the existing test support already runs `shell` presets). Do not add it to the CLI's `--harness` values.

- [ ] **Step 2: Run, watch fail, implement, run**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-daemon orchestrator a_second_orchestrator`
Expected: FAIL, then PASS. The Claude settings file written for an orchestrator is the existing hooks settings plus `"autoMemoryDirectory": "<memory_dir>"`, written to `<runtime>/orchestrator-<workspace>.json` (not `claude-hooks.json`, which agent panes share). Update the manager skill text in `skill_install.rs` and `assets/manager/SKILL.md` so step 1 reads `$FARCOOLER_CHARTER`, and its tests (`the_skill_promises_no_wake_while_none_exists` stays as is).

- [ ] **Step 3: Live check on a scratch runner**

Build, start a scratch daemon (`FARCOOLER_HOME=/tmp/fc-ws/home`, capture PID), register `/tmp/fc-ws/spike/repo`, `farcooler workspace start-orchestrator Main --harness claude` (Task 12 adds the CLI; if running this before Task 12, call the RPC through a test binary or defer this step to after Task 12). Open the pane (`farcooler terminal screen <id>`), ask "What word do your project rules tell you to say?" and "Print $FARCOOLER_CHARTER". Expected: PINEAPPLE, and the charter path. Stop the daemon by PID.

- [ ] **Step 4: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): start a workspace's orchestrator from its home, per harness"
```

### Task 12: CLI — `farcooler workspace`, `task move`, `worktree assign`, `terminal set-role`

**Files:**
- Create: `crates/cli/src/workspaces.rs`
- Modify: `crates/cli/src/main.rs` (`Command::Workspace(workspaces::WorkspaceCmd)`, `WorktreeCmd::Assign`, `TerminalCmd::SetRole`, `worktree list --json` adds workspace fields)
- Modify: `crates/cli/src/tasks.rs` (`TaskCmd::Move`, `--workspace` on `list`/`create`, `repository_for` → `board_for`, dispatch passes the workspace)
- Modify: `crates/core/src/lib.rs` if `pane_env::WORKSPACE` is not already exported for the CLI

**Interfaces:**
- Produces (CLI):
  ```
  farcooler workspace create <repo> --name <name> --prefix <prefix>
  farcooler workspace list [<repo>]                   # --json: {workspaces:[{id, repository, name, task_prefix, is_main, ordinal, orchestrator, home, charter}]}
  farcooler workspace show <ws>
  farcooler workspace rename <ws> <name>
  farcooler workspace set-prefix <ws> <prefix>
  farcooler workspace delete <ws>
  farcooler workspace start-orchestrator <ws> --harness claude|codex|cursor [--replace]
  farcooler worktree assign <worktree> --to <ws>
  farcooler terminal set-role <terminal> shell|agent|orchestrator
  farcooler task move <key>... --to <ws>
  farcooler task list [--workspace <ws>] [--repo <repo>] …   # default: $FARCOOLER_WORKSPACE, else the whole repository
  farcooler task create --workspace <ws> …                   # default: $FARCOOLER_WORKSPACE, else the repository's Main
  ```
  `<ws>` resolves by exact name within `--repo` (or the only repository, or `$FARCOOLER_WORKSPACE`'s repository), case-insensitive, then by task prefix, then by id suffix — refusing ambiguity by listing candidates, the way `resolve_repository` (:3331) does.
  `farcooler worktree list --json` rows gain `workspace` (id or null), `claim_source`, `foreign_writers` (workspace names); the envelope gains `workspaces` (the same objects as `workspace list --json`), so the Mac reads the fleet in one call.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_old_create_spelling_points_at_worktree() {
    let err = Cli::try_parse_from(["farcooler", "workspace", "create", "repo", "fix-it", "--branch", "fix-it"])
        .map(|cli| crate::run_parse_only(cli))
        .unwrap_err()
        .to_string();
    assert!(err.contains("farcooler worktree create"), "{err}");
}

#[test]
fn workspace_commands_parse() {
    for line in [
        "farcooler workspace create repo --name Billing --prefix bil",
        "farcooler --json workspace list",
        "farcooler workspace rename Billing Payments",
        "farcooler workspace set-prefix Billing pay",
        "farcooler workspace delete Billing",
        "farcooler workspace start-orchestrator Billing --harness codex --replace",
        "farcooler worktree assign fix-it --to Billing",
        "farcooler terminal set-role abc orchestrator",
        "farcooler task move ov-3 ov-4 --to Billing",
        "farcooler task list --workspace Billing",
    ] {
        Cli::try_parse_from(line.split_whitespace()).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
}
```

For the first test: give `workspace create` a hidden `--branch` arg (`#[arg(long, hide = true)] branch: Option<String>`) and a hidden positional catch-all; when `--branch` is present, fail with: `workspace create makes a workstream now. To make a worktree, run: farcooler worktree create <repo> <name> --branch <branch>`. Implement `run_parse_only` as the validation step `workspace()` runs before connecting, so the test needs no daemon.

Dispatch test, extending `FakeLink`: `dispatch_into_a_new_worktree_claims_it_for_the_tasks_workspace` — asserts `link.sent("worktree.create")`'s `WorktreeCreate.workspace_id` equals the task's `workspace_id`.

A board-default test: `task_list_defaults_to_the_panes_workspace` — with `FARCOOLER_WORKSPACE` set (use the existing env-injection seam in `tasks.rs`, see `resolve_key`), `task.list` is sent with that `workspace_id`; without it and with `--repo`, it is sent with no `workspace_id`.

- [ ] **Step 2: Run, watch fail, implement, run**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-cli`
Expected: FAIL, then PASS. Human output of `task list` without a workspace prints a `WORKSPACE` column; with one, it does not. Update `every_command_the_manager_skill_names_parses` inputs if the skill changed.

- [ ] **Step 3: Commit**

```bash
git add crates/cli crates/core
git commit -m "feat(cli): farcooler workspace, task move, worktree assign"
```

### Task 13: Claiming worktrees

**Files:**
- Create: `crates/daemon/src/claims.rs`
- Create: `crates/daemon/src/proc_cwd.rs`
- Modify: `crates/daemon/src/hook_ingress.rs` (`announced_terminal` :305–340: after resolving a terminal and a `cwd`, call `claims::observe(svc, terminal, cwd, ClaimSource::Hook)`; extend the Claude path to resolve the terminal by session id so Claude hooks count too)
- Modify: `crates/daemon/src/reconcile.rs` (after adopting at :185, if the row is unclaimed, run `claims::scan_processes` once for that repository)
- Modify: `crates/daemon/src/service.rs` (`create_worktree_with`/`adopt_branch` claim explicitly when given a workspace; `worktree_view` fills `workspace_id`, `claim_source`, `foreign_writer_workspace_ids`)

**Interfaces:**
- Produces:
  ```rust
  /// The worktree whose path contains `cwd`, choosing the longest such path.
  pub fn containing_worktree<'a>(worktrees: &'a [Worktree], cwd: &Path) -> Option<&'a Worktree>;
  /// One observation of a terminal working in `cwd`.
  pub async fn observe(svc: &Service, terminal: Uuid, cwd: &Path, source: ClaimSource) -> Result<()>;
  /// For every unclaimed worktree in `repository`, read the working directory of
  /// every process under every live non-orchestrator pane and observe it.
  pub async fn scan_processes(svc: &Service, repository: Uuid) -> Result<()>;
  // Service gains: foreign_writers: Mutex<HashMap<Uuid /*worktree*/, HashSet<Uuid /*workspace*/>>>,
  //   cleared for a worktree when its terminals from that workspace close.
  // proc_cwd.rs:
  pub fn cwd_of(pid: i32) -> Option<PathBuf>; // macOS: proc_pidinfo(PROC_PIDVNODEPATHINFO); Linux: read_link(/proc/<pid>/cwd)
  pub fn descendants(table: &Foreground, root_pgid: i32) -> Vec<i32>;
  ```
  `observe` rules, in order: terminal missing or `role == Orchestrator` → nothing; terminal has no `workspace_id` → nothing; no containing worktree → nothing; worktree unclaimed → `claim_worktree(worktree, terminal.workspace_id, source)` and announce fleet changed; claimed by another workspace → insert into `foreign_writers` and announce fleet changed if it was new; claimed by the same workspace → nothing.

- [ ] **Step 1: Write the failing tests** (`claims.rs` `mod tests`)

```rust
fn wt(path: &str) -> Worktree {
    Worktree { id: Uuid::new_v4(), repository_id: Uuid::nil(), branch: String::new(),
        worktree_path: path.into(), hidden: false, creation_failed: false, is_main_checkout: false,
        worktree_missing: false, ordinal: 0, resource_version: 1, workspace_id: None, claim_source: None }
}

/// The fixture repository's main checkout row, which `fixture()` registers.
fn main_worktree(svc: &Service, repo: Uuid) -> Uuid {
    svc.store.list_worktrees_for_repository(repo).unwrap()
        .into_iter().find(|w| w.is_main_checkout).expect("fixture registers its main checkout").id
}

#[test]
fn the_longest_worktree_path_wins() {
    let rows = [wt("/r"), wt("/r/.worktrees/x"), wt("/r/.worktrees/xy")];
    let hit = containing_worktree(&rows, Path::new("/r/.worktrees/x/src/lib")).unwrap();
    assert_eq!(hit.worktree_path, "/r/.worktrees/x");
    let hit = containing_worktree(&rows, Path::new("/r/.worktrees/xy")).unwrap();
    assert_eq!(hit.worktree_path, "/r/.worktrees/xy", "a path prefix is not a directory prefix");
    assert_eq!(containing_worktree(&rows, Path::new("/elsewhere")), None);
}

#[tokio::test]
async fn an_agent_working_in_an_unclaimed_worktree_claims_it_for_its_workspace() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
    let wt = svc.store.create_unclaimed_worktree_for_test(repo, "/fixture/.worktrees/x");
    let term = svc.store.create_terminal_for_test(main_worktree(&svc, repo), billing.id);
    observe(&svc, term, Path::new("/fixture/.worktrees/x/src"), ClaimSource::Hook).await.unwrap();
    let row = svc.store.get_worktree(wt).unwrap();
    assert_eq!((row.workspace_id, row.claim_source), (Some(billing.id), Some(ClaimSource::Hook)));
}

#[tokio::test]
async fn an_orchestrator_never_claims() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
    let wt = svc.store.create_unclaimed_worktree_for_test(repo, "/fixture/.worktrees/x");
    let term = svc.store.create_terminal_for_test(main_worktree(&svc, repo), billing.id);
    svc.store.set_terminal_role(term, TerminalRole::Orchestrator).unwrap();
    observe(&svc, term, Path::new("/fixture/.worktrees/x"), ClaimSource::Hook).await.unwrap();
    assert_eq!(svc.store.get_worktree(wt).unwrap().workspace_id, None);
}

#[tokio::test]
async fn working_in_another_workspaces_worktree_is_reported_not_stolen() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
    let main_wt = main_worktree(&svc, repo);
    let term = svc.store.create_terminal_for_test(main_wt, billing.id);
    let path = svc.store.get_worktree(main_wt).unwrap().worktree_path;
    observe(&svc, term, Path::new(&path), ClaimSource::Hook).await.unwrap();
    assert_eq!(svc.store.get_worktree(main_wt).unwrap().workspace_id, Some(main.id));
    assert!(svc.foreign_writers(main_wt).contains(&billing.id));
}
```

And hook-payload fixture tests in `hook_ingress.rs`'s tests: for each of `claude`, `codex`, `cursor`, a recorded payload (taken from Task 1's spike; store under `crates/daemon/tests/fixtures/hooks/<harness>-cwd.json`) whose `cwd` is inside an unclaimed worktree results in that worktree being claimed by the pane's workspace.

`proc_cwd` test (macOS and Linux both): `cwd_of(std::process::id() as i32) == std::env::current_dir()`.

- [ ] **Step 2: Run, watch fail, implement, run**

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-daemon claims proc_cwd hook_ingress`
Expected: FAIL, then PASS. `containing_worktree` compares canonicalized paths component-wise (`Path::starts_with`), which is what makes `/r/.worktrees/xy` not match `/r/.worktrees/x`. Break it to a string `starts_with`, watch `the_longest_worktree_path_wins` fail, restore.

- [ ] **Step 3: Dispatch warning**

In `crates/cli/src/tasks.rs` `dispatch`, when the chosen worktree's `foreign_writer_workspace_ids` is non-empty or its `workspace_id` differs from the task's, print the warning `A terminal from <Workspace> is working in a worktree <Owner> owns.` (the existing busy-lane warning is the pattern). Test it with `FakeLink`.

- [ ] **Step 4: Commit**

```bash
git add crates/daemon crates/cli
git commit -m "feat: worktrees are claimed by the workspace whose agent works in them"
```

### Task 14: Shared models — workspaces in the fleet, boards keyed by workspace

**Files:**
- Modify: `crates/client/src/session.rs` (fleet JSON gains `workspaces`; each worktree gains `workspace`, `claim_source`, `foreign_writers`; each terminal gains `workspace`, `role`; `FleetEvent::Task` gains `workspace`; `task.list` takes `workspace`), `crates/client/src/tasks_json.rs`
- Modify: `apps/shared/AgentKit/Sources/AgentKit/CoreModel.swift` (`struct Workspace` — new; `Worktree.workspace`, `Terminal.workspace`, `Terminal.role`)
- Modify: `apps/shared/AgentKit/Sources/AgentKit/TaskBoardModel.swift` (`TaskRow.workspaceID`)
- Create: `apps/shared/AgentKit/Sources/AgentKit/WorkspaceGroups.swift`
- Create: `apps/android/app/src/main/java/com/farcooler/model/WorkspaceGroups.kt`
- Modify: `apps/android/app/src/main/java/com/farcooler/model/Model.kt`, `model/TaskBoard.kt`
- Test: `apps/shared/AgentKit/Tests/AgentKitTests/WorkspaceGroupsTests.swift`, `apps/android/app/src/test/java/com/farcooler/model/WorkspaceGroupsTest.kt`

**Interfaces:**
- Produces (Swift; Kotlin mirrors it):
  ```swift
  public struct WorkspaceGroup: Identifiable, Equatable {
      public let workspace: WorkspaceSummary            // id, name, taskPrefix, isMain
      public let orchestrator: String?                  // terminal id
      public let worktrees: [String]                    // worktree ids, runner order
      public var id: String { workspace.id }
  }
  public struct RepositoryGroups: Equatable {
      public let repository: String                     // repository id
      public let workspaces: [WorkspaceGroup]           // Main first, then ordinal
      public let unclaimed: [String]                    // worktree ids
  }
  public enum WorkspaceGrouping {
      public static func group(repository: String, workspaces: [WorkspaceSummary],
          worktrees: [(id: String, workspace: String?)], orchestrators: [String: String]) -> RepositoryGroups
  }
  ```

- [ ] **Step 1: Write the failing tests**

```swift
import Testing
@testable import AgentKit

struct WorkspaceGroupsTests {
    let main = WorkspaceSummary(id: "m", name: "Main", taskPrefix: "ov", isMain: true, ordinal: 1)
    let billing = WorkspaceSummary(id: "b", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 0)

    @Test func mainComesFirstAndAWorkspaceWithNothingIsStillShown() {
        let g = WorkspaceGrouping.group(repository: "r", workspaces: [billing, main],
            worktrees: [("w1", "m")], orchestrators: [:])
        #expect(g.workspaces.map(\.workspace.id) == ["m", "b"])
        #expect(g.workspaces[1].worktrees.isEmpty)
    }

    @Test func unclaimedWorktreesAreTheirOwnGroup() {
        let g = WorkspaceGrouping.group(repository: "r", workspaces: [main],
            worktrees: [("w1", "m"), ("w2", nil)], orchestrators: [:])
        #expect(g.unclaimed == ["w2"])
        #expect(g.workspaces[0].worktrees == ["w1"])
    }

    @Test func aWorktreeOwnedByAnUnknownWorkspaceIsUnclaimedRatherThanLost() {
        let g = WorkspaceGrouping.group(repository: "r", workspaces: [main],
            worktrees: [("w1", "gone")], orchestrators: [:])
        #expect(g.unclaimed == ["w1"])
    }
}
```

Kotlin: the same three cases in `WorkspaceGroupsTest.kt`. Decode tests: extend AgentKit's fleet decode test and `FleetDecodeTest.kt` with a fixture containing `workspaces` and the new fields.

- [ ] **Step 2: Run, watch fail, implement, run**

```bash
swift test --package-path apps/shared/AgentKit
CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-client
(cd apps/android && JAVA_HOME=/opt/homebrew/opt/openjdk@17 ANDROID_HOME="$HOME/Library/Android/sdk" ./gradlew testInstrumentedUnitTest)
```
Expected: FAIL, then PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/client apps/shared apps/android/app/src/main/java/com/farcooler/model apps/android/app/src/test
git commit -m "feat: the fleet carries workspaces, grouped by one rule on every app"
```

### Task 15: Mac — sidebar grouped by workspace, a board per workspace

**Files:**
- Modify: `apps/macos/Sources/FarCooler/Model.swift` (`Fleet.workspaces: [Workspace]`, `Worktree.workspace: String?`, `Terminal.workspace`, `Terminal.role`)
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift` (`Selection.board(host:repository:)` → `.board(host:workspace:)`; `groups` :597 builds `RepositoryGroups` via `WorkspaceGrouping`; `sidebar` :845 draws a `WorkspaceHeader` per workspace with its `BoardRow`, orchestrator row and worktree rows, then `UnclaimedWorktrees`; `boardStores` keyed `"\(host)/\(workspace.id)"`; `boardStore(for:client:host:)` takes a workspace)
- Modify: `apps/macos/Sources/FarCooler/SidebarViews.swift` (new `WorkspaceHeader`, `UnclaimedWorktrees` modeled on `HiddenWorktrees` :1195; `BoardRow` observes `client.boardGeneration(for: store.workspace.id)`)
- Modify: `apps/macos/Sources/FarCooler/TaskBoard.swift` (`TaskBoardStore.workspace` replaces `.repository`; reads `task list --workspace <id> --json`)
- Modify: `apps/macos/Sources/FarCooler/DaemonClient.swift` (`boardGenerations` keyed by workspace id from `TaskEvent.workspace`; `taskBoard(workspace:)`)
- Modify: `apps/macos/Sources/FarCooler/EventStream.swift` (`TaskEvent.workspace`)
- Test: `apps/macos/Tests/CeremonyTests/BoardSidebarTests.swift`

AgentKit's AgentKit sources are not visible to the Mac target (see `CoreModel.swift`'s header); if `WorkspaceGroups.swift` is not in a module the Mac links, move it to a public AgentKit target the Mac already imports (as `TaskBoardModel` is), or duplicate nothing and make it public there.

- [ ] **Step 1: Write the failing tests** (`BoardSidebarTests.swift`)

`TaskBoardStore(client:repository:)` becomes `TaskBoardStore(client:workspace:)`, `TaskEvent(repository:actor:)` becomes `TaskEvent(repository:workspace:actor:)`, and the `Reads` stub records the `--workspace` argument instead of `--repo`. Existing tests at :112–:465 move over mechanically (`repoA`/`repoB` become two workspace ids **in one repository**, which is the case that matters now). New:

```swift
private static let main = "0198f2c0-0000-7000-8000-00000000000m"
private static let billing = "0198f2c0-0000-7000-8000-00000000000b"

/// Two boards in ONE repository: an event naming Billing re-reads Billing only.
@Test func aBoardEventReReadsOnlyTheWorkspaceItNames() async {
    let reads = Reads()
    let client = client(reads)
    let m = TaskBoardStore(client: client, workspace: Self.summary(Self.main, "Main"))
    let b = TaskBoardStore(client: client, workspace: Self.summary(Self.billing, "Billing"))
    await m.readIfNeverRead()
    await b.readIfNeverRead()
    client.boardMoved(TaskEvent(repository: Self.repoA, workspace: Self.billing, actor: "user"))
    await m.reloadIfMoved()
    await b.reloadIfMoved()
    #expect(reads.count(Self.billing) == 2)
    #expect(reads.count(Self.main) == 1, "Main's board was read again for Billing's change")
}

/// The workspace level is drawn even when Main is the only workspace.
@Test func theSidebarShowsTheWorkspaceLevelEvenWithOnlyMain() {
    let rows = ContentView.sidebarRows(fleet: Self.fleet(
        workspaces: [Self.summary(Self.main, "Main", isMain: true)],
        worktrees: [Self.worktree("lane", workspace: Self.main)]))
    #expect(rows.map(\.kind) == [.repository, .workspace("Main"), .board, .orchestrator, .worktree("lane")])
}

@Test func unclaimedWorktreesSitBelowTheWorkspaces() {
    let rows = ContentView.sidebarRows(fleet: Self.fleet(
        workspaces: [Self.summary(Self.main, "Main", isMain: true)],
        worktrees: [Self.worktree("lane", workspace: Self.main), Self.worktree("stray", workspace: nil)]))
    #expect(rows.map(\.kind).last == .unclaimed(count: 1))
}
```

`ContentView.sidebarRows(fleet:)` is a new static, pure function that `sidebar` renders from — extracting it is what makes the grouping testable without a view. `summary`, `fleet` and `worktree(_:workspace:)` are fixture helpers you add beside `terminal(...)`.

- [ ] **Step 2: Run, watch fail, implement, run**

Run: `PATH="$HOME/.cargo/bin:$PATH" apps/macos/build-app.sh && swift test --package-path apps/macos`
Expected: FAIL, then PASS. Copy: the header shows the workspace name; the orchestrator row reads "Orchestrator" with the harness name as secondary text, or "No orchestrator" dimmed when none; the unclaimed group reads "Unclaimed" with a count, collapsed by default. ⇧⌘B opens the board of the selected worktree's workspace (or Main's when the selection is in Unclaimed).

- [ ] **Step 3: Look at it**

Run the built app against the Canary runner (`open` the built bundle — do not quit the owner's running app; if the bundle ids collide, ask the owner before launching) or against a scratch runner with two workspaces, and screenshot the sidebar. Confirm: repository → Main → Board, orchestrator, worktrees; a second workspace below it; Unclaimed last.

- [ ] **Step 4: Commit**

```bash
git add apps/macos
git commit -m "feat(macos): the sidebar is repository, workspace, then its board and worktrees"
```

### Task 16: iOS and Android — group by workspace

**Files:**
- Modify (iOS): `apps/shared/AgentKit/Sources/AgentKit/ShellRunnerSections.swift`, `ShellNavigation.swift` (within each runner section, sub-sections per repository → workspace via `WorkspaceGrouping`), `RunnerBoards.swift` (`RunnerBoardRow` keyed by workspace id; a row per non-empty workspace board, as today's non-empty rule), `apps/ios/FarCooler/ShellOverview.swift` (`ShellBoardRow` :466 per workspace; accessibility id `shell-board-<runner>-<workspace>`), `apps/ios/FarCooler/TaskBoardView.swift` (`BoardSheet{runner, workspace, name}`), `apps/ios/FarCooler/Connection.swift` (`readBoard(_ workspace:)`, `case "task"` reads `workspace`)
- Modify (Android): `ui/FleetScreen.kt` (`FleetBody` :194 inserts a workspace header before each workspace's entries and an "Unclaimed" group at the end of each repository), `ui/BoardScreen.kt`, `ui/NeedsYouScreen.kt:277`, `ui/Navigation.kt:142` (`Route.Board(hostId, workspaceId)`), `net/Connection.kt:506`
- Test: `apps/shared/AgentKit/Tests/AgentKitTests/RunnerBoardsTests.swift`, `apps/ios/FarCoolerUITests/ShellBoardTests.swift`, `apps/android/app/src/test/java/com/farcooler/net/BoardReadsTest.kt`, `model/TaskBoardTest.kt`

- [ ] **Step 1: Write the failing tests**

AgentKit `RunnerBoardsTests`: `twoWorkspacesInOneRepositoryAreTwoBoardRows` — two workspaces with tasks yield two `RunnerBoardRow`s whose ids are the workspace ids and whose titles are the workspace names. Android `BoardReadsTest`: `aBoardIsReadByWorkspace` — `readBoard` sends `task.list` with `workspace`. iOS UI test `ShellBoardTests`: extend the fixture harness (`ShellHarness.swift:117/473`) with a second workspace and assert both `shell-board-<runner>-<ws>` rows exist.

- [ ] **Step 2: Run, watch fail, implement, run**

```bash
swift test --package-path apps/shared/AgentKit
./scripts/build-ios-frameworks.sh && python3 apps/ios/generate-project.py
./scripts/demo-host.sh   # as the UI tests require
./scripts/ios-ui-tests.sh FarCoolerUITests/ShellBoardTests
./scripts/build-android-libs.sh && (cd apps/android && JAVA_HOME=/opt/homebrew/opt/openjdk@17 ANDROID_HOME="$HOME/Library/Android/sdk" ./gradlew testInstrumentedUnitTest assembleDebug assembleAndroidTest)
```
Expected: FAIL, then PASS. Confirm the iOS UI test actually ran (not skipped) from its output count.

- [ ] **Step 3: Commit**

```bash
git add apps/shared apps/ios apps/android
git commit -m "feat(ios, android): boards and cards are grouped by workspace"
```

### Task 17: The manager skill — splitting a workstream off

**Files:**
- Modify: `crates/daemon/assets/manager/SKILL.md`
- Modify: `crates/daemon/src/skill_install.rs` (tests)
- Modify: `scripts/manager-skill-pressure/` (fake CLI answers `workspace`, `task move`, `worktree assign`, `workspace start-orchestrator`)
- Modify: `crates/cli/src/tasks.rs` (`every_command_the_manager_skill_names_parses` covers the new lines)

- [ ] **Step 1: Write the failing skill test** (`skill_install.rs`)

```rust
#[test]
fn the_skill_teaches_splitting_with_a_handoff_before_the_new_orchestrator() {
    let text = render(Harness::Claude, "farcooler");
    let split = text.find("## Splitting a workstream off").expect("no split section");
    let section = &text[split..];
    let order = ["workspace create", "task move", "worktree assign", "handoff", "workspace start-orchestrator"];
    let at: Vec<usize> = order.iter().map(|s| section.find(s).unwrap_or_else(|| panic!("missing {s}"))).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "the handoff must be written before the new orchestrator starts");
    assert!(text.contains("$FARCOOLER_CHARTER"));
    assert!(!text.contains(".farcooler/manager.md"));
}
```

- [ ] **Step 2: Run, watch fail, write the section, run**

Section text (adapt to the skill's voice and `{{cli}}`):

```markdown
## Splitting a workstream off

Do this when the owner asks, or ask them first when one thread is crowding the
rest of this conversation. The split exists to give that thread its own
conversation, so the handoff matters more than the moves.

1. `{{cli}} workspace create <repo> --name <Name> --prefix <prefix>` — ask the
   owner for both; suggest a prefix of 2–4 letters.
2. Edit the new charter (`{{cli}} --json workspace show <Name>` gives its path;
   it starts as a copy of this one) down to what this workstream needs.
3. `{{cli}} task move <key>… --to <Name>` for its tasks, and
   `{{cli}} worktree assign <worktree> --to <Name>` for the worktrees its
   agents are using.
4. Write the handoff: a `decision` note on each moved task saying why it moved,
   and one `comment` note on the workstream's main task carrying what this
   conversation knows that the board does not — open questions, the owner's
   preferences for this work, what was tried and dropped. A decision that is
   not a note did not happen.
5. `{{cli}} workspace start-orchestrator <Name> --harness <harness>`, then tell
   the owner where to find it and which task holds the handoff.

After the split, this workstream is not yours. Don't dispatch into its
worktrees; if you see its work, tell the owner.
```

Run: `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-daemon skill_install && CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test -p farcooler-cli every_command_the_manager_skill_names_parses`, then the pressure scenario script in `scripts/manager-skill-pressure/` with a split scenario added (follow its README).
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/daemon crates/cli scripts/manager-skill-pressure
git commit -m "feat(skill): the manager splits a workstream off with a handoff"
```

### Task 18: Phase B gate

- [ ] **Step 1:** `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo test --workspace` and `CARGO_BUILD_JOBS=6 ~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`; `swift test --package-path apps/shared/AgentKit`; `swift test --package-path apps/macos`; the iOS build-for-testing command; the Android gradle tasks. Expected: all PASS.
- [ ] **Step 2:** Repeat Task 7 Step 2's Canary-copy check with this branch: every `ov-N` card is on Main; `farcooler workspace list overnight` shows `Main  ov`; `farcooler workspace create overnight --name Scratch --prefix scr`, create and move a task, delete it back; stop the scratch daemon by PID.
- [ ] **Step 3:** Update the spec's status line to "implemented", noting any recipe row Task 11 changed. Rebase onto main (no merge commit). If the session cannot merge, give the owner the squash-and-rebase commands.

```bash
git add docs/superpowers/specs/2026-09-27-workspaces-as-workstreams-design.md
git commit -m "docs: workspaces as workstreams is built"
```
