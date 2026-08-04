# Worktree Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make git the source of truth for which worktrees exist, so registering a repository fills the sidebar with every worktree it already has and the two never drift.

**Architecture:** A reconciler in the daemon diffs `git worktree list --porcelain` against the `workspaces` table on a gated tick inside the existing watcher loop, adopting unknown worktrees and resolving vanished ones. Archive becomes hide. The import sheet and the adopt-main gesture are deleted because reconciliation makes both unnecessary.

**Tech Stack:** Rust (tokio, rusqlite, prost/protobuf), Swift/SwiftUI (macOS app), SQLite.

Spec: `docs/superpowers/specs/2026-08-02-worktree-management-design.md`

## Global Constraints

- **Git is the source of truth for which worktrees exist.** No client ever computes the worktree list; the daemon derives and broadcasts it, exactly as it already does for terminal state.
- **"Empty" means zero rows in `terminals` for that workspace.** That is the whole test for whether a vanished worktree's row may be deleted. There is no second condition.
- **The branch always survives a worktree removal.** No task in this plan may add a `git branch -d`/`-D` call outside the existing `rollback_worktree`.
- **Prunable git records are never adopted.** They point at directories that are already gone.
- **Migrations are forward-only.** Append to `MIGRATIONS` in `crates/store/src/migrate.rs`; never edit an existing migration function.
- **New proto fields are optional; existing tags are never reused for a different meaning.** `WORKSPACE_STATE_ARCHIVED` → `WORKSPACE_STATE_HIDDEN` keeps tag 5 because it is the same state renamed.
- **Rust:** `cargo test --workspace` must pass. **Swift:** `apps/macos/build-app.sh` must build.
- Commit after every task.

---

### Task 1: Schema migration and the `Workspace` row shape

**Files:**
- Modify: `crates/store/src/migrate.rs` (append migration 0006, extend tests)
- Modify: `crates/store/src/models.rs:73-96` (`Workspace`, `row_to_workspace`)
- Modify: `crates/store/src/store.rs:267-359` (create/get/list/update workspace)
- Test: `crates/store/src/migrate.rs` (`mod tests`), `crates/store/src/store.rs` (`mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `models::Workspace` gains `hidden: bool` (replacing `archived: bool`), `is_main_checkout: bool`, `worktree_missing: bool`.
  - `Store::create_workspace(repository_id: Uuid, task_name: &str, branch: &str, worktree_path: &str, is_main_checkout: bool) -> Result<Workspace>` — one new trailing parameter.
  - `Store::set_workspace_flags(id: Uuid, expected_version: u64, hidden: bool, worktree_missing: bool) -> Result<Workspace>` — new, for the two flags the reconciler and hide/unhide touch.
  - `Store::update_workspace` keeps its shape but its `archived` parameter is renamed `hidden`.

- [ ] **Step 1: Write the failing migration test**

Add to `crates/store/src/migrate.rs` `mod tests`:

```rust
    /// A database written before 0006 opens, and its archived rows land hidden.
    ///
    /// Against a hand-built v5 schema rather than a fixture file: the thing
    /// under test is that the rename carries data, and a fixture would only
    /// prove the fixture was written correctly.
    #[test]
    fn archived_rows_become_hidden() {
        let mut conn = open();
        // Everything up to and including 0005, which is where `archived` lived.
        for m in &MIGRATIONS[..5] {
            let tx = conn.transaction().unwrap();
            m(&tx).unwrap();
            tx.commit().unwrap();
        }
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1);
             INSERT INTO workspaces VALUES (x'04', x'03', 'old', 'main', '/r/wt', 1, 0, 1);",
        )
        .unwrap();

        migrate(&mut conn, 5).unwrap();

        let hidden: bool = conn
            .query_row("SELECT hidden FROM workspaces WHERE id = x'04'", [], |r| r.get(0))
            .unwrap();
        assert!(hidden, "an archived workspace is a hidden one");

        let main: bool = conn
            .query_row("SELECT is_main_checkout FROM workspaces WHERE id = x'04'", [], |r| r.get(0))
            .unwrap();
        assert!(!main, "pre-existing rows default to not-main; reconcile corrects them");
    }

    /// One path, one row. The reconciler and `create_workspace` can race, and
    /// the index is what turns that into an error instead of a duplicate.
    #[test]
    fn one_row_per_worktree_path() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1);
             INSERT INTO workspaces VALUES (x'04', x'03', 'a', 'main', '/r/wt', 0, 0, 1, 0, 0);",
        )
        .unwrap();

        let second = conn.execute_batch(
            "INSERT INTO workspaces VALUES (x'05', x'03', 'b', 'main', '/r/wt', 0, 0, 1, 0, 0);",
        );
        assert!(second.is_err(), "a second row for the same path is refused");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p farcooler-store archived_rows_become_hidden one_row_per_worktree_path
```

Expected: FAIL — `no such column: hidden`.

- [ ] **Step 3: Write migration 0006**

In `crates/store/src/migrate.rs`, add to the `MIGRATIONS` array:

```rust
const MIGRATIONS: &[Migration] = &[
    migration_0001_initial_schema,
    migration_0002_pane_groups,
    migration_0003_drop_pane_groups,
    migration_0004_pane_mode,
    migration_0005_drop_loss_dismissed,
    migration_0006_worktrees_are_managed,
];
```

And the function, at the end of the migration functions:

```rust
/// Far Cooler manages worktrees, so the table describes worktrees.
///
/// `archived` becomes `hidden` because there was only ever one concept.
/// Archiving meant "hide it without touching git", which is what hiding means,
/// and having both words for it made users guess which one deleted files.
///
/// `is_main_checkout` replaces a comparison against the task name. The old
/// client test was `task == "main"`, which a linked worktree in a directory
/// called `main` would defeat — and the thing that guarded was whether the UI
/// offers to delete the directory you work in. Now that every worktree is
/// adopted automatically that collision is ordinary rather than exotic.
///
/// `worktree_missing` is stored rather than derived because the reconciler is
/// the only thing that knows. `derive::derive_workspace` runs on every read and
/// has no business shelling out to git.
///
/// The unique index is a backstop, not the mechanism: `Service` serializes per
/// repository so the race cannot normally happen. It exists so that if that
/// lock is ever lost in a refactor, the symptom is an error rather than two
/// sidebar rows for one directory.
fn migration_0006_worktrees_are_managed(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE workspaces RENAME COLUMN archived TO hidden;
        ALTER TABLE workspaces ADD COLUMN is_main_checkout INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE workspaces ADD COLUMN worktree_missing INTEGER NOT NULL DEFAULT 0;
        CREATE UNIQUE INDEX workspaces_one_per_path
            ON workspaces (repository_id, worktree_path);
        "#,
    )
}
```

- [ ] **Step 4: Update the row shape**

In `crates/store/src/models.rs`, replace the `Workspace` struct and `row_to_workspace`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub task_name: String,
    pub branch: String,
    pub worktree_path: String,
    /// The user asked not to see it. Never touches git.
    pub hidden: bool,
    pub creation_failed: bool,
    /// The repository's own checkout, as git reports it. Never removable.
    pub is_main_checkout: bool,
    /// git no longer lists this worktree, but the row carries terminals worth
    /// keeping. Set by the reconciler; cleared if the worktree comes back.
    pub worktree_missing: bool,
    pub resource_version: u64,
}

pub(crate) fn row_to_workspace(row: &Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: get_uuid(row, 0)?,
        repository_id: get_uuid(row, 1)?,
        task_name: row.get(2)?,
        branch: row.get(3)?,
        worktree_path: row.get(4)?,
        hidden: row.get(5)?,
        creation_failed: row.get(6)?,
        resource_version: row.get::<_, i64>(7)? as u64,
        is_main_checkout: row.get(8)?,
        worktree_missing: row.get(9)?,
    })
}
```

- [ ] **Step 5: Update the store's workspace functions**

In `crates/store/src/store.rs`, replace the workspace block (currently lines 267–359). The `SELECT` column list is shared by three functions, so it is named once:

```rust
    // ---- workspaces ----

    /// Every column of `workspaces`, in the order `row_to_workspace` reads them.
    /// Named once because three queries share it and a drifting column order is
    /// a silent field swap rather than a compile error.
    const WORKSPACE_COLUMNS: &'static str = "id, repository_id, task_name, branch, \
         worktree_path, hidden, creation_failed, resource_version, is_main_checkout, \
         worktree_missing";

    pub fn create_workspace(
        &self,
        repository_id: Uuid,
        task_name: &str,
        branch: &str,
        worktree_path: &str,
        is_main_checkout: bool,
    ) -> Result<Workspace> {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                "INSERT INTO workspaces
                 (id, repository_id, task_name, branch, worktree_path, hidden,
                  creation_failed, resource_version, is_main_checkout, worktree_missing)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 1, ?6, 0)",
                params![
                    uuid_blob(id),
                    uuid_blob(repository_id),
                    task_name,
                    branch,
                    worktree_path,
                    is_main_checkout
                ],
            )
            .map_err(map_err)?;
        Ok(Workspace {
            id,
            repository_id,
            task_name: task_name.to_string(),
            branch: branch.to_string(),
            worktree_path: worktree_path.to_string(),
            hidden: false,
            creation_failed: false,
            is_main_checkout,
            worktree_missing: false,
            resource_version: 1,
        })
    }

    pub fn get_workspace(&self, id: Uuid) -> Result<Workspace> {
        self.conn()
            .query_row(
                &format!("SELECT {} FROM workspaces WHERE id = ?1", Self::WORKSPACE_COLUMNS),
                params![uuid_blob(id)],
                row_to_workspace,
            )
            .map_err(map_err)
    }

    pub fn list_workspaces_for_repository(&self, repository_id: Uuid) -> Result<Vec<Workspace>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM workspaces WHERE repository_id = ?1",
                Self::WORKSPACE_COLUMNS
            ))
            .map_err(map_err)?;
        let rows =
            stmt.query_map(params![uuid_blob(repository_id)], row_to_workspace).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_workspace(
        &self,
        id: Uuid,
        expected_version: u64,
        task_name: &str,
        branch: &str,
        worktree_path: &str,
        hidden: bool,
        creation_failed: bool,
    ) -> Result<Workspace> {
        self.run_versioned(
            "UPDATE workspaces
             SET task_name = ?1, branch = ?2, worktree_path = ?3, hidden = ?4, creation_failed = ?5, resource_version = ?6
             WHERE id = ?7 AND resource_version = ?8",
            &[
                &task_name,
                &branch,
                &worktree_path,
                &hidden,
                &creation_failed,
                &(expected_version as i64 + 1),
                &uuid_blob(id),
                &(expected_version as i64),
            ],
            "SELECT 1 FROM workspaces WHERE id = ?1",
            &[&uuid_blob(id)],
        )?;
        self.get_workspace(id)
    }

    /// The two flags that are opinions rather than facts about the work.
    ///
    /// Separate from `update_workspace` because both callers — hide/unhide and
    /// the reconciler — want to change exactly one thing, and passing the other
    /// five fields back unchanged is how a rename gets silently reverted by a
    /// concurrent write.
    pub fn set_workspace_flags(
        &self,
        id: Uuid,
        expected_version: u64,
        hidden: bool,
        worktree_missing: bool,
    ) -> Result<Workspace> {
        self.run_versioned(
            "UPDATE workspaces
             SET hidden = ?1, worktree_missing = ?2, resource_version = ?3
             WHERE id = ?4 AND resource_version = ?5",
            &[
                &hidden,
                &worktree_missing,
                &(expected_version as i64 + 1),
                &uuid_blob(id),
                &(expected_version as i64),
            ],
            "SELECT 1 FROM workspaces WHERE id = ?1",
            &[&uuid_blob(id)],
        )?;
        self.get_workspace(id)
    }

    pub fn delete_workspace(&self, id: Uuid, expected_version: u64) -> Result<()> {
        self.run_versioned(
            "DELETE FROM workspaces WHERE id = ?1 AND resource_version = ?2",
            &[&uuid_blob(id), &(expected_version as i64)],
            "SELECT 1 FROM workspaces WHERE id = ?1",
            &[&uuid_blob(id)],
        )
    }
```

- [ ] **Step 6: Fix every call site the signature change breaks**

```bash
cargo build --workspace 2>&1 | grep -E "^error" -A 5
```

Expected breakages and their fixes:
- `crates/store/src/store.rs` `mod tests` — every `create_workspace(repo.id, "task", "feature/x", "/wt/x")` gains a trailing `false`. Give each test a **distinct** `worktree_path`; the new unique index makes two tests sharing `/wt/x` in one database a failure.
- `crates/store/src/store.rs:712` — `update_workspace(..., true, false)` still compiles; the parameter is only renamed.
- `crates/daemon/src/service.rs` — `create_workspace` calls in `create_workspace`, `import_worktree`, `main_workspace`, `adopt_branch` gain a trailing `false`. `main_workspace` passes `true`. Every `ws.archived` becomes `ws.hidden`.
- `crates/core/src/derive.rs` — the `archived` parameter of `derive_workspace`; leave for Task 2.

Add one store test proving the new flags round-trip:

```rust
    #[test]
    fn workspace_flags_round_trip() {
        let s = fresh();
        let root = s.create_repository_root(Uuid::now_v7(), "/r", 0).unwrap();
        let repo = s.create_repository(Uuid::now_v7(), root.id, "r", "/r/.git", "").unwrap();
        let ws = s.create_workspace(repo.id, "task", "feature/x", "/wt/flags", false).unwrap();
        assert!(!ws.hidden && !ws.worktree_missing);

        let hidden = s.set_workspace_flags(ws.id, ws.resource_version, true, false).unwrap();
        assert!(hidden.hidden, "hidden is set");
        assert_eq!(hidden.task_name, "task", "the name is untouched");

        let missing = s.set_workspace_flags(hidden.id, hidden.resource_version, true, true).unwrap();
        assert!(missing.hidden && missing.worktree_missing);
    }
```

> Match `fresh()` / `create_repository_root` to whatever the existing tests in that file already use — read the top of `mod tests` before writing this.

- [ ] **Step 7: Run the store tests**

```bash
cargo test -p farcooler-store
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/store
git commit -m "feat(store): worktrees are managed, so the schema says so

archived becomes hidden: there was only ever one concept, and two words
for it made people guess which one deleted files. is_main_checkout
replaces comparing the task name against \"main\", which a worktree in a
directory called main would defeat. worktree_missing is stored because
the reconciler is the only thing that can know it.

The unique index on (repository_id, worktree_path) is a backstop for the
per-repository lock, so losing that lock in a future refactor surfaces as
an error rather than two sidebar rows for one directory.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `hidden` and `worktree_missing` in the derived state

**Files:**
- Modify: `proto/farcooler.proto:436-443` (`WorkspaceState`)
- Modify: `crates/core/src/derive.rs:113-137` (`derive_workspace`) and its tests
- Modify: `crates/daemon/src/service.rs:1389` (the `derive_workspace` call)
- Modify: `crates/cli/src/main.rs:2132` (the state → string map)

**Interfaces:**
- Consumes: `models::Workspace.hidden`, `.worktree_missing` from Task 1.
- Produces: `derive_workspace(hidden: bool, worktree_missing: bool, creation_failed: bool, terminals: &[(TerminalRecord, DerivedTerminal)]) -> WorkspaceState`, and the wire strings `"hidden"` and `"worktree_missing"`.

- [ ] **Step 1: Write the failing tests**

In `crates/core/src/derive.rs` `mod tests`:

```rust
    #[test]
    fn hidden_beats_everything_else() {
        let r = record(TerminalIntent::Running, true);
        let s = derive_workspace(true, false, false, &[(r, derived(TerminalState::Running))]);
        assert_eq!(s, WorkspaceState::Hidden, "hiding is the user's decision, not a symptom");
    }

    /// A missing worktree outranks a lost terminal.
    ///
    /// Both are errors, and the terminal is lost BECAUSE the directory went
    /// away. Reporting the symptom would send someone to restart a terminal in
    /// a directory that no longer exists.
    #[test]
    fn a_missing_worktree_outranks_a_lost_terminal() {
        let r = record(TerminalIntent::Running, true);
        let s = derive_workspace(false, true, false, &[(r, derived(TerminalState::Lost))]);
        assert_eq!(s, WorkspaceState::WorktreeMissing);
    }
```

Update the three existing `derive_workspace(false, false, &[...])` calls at lines ~291, ~298, ~305 to `derive_workspace(false, false, false, &[...])`.

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p farcooler-core derive
```

Expected: FAIL — `WorkspaceState::Hidden` does not exist, and arity is wrong.

- [ ] **Step 3: Rename and extend the proto enum**

In `proto/farcooler.proto`, replace the `WorkspaceState` enum:

```proto
enum WorkspaceState {
  WORKSPACE_STATE_UNSPECIFIED = 0;
  WORKSPACE_STATE_CREATING = 1;
  WORKSPACE_STATE_READY = 2;
  WORKSPACE_STATE_ACTIVE = 3;
  WORKSPACE_STATE_ERROR = 4;
  // Tag 5 was ARCHIVED. Same state, one word: archiving never touched git and
  // neither does hiding, so having both meant users guessing which one deleted
  // files. Reusing the tag is correct here and only here — it is a rename, not
  // a new meaning wearing an old number.
  WORKSPACE_STATE_HIDDEN = 5;
  // git no longer lists this worktree, but the row carries terminals worth
  // keeping. A row with nothing in it is deleted instead, never shown.
  WORKSPACE_STATE_WORKTREE_MISSING = 6;
}
```

- [ ] **Step 4: Extend `derive_workspace`**

In `crates/core/src/derive.rs`:

```rust
/// A workspace's state, from the durable facts plus its terminals.
///
/// Ordered by what the user must act on. Hidden first because it is the user's
/// own decision and outranks anything the machine noticed; a missing worktree
/// next because every terminal in it is lost as a consequence, and reporting
/// the consequence would send someone to restart a process in a directory that
/// is not there.
pub fn derive_workspace(
    hidden: bool,
    worktree_missing: bool,
    creation_failed: bool,
    terminals: &[(TerminalRecord, DerivedTerminal)],
) -> WorkspaceState {
    if hidden {
        return WorkspaceState::Hidden;
    }
    if worktree_missing {
        return WorkspaceState::WorktreeMissing;
    }
    if creation_failed {
        return WorkspaceState::Error;
    }

    // A loss is unresolved for exactly as long as the record exists: dismissing
    // one deletes it, and restarting one replaces it. There is no acknowledged
    // -but-still-listed state, because a row that can never say anything again
    // is not evidence, it is clutter.
    if terminals.iter().any(|(_, d)| d.state == TerminalState::Lost) {
        return WorkspaceState::Error;
    }

    let any_live = terminals
        .iter()
        .any(|(_, d)| matches!(d.state, TerminalState::Running | TerminalState::Starting));
    if any_live { WorkspaceState::Active } else { WorkspaceState::Ready }
}
```

- [ ] **Step 5: Update the two callers**

`crates/daemon/src/service.rs`, in `workspace_view`:

```rust
        let state = derive::derive_workspace(
            ws.hidden,
            ws.worktree_missing,
            ws.creation_failed,
            &pairs,
        );
```

`crates/cli/src/main.rs`, in the state → string map around line 2132:

```rust
        WorkspaceState::Hidden => "hidden",
        WorkspaceState::WorktreeMissing => "worktree_missing",
```

- [ ] **Step 6: Run the tests**

```bash
cargo test -p farcooler-core && cargo build --workspace
```

Expected: PASS, and the workspace builds.

- [ ] **Step 7: Commit**

```bash
git add proto crates/core crates/daemon crates/cli
git commit -m "feat(core): derive hidden and worktree-missing workspace states

Hidden outranks everything because it is the user's own decision rather
than something the machine noticed. A missing worktree outranks a lost
terminal because the terminal is lost as a consequence of the directory
going away, and reporting the consequence sends someone to restart a
process in a directory that is not there.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Serialize mutations per repository

**Files:**
- Modify: `crates/daemon/src/service.rs` (`Service` struct, `open_in`, `create_workspace`, `adopt_branch`, `remove_worktree`)
- Modify: `crates/daemon/src/git.rs:1-6` (the module header that claims this already happens)
- Test: `crates/daemon/src/service.rs` (`mod tests`, or a new `#[cfg(test)] mod lock_tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `Service::repo_lock(&self, repository_id: Uuid) -> Arc<tokio::sync::Mutex<()>>`, whose guard must be held across any sequence that runs a git worktree mutation and then writes a workspace row. Task 4's reconciler takes the same lock.

- [ ] **Step 1: Write the failing test**

Add to `crates/daemon/src/service.rs`:

```rust
#[cfg(test)]
mod lock_tests {
    use super::*;

    /// Two calls for the same repository get the same lock; different
    /// repositories do not block each other.
    ///
    /// The identity matters more than it looks: a lock built fresh per call
    /// would compile, pass a casual reading, and serialize nothing at all.
    #[tokio::test]
    async fn one_lock_per_repository() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Service::open_in(dir.path().to_path_buf()).await.unwrap();

        let a = Uuid::now_v7();
        let b = Uuid::now_v7();

        assert!(Arc::ptr_eq(&svc.repo_lock(a), &svc.repo_lock(a)), "same repository, same lock");
        assert!(!Arc::ptr_eq(&svc.repo_lock(a), &svc.repo_lock(b)), "one repository never blocks another");

        let held = svc.repo_lock(a).lock_owned().await;
        assert!(svc.repo_lock(a).try_lock().is_err(), "a held lock excludes a second holder");
        assert!(svc.repo_lock(b).try_lock().is_ok(), "and only that repository");
        drop(held);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p farcooler-daemon one_lock_per_repository
```

Expected: FAIL — no method `repo_lock`.

- [ ] **Step 3: Add the lock registry**

In `crates/daemon/src/service.rs`, add a field to `Service`:

```rust
    /// One mutex per repository, created on first use and never removed.
    ///
    /// Held across any sequence that mutates git and then writes a workspace
    /// row. `create_workspace` runs `git worktree add` and then inserts; the
    /// reconciler lists worktrees and then adopts what has no row. Without
    /// this, a reconcile landing between those two halves sees a worktree with
    /// no row, adopts it under the directory name, and the original call then
    /// inserts a second row for the same path.
    ///
    /// `git.rs` has claimed since it was written that creation "is serialized
    /// per repository". It was not; nothing depended on it until the reconciler
    /// existed.
    ///
    /// Never pruned. A `Mutex<()>` is two words, repositories are counted in
    /// tens, and a registry that removes entries has to prove nobody is waiting
    /// on the one it is removing.
    repo_locks: std::sync::Mutex<std::collections::HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
```

Initialize it in `Service::open_in` alongside the other fields:

```rust
            repo_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
```

And the accessor:

```rust
    /// The lock guarding one repository's git-plus-metadata sequences.
    pub fn repo_lock(&self, repository_id: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        // A std mutex, not a tokio one: this holds only long enough to clone an
        // Arc out of a map, and awaiting to look up a lock would be a lock to
        // reach a lock.
        let mut locks = self.repo_locks.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(locks.entry(repository_id).or_default())
    }
```

- [ ] **Step 4: Take the lock in the three mutating paths**

In `create_workspace`, immediately after `validate::branch_name(branch)?;`:

```rust
        // Held until this function returns: everything below is "mutate git,
        // then write the row", and the reconciler must not see the gap.
        let lock = self.repo_lock(repository_id);
        let _guard = lock.lock().await;
```

Add the identical two lines to `adopt_branch` (after its `validate::branch_name` call).

In `remove_worktree`, after `let repo = self.store.get_repository(ws.repository_id)?;`:

```rust
        let lock = self.repo_lock(repo.id);
        let _guard = lock.lock().await;
```

- [ ] **Step 5: Correct the lie in the git module header**

In `crates/daemon/src/git.rs`, replace lines 1–6:

```rust
//! Git worktree transaction.
//!
//! Never silently reuses an existing branch or worktree path. If metadata fails
//! after git succeeded, only a newly created CLEAN worktree and newly created
//! UNPUSHED branch are removed; otherwise the artifacts are preserved and
//! manual recovery is surfaced.
//!
//! Serialization is NOT this module's job. `Service::repo_lock` holds a mutex
//! across the whole mutate-git-then-write-the-row sequence, because that is the
//! span the reconciler must not observe half of, and nothing at this level can
//! see the metadata half.
```

- [ ] **Step 6: Run the tests**

```bash
cargo test -p farcooler-daemon
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/daemon
git commit -m "fix(daemon): actually serialize worktree mutations per repository

git.rs has claimed since it was written that workspace creation is
serialized per repository. It was not, and nothing depended on it until
the reconciler arrived: create_workspace runs git worktree add and then
inserts, and a reconcile landing between those two halves would adopt the
worktree under its directory name before the insert produced a second row
for the same path.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: The reconciler

**Files:**
- Create: `crates/daemon/src/reconcile.rs`
- Modify: `crates/daemon/src/lib.rs` (declare the module)
- Modify: `crates/daemon/src/service.rs` (`import_worktree` signature; `register_repository` reconciles)
- Test: `crates/daemon/src/reconcile.rs` (`mod tests`, against real temporary repositories)

**Interfaces:**
- Consumes: `Service::repo_lock` (Task 3), `Store::create_workspace(.., is_main_checkout)` and `Store::set_workspace_flags` (Task 1), `git::list_worktrees` (existing).
- Produces:
  - `reconcile::Outcome { pub adopted: usize, pub dropped: usize, pub missing: usize }` — `Outcome::is_quiet(&self) -> bool` is true when all three are zero, so the caller can skip broadcasting.
  - `reconcile::repository(svc: &Service, repository_id: Uuid) -> Result<Outcome>` — takes the repository's lock itself.
  - `reconcile::all(svc: &Service) -> Result<Outcome>` — every registered repository, summed.

- [ ] **Step 1: Write the failing tests**

Create `crates/daemon/src/reconcile.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git;

    /// A service with one registered repository, on a real git repo on disk.
    ///
    /// Real repositories rather than fixtures throughout this file: the shape of
    /// `git worktree list --porcelain` is the thing under test, and a
    /// hand-written fixture would only prove I can copy it.
    async fn fixture() -> (tempfile::TempDir, Arc<Service>, Uuid) {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        for args in [
            vec!["init", "-q", "-b", "main", "."],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
            vec!["commit", "-q", "--allow-empty", "-m", "base"],
        ] {
            git::git(&repo, &args).await.unwrap();
        }

        let svc = Arc::new(Service::open_in(state).await.unwrap());
        svc.add_root(dir.path()).await.unwrap();
        let registered = svc.register_repository(&repo).await.unwrap();
        (dir, svc, registered.id)
    }

    fn names(svc: &Service, repo: Uuid) -> Vec<String> {
        let mut n: Vec<String> = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .map(|w| w.task_name)
            .collect();
        n.sort();
        n
    }

    #[tokio::test]
    async fn registering_a_repository_adopts_its_main_checkout() {
        let (_dir, svc, repo) = fixture().await;
        let all = svc.store.list_workspaces_for_repository(repo).unwrap();
        assert_eq!(all.len(), 1, "the main checkout, and only it: {all:?}");
        assert!(all[0].is_main_checkout, "flagged, not inferred from its name");
    }

    #[tokio::test]
    async fn a_worktree_made_outside_far_cooler_appears() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.adopted, 1);
        assert_eq!(names(&svc, repo), vec!["repo".to_string(), "side".to_string()]);
    }

    /// Adoption is idempotent. A second pass over an unchanged repository must
    /// change nothing, or every tick would add a row.
    #[tokio::test]
    async fn a_second_pass_changes_nothing() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();

        repository(&svc, repo).await.unwrap();
        let second = repository(&svc, repo).await.unwrap();
        assert!(second.is_quiet(), "nothing to say: {second:?}");
        assert_eq!(names(&svc, repo).len(), 2);
    }

    #[tokio::test]
    async fn a_removed_worktree_with_nothing_in_it_disappears() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        let repo_path = dir.path().join("repo");
        git::git(&repo_path, &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        git::git(&repo_path, &["worktree", "remove", "--force", side.to_str().unwrap()])
            .await
            .unwrap();

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.dropped, 1);
        assert_eq!(names(&svc, repo), vec!["repo".to_string()]);
    }

    #[tokio::test]
    async fn a_removed_worktree_holding_terminals_survives_as_missing() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        let repo_path = dir.path().join("repo");
        git::git(&repo_path, &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();
        // A record is enough. The point is that the row carries something worth
        // keeping, not that a process is alive.
        svc.store
            .create_terminal(ws.id, "shell", "shell", TerminalIntent::Stopped, 80, 24)
            .unwrap();

        git::git(&repo_path, &["worktree", "remove", "--force", side.to_str().unwrap()])
            .await
            .unwrap();

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.dropped, 0, "a row with a terminal in it is never deleted for us");
        assert_eq!(out.missing, 1);

        let after = svc.store.get_workspace(ws.id).unwrap();
        assert!(after.worktree_missing);
    }

    /// A worktree that comes back clears the flag rather than staying broken.
    #[tokio::test]
    async fn a_returning_worktree_stops_being_missing() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        let repo_path = dir.path().join("repo");
        git::git(&repo_path, &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();
        svc.store
            .create_terminal(ws.id, "shell", "shell", TerminalIntent::Stopped, 80, 24)
            .unwrap();

        git::git(&repo_path, &["worktree", "remove", "--force", side.to_str().unwrap()])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        git::git(&repo_path, &["worktree", "add", "-q", side.to_str().unwrap(), "feat/side"])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        assert!(!svc.store.get_workspace(ws.id).unwrap().worktree_missing);
    }

    /// Hiding is the user's decision and survives a pass.
    #[tokio::test]
    async fn a_hidden_worktree_stays_hidden() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();
        svc.store.set_workspace_flags(ws.id, ws.resource_version, true, false).unwrap();

        repository(&svc, repo).await.unwrap();
        assert!(svc.store.get_workspace(ws.id).unwrap().hidden);
    }

    /// A prunable record points at a directory that is gone. Adopting one would
    /// create a row for a worktree that does not exist.
    #[tokio::test]
    async fn a_prunable_record_is_not_adopted() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        // The directory removed behind git's back, which is what makes the
        // record prunable rather than absent.
        std::fs::remove_dir_all(&side).unwrap();

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.adopted, 0);
        assert_eq!(names(&svc, repo), vec!["repo".to_string()]);
    }
}
```

> `create_terminal`'s exact signature is whatever `crates/store/src/store.rs:363` declares — read it and match the call. If it needs more arguments, pass the same defaults the daemon's own `create_terminal` uses.
> `svc.store` must be reachable from the test. If the field is private, add `#[cfg(test)] pub(crate)` visibility or a `pub(crate) fn store(&self) -> &Store` accessor rather than making it public.

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p farcooler-daemon reconcile
```

Expected: FAIL — module does not compile, `repository` undefined.

- [ ] **Step 3: Write the reconciler**

At the top of `crates/daemon/src/reconcile.rs`:

```rust
//! Keeping the sidebar and git in agreement about which worktrees exist.
//!
//! Git is the source of truth. The `workspaces` table is a cache keyed by
//! canonical worktree path, plus the things only Far Cooler knows: terminals,
//! layouts, agent sessions, and whether the user hid the row.
//!
//! This exists because Far Cooler only ever knew about work it started itself.
//! Someone arriving with a repository they had used for months spent their
//! first hour re-creating by hand what was already on disk, and a
//! `git worktree add` typed in a terminal never showed up at all.
//!
//! One rule governs deletion: a row whose worktree git no longer lists is
//! removed ONLY when it holds no terminals. Everything else is kept and flagged
//! missing. A `git worktree list` that raced a `mv` must not be able to destroy
//! an agent transcript.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use farcooler_core::Result;
use uuid::Uuid;

use crate::git;
use crate::service::{Service, canonical_or_raw};

/// What one pass changed. All zero means there is nothing to tell clients.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub adopted: usize,
    pub dropped: usize,
    pub missing: usize,
}

impl Outcome {
    /// Nothing changed, so nothing needs broadcasting.
    pub fn is_quiet(&self) -> bool {
        self.adopted == 0 && self.dropped == 0 && self.missing == 0
    }

    fn absorb(&mut self, other: Outcome) {
        self.adopted += other.adopted;
        self.dropped += other.dropped;
        self.missing += other.missing;
    }
}

/// Reconcile one repository against git.
///
/// Takes the repository's lock, so it cannot observe the gap inside
/// `Service::create_workspace` between `git worktree add` and the row insert.
pub async fn repository(svc: &Service, repository_id: Uuid) -> Result<Outcome> {
    let lock = svc.repo_lock(repository_id);
    let _guard = lock.lock().await;

    let repo = svc.store().get_repository(repository_id)?;
    let repo_path = svc.repository_worktree(&repo);

    let found = git::list_worktrees(&repo_path).await?;
    let known = svc.store().list_workspaces_for_repository(repository_id)?;

    let mut outcome = Outcome::default();

    // ---- what git has that we do not ----

    let registered: HashSet<PathBuf> =
        known.iter().map(|w| PathBuf::from(canonical_or_raw(&w.worktree_path))).collect();

    for worktree in &found {
        // A prunable record points at a directory that is gone; git will drop
        // it on the next prune. Adopting one creates a row for nothing.
        if worktree.prunable {
            continue;
        }
        let path = PathBuf::from(canonical_or_raw(&worktree.path));
        if registered.contains(&path) {
            continue;
        }
        if !Path::new(&worktree.path).is_dir() {
            continue;
        }

        let branch = worktree.branch.clone().unwrap_or_else(|| {
            // A detached worktree still has a commit, and that is the honest
            // thing to call it rather than inventing a branch name.
            format!("detached at {}", worktree.head.chars().take(8).collect::<String>())
        });

        let name = Path::new(&worktree.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| branch.clone());

        // A name git handed us, not one a user typed. If the directory is named
        // something the validator refuses, skipping it silently would make a
        // worktree permanently invisible with no way to find out why.
        if let Err(e) = farcooler_core::validate::task_name(&name) {
            tracing::warn!(path = %worktree.path, error = ?e, "worktree name is not usable");
            continue;
        }

        match svc.store().create_workspace(
            repository_id,
            &name,
            &branch,
            &worktree.path,
            worktree.is_main,
        ) {
            Ok(_) => outcome.adopted += 1,
            // The unique index rejecting this means another writer got there
            // first, which is the index doing its job rather than an error
            // worth propagating. Everything else is worth knowing about.
            Err(e) => tracing::warn!(path = %worktree.path, error = ?e, "could not adopt worktree"),
        }
    }

    // ---- what we have that git does not ----

    let live: HashSet<PathBuf> = found
        .iter()
        .filter(|w| !w.prunable)
        .map(|w| PathBuf::from(canonical_or_raw(&w.path)))
        .collect();

    for ws in &known {
        let path = PathBuf::from(canonical_or_raw(&ws.worktree_path));
        let gone = !live.contains(&path) || !Path::new(&ws.worktree_path).is_dir();

        if !gone {
            // Back from the dead: a worktree re-added at the same path clears
            // the flag, rather than leaving a row broken forever because it was
            // once missing for a tick.
            if ws.worktree_missing {
                let _ = svc.store().set_workspace_flags(ws.id, ws.resource_version, ws.hidden, false);
            }
            continue;
        }

        // The whole test. Agent sessions hang off terminals, so a workspace with
        // no terminals has no transcript either.
        let empty = svc.store().list_terminals_for_workspace(ws.id)?.is_empty();

        if empty {
            match svc.store().delete_workspace(ws.id, ws.resource_version) {
                Ok(()) => outcome.dropped += 1,
                Err(e) => tracing::warn!(error = ?e, "could not drop a vanished workspace"),
            }
        } else if !ws.worktree_missing {
            match svc.store().set_workspace_flags(ws.id, ws.resource_version, ws.hidden, true) {
                Ok(_) => outcome.missing += 1,
                Err(e) => tracing::warn!(error = ?e, "could not flag a vanished workspace"),
            }
        }
    }

    Ok(outcome)
}

/// Reconcile every registered repository.
///
/// One repository failing does not stop the rest: a repository whose directory
/// was unmounted must not freeze the sidebar for every other project.
pub async fn all(svc: &Service) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    for repo in svc.list_repositories()? {
        match repository(svc, repo.id).await {
            Ok(one) => outcome.absorb(one),
            Err(e) => tracing::warn!(repository = %repo.id, error = ?e, "reconcile failed"),
        }
    }
    Ok(outcome)
}
```

- [ ] **Step 4: Expose what the reconciler needs**

In `crates/daemon/src/lib.rs`, add alongside the other module declarations:

```rust
pub mod reconcile;
```

In `crates/daemon/src/service.rs`, make three private items reachable:

```rust
    /// The store, for the reconciler. Read and write both: reconciliation IS a
    /// store operation, and routing it through a dozen forwarding methods on
    /// `Service` would add indirection without adding a boundary.
    pub fn store(&self) -> &farcooler_store::Store {
        &self.store
    }
```

Change `fn repository_worktree` to `pub fn repository_worktree`, and `fn canonical_or_raw` to `pub fn canonical_or_raw` (it is a free function near the bottom of the file).

- [ ] **Step 5: Reconcile on registration**

In `crates/daemon/src/service.rs`, `register_repository` currently ends with `self.store.create_repository(...)`. Replace that tail:

```rust
        let repository = self.store.create_repository(
            self.host_id,
            root.id,
            &display_name,
            &git_dir.to_string_lossy(),
            &remote,
        )?;

        // Synchronously, before returning: adding a project should fill the
        // sidebar by the time the sheet closes, not a tick later. A failure
        // here is logged rather than propagated — the repository IS registered,
        // and the next tick reconciles it anyway.
        if let Err(e) = crate::reconcile::repository(self, repository.id).await {
            tracing::warn!(error = ?e, "could not reconcile a freshly registered repository");
        }

        Ok(repository)
```

- [ ] **Step 6: Run the tests**

```bash
cargo test -p farcooler-daemon reconcile
```

Expected: PASS, all eight tests.

- [ ] **Step 7: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): reconcile worktrees against git

Registering a repository now adopts every worktree it has, main checkout
included, and a pass resolves anything that changed since. A worktree made
in a terminal appears; one removed in a terminal disappears if its row
holds nothing, and is flagged missing if it holds terminals.

That last rule is the important one. A git worktree list that raced a mv
must not be able to delete an agent transcript, so emptiness -- zero
terminal rows, which is also zero agent sessions -- is the only license to
remove a record.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Reconcile on a gated tick, and tell clients

**Files:**
- Modify: `crates/daemon/src/watch.rs` (constants, `Watcher` state, `run`)
- Test: `crates/daemon/src/watch.rs` (`mod tests`)

**Interfaces:**
- Consumes: `reconcile::all` and `Outcome::is_quiet` (Task 4).
- Produces: `watch::worktrees_changed(git_common_dir: &Path, since: Option<SystemTime>) -> Option<SystemTime>` — returns the new mtime when `$GIT_COMMON_DIR/worktrees` has moved since `since`, `None` when it has not.

- [ ] **Step 1: Write the failing test**

In `crates/daemon/src/watch.rs` `mod tests`:

```rust
    /// The gate that keeps this off the hot path.
    ///
    /// Both `worktree add` and `worktree remove` create or delete a directory
    /// under `$GIT_COMMON_DIR/worktrees`, which moves its mtime. Scanning every
    /// repository every second would spawn a git process per repository per
    /// second to learn nothing.
    #[test]
    fn the_gate_opens_only_when_the_worktrees_directory_moves() {
        let dir = tempfile::tempdir().unwrap();
        let common = dir.path().join(".git");
        let worktrees = common.join("worktrees");
        std::fs::create_dir_all(&worktrees).unwrap();

        let first = worktrees_changed(&common, None).expect("no baseline means changed");
        assert_eq!(worktrees_changed(&common, Some(first)), None, "unchanged stays shut");

        // Coarse filesystem timestamps: without this the write can land inside
        // the same tick as the read and the mtime genuinely does not move.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::create_dir(worktrees.join("side")).unwrap();

        assert!(worktrees_changed(&common, Some(first)).is_some(), "adding a worktree opens it");
    }

    /// A repository with no linked worktrees has no such directory, and that is
    /// not a change — it is the normal state of most repositories.
    #[test]
    fn a_repository_with_no_linked_worktrees_does_not_thrash_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        let common = dir.path().join(".git");
        std::fs::create_dir_all(&common).unwrap();

        assert_eq!(worktrees_changed(&common, None), None, "nothing there, nothing to scan");
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p farcooler-daemon the_gate_opens_only_when
```

Expected: FAIL — `worktrees_changed` undefined.

- [ ] **Step 3: Write the gate**

In `crates/daemon/src/watch.rs`, near the other free functions:

```rust
/// Whether a repository's worktrees have changed since we last looked.
///
/// `git worktree add` creates a directory under `$GIT_COMMON_DIR/worktrees` and
/// `git worktree remove` deletes one; either moves that directory's mtime. One
/// `stat` per repository per tick is nothing, where one `git worktree list` per
/// repository per tick is a process spawn per repository per second, almost
/// always to learn that nothing happened.
///
/// Returns the new mtime when it moved, `None` when it did not. A repository
/// with no linked worktrees has no such directory and reports no change, which
/// is correct: the backstop pass in `run` still covers anything this misses.
pub fn worktrees_changed(
    git_common_dir: &std::path::Path,
    since: Option<std::time::SystemTime>,
) -> Option<std::time::SystemTime> {
    let modified = std::fs::metadata(git_common_dir.join("worktrees")).ok()?.modified().ok()?;
    match since {
        Some(previous) if previous >= modified => None,
        _ => Some(modified),
    }
}
```

- [ ] **Step 4: Drive it from the loop**

Add near `SAMPLE_INTERVAL` / `BACKSTOP_INTERVAL`:

```rust
/// How often every repository is reconciled regardless of what the gate says.
///
/// The gate watches one directory, and something could change a worktree
/// without touching it — a filesystem with no mtime granularity, a `git
/// worktree repair`, a restore from backup. Thirty seconds is well under how
/// long anyone would stare at a stale sidebar and far above what the scan costs.
const RECONCILE_BACKSTOP: Duration = Duration::from_secs(30);
```

Add a field to `Watcher`:

```rust
    /// Last observed mtime of each repository's `worktrees` directory.
    ///
    /// A std mutex: held only across a map lookup, never across an await.
    worktree_marks: std::sync::Mutex<HashMap<Uuid, std::time::SystemTime>>,
```

Initialize it in `Watcher::new`:

```rust
            worktree_marks: std::sync::Mutex::new(HashMap::new()),
```

Add the pass, as a method on `Watcher`:

```rust
    /// Reconcile repositories whose worktrees moved, or all of them if forced.
    ///
    /// Broadcasts only when something actually changed, for the same reason
    /// `sample` does: a fleet where nothing is happening produces no traffic,
    /// which is what makes a phone holding an SSH session overnight reasonable.
    async fn reconcile_worktrees(&self, force: bool) {
        let Ok(repositories) = self.service.list_repositories() else { return };

        let mut changed = false;
        for repo in repositories {
            let common = std::path::PathBuf::from(&repo.canonical_git_dir);
            let previous = self.worktree_marks.lock().unwrap_or_else(|e| e.into_inner())
                .get(&repo.id)
                .copied();

            let moved = worktrees_changed(&common, previous);
            if moved.is_none() && !force {
                continue;
            }
            if let Some(mark) = moved {
                self.worktree_marks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(repo.id, mark);
            }

            match crate::reconcile::repository(&self.service, repo.id).await {
                Ok(outcome) => changed |= !outcome.is_quiet(),
                Err(e) => tracing::warn!(repository = %repo.id, error = ?e, "reconcile failed"),
            }
        }

        if changed {
            // The same broadcast a terminal change makes. Clients re-read the
            // fleet; nothing here needs its own event type, because "the fleet
            // changed" is exactly what happened.
            self.announce_fleet_changed();
        }
    }
```

> `announce_fleet_changed` is whatever this file already uses to tell clients the fleet moved. Read `sample()` and `announce()` and reuse that path verbatim rather than inventing a second one. If `sample()` only ever announces per-terminal, send the same `Event` payload it uses for a workspace-level change; if there is genuinely no such payload, add one named `FleetChanged` carrying nothing, and have clients treat it as "re-read".

Extend `run`:

```rust
    /// Run until cancelled.
    pub async fn run(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(SAMPLE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut backstop = tokio::time::interval(BACKSTOP_INTERVAL);
        backstop.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut worktrees = tokio::time::interval(RECONCILE_BACKSTOP);
        worktrees.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick of an interval completes immediately, and comparing
        // the inventory against itself before anything has had a chance to
        // diverge would only ever report a false alarm.
        backstop.tick().await;
        worktrees.tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    self.sample().await;
                    // Gated: one stat per repository, and a git process only
                    // for repositories whose worktrees actually moved.
                    self.reconcile_worktrees(false).await;
                }
                _ = backstop.tick() => self.service.backstop_reconcile().await,
                _ = worktrees.tick() => self.reconcile_worktrees(true).await,
            }
        }
    }
```

- [ ] **Step 5: Run the tests**

```bash
cargo test -p farcooler-daemon
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): reconcile worktrees on a gated tick

git worktree add and remove both move the mtime of \$GIT_COMMON_DIR/
worktrees, so one stat per repository per tick decides whether a git
process is worth spawning at all. A forced pass every 30s covers anything
the gate cannot see.

Clients are told only when something changed, so a fleet where nothing is
happening still produces no traffic.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Hide replaces archive, end to end

**Files:**
- Modify: `crates/daemon/src/service.rs` (`archive_workspace`, `restore_workspace`)
- Modify: `crates/daemon/src/rpc.rs:80-81, 388-398` (scope table and dispatch)
- Modify: `crates/cli/src/main.rs:359-362, 1170-1183` (subcommands and handlers)
- Test: `crates/daemon/src/service.rs` or `crates/daemon/tests/rpc_over_socket.rs`

**Interfaces:**
- Consumes: `Store::set_workspace_flags` (Task 1).
- Produces: `Service::hide_workspace(id) -> Result<Workspace>`, `Service::unhide_workspace(id) -> Result<Workspace>`; RPC methods `workspace.hide` / `workspace.unhide`; CLI `farcooler workspace hide|unhide <workspace>`.

- [ ] **Step 1: Write the failing test**

In `crates/daemon/src/service.rs` `mod tests` (or the reconcile fixture pattern if that file has no test module):

```rust
    /// Hiding is a view preference, so a running terminal does not block it.
    ///
    /// Archiving refused, which was right when archiving meant "done with
    /// this". A refusal on a hide reads as a bug, and the risk it guarded --
    /// losing sight of a running agent -- is handled in the sidebar instead,
    /// where the Hidden header carries an attention dot.
    #[tokio::test]
    async fn hiding_a_workspace_with_a_running_terminal_is_allowed() {
        let (_dir, svc, repo) = fixture().await;
        let ws = svc
            .store()
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        svc.store()
            .create_terminal(ws.id, "agent", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();

        let hidden = svc.hide_workspace(ws.id).await.unwrap();
        assert!(hidden.hidden);

        let back = svc.unhide_workspace(hidden.id).await.unwrap();
        assert!(!back.hidden);
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p farcooler-daemon hiding_a_workspace
```

Expected: FAIL — no method `hide_workspace`.

- [ ] **Step 3: Replace the service methods**

In `crates/daemon/src/service.rs`, replace `archive_workspace` and `restore_workspace` entirely:

```rust
    /// Take a workspace out of the main list. Never changes git data.
    ///
    /// Deliberately unconditional. Its predecessor refused while a managed
    /// terminal was running, which fit "archive" — a lifecycle step meaning
    /// done with this — and does not fit hiding, which is a view preference.
    /// A view preference that fails with an error reads as a bug.
    ///
    /// The risk that refusal guarded is real: hide a worktree and its running
    /// agent stops being visible. It is handled where it belongs, in the
    /// sidebar, whose `Hidden (n)` header carries an attention dot when
    /// anything inside it wants the user.
    pub async fn hide_workspace(&self, id: Uuid) -> Result<models::Workspace> {
        let ws = self.store.get_workspace(id)?;
        if ws.hidden {
            return Ok(ws);
        }
        self.store.set_workspace_flags(id, ws.resource_version, true, ws.worktree_missing)
    }

    /// Bring a hidden workspace back into the main list.
    ///
    /// Hiding never touched git, so this never has to reconstruct anything. If
    /// the worktree went away while it was hidden the reconciler has already
    /// said so, and the row comes back carrying that fact rather than pretending
    /// otherwise.
    pub async fn unhide_workspace(&self, id: Uuid) -> Result<models::Workspace> {
        let ws = self.store.get_workspace(id)?;
        if !ws.hidden {
            return Ok(ws);
        }
        self.store.set_workspace_flags(id, ws.resource_version, false, ws.worktree_missing)
    }
```

- [ ] **Step 4: Rename the RPC methods**

In `crates/daemon/src/rpc.rs`, in the scope table replace `| "workspace.archive"` and `| "workspace.restore"` with:

```rust
        | "workspace.hide"
        | "workspace.unhide"
```

And in the dispatch, replace the `workspace.archive` / `workspace.restore` arms:

```rust
            "workspace.hide" => {
                let ws = svc.hide_workspace(Self::target(&req)?).await?;
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }

            "workspace.unhide" => {
                let ws = svc.unhide_workspace(Self::target(&req)?).await?;
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }
```

- [ ] **Step 5: Rename the CLI subcommands**

In `crates/cli/src/main.rs`, in `enum WorkspaceCmd` replace the `Archive` and `Restore` variants:

```rust
    /// Take a worktree out of the main list. Never changes git data.
    Hide { workspace: String },
    /// Bring a hidden worktree back.
    Unhide { workspace: String },
```

And in the handler around line 1170, rename both arms and the method strings they send:

```rust
        WorkspaceCmd::Hide { workspace } => {
            // ...same body as the old Archive arm, with "workspace.archive"
            // replaced by "workspace.hide"
        }
        WorkspaceCmd::Unhide { workspace } => {
            // ...same body as the old Restore arm, with "workspace.restore"
            // replaced by "workspace.unhide"
        }
```

- [ ] **Step 6: Sweep for leftovers**

```bash
grep -rn "archive\|Archive\|restore_workspace\|WorkspaceState::Archived" crates/ proto/ --include=*.rs --include=*.proto
```

Expected: no hits outside `rollback_worktree`'s prose and the migration comment explaining the rename. Fix any that remain.

- [ ] **Step 7: Run the tests**

```bash
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates proto
git commit -m "feat: hide replaces archive

One concept, not two. Archiving meant hide-without-touching-git, which is
what hiding means, and having both words made people guess which one
deleted files.

Hiding no longer refuses while a terminal runs. That refusal fit a
lifecycle step; on a view preference it reads as a bug. The risk it
guarded moves to the sidebar, where the Hidden header carries an attention
dot when something inside wants you.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Delete the import sheet's daemon surface

**Files:**
- Modify: `proto/farcooler.proto:66, 733-740` (`WorktreeImport` payload and message)
- Modify: `crates/daemon/src/rpc.rs:75-132, 354-363, 382-386` (scope table, `workspace.import`, `workspace.main`)
- Modify: `crates/daemon/src/service.rs` (delete `import_worktree`, `main_workspace`, `discover_worktrees`)
- Modify: `crates/cli/src/main.rs:308-354, 886-900, 1053-1100` (`Main`, `Discover`, `Import` subcommands)
- Modify: `crates/daemon/tests/rpc_over_socket.rs`, `crates/client/tests/against_a_real_daemon.rs` (any test exercising the deleted methods)

**Interfaces:**
- Consumes: Task 4's reconciler, which is what makes all of this dead.
- Produces: nothing. This task only removes.

- [ ] **Step 1: Find every reference**

```bash
grep -rn "workspace.import\|workspace.main\|WorktreeImport\|import_worktree\|main_workspace\|discover_worktrees\|worktree.list" \
  crates/ proto/ apps/ --include=*.rs --include=*.proto --include=*.swift
```

Keep `worktree.list` and `discover_worktrees` **only if** the grep shows a caller other than the deleted sheet. The spec keeps `worktree.list` as a host-admin diagnostic; if nothing calls `discover_worktrees` but the RPC, simplify it to return `git::list_worktrees` unfiltered, since "not yet registered" is no longer a meaningful category.

- [ ] **Step 2: Delete the proto surface**

In `proto/farcooler.proto`, remove `WorktreeImport worktree_import = 30;` from the `Request` payload oneof and delete the `WorktreeImport` message (lines ~733-740). Leave a comment where the field was:

```proto
    // 30 was WorktreeImport. Worktrees are adopted by the daemon's reconciler
    // now, so there is nothing for a client to import. The tag is retired
    // rather than reused.
```

- [ ] **Step 3: Delete the RPC arms and scope entries**

Remove `| "workspace.import"` and `| "workspace.main"` from the scope table, and delete the `"workspace.import"` and `"workspace.main"` dispatch arms.

- [ ] **Step 4: Delete the service methods**

Remove `Service::import_worktree` (lines ~416-467) and `Service::main_workspace` (lines ~469-509). The reconciler in `crates/daemon/src/reconcile.rs` now owns both jobs.

- [ ] **Step 5: Delete the CLI subcommands**

Remove `Main`, `Discover`, and `Import` from `enum WorkspaceCmd` and their handler arms.

- [ ] **Step 6: Build and fix the fallout**

```bash
cargo build --workspace 2>&1 | grep -E "^error" -A 5
cargo test --workspace
```

Expected: PASS. Any test that exercised import or main is deleted, not weakened — the behavior it covered is now covered by Task 4's reconciler tests.

- [ ] **Step 7: Commit**

```bash
git add crates proto
git commit -m "refactor: delete import and adopt-main, which reconciliation replaces

Both existed to make by hand what now exists on its own. workspace.import
registered a worktree the daemon can see for itself, and workspace.main
adopted the one checkout that was somehow not a workspace.

Tag 30 in the Request payload is retired rather than reused.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Typed confirmation only when the worktree is dirty

**Files:**
- Modify: `crates/daemon/src/rpc.rs:424-443` (`workspace.remove_worktree`)
- Modify: `crates/daemon/src/service.rs` (`remove_worktree`)
- Test: `crates/daemon/src/reconcile.rs` test fixture reused, or `crates/daemon/tests/rpc_over_socket.rs`

**Interfaces:**
- Consumes: `git::is_dirty` (existing, `crates/daemon/src/git.rs:285`).
- Produces: `Service::removal_needs_confirmation(&self, id: Uuid) -> Result<bool>` — true when the worktree has uncommitted or untracked changes.

- [ ] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn a_clean_worktree_needs_no_typed_confirmation() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        let ws = svc
            .store()
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        assert!(!svc.removal_needs_confirmation(ws.id).await.unwrap());
    }

    /// Uncommitted work is the whole reason the typed confirmation exists.
    #[tokio::test]
    async fn a_dirty_worktree_demands_the_name() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        std::fs::write(side.join("scratch.txt"), "work in progress").unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        let ws = svc
            .store()
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        assert!(svc.removal_needs_confirmation(ws.id).await.unwrap());
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p farcooler-daemon needs_no_typed_confirmation demands_the_name
```

Expected: FAIL — no method `removal_needs_confirmation`.

- [ ] **Step 3: Add the predicate**

In `crates/daemon/src/service.rs`, beside `remove_worktree`:

```rust
    /// Whether removing this worktree should demand its name typed out.
    ///
    /// Only when there is uncommitted or untracked work in it. Everything
    /// committed survives in the branch, which removal never touches, so a
    /// clean worktree is recoverable by re-adding it.
    ///
    /// Demanding the name every time is worse than demanding it sometimes:
    /// people type it without reading it, and then the one gesture meant to
    /// stop a mistake is the mistake's accomplice.
    ///
    /// A worktree whose directory is already gone is not dirty and cannot be
    /// inspected, so it needs no confirmation either — there is nothing left to
    /// lose.
    pub async fn removal_needs_confirmation(&self, id: Uuid) -> Result<bool> {
        let ws = self.store.get_workspace(id)?;
        if !std::path::Path::new(&ws.worktree_path).is_dir() {
            return Ok(false);
        }
        // A worktree we cannot inspect is treated as dirty. Guessing "clean"
        // here would skip the confirmation on exactly the repositories where
        // something is already wrong.
        Ok(git::is_dirty(std::path::Path::new(&ws.worktree_path)).await.unwrap_or(true))
    }
```

- [ ] **Step 4: Apply it in the RPC**

In `crates/daemon/src/rpc.rs`, replace the confirmation check inside `"workspace.remove_worktree"`:

```rust
                // Checked HERE rather than in the client, because a client that
                // skips the dialog must still be refused. Demanded only for a
                // dirty worktree: everything committed lives in the branch,
                // which this never touches.
                if svc.removal_needs_confirmation(id).await? {
                    let Some(request::Payload::TypedConfirmation(p)) = req.payload else {
                        return Err(DomainError::ConfirmationRequired);
                    };
                    if p.typed_confirmation.trim() != ws.task_name {
                        return Err(DomainError::ConfirmationRequired);
                    }
                }
```

Note the payload destructure moves inside the `if`, so the arm must fetch `ws` before it. Reorder the arm so `let ws = svc.list_workspaces()?...` comes first, then this block, then `svc.remove_worktree(id).await?`.

- [ ] **Step 5: Make the main checkout unremovable at the type level**

`remove_worktree` already refuses the main checkout by comparing paths. Replace that comparison with the flag, which is now a fact rather than an inference:

```rust
        // Never the repository's own checkout. The flag comes from git's own
        // worktree list, so this does not depend on a path comparison agreeing
        // with however the repository was registered.
        if ws.is_main_checkout {
            return Err(DomainError::InvalidArgument { what: "the main checkout" });
        }
```

- [ ] **Step 6: Update the CLI's `--confirm`**

In `crates/cli/src/main.rs`, `RemoveWorktree`'s `--confirm` becomes optional:

```rust
    /// Remove the worktree. Keeps the branch and everything committed.
    RemoveWorktree {
        workspace: String,
        /// The workspace's exact name. Required only when the worktree has
        /// uncommitted work in it; the daemon is what decides.
        #[arg(long)]
        confirm: Option<String>,
    },
```

Send `TypedConfirmation` with an empty string when `confirm` is `None`, so the daemon's own check is the one that decides.

- [ ] **Step 7: Run the tests**

```bash
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates
git commit -m "feat(daemon): demand a typed name only for a dirty worktree

Everything committed lives in the branch, which removal never touches, so
a clean worktree is recoverable by re-adding it. Demanding its name every
time trains people to type it without reading it, which spends the one
gesture meant to stop a mistake.

The main checkout is now refused by its flag rather than by comparing
paths, so the refusal does not depend on the path comparison agreeing with
however the repository was registered.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: The sidebar shows every worktree

**Files:**
- Modify: `apps/macos/Sources/FarCooler/Model.swift:31-130, 365-380` (`Workspace`, `WorkspaceState`)
- Modify: `apps/macos/Sources/FarCooler/DaemonClient.swift:544-560` (hide/unhide/remove)
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift:272-381, 449-470` (grouping, sidebar, `+` menu)
- Modify: `apps/macos/Sources/FarCooler/SidebarViews.swift:87, 230, 475, 494, 586` (row menus, state color)
- Delete: `apps/macos/Sources/FarCooler/ImportWorktrees.swift`
- Modify: `apps/macos/generate-project.py` or `apps/macos/build-app.sh` if either lists sources explicitly

**Interfaces:**
- Consumes: the CLI's fleet JSON, which now carries `state: "hidden"` and `state: "worktree_missing"`, plus a new `is_main_checkout` boolean.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Emit `is_main_checkout` in the fleet JSON**

In `crates/cli/src/main.rs`, in the fleet JSON object around line 938, add beside `"short"`:

```rust
                            "is_main_checkout": w.is_main_checkout,
```

> Confirm the local variable name by reading the surrounding block; it is whichever binding holds the workspace.

Verify:

```bash
cargo run -p farcooler-cli -- --json workspace list | head -40
```

Expected: each workspace carries `is_main_checkout`.

- [ ] **Step 2: Update the Swift model**

In `apps/macos/Sources/FarCooler/Model.swift`, replace the `isMainCheckout` computed property (line 59) with a decoded field:

```swift
    /// Whether this workspace IS the repository's own checkout.
    ///
    /// From the daemon, which gets it from `git worktree list`. It used to be
    /// `task == "main"`, which a linked worktree in a directory called `main`
    /// would defeat — and what it guards is whether this app offers to delete
    /// the directory you work in.
    ///
    /// Optional because every field added after the first release is: a client
    /// meeting an older daemon must not fail to decode the entire fleet over
    /// one absent key and show "no workspaces" for a host full of them.
    var isMainCheckout: Bool { is_main_checkout ?? false }
    // swiftlint:disable:next identifier_name
    var is_main_checkout: Bool?

    /// The user asked not to see this one.
    var isHidden: Bool { state == "hidden" }

    /// git no longer lists this worktree, but the row carries terminals.
    var worktreeMissing: Bool { state == "worktree_missing" }
```

> If the file's `CodingKeys` are explicit rather than synthesized, add `case is_main_checkout` there and name the Swift property `isMainCheckoutRaw` instead. Read the struct before writing.

In the `TerminalStatus`-style enum at line 365, replace `archived` with:

```swift
    case running, starting, exited, lost, error, ready, active, hidden, worktreeMissing, unknown
```

and at line 376:

```swift
        case "hidden": return .hidden
        case "worktree_missing": return .worktreeMissing
```

- [ ] **Step 3: Rename the client calls**

In `apps/macos/Sources/FarCooler/DaemonClient.swift`:

```swift
    func hideWorkspace(_ workspace: String) async {
        _ = await run(["workspace", "hide", workspace])
        await refresh()
    }

    func unhideWorkspace(_ workspace: String) async {
        _ = await run(["workspace", "unhide", workspace])
        await refresh()
    }
```

> Match the existing method bodies — if `archiveWorkspace` did not call `refresh()`, do not add it; keep the shape it had.

`removeWorktree(_:confirm:)` keeps its signature, but `confirm` may now be empty. Change the argument construction so an empty string omits `--confirm` entirely:

```swift
    func removeWorktree(_ workspace: String, confirm: String) async {
        var args = ["workspace", "remove-worktree", workspace]
        if !confirm.isEmpty { args += ["--confirm", confirm] }
        _ = await run(args)
        await refresh()
    }
```

- [ ] **Step 4: Split each project's rows into visible and hidden**

In `apps/macos/Sources/FarCooler/ContentView.swift`, replace the `groups` computed property (lines 272-294) so each group carries both lists:

```swift
    /// Worktrees matching the search, grouped by project.
    ///
    /// Hidden ones are separated rather than filtered out: they still belong to
    /// the project, and a collapsed section at the bottom is how you get back to
    /// one. Grouped rather than filtered by host: you work across machines at
    /// once, and a host picker would make a remote agent something to go and
    /// look for — the host is appended to the key only when there is more than
    /// one machine to tell apart.
    private var groups: [(String, [Workspace], [Workspace])] {
        let visible = client.fleet.workspaces.filter { $0.matches(query) }
        let hosts = Set(visible.map { $0.host ?? "" })
        var order: [String] = []
        var byProject: [String: [Workspace]] = [:]

        for workspace in visible {
            let project = (workspace.repository ?? "").isEmpty
                ? "Ungrouped" : workspace.repository!
            let host = workspace.host ?? ""
            let key = hosts.count > 1 && !host.isEmpty ? "\(project) · \(host)" : project
            if byProject[key] == nil { order.append(key) }
            byProject[key, default: []].append(workspace)
        }

        return order.map { key in
            let all = byProject[key] ?? []
            // The main checkout first, then everything else in the order the
            // daemon listed it. The one row you cannot delete is the one row
            // that should not move around.
            let shown = all.filter { !$0.isHidden }
                .sorted { a, b in a.isMainCheckout && !b.isMainCheckout }
            return (key, shown, all.filter(\.isHidden))
        }
    }
```

- [ ] **Step 5: Render the hidden section**

Add state beside the existing `expanded` set:

```swift
    /// Which projects have their hidden worktrees showing. Collapsed is the
    /// point of hiding, so absence means collapsed.
    @State private var hiddenExpanded: Set<String> = []
```

In the sidebar's `ForEach(groups, ...)`, change the destructuring and append the section:

```swift
                        ForEach(groups, id: \.0) { project, workspaces, hidden in
                            ProjectHeader(
                                name: project,
                                count: workspaces.count,
                                onNewWorktree: { newWorktree(in: project) },
                                onNewTerminal: { Task { await newMainTerminal(in: project) } }
                            )
                            ForEach(workspaces) { ws in
                                WorkspaceSection(
                                    workspace: ws,
                                    isExpanded: expanded.contains(ws.id),
                                    selection: $selection,
                                    onToggle: { toggle(ws.id) },
                                    onNewTerminal: { newTerminal(in: ws) },
                                    onHide: { Task { await client.hideWorkspace(ws.short) } },
                                    onUnhide: { Task { await client.unhideWorkspace(ws.short) } },
                                    onRemove: { removeWorkspace = ws },
                                    onTerminalAction: { term, action in
                                        Task { await run(action, on: term) }
                                    },
                                    layouts: client.layouts[ws.id] ?? [],
                                    onMoveToLayout: { term, group in
                                        moveToLayout(term, in: ws, group: group)
                                    },
                                    onDropTogether: { dragged, onto in
                                        placePane(dragged, onto: onto.id, side: .right, in: ws)
                                    },
                                    tiled: Set(client.activeGroup(ws.id)?.terminals ?? [])
                                )
                            }
                            if !hidden.isEmpty {
                                HiddenWorktrees(
                                    project: project,
                                    worktrees: hidden,
                                    isExpanded: hiddenExpanded.contains(project),
                                    onToggle: {
                                        if hiddenExpanded.contains(project) {
                                            hiddenExpanded.remove(project)
                                        } else {
                                            hiddenExpanded.insert(project)
                                        }
                                    },
                                    onUnhide: { ws in
                                        Task { await client.unhideWorkspace(ws.short) }
                                    }
                                )
                            }
                        }
```

- [ ] **Step 6: Write the hidden section view**

In `apps/macos/Sources/FarCooler/SidebarViews.swift`:

```swift
/// The worktrees a project has been told to stop showing.
///
/// A section rather than a filter, because hiding is reversible and something
/// reversible needs a way back that is not the Settings window. Collapsed by
/// default: the whole point of hiding is that these are not in the way.
///
/// The attention dot on the header is what makes hiding safe to allow while an
/// agent runs. The daemon no longer refuses that — a view preference that fails
/// with an error reads as a bug — so this is where "something in here wants you"
/// gets said.
struct HiddenWorktrees: View {
    let project: String
    let worktrees: [Workspace]
    let isExpanded: Bool
    let onToggle: () -> Void
    let onUnhide: (Workspace) -> Void

    private var attention: Int {
        worktrees.flatMap(\.terminals).filter(\.status.wantsAttention).count
    }

    var body: some View {
        SidebarRow(indent: 0) {
            Button(action: onToggle) {
                HStack(spacing: SidebarGrid.gap) {
                    Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.tertiary)
                        .frame(width: SidebarGrid.gutter - SidebarGrid.gap, alignment: .leading)
                    Text("Hidden")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary)
                    Text("\(worktrees.count)")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                    if attention > 0 {
                        Circle()
                            .fill(Color.orange)
                            .frame(width: 5, height: 5)
                            .help("\(attention) waiting on you, inside a hidden worktree")
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .padding(.vertical, 3)

        if isExpanded {
            ForEach(worktrees) { ws in
                SidebarRow(indent: 1) {
                    HStack(spacing: SidebarGrid.gap) {
                        Text(ws.task)
                            .font(.system(size: 12))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                        Text(ws.branch)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                        Spacer(minLength: 0)
                        Button("Unhide") { onUnhide(ws) }
                            .buttonStyle(.plain)
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    }
                    .contentShape(Rectangle())
                }
                .padding(.vertical, 2)
            }
        }
    }
}
```

> `SidebarRow` returns a single view; a `struct`'s `body` cannot be two siblings without a container. Wrap the whole `body` in a `VStack(alignment: .leading, spacing: 0)`.

- [ ] **Step 7: Update the row menus**

In `apps/macos/Sources/FarCooler/SidebarViews.swift`, rename the `onArchive` closure property on both row types (lines 87 and 494) to `onHide`, add `onUnhide`, and replace the `Button("Archive", action: onArchive)` menu items (lines 230 and 586) with:

```swift
                    Button("Hide", action: onHide)
                    // Absent, not disabled, for the main checkout. A daemon-side
                    // refusal is a safety net; the button should not be there to
                    // press.
                    if !workspace.isMainCheckout {
                        Button("Remove Worktree…", action: onRemove)
                    }
```

Remove any existing `onRemove` menu item that was unconditional, so removal appears exactly once.

At line 475, replace the archived color case:

```swift
        case .hidden: return Color.secondary.opacity(0.4)
        case .worktreeMissing: return Color.orange.opacity(0.7)
```

- [ ] **Step 8: Delete the import sheet**

```bash
git rm apps/macos/Sources/FarCooler/ImportWorktrees.swift
```

In `ContentView.swift`, remove the `SidebarMenuItem(title: "Import existing worktrees…") { openImport() }` entry (line 461) and the separator that only existed to set it apart, the `openImport()` function, the `showImport`/`importProject` state, and the `.sheet` presenting `ImportWorktrees`. Remove `ExistingWorktree`/`WorktreeList` references anywhere they survive.

If `apps/macos/generate-project.py` or `build-app.sh` enumerates source files explicitly, remove the entry there too.

- [ ] **Step 9: Build and look at it**

```bash
./apps/macos/build-app.sh
```

Expected: builds clean. Launch it, register a repository that has worktrees, and confirm: every worktree appears without an import step, the main checkout is first and has no Remove item, Hide moves a row into a collapsed `Hidden (n)` section, and Unhide brings it back.

- [ ] **Step 10: Commit**

```bash
git add apps crates/cli
git commit -m "feat(macos): the sidebar shows every worktree

Registering a repository fills the sidebar. The main checkout sits first
and has no Remove item -- absent rather than disabled, because a
daemon-side refusal is a safety net and not a design. Hidden worktrees
move into a collapsed section per project whose header carries an
attention dot, which is what makes hiding safe to allow while an agent is
running.

isMainCheckout comes from the daemon now instead of comparing the task
name against \"main\", which a worktree in a directory called main would
have defeated.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 10: The missing-worktree row, and the removal sheet

**Files:**
- Modify: `apps/macos/Sources/FarCooler/SidebarViews.swift` (`WorkspaceSection` header)
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift:250-270` (the remove confirmation sheet)
- Modify: `apps/macos/Sources/FarCooler/Sheets.swift` (whichever type draws the confirmation)

**Interfaces:**
- Consumes: `Workspace.worktreeMissing` and `Workspace.isMainCheckout` (Task 9).
- Produces: nothing.

- [ ] **Step 1: Render a missing worktree honestly**

In `WorkspaceSection`'s header row, beside the branch label:

```swift
                    if workspace.worktreeMissing {
                        // Said plainly, because every terminal in it is dead and
                        // the reason is not something the user can work out from
                        // a color. The row survives at all because it holds
                        // terminals worth keeping — an empty one is deleted by
                        // the daemon without asking.
                        Text("worktree gone")
                            .font(.system(size: 10))
                            .foregroundStyle(.orange)
                    }
```

And in its context menu, so the row can be got rid of:

```swift
                    if workspace.worktreeMissing {
                        Button("Dismiss", action: onRemove)
                    }
```

> `onRemove` routes to `workspace remove-worktree`, which for a directory that no longer exists deletes the row and needs no confirmation — `removal_needs_confirmation` returns false when the path is not a directory (Task 8).

- [ ] **Step 2: Make the confirmation conditional**

In `ContentView.swift`, the sheet driven by `removeWorkspace` currently always demands the typed name. The client cannot know whether the worktree is dirty without asking, and the daemon is the authority — so the sheet asks for the name only when the daemon refuses without one:

```swift
            .sheet(item: $removeWorkspace) { ws in
                RemoveWorktreeSheet(
                    workspace: ws,
                    onConfirm: { typed in
                        await client.removeWorktree(ws.short, confirm: typed)
                    }
                )
            }
```

In `Sheets.swift`, `RemoveWorktreeSheet` starts with a plain Remove button and no text field. On a `ConfirmationRequired` error from the daemon it reveals the field and the sentence explaining why:

```swift
/// Removing a worktree deletes its directory. The branch survives.
///
/// The name is demanded only when there is uncommitted work in it, and the
/// daemon decides that — the client cannot see the working tree. So this asks
/// once without a field, and reveals one if the daemon says confirmation is
/// required. Demanding it every time trains people to type it without reading
/// it, which spends the one gesture meant to stop a mistake.
struct RemoveWorktreeSheet: View {
    let workspace: Workspace
    let onConfirm: (String) async -> Bool

    @Environment(\.dismiss) private var dismiss
    @State private var typed = ""
    @State private var needsName = false
    @State private var working = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Remove \(workspace.task)?").font(.headline)
            Text("The directory is deleted. The branch \(workspace.branch) and everything committed to it survive.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            if needsName {
                Text("There is uncommitted work here. Type \(workspace.task) to confirm.")
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
                TextField(workspace.task, text: $typed)
                    .textFieldStyle(.roundedBorder)
            }

            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(working ? "Removing…" : "Remove") {
                    Task {
                        working = true
                        let ok = await onConfirm(typed)
                        working = false
                        if ok { dismiss() } else { needsName = true }
                    }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(working || (needsName && typed != workspace.task))
            }
        }
        .padding(18)
        .frame(width: 420)
    }
}
```

`DaemonClient.removeWorktree` must therefore return whether it succeeded:

```swift
    func removeWorktree(_ workspace: String, confirm: String) async -> Bool {
        var args = ["workspace", "remove-worktree", workspace]
        if !confirm.isEmpty { args += ["--confirm", confirm] }
        let result = await run(args)
        await refresh()
        return result.ok
    }
```

> Match `run`'s actual return type — read `DaemonClient.run` and use whatever field reports success.

- [ ] **Step 3: Build and exercise both paths**

```bash
./apps/macos/build-app.sh
```

Then, by hand: remove a clean worktree (one click, no typing), and remove one with an uncommitted file (the field appears, the name is demanded). Confirm the branch still exists afterwards:

```bash
git -C <repo> branch --list
```

- [ ] **Step 4: Commit**

```bash
git add apps
git commit -m "feat(macos): removal asks for a name only when work would be lost

The daemon decides, because the client cannot see the working tree: the
sheet offers a plain Remove, and reveals the field only if the daemon says
confirmation is required. A worktree whose directory is already gone needs
neither.

A row whose worktree vanished says so, and can be dismissed.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 11: Documentation and a full-workspace check

**Files:**
- Modify: `docs/farcooler-design.md` (the worktree/workspace section)
- Modify: `README.md` if it describes importing worktrees
- Modify: `TODOS.md` if it lists any of this

- [ ] **Step 1: Find what the docs still claim**

```bash
grep -rn "import\|archive\|adopt\|worktree" docs/farcooler-design.md README.md TODOS.md | head -40
```

- [ ] **Step 2: Rewrite the model section**

In `docs/farcooler-design.md`, wherever it describes workspaces as created-by-Far-Cooler, replace with the reconciled model. Keep it to a paragraph:

```markdown
Git is the source of truth for which worktrees exist. Registering a repository
adopts every worktree it has, main checkout included, and a reconcile pass keeps
the two in agreement — a `git worktree add` typed in a terminal appears in the
sidebar, and a `git worktree remove` takes its row with it.

A row is deleted on the strength of git's word only when it holds no terminals.
Anything else is kept and marked "worktree gone", because a `git worktree list`
that raced a `mv` must not be able to destroy an agent transcript.

Hiding takes a worktree out of the main list and never touches git. Removing one
deletes its directory and never touches its branch.
```

- [ ] **Step 3: Full check**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && ./apps/macos/build-app.sh
```

Expected: all three pass.

- [ ] **Step 4: Final sweep for dead references**

```bash
grep -rni "archiv\|ImportWorktrees\|workspace.main\|workspace.import\|discover_worktrees" \
  crates/ apps/ proto/ docs/farcooler-design.md README.md
```

Expected: hits only in `migrate.rs`'s explanatory comment and this plan's own history. Fix anything else.

- [ ] **Step 5: Commit**

```bash
git add docs README.md TODOS.md
git commit -m "docs: Far Cooler manages worktrees

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
|---|---|
| Git is the source of truth; table is a cache | 4 |
| Reconciliation: adopt, drop-if-empty, flag-if-not | 4 |
| Prunable records skipped | 4 |
| Cadence: mtime gate + 30s backstop + on registration | 4, 5 |
| The race; per-repository lock; unique index | 1, 3 |
| Main checkout adopted, flagged, unremovable | 4, 8, 9 |
| Schema: rename + two columns + index | 1 |
| `worktree_missing` → new proto state | 2 |
| Hide replaces archive, no running-terminal refusal | 6 |
| Hidden section collapsed per project, attention dot | 9 |
| Remove: discoverable, branch survives, typed only when dirty | 8, 10 |
| Deleted: import sheet, `workspace.import`, `workspace.main` | 7, 9 |
| Tests: reconciler, migration, uniqueness | 1, 4 |
| Docs | 11 |

**Known soft spots, called out rather than hidden:**

- Task 5's `announce_fleet_changed` is named but not defined here, because the existing broadcast path in `watch.rs` has to be read to know which `Event` payload to reuse. The step says so explicitly rather than inventing a second event type.
- Task 9 Step 2's `CodingKeys` handling depends on whether `Workspace`'s are synthesized. The step says to read the struct first and gives both shapes.
- Task 4's `create_terminal` call in tests must match the store's real signature. Flagged in the step.

These are the three places where the plan says "read this first" instead of guessing. Everything else is exact.
