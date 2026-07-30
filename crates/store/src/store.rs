//! `Store`: the single durable connection.
//!
//! Every mutation either fully succeeds and bumps `resource_version`, or fails
//! and leaves the row untouched. There is no method here, and there must never
//! be one, that writes anything resembling "this terminal is running" -- that
//! fact is derived from tmux on every read, never stored. See the crate root
//! docs and `overnight_core::derive`.

use std::path::Path;

use overnight_core::{DomainError, Result};
use overnight_core::derive::TerminalRecord;
use overnight_core::preconditions::check_idempotency_replay;
use overnight_protocol::v1::TerminalIntent;
use rusqlite::{Connection, OptionalExtension, ToSql, params};
use uuid::Uuid;

use crate::error::map_err;
use crate::migrate;
use crate::models::{
    IdempotencyRecord, Repository, RepositoryRoot, Terminal, TerminalUpdate, Workspace, get_uuid,
    row_to_repository, row_to_repository_root, row_to_terminal, row_to_workspace, uuid_blob,
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
    /// SQLite serialises writes regardless, and every call here is short.
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
        conn.execute_batch(
            "PRAGMA foreign_keys = ON; \
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
    fn run_versioned(
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

    // ---- workspaces ----

    pub fn create_workspace(
        &self,
        repository_id: Uuid,
        task_name: &str,
        branch: &str,
        worktree_path: &str,
    ) -> Result<Workspace> {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                "INSERT INTO workspaces
                 (id, repository_id, task_name, branch, worktree_path, archived, creation_failed, resource_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 1)",
                params![uuid_blob(id), uuid_blob(repository_id), task_name, branch, worktree_path],
            )
            .map_err(map_err)?;
        Ok(Workspace {
            id,
            repository_id,
            task_name: task_name.to_string(),
            branch: branch.to_string(),
            worktree_path: worktree_path.to_string(),
            archived: false,
            creation_failed: false,
            resource_version: 1,
        })
    }

    pub fn get_workspace(&self, id: Uuid) -> Result<Workspace> {
        self.conn()
            .query_row(
                "SELECT id, repository_id, task_name, branch, worktree_path, archived, creation_failed, resource_version
                 FROM workspaces WHERE id = ?1",
                params![uuid_blob(id)],
                row_to_workspace,
            )
            .map_err(map_err)
    }

    pub fn list_workspaces_for_repository(&self, repository_id: Uuid) -> Result<Vec<Workspace>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, repository_id, task_name, branch, worktree_path, archived, creation_failed, resource_version
                 FROM workspaces WHERE repository_id = ?1",
            )
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
        archived: bool,
        creation_failed: bool,
    ) -> Result<Workspace> {
        self.run_versioned(
            "UPDATE workspaces
             SET task_name = ?1, branch = ?2, worktree_path = ?3, archived = ?4, creation_failed = ?5, resource_version = ?6
             WHERE id = ?7 AND resource_version = ?8",
            &[
                &task_name,
                &branch,
                &worktree_path,
                &archived,
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
                  exit_code, exit_signal, loss_dismissed, lease_generation, epoch,
                  "columns", "rows", resource_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, NULL, NULL, 0, 0, 0, ?6, ?7, 1)"#,
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
            loss_dismissed: false,
            lease_generation: 0,
            epoch: 0,
            columns,
            rows,
            resource_version: 1,
        })
    }

    pub fn get_terminal(&self, id: Uuid) -> Result<Terminal> {
        self.conn()
            .query_row(
                r#"SELECT id, workspace_id, title, command_preset, intent, runtime_confirmed,
                          exit_code, exit_signal, loss_dismissed, lease_generation, epoch,
                          "columns", "rows", resource_version
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
                          exit_code, exit_signal, loss_dismissed, lease_generation, epoch,
                          "columns", "rows", resource_version
                   FROM terminals WHERE workspace_id = ?1"#,
            )
            .map_err(map_err)?;
        let rows = stmt.query_map(params![uuid_blob(workspace_id)], row_to_terminal).map_err(map_err)?;
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
                loss_dismissed: t.loss_dismissed,
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
                   exit_code = ?5, exit_signal = ?6, loss_dismissed = ?7, lease_generation = ?8,
                   epoch = ?9, "columns" = ?10, "rows" = ?11, resource_version = ?12
               WHERE id = ?13 AND resource_version = ?14"#,
            &[
                &update.title,
                &update.command_preset,
                &(update.intent as i32),
                &update.runtime_confirmed,
                &update.exit_code,
                &update.exit_signal,
                &update.loss_dismissed,
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
    use overnight_protocol::v1::TerminalIntent;

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
            "loss_dismissed",
            "lease_generation",
            "epoch",
            "columns",
            "rows",
            "resource_version",
        ];
        assert_eq!(cols.len(), expected.len(), "unexpected column set: {cols:?}");
        for e in expected {
            assert!(cols.iter().any(|c| c == e), "missing expected column {e}, have {cols:?}");
        }
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
        let ws = s.create_workspace(repo.id, "task", "feature/x", "/wt/x").unwrap();
        assert!(!ws.archived);
        assert_eq!(s.get_workspace(ws.id).unwrap(), ws);

        let updated = s.update_workspace(ws.id, 1, "task", "feature/x", "/wt/x", true, false).unwrap();
        assert!(updated.archived);
        assert_eq!(updated.resource_version, 2);
    }

    #[test]
    fn terminal_round_trip_and_update() {
        let s = store();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_workspace(repo.id, "task", "feature/x", "/wt/x").unwrap();
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
            loss_dismissed: false,
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
        let dir = std::env::temp_dir().join(format!("overnight-store-open-{}", Uuid::now_v7()));
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
