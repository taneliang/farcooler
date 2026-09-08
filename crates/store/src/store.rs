//! `Store`: the single durable connection.
//!
//! Every mutation either fully succeeds and bumps `resource_version`, or fails
//! and leaves the row untouched. There is no method here, and there must never
//! be one, that writes anything resembling "this terminal is running" -- that
//! fact is derived from tmux on every read, never stored. See the crate root
//! docs and `farcooler_core::derive`.

use std::path::Path;

use farcooler_core::{DomainError, Result};
use farcooler_core::derive::TerminalRecord;
use farcooler_core::preconditions::check_idempotency_replay;
use farcooler_protocol::v1::TerminalIntent;
use rusqlite::{Connection, OptionalExtension, ToSql, params};
use uuid::Uuid;

use crate::error::map_err;
use crate::migrate;
use crate::models::{
    IdempotencyRecord, PaneMode, Repository, RepositoryRoot, Terminal, TerminalUpdate, Workspace,
    get_uuid, row_to_repository, row_to_repository_root, row_to_terminal, row_to_workspace,
    uuid_blob,
};

/// Idempotency keys are pruned once older than this, measured against the
/// `now_millis` the caller supplies (the store never reads the wall clock
/// itself, so tests can move time without waiting).
pub const IDEMPOTENCY_RETENTION_MILLIS: i64 = 24 * 60 * 60 * 1000;

pub struct Store {
    /// Behind a mutex so the store is `Sync`.
    ///
    /// rusqlite's `Connection` holds its handle in a `RefCell` and is `Send`
    /// but not `Sync`. The daemon serves connections from a multi-threaded
    /// runtime and one `Service` is shared by all of them, so the store has to
    /// cross threads. A mutex is also the honest model of the thing underneath:
    /// SQLite serializes writes regardless, and every call here is short.
    db: std::sync::Mutex<Connection>,
}

impl Store {
    /// The connection, locked.
    ///
    /// A poisoned lock means a previous caller panicked mid-query. The
    /// connection itself is unaffected, and refusing every later request
    /// because of one panic would turn a single bug into an outage.
    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Open (creating if absent) a database file, migrating it forward if
    /// needed. A pre-existing database that is behind the current schema gets
    /// a checksummed backup written next to it before anything is touched.
    pub fn open(path: impl AsRef<Path>) -> Result<Store> {
        let path = path.as_ref();
        let existed = path.exists();
        let mut conn = Connection::open(path).map_err(map_err)?;
        Self::init(&mut conn, existed.then_some(path))?;
        Ok(Store { db: std::sync::Mutex::new(conn) })
    }

    /// An in-memory database for tests: always fresh, nothing to back up.
    pub fn open_in_memory() -> Result<Store> {
        let mut conn = Connection::open_in_memory().map_err(map_err)?;
        Self::init(&mut conn, None)?;
        Ok(Store { db: std::sync::Mutex::new(conn) })
    }

    fn init(conn: &mut Connection, backup_source: Option<&Path>) -> Result<()> {
        // `recursive_triggers` is not decoration: without it, task_notes's
        // BEFORE DELETE trigger never fires for an `INSERT OR REPLACE` that
        // collides on id, because SQLite treats REPLACE's implicit delete as
        // internal and only runs delete triggers for it when this is on. See
        // `migration_0010_the_board` in migrate.rs for the full story.
        conn.execute_batch(
            "PRAGMA foreign_keys = ON; \
             PRAGMA recursive_triggers = ON; \
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);",
        )
        .map_err(map_err)?;

        let current = migrate::read_schema_version(conn)?;
        if current >= migrate::CURRENT_SCHEMA_VERSION {
            // Already current: migrating again would be pure overhead, and
            // running it is exactly what must stay safe if it does happen.
            return Ok(());
        }

        // A version above zero means real prior schema state worth
        // preserving. A brand-new database has nothing yet that a migration
        // could destroy, so there is nothing to back up.
        if current > 0
            && let Some(path) = backup_source
        {
            crate::backup::write_checksummed_backup(path, current)?;
        }

        migrate::migrate(conn, current)
    }

    /// Runs a versioned mutation whose WHERE clause already encodes the
    /// expected-version predicate. The common case (exactly one row matched)
    /// never pays for the extra lookup; only the failure path distinguishes
    /// "nothing there" (`NotFound`) from "something there but the version
    /// moved" (`ResourceConflict`).
    pub(crate) fn run_versioned(
        &self,
        mutate_sql: &str,
        mutate_params: &[&dyn ToSql],
        exists_sql: &str,
        exists_params: &[&dyn ToSql],
    ) -> Result<()> {
        let affected = self.conn().execute(mutate_sql, mutate_params).map_err(map_err)?;
        if affected == 1 {
            return Ok(());
        }
        let exists: Option<i64> = self
            .conn()
            .query_row(exists_sql, exists_params, |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        if exists.is_some() { Err(DomainError::ResourceConflict) } else { Err(DomainError::NotFound) }
    }

    // ---- repository roots ----

    pub fn create_repository_root(
        &self,
        host_id: Uuid,
        path: &str,
        created_at: i64,
    ) -> Result<RepositoryRoot> {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                "INSERT INTO repository_roots (id, host_id, path, created_at, resource_version)
                 VALUES (?1, ?2, ?3, ?4, 1)",
                params![uuid_blob(id), uuid_blob(host_id), path, created_at],
            )
            .map_err(map_err)?;
        Ok(RepositoryRoot { id, host_id, path: path.to_string(), created_at, resource_version: 1 })
    }

    pub fn get_repository_root(&self, id: Uuid) -> Result<RepositoryRoot> {
        self.conn()
            .query_row(
                "SELECT id, host_id, path, created_at, resource_version
                 FROM repository_roots WHERE id = ?1",
                params![uuid_blob(id)],
                row_to_repository_root,
            )
            .map_err(map_err)
    }

    pub fn list_repository_roots(&self) -> Result<Vec<RepositoryRoot>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, host_id, path, created_at, resource_version
                 FROM repository_roots ORDER BY created_at",
            )
            .map_err(map_err)?;
        let rows = stmt.query_map([], row_to_repository_root).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    pub fn delete_repository_root(&self, id: Uuid, expected_version: u64) -> Result<()> {
        self.run_versioned(
            "DELETE FROM repository_roots WHERE id = ?1 AND resource_version = ?2",
            &[&uuid_blob(id), &(expected_version as i64)],
            "SELECT 1 FROM repository_roots WHERE id = ?1",
            &[&uuid_blob(id)],
        )
    }

    // ---- repositories ----

    pub fn create_repository(
        &self,
        host_id: Uuid,
        repository_root_id: Uuid,
        display_name: &str,
        canonical_git_dir: &str,
        remote_summary: &str,
    ) -> Result<Repository> {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                "INSERT INTO repositories
                 (id, host_id, repository_root_id, display_name, canonical_git_dir, remote_summary, resource_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                params![
                    uuid_blob(id),
                    uuid_blob(host_id),
                    uuid_blob(repository_root_id),
                    display_name,
                    canonical_git_dir,
                    remote_summary,
                ],
            )
            .map_err(map_err)?;
        Ok(Repository {
            id,
            host_id,
            repository_root_id,
            display_name: display_name.to_string(),
            canonical_git_dir: canonical_git_dir.to_string(),
            remote_summary: remote_summary.to_string(),
            resource_version: 1,
            // The schema's own default for a freshly inserted row; assigned
            // later, once, by `Store::assign_task_key_prefix`.
            task_key_prefix: String::new(),
        })
    }

    pub fn get_repository(&self, id: Uuid) -> Result<Repository> {
        self.conn()
            .query_row(
                "SELECT id, host_id, repository_root_id, display_name, canonical_git_dir, remote_summary, resource_version, task_key_prefix
                 FROM repositories WHERE id = ?1",
                params![uuid_blob(id)],
                row_to_repository,
            )
            .map_err(map_err)
    }

    pub fn list_repositories_for_root(&self, repository_root_id: Uuid) -> Result<Vec<Repository>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, host_id, repository_root_id, display_name, canonical_git_dir, remote_summary, resource_version, task_key_prefix
                 FROM repositories WHERE repository_root_id = ?1",
            )
            .map_err(map_err)?;
        let rows =
            stmt.query_map(params![uuid_blob(repository_root_id)], row_to_repository).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    pub fn update_repository(
        &self,
        id: Uuid,
        expected_version: u64,
        display_name: &str,
        canonical_git_dir: &str,
        remote_summary: &str,
    ) -> Result<Repository> {
        self.run_versioned(
            "UPDATE repositories
             SET display_name = ?1, canonical_git_dir = ?2, remote_summary = ?3, resource_version = ?4
             WHERE id = ?5 AND resource_version = ?6",
            &[
                &display_name,
                &canonical_git_dir,
                &remote_summary,
                &(expected_version as i64 + 1),
                &uuid_blob(id),
                &(expected_version as i64),
            ],
            "SELECT 1 FROM repositories WHERE id = ?1",
            &[&uuid_blob(id)],
        )?;
        self.get_repository(id)
    }

    pub fn delete_repository(&self, id: Uuid, expected_version: u64) -> Result<()> {
        self.run_versioned(
            "DELETE FROM repositories WHERE id = ?1 AND resource_version = ?2",
            &[&uuid_blob(id), &(expected_version as i64)],
            "SELECT 1 FROM repositories WHERE id = ?1",
            &[&uuid_blob(id)],
        )
    }

    // ---- workspaces ----

    /// Every column of `workspaces`, in the order `row_to_workspace` reads them.
    /// Named once because three queries share it and a drifting column order is
    /// a silent field swap rather than a compile error.
    const WORKSPACE_COLUMNS: &'static str = "id, repository_id, branch, \
         worktree_path, hidden, creation_failed, resource_version, is_main_checkout, \
         worktree_missing, ordinal";

    /// How every listing of workspaces is ordered, in one place.
    ///
    /// `ordinal` is the user's rank and `worktree_path` is only a tie-break, so
    /// that even a database whose ordinals somehow collided still comes back in
    /// the same order twice running. Nothing here reads activity, attention or
    /// recency, and nothing here ever may: a card that moves on its own is a
    /// card you cannot reach for without looking.
    const WORKSPACE_ORDER: &'static str = "ORDER BY ordinal, worktree_path";

    pub fn create_workspace(
        &self,
        repository_id: Uuid,
        branch: &str,
        worktree_path: &str,
        is_main_checkout: bool,
    ) -> Result<Workspace> {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                // A new workspace goes at the END, which is one more than the
                // highest rank anything currently holds. `MAX` over an empty
                // table is NULL, so the first row on a runner lands at 0.
                //
                // Computed in the INSERT rather than read first and written
                // second: two creates racing through a read-then-write would
                // both see the same maximum and land on the same rank.
                "INSERT INTO workspaces
                 (id, repository_id, branch, worktree_path, hidden,
                  creation_failed, resource_version, is_main_checkout, worktree_missing,
                  ordinal)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 1, ?5, 0,
                         (SELECT COALESCE(MAX(ordinal), -1) + 1 FROM workspaces))",
                params![
                    uuid_blob(id),
                    uuid_blob(repository_id),
                    branch,
                    worktree_path,
                    is_main_checkout
                ],
            )
            .map_err(map_err)?;
        // Read back rather than assembled here: the rank was decided by the
        // INSERT's own subquery, so this is the only place that knows it, and
        // a caller handed a struct claiming ordinal 0 would draw the new card
        // at the top for as long as it held that copy.
        self.get_workspace(id)
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

    /// Every visible workspace on this runner, across repositories.
    ///
    /// For the fleet's own questions, which are not scoped to a project the way
    /// the sidebar's are. Hidden worktrees are left out: the user said not to
    /// see them, and that applies to a summary as much as to a list.
    pub fn list_all_workspaces(&self) -> Result<Vec<Workspace>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                // By the user's rank. This used to order by path, which was
                // stable but was not anybody's decision; `ordinal` starts out
                // as exactly that path order (see migration 0009) and then only
                // ever moves because somebody dragged a card.
                "SELECT {} FROM workspaces WHERE hidden = 0 {}",
                Self::WORKSPACE_COLUMNS,
                Self::WORKSPACE_ORDER
            ))
            .map_err(map_err)?;
        let rows = stmt.query_map([], row_to_workspace).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    pub fn list_workspaces_for_repository(&self, repository_id: Uuid) -> Result<Vec<Workspace>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM workspaces WHERE repository_id = ?1 {}",
                Self::WORKSPACE_COLUMNS,
                Self::WORKSPACE_ORDER
            ))
            .map_err(map_err)?;
        let rows =
            stmt.query_map(params![uuid_blob(repository_id)], row_to_workspace).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Every workspace whose repository is still registered, in fleet order.
    ///
    /// One query rather than a loop over repositories, because a loop is not an
    /// order: `list_repositories` has no `ORDER BY` of its own, so concatenating
    /// each repository's list would have made the fleet's order depend on the
    /// order the repositories happened to come back in. `ordinal` is ranked
    /// across the whole runner precisely so that this can be one statement.
    ///
    /// `EXISTS` rather than a join so the column list stays unqualified —
    /// `workspaces` and `repositories` share `id`, `host_id` and
    /// `resource_version`, and a join would make `WORKSPACE_COLUMNS` ambiguous.
    pub fn list_workspaces_in_order(&self) -> Result<Vec<Workspace>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM workspaces
                 WHERE EXISTS (
                     SELECT 1 FROM repositories WHERE repositories.id = workspaces.repository_id
                 ) {}",
                Self::WORKSPACE_COLUMNS,
                Self::WORKSPACE_ORDER
            ))
            .map_err(map_err)?;
        let rows = stmt.query_map([], row_to_workspace).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Put these workspaces in this order.
    ///
    /// A permutation of the positions the named rows ALREADY hold, not a
    /// renumbering of the list from zero. The caller is a client that dragged a
    /// card, and what it can see is rarely the whole table: hidden worktrees are
    /// filtered out of every sidebar, a grouped view sends one repository's
    /// cards, and a workspace created a second ago is in neither. Renumbering
    /// from zero would move every row the client could not see — silently, and
    /// to the end.
    ///
    /// So: collect the ranks these rows occupy, sort them, and deal them back
    /// out in the order asked for. Rows not named keep the exact rank they had,
    /// which means they keep their position relative to everything, and a
    /// reorder of one group leaves every other group where it was.
    ///
    /// Every id must exist, and no id may appear twice — both are a client
    /// sending nonsense rather than a race, and answering them with a partial
    /// reorder would leave a layout nobody chose.
    pub fn reorder_workspaces(&self, ordered: &[Uuid]) -> Result<()> {
        if ordered.is_empty() {
            return Ok(());
        }
        let unique: std::collections::BTreeSet<_> = ordered.iter().collect();
        if unique.len() != ordered.len() {
            return Err(DomainError::InvalidArgument { what: "workspace_ids" });
        }

        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;

        // (rank, path, version) for each id, in the order the client asked for.
        let mut held = Vec::with_capacity(ordered.len());
        for id in ordered {
            let row: Option<(i64, String, i64)> = tx
                .query_row(
                    "SELECT ordinal, worktree_path, resource_version FROM workspaces WHERE id = ?1",
                    params![uuid_blob(*id)],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(map_err)?;
            held.push(row.ok_or(DomainError::NotFound)?);
        }

        // The slots, in the order a listing would draw them — by rank, then by
        // path, which is `WORKSPACE_ORDER`. Sorting the ranks alone would be
        // enough while they are distinct; taking the tie-break from the same
        // place the query does is what keeps this true if they ever are not.
        let mut slots: Vec<(i64, &str)> =
            held.iter().map(|(ordinal, path, _)| (*ordinal, path.as_str())).collect();
        slots.sort();

        for (i, id) in ordered.iter().enumerate() {
            let (was, _, version) = &held[i];
            let now = slots[i].0;
            if *was == now {
                continue;
            }
            // `resource_version` is bumped because this is a mutation and every
            // mutation here bumps it — a client holding a stale copy of the row
            // is holding a stale POSITION, which is the whole point of the call.
            tx.execute(
                "UPDATE workspaces SET ordinal = ?1, resource_version = ?2 WHERE id = ?3",
                params![now, version + 1, uuid_blob(*id)],
            )
            .map_err(map_err)?;
        }

        tx.commit().map_err(map_err)?;
        Ok(())
    }

    pub fn update_workspace(
        &self,
        id: Uuid,
        expected_version: u64,
        branch: &str,
        worktree_path: &str,
        hidden: bool,
        creation_failed: bool,
    ) -> Result<Workspace> {
        self.run_versioned(
            "UPDATE workspaces
             SET branch = ?1, worktree_path = ?2, hidden = ?3, creation_failed = ?4, resource_version = ?5
             WHERE id = ?6 AND resource_version = ?7",
            &[
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

    /// The columns git owns, rewritten when the reconciler finds the row
    /// disagrees with `git worktree list`.
    ///
    /// A workspace row is a cache of a worktree, and these three are the part
    /// of it git decides: what the worktree is called, what branch is checked
    /// out there, and whether it is the repository's own checkout rather than
    /// a linked one. Nothing else revisits them once the row exists, so
    /// without this the cache is written once and then diverges forever — a
    /// `git checkout` by hand leaves the sidebar naming a stale branch, and
    /// migration 0006's `is_main_checkout DEFAULT 0` leaves every pre-0006
    /// main checkout claiming not to be one.
    ///
    /// Both columns in one statement because they are one fact from one writer,
    /// taken from a single `git worktree list` record: splitting them would mean
    /// two versioned updates, the second racing the version the first just
    /// bumped. `hidden` and `creation_failed` are deliberately absent for the
    /// reason `set_workspace_flags` exists at all — those are the user's
    /// opinions, not git's facts, and passing them back unchanged is how a
    /// concurrent hide gets silently reverted.
    ///
    /// The name used to be healed here too. It no longer exists to heal: a
    /// workspace is named by its worktree directory, read fresh on every access.
    pub fn set_workspace_identity(
        &self,
        id: Uuid,
        expected_version: u64,
        branch: &str,
        is_main_checkout: bool,
    ) -> Result<Workspace> {
        self.run_versioned(
            "UPDATE workspaces
             SET branch = ?1, is_main_checkout = ?2, resource_version = ?3
             WHERE id = ?4 AND resource_version = ?5",
            &[
                &branch,
                &is_main_checkout,
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

    // ---- terminals ----

    #[allow(clippy::too_many_arguments)]
    pub fn create_terminal(
        &self,
        workspace_id: Uuid,
        title: &str,
        command_preset: &str,
        intent: TerminalIntent,
        columns: u32,
        rows: u32,
    ) -> Result<Terminal> {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                r#"INSERT INTO terminals
                 (id, workspace_id, title, command_preset, intent, runtime_confirmed,
                  exit_code, exit_signal, lease_generation, epoch,
                  "columns", "rows", resource_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, NULL, NULL, 0, 0, ?6, ?7, 1)"#,
                params![
                    uuid_blob(id),
                    uuid_blob(workspace_id),
                    title,
                    command_preset,
                    intent as i32,
                    columns,
                    rows,
                ],
            )
            .map_err(map_err)?;
        Ok(Terminal {
            id,
            workspace_id,
            title: title.to_string(),
            command_preset: command_preset.to_string(),
            intent,
            runtime_confirmed: false,
            exit_code: None,
            exit_signal: None,
            lease_generation: 0,
            epoch: 0,
            columns,
            rows,
            resource_version: 1,
            pane_mode: PaneMode::Terminal,
            agent_session_id: None,
        })
    }

    pub fn get_terminal(&self, id: Uuid) -> Result<Terminal> {
        self.conn()
            .query_row(
                r#"SELECT id, workspace_id, title, command_preset, intent, runtime_confirmed,
                          exit_code, exit_signal, lease_generation, epoch,
                          "columns", "rows", resource_version, pane_mode, agent_session_id
                   FROM terminals WHERE id = ?1"#,
                params![uuid_blob(id)],
                row_to_terminal,
            )
            .map_err(map_err)
    }

    pub fn list_terminals_for_workspace(&self, workspace_id: Uuid) -> Result<Vec<Terminal>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                r#"SELECT id, workspace_id, title, command_preset, intent, runtime_confirmed,
                          exit_code, exit_signal, lease_generation, epoch,
                          "columns", "rows", resource_version, pane_mode, agent_session_id
                   FROM terminals WHERE workspace_id = ?1"#,
            )
            .map_err(map_err)?;
        let rows = stmt.query_map(params![uuid_blob(workspace_id)], row_to_terminal).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Every terminal claiming this agent session.
    ///
    /// The join a live agent session arrives on: a hook process knows its own
    /// `session_id`, its worktree and nothing else about Far Cooler, so this is
    /// the only question it can be answered by.
    ///
    /// Keyed on the column, so the caller is handed the claimants rather than
    /// every terminal on the runner to sift for itself. That is about where
    /// the filtering lives and what crosses the boundary, and NOT about the
    /// query plan: `terminals` has no DECLARED index, and none on
    /// `agent_session_id`, so this is a scan — `EXPLAIN QUERY PLAN` says
    /// `SCAN terminals`, which for a fleet of panes is a handful of rows.
    /// (`id BLOB PRIMARY KEY` is not an `INTEGER PRIMARY KEY` rowid alias, so
    /// SQLite does build `sqlite_autoindex_terminals_1` for it; that index
    /// serves `get_terminal` and nothing here.) Nothing about this read should
    /// be described as a lookup by identity, because it is not one.
    ///
    /// A `Vec` rather than an `Option`, because nothing constrains the column
    /// to be unique and two rows really can carry one id — a split pane copies
    /// it, and an adoption can write one somebody else already has. Resolving
    /// that here would be this layer guessing which pane a conversation belongs
    /// to; the caller refuses instead.
    pub fn terminals_with_agent_session(&self, agent_session_id: &str) -> Result<Vec<Terminal>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                r#"SELECT id, workspace_id, title, command_preset, intent, runtime_confirmed,
                          exit_code, exit_signal, lease_generation, epoch,
                          "columns", "rows", resource_version, pane_mode, agent_session_id
                   FROM terminals WHERE agent_session_id = ?1"#,
            )
            .map_err(map_err)?;
        let rows = stmt.query_map(params![agent_session_id], row_to_terminal).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Feeds the daemon's derivation rule directly: durable intent, nothing
    /// tmux would have to have told us first.
    pub fn load_terminal_records(&self, workspace_id: Uuid) -> Result<Vec<TerminalRecord>> {
        Ok(self
            .list_terminals_for_workspace(workspace_id)?
            .into_iter()
            .map(|t| TerminalRecord {
                id: t.id,
                workspace_id: t.workspace_id,
                intent: t.intent,
                runtime_confirmed: t.runtime_confirmed,
                exit_code: t.exit_code,
                exit_signal: t.exit_signal,
            })
            .collect())
    }

    pub fn update_terminal(
        &self,
        id: Uuid,
        expected_version: u64,
        update: TerminalUpdate,
    ) -> Result<Terminal> {
        self.run_versioned(
            r#"UPDATE terminals
               SET title = ?1, command_preset = ?2, intent = ?3, runtime_confirmed = ?4,
                   exit_code = ?5, exit_signal = ?6, lease_generation = ?7,
                   epoch = ?8, "columns" = ?9, "rows" = ?10, resource_version = ?11
               WHERE id = ?12 AND resource_version = ?13"#,
            &[
                &update.title,
                &update.command_preset,
                &(update.intent as i32),
                &update.runtime_confirmed,
                &update.exit_code,
                &update.exit_signal,
                &(update.lease_generation as i64),
                &(update.epoch as i64),
                &update.columns,
                &update.rows,
                &(expected_version as i64 + 1),
                &uuid_blob(id),
                &(expected_version as i64),
            ],
            "SELECT 1 FROM terminals WHERE id = ?1",
            &[&uuid_blob(id)],
        )?;
        self.get_terminal(id)
    }

    pub fn delete_terminal(&self, id: Uuid, expected_version: u64) -> Result<()> {
        self.run_versioned(
            "DELETE FROM terminals WHERE id = ?1 AND resource_version = ?2",
            &[&uuid_blob(id), &(expected_version as i64)],
            "SELECT 1 FROM terminals WHERE id = ?1",
            &[&uuid_blob(id)],
        )
    }

    /// Record which mode this terminal's pane is in, and the session it names.
    ///
    /// Version-checked, which not every mutation here is: two clients toggling
    /// the same pane must not both believe they won.
    ///
    /// A session id is never CLEARED by a mode change. Switching to terminal
    /// mode and back has to land on the same conversation, so `None` means
    /// "leave it alone" rather than "forget it".
    pub fn set_pane_mode(
        &self,
        id: Uuid,
        expected_version: u64,
        pane_mode: PaneMode,
        agent_session_id: Option<String>,
    ) -> Result<Terminal> {
        let changed = self
            .conn()
            .execute(
                r#"UPDATE terminals
                      SET pane_mode = ?1,
                          agent_session_id = COALESCE(?2, agent_session_id),
                          resource_version = resource_version + 1
                    WHERE id = ?3 AND resource_version = ?4"#,
                params![
                    pane_mode.as_i64(),
                    agent_session_id,
                    id.as_bytes().as_slice(),
                    expected_version as i64,
                ],
            )
            .map_err(map_err)?;

        if changed == 0 {
            // Either the terminal is gone or someone else moved it first. Both
            // are the caller's problem to re-read, not ours to paper over.
            return Err(DomainError::ResourceConflict);
        }
        self.get_terminal(id)
    }

    // ---- idempotency ----

    /// Record an idempotency key, or validate a replay against what is
    /// already recorded.
    ///
    /// Returns `Ok(true)` when this exact `(key, client_id)` was already
    /// recorded with the same `request_hash`, so the caller should return the
    /// ORIGINAL result rather than redoing the mutation. Returns `Ok(false)`
    /// for a fresh key, which is now recorded. A key reused with a different
    /// hash is a caller bug, not a race, and is rejected outright.
    pub fn check_idempotency(
        &self,
        key: &str,
        client_id: Uuid,
        request_hash: &str,
        now_millis: i64,
    ) -> Result<bool> {
        let existing: Option<String> = self
            .conn()
            .query_row(
                "SELECT request_hash FROM idempotency WHERE key = ?1 AND client_id = ?2",
                params![key, uuid_blob(client_id)],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?;

        let is_replay = check_idempotency_replay(existing.as_deref(), request_hash)?;
        if !is_replay {
            self.conn()
                .execute(
                    "INSERT INTO idempotency (key, client_id, request_hash, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![key, uuid_blob(client_id), request_hash, now_millis],
                )
                .map_err(map_err)?;
        }
        Ok(is_replay)
    }

    pub fn get_idempotency(&self, key: &str, client_id: Uuid) -> Result<Option<IdempotencyRecord>> {
        self.conn()
            .query_row(
                "SELECT key, client_id, request_hash, created_at FROM idempotency
                 WHERE key = ?1 AND client_id = ?2",
                params![key, uuid_blob(client_id)],
                |r| {
                    Ok(IdempotencyRecord {
                        key: r.get(0)?,
                        client_id: get_uuid(r, 1)?,
                        request_hash: r.get(2)?,
                        created_at: r.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(map_err)
    }

    /// Deletes idempotency rows older than the 24h retention window as of
    /// `now_millis`. Returns the number of rows pruned.
    pub fn prune_idempotency(&self, now_millis: i64) -> Result<usize> {
        let cutoff = now_millis - IDEMPOTENCY_RETENTION_MILLIS;
        self.conn()
            .execute("DELETE FROM idempotency WHERE created_at < ?1", params![cutoff])
            .map_err(map_err)
    }

    /// Column names of a table, for structural assertions like "this table
    /// must never grow a runtime-state column".
    #[cfg(test)]
    pub(crate) fn column_names(&self, table: &str) -> Vec<String> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")).unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use farcooler_protocol::v1::TerminalIntent;

    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    // ---- the one load-bearing rule ----

    #[test]
    fn terminals_table_has_no_runtime_state_column() {
        let s = store();
        let cols = s.column_names("terminals");
        for banned in ["state", "is_running", "running", "pid", "alive"] {
            assert!(
                !cols.iter().any(|c| c.eq_ignore_ascii_case(banned)),
                "terminals table must never carry a `{banned}` column: tmux is the sole \
                 authority for whether a process is alive right now, found columns {cols:?}"
            );
        }
        // And exactly the durable columns the design calls for, nothing more.
        let expected = [
            "id",
            "workspace_id",
            "title",
            "command_preset",
            "intent",
            "runtime_confirmed",
            "exit_code",
            "exit_signal",
            "lease_generation",
            "epoch",
            "columns",
            "rows",
            "resource_version",
            // Both of these are intent, which is why they are allowed to live
            // here. `pane_mode` says what the pane is FOR, exactly as
            // `command_preset` does. `agent_session_id` names a conversation
            // that outlives every pane hosting it, which is what makes
            // toggling pane mode land on the same conversation instead of a
            // new one. The conversation itself is never stored.
            "pane_mode",
            "agent_session_id",
        ];
        assert_eq!(cols.len(), expected.len(), "unexpected column set: {cols:?}");
        for e in expected {
            assert!(cols.iter().any(|c| c == e), "missing expected column {e}, have {cols:?}");
        }
    }

    /// `migrate.rs`'s own append-only tests build a bare connection and turn
    /// `recursive_triggers` on themselves to prove the trigger-plus-pragma
    /// combination works at all; none of them go through `Store::init`, so
    /// none of them actually prove `Store::init` sets that pragma. This one
    /// does, end to end through `Store::open_in_memory`, so a future edit
    /// that drops the pragma from `init` (rather than from the migration)
    /// fails here instead of only in a schema-level test that never runs it.
    #[test]
    fn a_stores_init_actually_closes_the_replace_gap_in_task_notes() {
        let s = store();
        s.conn()
            .execute_batch(
                "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
                 INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, '');
                 INSERT INTO tasks (id, repository_id, key, title, status, status_since, created_at, resource_version)
                     VALUES (x'04', x'03', 'fc-1', 'a task', 'backlog', 0, 0, 1);
                 INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra)
                     VALUES (x'05', x'04', 'decision', 'user', 0, 'because', '{}');",
            )
            .unwrap();

        let err = s
            .conn()
            .execute(
                "INSERT OR REPLACE INTO task_notes (id, task_id, kind, actor, at, body, extra)
                 VALUES (x'05', x'04', 'decision', 'user', 0, 'rewritten, same id', '{}')",
                [],
            )
            .expect_err("Store::init must turn recursive_triggers on, or REPLACE walks through");
        assert!(
            err.to_string().contains("append-only") || err.to_string().contains("trigger"),
            "the schema itself refuses, not a comment asking nicely: {err}"
        );

        let body: String = s
            .conn()
            .query_row("SELECT body FROM task_notes WHERE id = x'05'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(body, "because", "REPLACE keeps the id, so only the body proves nothing moved");
    }

    /// Round 1's `BEFORE DELETE` trigger on `task_notes` had no `WHEN`
    /// clause, and `ON DELETE CASCADE` fires delete triggers unconditionally
    /// — unlike `REPLACE`'s implicit delete, this is never gated by
    /// `recursive_triggers`. So the moment any task on a repository carried
    /// a note, deleting that repository aborted with "task_notes is
    /// append-only" and left every row in place, including the repository
    /// itself. Nothing in this file called the real `Store::delete_repository`
    /// before this test, which is exactly how that shipped: 44/44 green with
    /// the bug live.
    ///
    /// Goes through `Store::delete_repository`, the real function, not raw
    /// SQL, because a schema-level test proving the trigger has the right
    /// `WHEN` clause would not have proven this call site actually reaches
    /// it.
    #[test]
    fn deleting_a_repository_takes_its_whole_board_with_it() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "r", "/repos/one/.git", "").unwrap();
        let task_id = Uuid::now_v7();

        {
            let conn = s.conn();
            conn.execute(
                "INSERT INTO tasks (id, repository_id, key, title, status, status_since, created_at, resource_version)
                 VALUES (?1, ?2, 'fc-1', 'a task', 'backlog', 0, 0, 1)",
                params![uuid_blob(task_id), uuid_blob(repo.id)],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra)
                 VALUES (?1, ?2, 'decision', 'user', 0, 'because', '{}')",
                params![uuid_blob(Uuid::now_v7()), uuid_blob(task_id)],
            )
            .unwrap();
        } // drop the guard: delete_repository locks the same mutex itself.

        s.delete_repository(repo.id, repo.resource_version).unwrap();

        let conn = s.conn();
        let repos: i64 = conn
            .query_row(
                "SELECT count(*) FROM repositories WHERE id = ?1",
                params![uuid_blob(repo.id)],
                |r| r.get(0),
            )
            .unwrap();
        let tasks: i64 = conn
            .query_row(
                "SELECT count(*) FROM tasks WHERE repository_id = ?1",
                params![uuid_blob(repo.id)],
                |r| r.get(0),
            )
            .unwrap();
        let notes: i64 = conn
            .query_row(
                "SELECT count(*) FROM task_notes WHERE task_id = ?1",
                params![uuid_blob(task_id)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            (repos, tasks, notes),
            (0, 0, 0),
            "the repository, its task, and the task's note must all be gone"
        );
    }

    // ---- round trips ----

    #[test]
    fn repository_root_round_trip() {
        let s = store();
        let host = Uuid::now_v7();
        let created = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        assert_eq!(created.resource_version, 1);

        let fetched = s.get_repository_root(created.id).unwrap();
        assert_eq!(fetched, created);

        let listed = s.list_repository_roots().unwrap();
        assert_eq!(listed, vec![created]);
    }

    #[test]
    fn repository_round_trip_and_update() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/repos/one/.git", "origin").unwrap();
        assert_eq!(s.get_repository(repo.id).unwrap(), repo);

        let updated = s.update_repository(repo.id, 1, "renamed", "/repos/one/.git", "origin2").unwrap();
        assert_eq!(updated.display_name, "renamed");
        assert_eq!(updated.resource_version, 2);
        assert_eq!(s.get_repository(repo.id).unwrap(), updated);

        let listed = s.list_repositories_for_root(root.id).unwrap();
        assert_eq!(listed, vec![updated]);
    }

    #[test]
    fn workspace_round_trip_and_update() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_workspace(repo.id, "feature/x", "/wt/workspace", false).unwrap();
        assert!(!ws.hidden);
        assert_eq!(s.get_workspace(ws.id).unwrap(), ws);

        let updated =
            s.update_workspace(ws.id, 1, "feature/x", "/wt/workspace", true, false).unwrap();
        assert!(updated.hidden);
        assert_eq!(updated.resource_version, 2);
    }

    // ---- ordering ----
    //
    // The rule these guard is the whole reason `ordinal` exists: a card's
    // position is decided at creation and then only by the user. Nothing here
    // may ever come to depend on activity, attention, or when a pane last said
    // something — a list that rearranges itself is a list you cannot reach into
    // without reading it first.

    /// Three repositories, one runner, and everything the store hands back is
    /// in the order the rows were created.
    fn ordered_names(s: &Store, repo: Uuid) -> Vec<String> {
        s.list_workspaces_for_repository(repo).unwrap().iter().map(|w| w.name()).collect()
    }

    #[test]
    fn a_new_workspace_lands_at_the_end() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();

        // Created out of alphabetical order on purpose: if anything fell back
        // to sorting by name or path, this would come back sorted.
        let zebra = s.create_workspace(repo.id, "feat/z", "/wt/zebra", false).unwrap();
        let apple = s.create_workspace(repo.id, "feat/a", "/wt/apple", false).unwrap();
        let mango = s.create_workspace(repo.id, "feat/m", "/wt/mango", false).unwrap();

        assert_eq!((zebra.ordinal, apple.ordinal, mango.ordinal), (0, 1, 2));
        assert_eq!(ordered_names(&s, repo.id), vec!["zebra", "apple", "mango"]);
    }

    /// The rank is across the runner, not restarted per repository, so two
    /// projects' cards never collide on one number.
    #[test]
    fn the_rank_counts_across_every_repository_on_the_runner() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos", 1_000).unwrap();
        let one = s.create_repository(host, root.id, "one", "/one/.git", "").unwrap();
        let two = s.create_repository(host, root.id, "two", "/two/.git", "").unwrap();

        let a = s.create_workspace(one.id, "b", "/wt/a", false).unwrap();
        let b = s.create_workspace(two.id, "b", "/wt/b", false).unwrap();
        let c = s.create_workspace(one.id, "b", "/wt/c", false).unwrap();

        assert_eq!((a.ordinal, b.ordinal, c.ordinal), (0, 1, 2));
        assert_eq!(
            s.list_workspaces_in_order().unwrap().iter().map(|w| w.name()).collect::<Vec<_>>(),
            vec!["a", "b", "c"],
            "one list across the runner, in one order"
        );
    }

    /// Reordering is a permutation of the slots the named rows already hold.
    #[test]
    fn reordering_moves_the_cards_and_survives_a_reopen() {
        let dir = std::env::temp_dir().join(format!("farcooler-order-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.db");

        let (repo, a, b, c) = {
            let s = Store::open(&path).unwrap();
            let host = Uuid::now_v7();
            let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
            let repo = s.create_repository(host, root.id, "name", "/gitdir", "").unwrap();
            let a = s.create_workspace(repo.id, "b", "/wt/a", false).unwrap();
            let b = s.create_workspace(repo.id, "b", "/wt/b", false).unwrap();
            let c = s.create_workspace(repo.id, "b", "/wt/c", false).unwrap();
            assert_eq!(ordered_names(&s, repo.id), vec!["a", "b", "c"]);

            s.reorder_workspaces(&[c.id, a.id, b.id]).unwrap();
            assert_eq!(ordered_names(&s, repo.id), vec!["c", "a", "b"]);
            (repo.id, a.id, b.id, c.id)
        };

        // A daemon restart is a reopen of this file. An order held in memory
        // would be gone here, and an order derived from the work would be
        // whatever the work looked like now.
        let s = Store::open(&path).unwrap();
        assert_eq!(ordered_names(&s, repo), vec!["c", "a", "b"]);

        // And the version moved, so a client holding the old row knows its copy
        // of the position is stale.
        assert!(s.get_workspace(a).unwrap().resource_version > 1);

        // Asking for the order it is already in changes nothing.
        s.reorder_workspaces(&[c, a, b]).unwrap();
        assert_eq!(ordered_names(&s, repo), vec!["c", "a", "b"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A client sends the cards it can SEE. Everything else must stay exactly
    /// where it was — this is what stops a reorder in one project's group from
    /// throwing another project's cards, or every hidden worktree, to the end.
    #[test]
    fn reordering_a_subset_leaves_every_other_card_where_it_was() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos", 1_000).unwrap();
        let one = s.create_repository(host, root.id, "one", "/one/.git", "").unwrap();
        let two = s.create_repository(host, root.id, "two", "/two/.git", "").unwrap();

        // The group that will be dragged sits at ranks 2 and 3, NOT at 0 and 1.
        // That is the whole point of the fixture: a reorder that renumbered the
        // cards it was handed from zero would land them on top of `x` and `y`
        // and pass every assertion about relative order inside one group.
        let x = s.create_workspace(two.id, "b", "/wt/x", false).unwrap();
        let y = s.create_workspace(two.id, "b", "/wt/y", false).unwrap();
        let a = s.create_workspace(one.id, "b", "/wt/a", false).unwrap();
        let c = s.create_workspace(one.id, "b", "/wt/c", false).unwrap();
        assert_eq!((x.ordinal, y.ordinal, a.ordinal, c.ordinal), (0, 1, 2, 3));

        s.reorder_workspaces(&[c.id, a.id]).unwrap();

        assert_eq!(ordered_names(&s, one.id), vec!["c", "a"], "the group that moved");
        assert_eq!(ordered_names(&s, two.id), vec!["x", "y"], "the group that did not");
        assert_eq!(
            (s.get_workspace(c.id).unwrap().ordinal, s.get_workspace(a.id).unwrap().ordinal),
            (2, 3),
            "the two cards swapped the slots they held; they did not move to the front"
        );
        assert_eq!(
            (s.get_workspace(x.id).unwrap().ordinal, s.get_workspace(y.id).unwrap().ordinal),
            (0, 1),
            "an untouched card keeps its exact rank, not merely its relative one"
        );
        assert_eq!(
            s.list_workspaces_in_order().unwrap().iter().map(|w| w.name()).collect::<Vec<_>>(),
            vec!["x", "y", "c", "a"],
            "and the whole runner's list is what a client would draw"
        );
    }

    /// Nonsense is refused whole rather than applied halfway, because half a
    /// reorder is a layout nobody chose.
    #[test]
    fn a_reorder_naming_the_same_card_twice_or_a_stranger_is_refused() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "").unwrap();
        let a = s.create_workspace(repo.id, "b", "/wt/a", false).unwrap();
        let b = s.create_workspace(repo.id, "b", "/wt/b", false).unwrap();

        assert!(matches!(
            s.reorder_workspaces(&[a.id, a.id]),
            Err(DomainError::InvalidArgument { .. })
        ));
        assert!(matches!(
            s.reorder_workspaces(&[b.id, a.id, Uuid::now_v7()]),
            Err(DomainError::NotFound)
        ));
        assert_eq!(ordered_names(&s, repo.id), vec!["a", "b"], "nothing moved");
    }

    #[test]
    fn workspace_flags_round_trip() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_workspace(repo.id, "feature/x", "/wt/flags", false).unwrap();
        assert!(!ws.hidden && !ws.worktree_missing);

        let hidden = s.set_workspace_flags(ws.id, ws.resource_version, true, false).unwrap();
        assert!(hidden.hidden, "hidden is set");
        assert_eq!(hidden.name(), "flags", "the name is the worktree, and hiding moves nothing");

        let missing = s.set_workspace_flags(hidden.id, hidden.resource_version, true, true).unwrap();
        assert!(missing.hidden && missing.worktree_missing);
    }

    #[test]
    fn terminal_round_trip_and_update() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_workspace(repo.id, "feature/x", "/wt/terminal", false).unwrap();
        let term =
            s.create_terminal(ws.id, "shell", "claude", TerminalIntent::Running, 80, 24).unwrap();
        assert!(!term.runtime_confirmed);
        assert_eq!(s.get_terminal(term.id).unwrap(), term);

        let update = TerminalUpdate {
            title: "shell".into(),
            command_preset: "claude".into(),
            intent: TerminalIntent::Running,
            runtime_confirmed: true,
            exit_code: None,
            exit_signal: None,
            lease_generation: 1,
            epoch: 1,
            columns: 100,
            rows: 30,
        };
        let updated = s.update_terminal(term.id, 1, update).unwrap();
        assert!(updated.runtime_confirmed);
        assert_eq!(updated.columns, 100);
        assert_eq!(updated.resource_version, 2);

        let records = s.load_terminal_records(ws.id).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, term.id);
        assert_eq!(records[0].intent, TerminalIntent::Running);
        assert!(records[0].runtime_confirmed);
    }

    // ---- pane mode ----

    #[test]
    fn a_new_terminal_starts_in_terminal_pane_mode() {
        // Terminal-first is the product's default, and defaults belong in the
        // schema rather than in whichever caller remembered.
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_workspace(repo.id, "feature/x", "/wt/pane-mode", false).unwrap();
        let t = s.create_terminal(ws.id, "t", "claude", TerminalIntent::Running, 120, 40).unwrap();
        assert_eq!(t.pane_mode, PaneMode::Terminal);
        assert_eq!(t.agent_session_id, None);
    }

    #[test]
    fn a_session_id_survives_a_reopen_because_it_is_intent_not_runtime() {
        // The one thing about an agent session that must outlive tmux. The
        // conversation itself is never stored.
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_workspace(repo.id, "feature/x", "/wt/session-id", false).unwrap();
        let t = s.create_terminal(ws.id, "t", "claude", TerminalIntent::Running, 120, 40).unwrap();
        let updated =
            s.set_pane_mode(t.id, t.resource_version, PaneMode::Agent, Some("abc-123".into())).unwrap();
        assert_eq!(updated.pane_mode, PaneMode::Agent);

        let reopened = s.get_terminal(t.id).unwrap();
        assert_eq!(reopened.agent_session_id.as_deref(), Some("abc-123"));
    }

    /// The join a live agent session arrives on.
    ///
    /// A hook knows its own `session_id` and nothing else about Far Cooler, so
    /// this is the only question it can be answered by. Two rows may hold the
    /// same id — nothing constrains the column — so every claimant comes back
    /// and the caller decides; picking one here would be this layer guessing
    /// which pane somebody's conversation belongs to.
    #[test]
    fn a_session_id_finds_every_terminal_that_claims_it_and_no_others() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_workspace(repo.id, "feature/x", "/wt/sessions", false).unwrap();

        let mine = s.create_terminal(ws.id, "a", "claude", TerminalIntent::Running, 80, 24).unwrap();
        let mine =
            s.set_pane_mode(mine.id, mine.resource_version, PaneMode::Terminal, Some("s-1".into()))
                .unwrap();
        let other = s.create_terminal(ws.id, "b", "claude", TerminalIntent::Running, 80, 24).unwrap();
        s.set_pane_mode(other.id, other.resource_version, PaneMode::Terminal, Some("s-2".into()))
            .unwrap();
        // A terminal that has declared nothing must never be swept up by a
        // lookup for a session, which is what `agent_session_id = NULL`
        // matching a bound parameter would do under the wrong comparison.
        s.create_terminal(ws.id, "c", "shell", TerminalIntent::Running, 80, 24).unwrap();

        let found = s.terminals_with_agent_session("s-1").unwrap();
        assert_eq!(found.len(), 1, "one claimant, and not the other two rows");
        assert_eq!(found[0].id, mine.id);
        assert_eq!(found[0].agent_session_id.as_deref(), Some("s-1"), "the whole row comes back");

        assert!(
            s.terminals_with_agent_session("s-3").unwrap().is_empty(),
            "a session nobody declared has no claimant"
        );
    }

    /// Two rows on one id is the case the caller has to be able to see.
    #[test]
    fn two_terminals_claiming_one_session_both_come_back() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_workspace(repo.id, "feature/x", "/wt/ambiguous", false).unwrap();

        for title in ["a", "b"] {
            let t =
                s.create_terminal(ws.id, title, "claude", TerminalIntent::Running, 80, 24).unwrap();
            s.set_pane_mode(t.id, t.resource_version, PaneMode::Terminal, Some("shared".into()))
                .unwrap();
        }

        assert_eq!(
            s.terminals_with_agent_session("shared").unwrap().len(),
            2,
            "the ambiguity reaches the caller rather than being resolved here"
        );
    }

    // ---- optimistic concurrency ----

    #[test]
    fn stale_version_is_a_conflict_not_silently_applied() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();

        // First writer succeeds and moves the version to 2.
        s.update_repository(repo.id, 1, "first", "/gitdir", "origin").unwrap();

        // Second writer still thinks the version is 1: must be rejected, and
        // must not have applied its write.
        let err = s.update_repository(repo.id, 1, "second", "/gitdir", "origin").unwrap_err();
        assert!(matches!(err, DomainError::ResourceConflict));
        assert_eq!(s.get_repository(repo.id).unwrap().display_name, "first");
    }

    #[test]
    fn updating_a_missing_resource_is_not_found_not_conflict() {
        let s = store();
        let err = s.update_repository(Uuid::now_v7(), 1, "x", "y", "z").unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
    }

    #[test]
    fn delete_also_honors_expected_version() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();

        let err = s.delete_repository_root(root.id, 2).unwrap_err();
        assert!(matches!(err, DomainError::ResourceConflict));

        s.delete_repository_root(root.id, 1).unwrap();
        assert!(matches!(s.get_repository_root(root.id).unwrap_err(), DomainError::NotFound));
    }

    // ---- idempotency ----

    #[test]
    fn fresh_idempotency_key_is_recorded_and_not_a_replay() {
        let s = store();
        let client = Uuid::now_v7();
        let is_replay = s.check_idempotency("op-1", client, "hash-a", 1_000).unwrap();
        assert!(!is_replay);
        assert_eq!(s.get_idempotency("op-1", client).unwrap().unwrap().request_hash, "hash-a");
    }

    #[test]
    fn same_hash_replay_is_reported_and_not_double_recorded() {
        let s = store();
        let client = Uuid::now_v7();
        assert!(!s.check_idempotency("op-1", client, "hash-a", 1_000).unwrap());
        assert!(s.check_idempotency("op-1", client, "hash-a", 2_000).unwrap(), "same hash replays");
        // Still exactly one record, with the original timestamp.
        let rec = s.get_idempotency("op-1", client).unwrap().unwrap();
        assert_eq!(rec.created_at, 1_000);
    }

    #[test]
    fn different_hash_same_key_is_rejected() {
        let s = store();
        let client = Uuid::now_v7();
        assert!(!s.check_idempotency("op-1", client, "hash-a", 1_000).unwrap());
        let err = s.check_idempotency("op-1", client, "hash-b", 2_000).unwrap_err();
        assert!(matches!(err, DomainError::IdempotencyMismatch));
    }

    #[test]
    fn different_clients_do_not_collide_on_the_same_key() {
        let s = store();
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        assert!(!s.check_idempotency("shared-key", a, "hash-a", 1_000).unwrap());
        assert!(!s.check_idempotency("shared-key", b, "hash-b", 1_000).unwrap());
    }

    #[test]
    fn pruning_removes_only_entries_past_retention() {
        let s = store();
        let client = Uuid::now_v7();
        s.check_idempotency("old", client, "h", 0).unwrap();
        s.check_idempotency("new", client, "h", 1_000).unwrap();

        let now = IDEMPOTENCY_RETENTION_MILLIS + 500;
        let pruned = s.prune_idempotency(now).unwrap();
        assert_eq!(pruned, 1, "only the entry past 24h should be pruned");
        assert!(s.get_idempotency("old", client).unwrap().is_none());
        assert!(s.get_idempotency("new", client).unwrap().is_some());
    }

    // ---- file-backed open / migration idempotency ----

    #[test]
    fn file_backed_store_persists_and_reopen_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("farcooler-store-open-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("db.sqlite3");

        let host = Uuid::now_v7();
        let root_id = {
            let s = Store::open(&db_path).unwrap();
            let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
            root.id
        }; // dropped: connection closes, file remains on disk

        // Reopening an up-to-date database must not error and must not
        // re-run migrations against tables that already exist.
        let s = Store::open(&db_path).unwrap();
        let root = s.get_repository_root(root_id).unwrap();
        assert_eq!(root.path, "/repos/one");

        // Already current, so no backup should have been written.
        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(entries.len(), 1, "no backup file expected when reopening a current schema");

        std::fs::remove_dir_all(&dir).ok();
    }
}
