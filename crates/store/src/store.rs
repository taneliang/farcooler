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
use crate::compat::DatabaseSchema;
use crate::models::{
    IdempotencyRecord, PaneMode, Repository, RepositoryRoot, Terminal, TerminalRole, TerminalUpdate,
    Worktree, get_uuid, row_to_repository, row_to_repository_root, row_to_terminal, row_to_worktree,
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
    pub(crate) engine_incarnation: u64,
    /// Held for this Store's life: see `engine_cutover::owners`.
    _owner: Option<std::fs::File>,
    pub(crate) workflow_notice_waker: std::sync::Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>,
    pub(crate) workflow_board_listener: std::sync::Mutex<Option<crate::workflow_projection::BoardListener>>,
}

impl Store {
    pub(crate) fn from_engine_cutover_connection(conn: Connection, owner: Option<std::fs::File>) -> Result<Self> {
        let engine_incarnation = crate::engine_cutover::authority(&conn).map_err(|_|DomainError::OperationFailed)?.incarnation;
        Ok(Self { db:std::sync::Mutex::new(conn),engine_incarnation,workflow_notice_waker:Default::default(),workflow_board_listener:Default::default(),_owner:owner })
    }

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
        let owner = crate::engine_cutover::startup::admit_or_refuse(path)?;
        Self::open_normalizing(path).map(|store| Store { _owner: Some(owner), ..store })
    }

    /// Test-only: the pre-gate open, for fixtures that deliberately start from
    /// legacy, frozen or committed state. Production has no such entry.
    #[cfg(test)]
    pub(crate) fn open_fixture(path: impl AsRef<Path>) -> Result<Store> { Self::open_normalizing(path.as_ref()) }

    pub(crate) fn open_normalizing(path: &Path) -> Result<Store> {
        let existed = path.exists();
        let mut conn = Connection::open(path).map_err(map_err)?;
        Self::init(&mut conn, existed.then_some(path))?;
        let engine_incarnation = crate::engine_cutover::authority(&conn).map_err(|e| { tracing::warn!(error=%e,"cutover authority read failed"); DomainError::OperationFailed })?.incarnation;
        Ok(Store { db: std::sync::Mutex::new(conn), engine_incarnation, workflow_notice_waker: Default::default(), workflow_board_listener: Default::default(), _owner: None })
    }

    /// This database's schema, as `read_schema` reads a file's: what
    /// `host.get` tells a client deciding whether installing its own build
    /// here would be a downgrade.
    pub fn schema(&self) -> Result<DatabaseSchema> {
        let conn = self.conn();
        Ok(DatabaseSchema {
            version: migrate::read_schema_version(&conn)?,
            compatible_down_to: migrate::read_compatible_down_to(&conn)?,
        })
    }

    /// An in-memory database for tests: always fresh, nothing to back up.
    pub fn open_in_memory() -> Result<Store> {
        let mut conn = Connection::open_in_memory().map_err(map_err)?;
        Self::init(&mut conn, None)?;
        let engine_incarnation = crate::engine_cutover::authority(&conn).map_err(|e| { tracing::warn!(error=%e,"cutover authority read failed"); DomainError::OperationFailed })?.incarnation;
        Ok(Store { db: std::sync::Mutex::new(conn), engine_incarnation, workflow_notice_waker: Default::default(), workflow_board_listener: Default::default(), _owner: None })
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
        if current > migrate::CURRENT_SCHEMA_VERSION {
            // Written by a newer build: see `compat`.
            return crate::compat::open_newer(conn, current);
        }
        if current == migrate::CURRENT_SCHEMA_VERSION {
            // Already current: migrating again would be pure overhead, and
            // running it is exactly what must stay safe if it does happen.
            // Stamped here too, so a database migrated before the marker
            // existed carries it from its next open.
            if crate::engine_cutover::authority(conn).map_err(|e| { tracing::warn!(error=%e,"cutover authority read failed"); DomainError::OperationFailed })?.phase == "legacy" {
                crate::waits::sweep_unfitting(conn)?;
            }
            return migrate::stamp_compatible_down_to(conn);
        }

        // A version above zero means real prior schema state worth
        // preserving. A brand-new database has nothing yet that a migration
        // could destroy, so there is nothing to back up.
        if current > 0
            && let Some(path) = backup_source
        {
            crate::backup::write_checksummed_backup(path, current)?;
        }

        migrate::migrate(conn, current)?;
        if crate::engine_cutover::authority(conn).map_err(|e| { tracing::warn!(error=%e,"cutover authority read failed"); DomainError::OperationFailed })?.phase == "legacy" {
                crate::waits::sweep_unfitting(conn)?;
            }
        migrate::stamp_compatible_down_to(conn)
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
        })
    }

    pub fn get_repository(&self, id: Uuid) -> Result<Repository> {
        self.conn()
            .query_row(
                "SELECT id, host_id, repository_root_id, display_name, canonical_git_dir, remote_summary, resource_version
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
                "SELECT id, host_id, repository_root_id, display_name, canonical_git_dir, remote_summary, resource_version
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

    // ---- worktrees ----

    /// Every column of `worktrees`, in the order `row_to_worktree` reads them.
    /// Named once because three queries share it and a drifting column order is
    /// a silent field swap rather than a compile error.
    const WORKTREE_COLUMNS: &'static str = "id, repository_id, branch, \
         worktree_path, hidden, creation_failed, resource_version, is_main_checkout, \
         worktree_missing, ordinal, workspace_id, claim_source";

    /// How every listing of worktrees is ordered, in one place.
    ///
    /// `ordinal` is the user's rank and `worktree_path` is only a tie-break, so
    /// that even a database whose ordinals somehow collided still comes back in
    /// the same order twice running. Nothing here reads activity, attention or
    /// recency, and nothing here ever may: a card that moves on its own is a
    /// card you cannot reach for without looking.
    const WORKTREE_ORDER: &'static str = "ORDER BY ordinal, worktree_path";

    pub fn create_worktree(
        &self,
        repository_id: Uuid,
        branch: &str,
        worktree_path: &str,
        is_main_checkout: bool,
    ) -> Result<Worktree> {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                // A new worktree goes at the END, which is one more than the
                // highest rank anything currently holds. `MAX` over an empty
                // table is NULL, so the first row on a runner lands at 0.
                //
                // Computed in the INSERT rather than read first and written
                // second: two creates racing through a read-then-write would
                // both see the same maximum and land on the same rank.
                "INSERT INTO worktrees
                 (id, repository_id, branch, worktree_path, hidden,
                  creation_failed, resource_version, is_main_checkout, worktree_missing,
                  ordinal)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 1, ?5, 0,
                         (SELECT COALESCE(MAX(ordinal), -1) + 1 FROM worktrees))",
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
        self.get_worktree(id)
    }

    pub fn get_worktree(&self, id: Uuid) -> Result<Worktree> {
        self.conn()
            .query_row(
                &format!("SELECT {} FROM worktrees WHERE id = ?1", Self::WORKTREE_COLUMNS),
                params![uuid_blob(id)],
                row_to_worktree,
            )
            .map_err(map_err)
    }

    /// Every visible worktree on this runner, across repositories.
    ///
    /// For the fleet's own questions, which are not scoped to a project the way
    /// the sidebar's are. Hidden worktrees are left out: the user said not to
    /// see them, and that applies to a summary as much as to a list.
    pub fn list_all_worktrees(&self) -> Result<Vec<Worktree>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                // By the user's rank. This used to order by path, which was
                // stable but was not anybody's decision; `ordinal` starts out
                // as exactly that path order (see migration 0009) and then only
                // ever moves because somebody dragged a card.
                "SELECT {} FROM worktrees WHERE hidden = 0 {}",
                Self::WORKTREE_COLUMNS,
                Self::WORKTREE_ORDER
            ))
            .map_err(map_err)?;
        let rows = stmt.query_map([], row_to_worktree).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    pub fn list_worktrees_for_repository(&self, repository_id: Uuid) -> Result<Vec<Worktree>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM worktrees WHERE repository_id = ?1 {}",
                Self::WORKTREE_COLUMNS,
                Self::WORKTREE_ORDER
            ))
            .map_err(map_err)?;
        let rows =
            stmt.query_map(params![uuid_blob(repository_id)], row_to_worktree).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Every worktree whose repository is still registered, in fleet order.
    ///
    /// One query rather than a loop over repositories, because a loop is not an
    /// order: `list_repositories` has no `ORDER BY` of its own, so concatenating
    /// each repository's list would have made the fleet's order depend on the
    /// order the repositories happened to come back in. `ordinal` is ranked
    /// across the whole runner precisely so that this can be one statement.
    ///
    /// `EXISTS` rather than a join so the column list stays unqualified —
    /// `worktrees` and `repositories` share `id`, `host_id` and
    /// `resource_version`, and a join would make `WORKTREE_COLUMNS` ambiguous.
    pub fn list_worktrees_in_order(&self) -> Result<Vec<Worktree>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM worktrees
                 WHERE EXISTS (
                     SELECT 1 FROM repositories WHERE repositories.id = worktrees.repository_id
                 ) {}",
                Self::WORKTREE_COLUMNS,
                Self::WORKTREE_ORDER
            ))
            .map_err(map_err)?;
        let rows = stmt.query_map([], row_to_worktree).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Put these worktrees in this order.
    ///
    /// A permutation of the positions the named rows ALREADY hold, not a
    /// renumbering of the list from zero. The caller is a client that dragged a
    /// card, and what it can see is rarely the whole table: hidden worktrees are
    /// filtered out of every sidebar, a grouped view sends one repository's
    /// cards, and a worktree created a second ago is in neither. Renumbering
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
    pub fn reorder_worktrees(&self, ordered: &[Uuid]) -> Result<()> {
        if ordered.is_empty() {
            return Ok(());
        }
        let unique: std::collections::BTreeSet<_> = ordered.iter().collect();
        if unique.len() != ordered.len() {
            return Err(DomainError::InvalidArgument { what: "worktree_ids" });
        }

        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;

        // (rank, path, version) for each id, in the order the client asked for.
        let mut held = Vec::with_capacity(ordered.len());
        for id in ordered {
            let row: Option<(i64, String, i64)> = tx
                .query_row(
                    "SELECT ordinal, worktree_path, resource_version FROM worktrees WHERE id = ?1",
                    params![uuid_blob(*id)],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(map_err)?;
            held.push(row.ok_or(DomainError::NotFound)?);
        }

        // The slots, in the order a listing would draw them — by rank, then by
        // path, which is `WORKTREE_ORDER`. Sorting the ranks alone would be
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
                "UPDATE worktrees SET ordinal = ?1, resource_version = ?2 WHERE id = ?3",
                params![now, version + 1, uuid_blob(*id)],
            )
            .map_err(map_err)?;
        }

        tx.commit().map_err(map_err)?;
        Ok(())
    }

    pub fn update_worktree(
        &self,
        id: Uuid,
        expected_version: u64,
        branch: &str,
        worktree_path: &str,
        hidden: bool,
        creation_failed: bool,
    ) -> Result<Worktree> {
        self.run_versioned(
            "UPDATE worktrees
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
            "SELECT 1 FROM worktrees WHERE id = ?1",
            &[&uuid_blob(id)],
        )?;
        self.get_worktree(id)
    }

    /// The two flags that are opinions rather than facts about the work.
    ///
    /// Separate from `update_worktree` because both callers — hide/unhide and
    /// the reconciler — want to change exactly one thing, and passing the other
    /// five fields back unchanged is how a rename gets silently reverted by a
    /// concurrent write.
    pub fn set_worktree_flags(
        &self,
        id: Uuid,
        expected_version: u64,
        hidden: bool,
        worktree_missing: bool,
    ) -> Result<Worktree> {
        self.run_versioned(
            "UPDATE worktrees
             SET hidden = ?1, worktree_missing = ?2, resource_version = ?3
             WHERE id = ?4 AND resource_version = ?5",
            &[
                &hidden,
                &worktree_missing,
                &(expected_version as i64 + 1),
                &uuid_blob(id),
                &(expected_version as i64),
            ],
            "SELECT 1 FROM worktrees WHERE id = ?1",
            &[&uuid_blob(id)],
        )?;
        self.get_worktree(id)
    }

    /// The columns git owns, rewritten when the reconciler finds the row
    /// disagrees with `git worktree list`.
    ///
    /// A worktree row is a cache of what git knows, and these three are the part
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
    /// reason `set_worktree_flags` exists at all — those are the user's
    /// opinions, not git's facts, and passing them back unchanged is how a
    /// concurrent hide gets silently reverted.
    ///
    /// The name used to be healed here too. It no longer exists to heal: a
    /// worktree is named by its directory, read fresh on every access.
    pub fn set_worktree_identity(
        &self,
        id: Uuid,
        expected_version: u64,
        branch: &str,
        is_main_checkout: bool,
    ) -> Result<Worktree> {
        self.run_versioned(
            "UPDATE worktrees
             SET branch = ?1, is_main_checkout = ?2, resource_version = ?3
             WHERE id = ?4 AND resource_version = ?5",
            &[
                &branch,
                &is_main_checkout,
                &(expected_version as i64 + 1),
                &uuid_blob(id),
                &(expected_version as i64),
            ],
            "SELECT 1 FROM worktrees WHERE id = ?1",
            &[&uuid_blob(id)],
        )?;
        self.get_worktree(id)
    }

    pub fn delete_worktree(&self, id: Uuid, expected_version: u64) -> Result<()> {
        self.run_versioned(
            "DELETE FROM worktrees WHERE id = ?1 AND resource_version = ?2",
            &[&uuid_blob(id), &(expected_version as i64)],
            "SELECT 1 FROM worktrees WHERE id = ?1",
            &[&uuid_blob(id)],
        )
    }

    // ---- terminals ----

    #[allow(clippy::too_many_arguments)]
    pub fn create_terminal(
        &self,
        worktree_id: Uuid,
        title: &str,
        command_preset: &str,
        intent: TerminalIntent,
        columns: u32,
        rows: u32,
    ) -> Result<Terminal> {
        self.create_terminal_for_task(worktree_id, title, command_preset, intent, columns, rows, None)
    }

    /// `create_terminal`, recording the task the terminal is opened for.
    ///
    /// In the same INSERT rather than a write after it, so there is no moment
    /// at which the pane exists and its task does not. A task id that names no
    /// task is refused by the foreign key.
    ///
    /// The terminal's workspace starts as its worktree's owner, read in the
    /// same INSERT (none, if the worktree is unclaimed), and its role follows
    /// from the preset (`TerminalRole::for_preset`). Both can be changed
    /// afterwards: `Store::set_terminal_workspace`, `Store::set_terminal_role`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_terminal_for_task(
        &self,
        worktree_id: Uuid,
        title: &str,
        command_preset: &str,
        intent: TerminalIntent,
        columns: u32,
        rows: u32,
        task_id: Option<Uuid>,
    ) -> Result<Terminal> {
        self.insert_terminal(worktree_id, title, command_preset, intent, columns, rows, task_id, None)
    }

    /// `create_terminal_for_task` for a terminal a split makes: it records
    /// `split_of`, the terminal it was split from, and whether that one was
    /// the orchestrator then (`from_orchestrator`), in the same INSERT. What
    /// lets a client tell a pane somebody split beside the orchestrator on
    /// purpose from one that landed in its window some other way.
    #[allow(clippy::too_many_arguments)]
    pub fn create_split_terminal(
        &self,
        worktree_id: Uuid,
        title: &str,
        command_preset: &str,
        intent: TerminalIntent,
        columns: u32,
        rows: u32,
        task_id: Option<Uuid>,
        split_of: Uuid,
        from_orchestrator: bool,
    ) -> Result<Terminal> {
        self.insert_terminal(
            worktree_id,
            title,
            command_preset,
            intent,
            columns,
            rows,
            task_id,
            Some((split_of, from_orchestrator)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_terminal(
        &self,
        worktree_id: Uuid,
        title: &str,
        command_preset: &str,
        intent: TerminalIntent,
        columns: u32,
        rows: u32,
        task_id: Option<Uuid>,
        split_of: Option<(Uuid, bool)>,
    ) -> Result<Terminal> {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                r#"INSERT INTO terminals
                 (id, worktree_id, title, command_preset, intent, runtime_confirmed,
                  exit_code, exit_signal, lease_generation, epoch,
                  "columns", "rows", resource_version, task_id, workspace_id, role, split_of,
                  split_of_orchestrator)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, NULL, NULL, 0, 0, ?6, ?7, 1, ?8,
                         (SELECT workspace_id FROM worktrees WHERE id = ?2), ?9, ?10, ?11)"#,
                params![
                    uuid_blob(id),
                    uuid_blob(worktree_id),
                    title,
                    command_preset,
                    intent as i32,
                    columns,
                    rows,
                    task_id.map(uuid_blob),
                    TerminalRole::for_preset(command_preset).as_i64(),
                    split_of.map(|(from, _)| uuid_blob(from)),
                    split_of.map(|(_, orchestrator)| orchestrator),
                ],
            )
            .map_err(map_err)?;
        // Read back: the workspace was decided by the INSERT's own subquery.
        self.get_terminal(id)
    }

    pub fn get_terminal(&self, id: Uuid) -> Result<Terminal> {
        self.conn()
            .query_row(
                r#"SELECT id, worktree_id, title, command_preset, intent, runtime_confirmed,
                          exit_code, exit_signal, lease_generation, epoch,
                          "columns", "rows", resource_version, pane_mode, agent_session_id, task_id,
                          workspace_id, role, split_of, split_of_orchestrator, web_url
                   FROM terminals WHERE id = ?1"#,
                params![uuid_blob(id)],
                row_to_terminal,
            )
            .map_err(map_err)
    }

    pub fn list_terminals_for_worktree(&self, worktree_id: Uuid) -> Result<Vec<Terminal>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                r#"SELECT id, worktree_id, title, command_preset, intent, runtime_confirmed,
                          exit_code, exit_signal, lease_generation, epoch,
                          "columns", "rows", resource_version, pane_mode, agent_session_id, task_id,
                          workspace_id, role, split_of, split_of_orchestrator, web_url
                   FROM terminals WHERE worktree_id = ?1"#,
            )
            .map_err(map_err)?;
        let rows = stmt.query_map(params![uuid_blob(worktree_id)], row_to_terminal).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// The terminals opened for `task` (`create_terminal_for_task`), oldest
    /// first. Any state: a caller that wants the live ones asks tmux.
    pub fn terminals_for_task(&self, task: Uuid) -> Result<Vec<Terminal>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                r#"SELECT id, worktree_id, title, command_preset, intent, runtime_confirmed,
                          exit_code, exit_signal, lease_generation, epoch,
                          "columns", "rows", resource_version, pane_mode, agent_session_id, task_id,
                          workspace_id, role, split_of, split_of_orchestrator, web_url
                   FROM terminals WHERE task_id = ?1 ORDER BY rowid"#,
            )
            .map_err(map_err)?;
        let rows = stmt.query_map(params![uuid_blob(task)], row_to_terminal).map_err(map_err)?;
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
                r#"SELECT id, worktree_id, title, command_preset, intent, runtime_confirmed,
                          exit_code, exit_signal, lease_generation, epoch,
                          "columns", "rows", resource_version, pane_mode, agent_session_id, task_id,
                          workspace_id, role, split_of, split_of_orchestrator, web_url
                   FROM terminals WHERE agent_session_id = ?1"#,
            )
            .map_err(map_err)?;
        let rows = stmt.query_map(params![agent_session_id], row_to_terminal).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Feeds the daemon's derivation rule directly: durable intent, nothing
    /// tmux would have to have told us first.
    pub fn load_terminal_records(&self, worktree_id: Uuid) -> Result<Vec<TerminalRecord>> {
        Ok(self
            .list_terminals_for_worktree(worktree_id)?
            .into_iter()
            .map(|t| TerminalRecord {
                id: t.id,
                worktree_id: t.worktree_id,
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
        self.wake_workflow_notices();
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
    ///
    /// `bump_epoch` is asked for rather than decided here, and the parameter
    /// exists because the callers genuinely disagree. A pane-mode TOGGLE
    /// replaces the program in the pane, so every byte offset a client holds
    /// against this terminal is meaningless and the epoch has to move or the
    /// client will keep reading from a position in a stream that no longer
    /// exists. The other callers replace nothing: `create_terminal` and
    /// `split_terminal` write a declared session id onto a terminal that has
    /// not been drawn yet, `mark_changes_pane` runs on the same brand new
    /// terminal, `restart_terminal` has already bumped the epoch once for the
    /// same event through `update_terminal`, and the supervisor's
    /// `remember_session` writes down a name for a conversation that is
    /// already running in the pane it is named after. A blanket bump inside
    /// here would make all five of those hand every attached client a full
    /// re-read for nothing.
    pub fn set_pane_mode(
        &self,
        id: Uuid,
        expected_version: u64,
        pane_mode: PaneMode,
        agent_session_id: Option<String>,
        bump_epoch: bool,
    ) -> Result<Terminal> {
        let changed = self
            .conn()
            .execute(
                r#"UPDATE terminals
                      SET pane_mode = ?1,
                          agent_session_id = COALESCE(?2, agent_session_id),
                          epoch = epoch + ?5,
                          resource_version = resource_version + 1
                    WHERE id = ?3 AND resource_version = ?4"#,
                params![
                    pane_mode.as_i64(),
                    agent_session_id,
                    id.as_bytes().as_slice(),
                    expected_version as i64,
                    i64::from(bump_epoch),
                ],
            )
            .map_err(map_err)?;

        if changed == 0 {
            // Either the terminal is gone or someone else moved it first. Both
            // are the caller's problem to re-read, not ours to paper over.
            return Err(DomainError::ResourceConflict);
        }
        self.wake_workflow_notices();
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
#[path = "store_tests.rs"]
mod tests;
