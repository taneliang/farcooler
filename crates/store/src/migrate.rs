//! Forward-only migrations within a major version.
//!
//! Each migration takes the schema from version N to N+1. `schema_version` in
//! `meta` is the durable watermark: reopening an already-current database is a
//! no-op rather than reapplying DDL, which is what makes running migrations
//! twice safe.

use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::error::map_err;

type Migration = fn(&Transaction) -> rusqlite::Result<()>;

const MIGRATIONS: &[Migration] =
    &[migration_0001_initial_schema, migration_0002_pane_groups];

pub(crate) const CURRENT_SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub(crate) fn read_schema_version(conn: &Connection) -> overnight_core::Result<u32> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0))
        .optional()
        .map_err(map_err)?;
    Ok(raw.and_then(|s| s.parse().ok()).unwrap_or(0))
}

/// Apply every migration from `from_version` up to `CURRENT_SCHEMA_VERSION` in
/// one transaction, then advance the watermark. A no-op when already current.
pub(crate) fn migrate(conn: &mut Connection, from_version: u32) -> overnight_core::Result<()> {
    if from_version >= CURRENT_SCHEMA_VERSION {
        return Ok(());
    }

    let tx = conn.transaction().map_err(map_err)?;
    for m in &MIGRATIONS[from_version as usize..] {
        m(&tx).map_err(map_err)?;
    }
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [CURRENT_SCHEMA_VERSION.to_string()],
    )
    .map_err(map_err)?;
    tx.commit().map_err(map_err)?;
    Ok(())
}

fn migration_0001_initial_schema(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE repository_roots (
            id BLOB PRIMARY KEY,
            host_id BLOB NOT NULL,
            path TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        );

        CREATE TABLE repositories (
            id BLOB PRIMARY KEY,
            host_id BLOB NOT NULL,
            repository_root_id BLOB NOT NULL REFERENCES repository_roots(id),
            display_name TEXT NOT NULL,
            canonical_git_dir TEXT NOT NULL,
            remote_summary TEXT NOT NULL,
            resource_version INTEGER NOT NULL
        );

        CREATE TABLE workspaces (
            id BLOB PRIMARY KEY,
            repository_id BLOB NOT NULL REFERENCES repositories(id),
            task_name TEXT NOT NULL,
            branch TEXT NOT NULL,
            worktree_path TEXT NOT NULL,
            archived INTEGER NOT NULL,
            creation_failed INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        );

        -- State ownership splits by durability. tmux is the sole authority for
        -- whether a process is alive right now, so runtime state never lives
        -- here: no `state`, no `is_running`, no `pid`. This table stores only
        -- intent, confirmation that creation once proved a live pane, and
        -- exit facts actually observed. There is deliberately no column a
        -- stale "running" could ever occupy. See overnight_core::derive.
        CREATE TABLE terminals (
            id BLOB PRIMARY KEY,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id),
            title TEXT NOT NULL,
            command_preset TEXT NOT NULL,
            intent INTEGER NOT NULL,
            runtime_confirmed INTEGER NOT NULL,
            exit_code INTEGER,
            exit_signal INTEGER,
            loss_dismissed INTEGER NOT NULL,
            lease_generation INTEGER NOT NULL,
            epoch INTEGER NOT NULL,
            "columns" INTEGER NOT NULL,
            "rows" INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        );

        CREATE TABLE idempotency (
            key TEXT NOT NULL,
            client_id BLOB NOT NULL,
            request_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (key, client_id)
        );
        "#,
    )
}

/// Tiling: which terminals a person wants to see together, and how.
///
/// Durable for the same reason worktrees are. tmux is the authority for what is
/// alive, but nothing about tmux knows that these three agents belong on screen
/// together and that fourth one does not — that is a decision, and decisions are
/// the half of the world this database owns.
///
/// The alternative was holding it in the Mac app's view state, which would have
/// made it invisible to the CLI and therefore invisible to agents. An agent that
/// can open a terminal but not place it is only half automatable.
fn migration_0002_pane_groups(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE pane_groups (
            id BLOB PRIMARY KEY,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id),
            name TEXT NOT NULL,
            preset INTEGER NOT NULL,
            ratio REAL NOT NULL,
            -- The pane filling the group alone. Nullable because not zoomed is
            -- the normal state, not a special one.
            zoomed BLOB,
            focused BLOB,
            active INTEGER NOT NULL,
            position INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        );

        CREATE INDEX pane_groups_by_workspace ON pane_groups (workspace_id, position);

        -- A terminal is in at most ONE group, and the primary key is what
        -- enforces that rather than every caller remembering to check. Adding a
        -- pane to a second group moves it, which is also what tmux does.
        --
        -- Absence from this table is meaningful: those are the background
        -- terminals, still listed and still running, just not on screen. That is
        -- why nothing tiles until it is asked to.
        CREATE TABLE pane_members (
            terminal_id BLOB PRIMARY KEY REFERENCES terminals(id) ON DELETE CASCADE,
            group_id BLOB NOT NULL REFERENCES pane_groups(id) ON DELETE CASCADE,
            position INTEGER NOT NULL
        );

        CREATE INDEX pane_members_by_group ON pane_members (group_id, position);
        "#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn fresh_database_starts_at_version_zero() {
        let conn = open();
        assert_eq!(read_schema_version(&conn).unwrap(), 0);
    }

    #[test]
    fn migrating_advances_the_watermark() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        assert_eq!(read_schema_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn migrating_twice_is_a_safe_no_op() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        // Running again must not try to re-create tables that already exist.
        let from = read_schema_version(&conn).unwrap();
        migrate(&mut conn, from).unwrap();
        assert_eq!(read_schema_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn migration_creates_every_expected_table() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let names: Vec<String> =
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        for expected in
        [
            "repository_roots",
            "repositories",
            "workspaces",
            "terminals",
            "idempotency",
            "meta",
            "pane_groups",
            "pane_members",
        ]
        {
            assert!(names.iter().any(|n| n == expected), "missing table {expected}");
        }
    }
}
