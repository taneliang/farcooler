//! Forward-only migrations within a major version.
//!
//! Each migration takes the schema from version N to N+1. `schema_version` in
//! `meta` is the durable watermark: reopening an already-current database is a
//! no-op rather than reapplying DDL, which is what makes running migrations
//! twice safe.

use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::error::map_err;

type Migration = fn(&Transaction) -> rusqlite::Result<()>;

const MIGRATIONS: &[Migration] = &[
    migration_0001_initial_schema,
    migration_0002_pane_groups,
    migration_0003_drop_pane_groups,
    migration_0004_pane_mode,
    migration_0005_drop_loss_dismissed,
    migration_0006_worktrees_are_managed,
    migration_0007_review,
];

pub(crate) const CURRENT_SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub(crate) fn read_schema_version(conn: &Connection) -> farcooler_core::Result<u32> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0))
        .optional()
        .map_err(map_err)?;
    Ok(raw.and_then(|s| s.parse().ok()).unwrap_or(0))
}

/// Apply every migration from `from_version` up to `CURRENT_SCHEMA_VERSION` in
/// one transaction, then advance the watermark. A no-op when already current.
pub(crate) fn migrate(conn: &mut Connection, from_version: u32) -> farcooler_core::Result<()> {
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
        -- stale "running" could ever occupy. See farcooler_core::derive.
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

/// Tiling stopped being stored.
///
/// 0002 added a durable split model — groups, membership, five preset
/// arrangements — and it was the wrong half of the split between what tmux owns
/// and what this database owns. tmux already has split trees, named layouts,
/// dividers and zoom, and it is already the authority for what is running; an
/// arrangement of live processes is runtime, not intent. Keeping a second copy
/// meant a third in every client that drew it.
///
/// Nothing is migrated because nothing can be: the rows described panes in a tmux
/// server that has almost certainly been restarted since, and a layout whose
/// processes are gone is not a layout. Dropped rather than left in place, so the
/// schema does not describe a model the code no longer has.
///
/// 0002 is kept above it. Migrations are forward-only and a database that has
/// never seen 0002 still has to reach the same schema as one that has, which
/// means creating the tables and then dropping them.
fn migration_0003_drop_pane_groups(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        DROP TABLE IF EXISTS pane_members;
        DROP TABLE IF EXISTS pane_groups;
        "#,
    )
}

/// Agent pane mode, and the session id that outlives every pane hosting it.
///
/// `agent_session_id` is intent, in the same sense as the branch: it says what
/// this terminal is FOR. The conversation it names is never stored here — the
/// shim holds that in memory and says `Gap` where it cannot account for it.
fn migration_0004_pane_mode(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE terminals ADD COLUMN pane_mode INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE terminals ADD COLUMN agent_session_id TEXT;
        "#,
    )
}

/// Dismissing a loss deletes the record, so there is nothing left to flag.
///
/// The column held "the user has seen this loss", which kept a terminal listed
/// as lost forever while no longer holding its workspace in `error`. That is a
/// row that can never say anything again and cannot be got rid of — every
/// client drew a Dismiss button that visibly did nothing. Dismissal now removes
/// the terminal, which is what the button always claimed to do.
///
/// The invariant it was protecting is untouched: no exit is ever claimed that
/// was not observed. Forgetting a terminal at the user's explicit request is
/// not the same as inventing an exit code for it.
fn migration_0005_drop_loss_dismissed(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE terminals DROP COLUMN loss_dismissed;")
}

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
/// What a worktree is compared against, and whether you have read it.
///
/// Edited in place rather than followed by an 0008 that drops what it just
/// created: 0007 is unreleased and lives on this branch alone, so no database
/// anywhere has ever run the version that made the buffer's tables.
///
/// The buffer itself — entries, anchors, dispatches, attachments — is gone. It
/// existed because there was nowhere to put a diff, so a comment had to be held
/// somewhere until you could type it at an agent. With a diff tile beside an
/// agent tile there is nothing to hold.
fn migration_0007_review(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        -- What a workspace is compared against, when the user pinned it.
        CREATE TABLE review_bases (
            workspace_id BLOB PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
            base_ref TEXT NOT NULL
        );

        -- "I have looked at this worktree." Stores the CHEAP gate as well as
        -- the digest, so the fleet can answer "changed since you looked" with
        -- two stats instead of a `git status` per worktree.
        CREATE TABLE review_reviewed (
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            branch TEXT NOT NULL,
            head_commit TEXT NOT NULL,
            worktree_digest TEXT NOT NULL,
            gate_head INTEGER NOT NULL DEFAULT 0,
            gate_index INTEGER NOT NULL DEFAULT 0,
            marked_at INTEGER NOT NULL,
            PRIMARY KEY (workspace_id, branch)
        );

        -- One branch's parent in a stack, when inference got it wrong and the
        -- user said so.
        CREATE TABLE review_stack_parents (
            repository_id BLOB NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            branch TEXT NOT NULL,
            parent_branch TEXT NOT NULL,
            PRIMARY KEY (repository_id, branch)
        );
        "#,
    )
}

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
        ]
        {
            assert!(names.iter().any(|n| n == expected), "missing table {expected}");
        }
        for gone in ["pane_groups", "pane_members"] {
            assert!(
                !names.iter().any(|n| n == gone),
                "{gone} was dropped: tiling is tmux's, not ours"
            );
        }
    }

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
}
