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
    migration_0008_drop_task_name,
    migration_0009_workspace_order,
    migration_0010_the_board,
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

/// A workspace is named by its worktree, so it stores no name.
///
/// The column held a title typed separately from the branch, and the New
/// workspace sheet derived the branch from it — so one answer was given twice
/// and then nothing kept the two related. Most rows ended up restating their
/// branch (`review` beside `feat/review`), worktrees the reconciler adopted were
/// titled after their directory anyway, and the pairs drifted (`add tests`
/// beside `feat/tests`).
///
/// The name now comes from the worktree's directory on every read. Not from the
/// branch: one worktree hosts a stack of commits over its life, so the branch
/// inside it changes as the stack is built, and naming a workspace after it
/// would rename the workspace whenever the work moved forward.
///
/// This drops data. What it drops is a name that could differ from the
/// directory it described, and every row's directory is still right there — a
/// workspace at `…/overnight-rate-limiting` reads "overnight rate limiting"
/// rather than the "rate limiting" someone typed. A wordier row for a few days
/// is the cost of there being one name, and worktrees are short-lived.
fn migration_0008_drop_task_name(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE workspaces DROP COLUMN task_name;")
}

/// Where a workspace sits in the list, decided once and then only by the user.
///
/// Until this column there was nothing to order by. `list_workspaces_for_
/// repository` had no `ORDER BY` at all, so SQLite handed back whatever the
/// query plan yielded and that changes as rows are updated — a sidebar that
/// rearranged itself while you read it. Adding an `ORDER BY` would not have
/// fixed it, because the table had no column worth ordering by: no
/// `created_at`, and `id` is a blob.
///
/// A stored rank rather than a sort key computed from the row. The rule this
/// exists to serve is that a card NEVER moves on its own — not for activity,
/// not for attention, not for recency — because a position that stays put is
/// what makes reaching for one without looking possible. Anything derived from
/// the work would move.
///
/// One rank across the runner, not one per repository. A client may draw the
/// fleet flat or grouped, and a per-repository rank is only an order once you
/// also have an order for repositories — which nothing here has. Reordering
/// inside one group still works: `Store::reorder_workspaces` permutes rows
/// among the positions they already hold, so a group's cards never leave the
/// slots the group occupies.
///
/// **Existing rows are ranked main checkout first, then by `worktree_path`.**
///
/// Creation order is the rule going forward and is simply not recoverable for
/// rows written before this: no column ever recorded it, and `id` is only a
/// proxy — a `Uuid::now_v7` for rows this version wrote, but every worktree the
/// reconciler adopts when it first sees a repository is minted in one batch, so
/// for those it records the order `git worktree list` happened to print rather
/// than anything the user did.
///
/// `worktree_path` is the one column that is stable, effectively unique (there
/// is already a unique index on it per repository), and predictable to a person
/// without being told the rule: it reads alphabetically. `is_main_checkout`
/// comes first because that is where the repository's own checkout already sits
/// in the Mac's sidebar, which partitioned it to the top of every project group
/// — so nobody's list moves on upgrade. Both together are a TOTAL order once
/// `id` breaks the tie that cannot normally happen, and every card lands
/// somewhere a person can explain without being shown the code.
fn migration_0009_workspace_order(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE workspaces ADD COLUMN ordinal INTEGER NOT NULL DEFAULT 0;

        -- How many rows sort before this one, which IS its rank. Main
        -- checkouts first (1 before 0, hence the descending comparison), then
        -- by path, then by `id` to break a tie that cannot normally happen —
        -- the unique index is per repository, and two repositories cannot share
        -- a worktree directory. Three keys so the backfill is a TOTAL order
        -- rather than one with two rows left arbitrary.
        --
        -- A correlated count rather than a window function: it is the same
        -- answer, it reads as what it means, and the row counts here are tens.
        UPDATE workspaces SET ordinal = (
            SELECT COUNT(*) FROM workspaces AS earlier
            WHERE earlier.is_main_checkout > workspaces.is_main_checkout
               OR (earlier.is_main_checkout = workspaces.is_main_checkout
                   AND (earlier.worktree_path < workspaces.worktree_path
                        OR (earlier.worktree_path = workspaces.worktree_path
                            AND earlier.id < workspaces.id)))
        );

        CREATE INDEX workspaces_by_ordinal ON workspaces (ordinal);
        "#,
    )
}

/// The board: tasks, their notes, and what blocks what.
///
/// **`task_notes` is append-only, and two triggers say so.** The split this
/// whole design rests on is that current understanding may be revised while
/// the record of how it was reached may not; a note that can be silently
/// rewritten is indistinguishable from one that was always that way, which
/// makes the decision log worth nothing exactly when somebody leans on it.
/// Correcting the record is a new note carrying `supersedes`.
///
/// `UPDATE` is the obvious rewrite and `DELETE` the obvious erasure, so each
/// gets its own `BEFORE` trigger raising the same `ABORT`. Neither is enough
/// on its own against `INSERT OR REPLACE` (an ordinary Rust upsert idiom):
/// `REPLACE` on a `PRIMARY KEY` collision is an implicit delete-then-insert
/// that keeps the row's id, so a rewrite via `REPLACE` is worse than an
/// `UPDATE` — a reader querying by id cannot even tell the row was ever
/// touched. SQLite only fires delete triggers for that implicit delete when
/// `recursive_triggers` is on, which is why `Store::init` sets it right next
/// to `PRAGMA foreign_keys = ON`: the trigger here is necessary but silently
/// incomplete without that pragma.
///
/// The delete trigger only fires while the note's task still exists (see
/// its `WHEN` clause). Append-only means a note cannot be revised or removed
/// from a *live* log — it does not mean the record outlives the task it is
/// about. `ON DELETE CASCADE`, unlike `REPLACE`'s implicit delete, is never
/// gated by `recursive_triggers`, so an unconditional trigger here would
/// also fire for the cascade out of `tasks` and `Store::delete_repository`
/// would fail outright the moment any task on the repository had a note —
/// which is exactly what shipped in fix round 1 and had to be reverted.
///
/// `status_since` is stored rather than derived from the notes. The board's
/// most important column is how long a task has been sitting still, and a list
/// view must not read every task's history to draw a row.
fn migration_0010_the_board(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE repositories ADD COLUMN task_key_prefix TEXT NOT NULL DEFAULT '';

        -- Every existing row defaults to '', so a plain UNIQUE would refuse
        -- to even add the column to a database with two repositories
        -- already in it. A partial index allows any number of empties and
        -- only starts enforcing uniqueness once a prefix is actually
        -- assigned, which is the "resolved when the second repository is
        -- registered, once" promise from the design.
        CREATE UNIQUE INDEX repositories_one_task_prefix
            ON repositories (task_key_prefix) WHERE task_key_prefix != '';

        CREATE TABLE tasks (
            id BLOB PRIMARY KEY NOT NULL,
            -- Deleting a repository cascades here, and from here on to every
            -- note the task ever carried (see task_notes.task_id below). The
            -- notes' append-only triggers do not block this: they guard a
            -- note against being touched while its task is still around, not
            -- against leaving with it. So deleting a repository DOES succeed
            -- and DOES take its whole board with it, decision log included —
            -- the same rule as the ephemeral rows (workspaces, terminals)
            -- this pattern was written for, even though a task's notes are a
            -- decision log the design explicitly wants kept while the task
            -- lives. Left as CASCADE deliberately — changing it is a product
            -- decision, not this migration's to make — but a reader of the
            -- schema should not have to discover the consequence by testing
            -- repository deletion.
            repository_id BLOB NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            key TEXT NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL,
            status_since INTEGER NOT NULL,
            intent TEXT NOT NULL DEFAULT '',
            acceptance TEXT NOT NULL DEFAULT '[]',
            constraints TEXT NOT NULL DEFAULT '[]',
            labels TEXT NOT NULL DEFAULT '[]',
            workspace_id BLOB REFERENCES workspaces(id) ON DELETE SET NULL,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL,
            UNIQUE (repository_id, key)
        );

        CREATE INDEX tasks_by_repository ON tasks (repository_id, status);

        CREATE TABLE task_notes (
            id BLOB PRIMARY KEY NOT NULL,
            -- A task deleted (directly, or by its repository cascading, see
            -- tasks.repository_id above) takes its whole decision log with
            -- it, and succeeds in doing so: the append-only trigger below is
            -- guarded to fire only while the task still exists, so it stops
            -- a note being rewritten or erased one at a time out of a LIVE
            -- log, and gets out of the way of the log leaving as a unit when
            -- its task goes.
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            actor TEXT NOT NULL,
            at INTEGER NOT NULL,
            body TEXT NOT NULL,
            extra TEXT NOT NULL DEFAULT '{}',
            supersedes BLOB REFERENCES task_notes(id)
        );

        CREATE INDEX task_notes_by_task ON task_notes (task_id, at);
        CREATE INDEX task_notes_by_kind ON task_notes (task_id, kind);

        -- The record refuses to be rewritten. See this function's doc.
        CREATE TRIGGER task_notes_forbid_update
        BEFORE UPDATE ON task_notes
        BEGIN
            SELECT RAISE(ABORT, 'task_notes is append-only; supersede instead');
        END;

        -- The other half of append-only: no erasing a note either, whether
        -- by a direct DELETE or by REPLACE's implicit one. See this
        -- function's doc for why REPLACE also needs recursive_triggers on.
        --
        -- The WHEN clause is not a loophole: it is what makes "append-only"
        -- mean "cannot be revised while its task lives" rather than
        -- "outlives its task forever". ON DELETE CASCADE out of tasks (see
        -- tasks.repository_id above) fires this trigger too, and unlike
        -- REPLACE's implicit delete that firing is NOT gated by
        -- recursive_triggers — an unconditional trigger here would make
        -- Store::delete_repository fail outright the instant any task on
        -- the repository had a note, deleting nothing. Guarding on the task
        -- still existing lets a note go with its task while still refusing
        -- to let a note be deleted out from under a task that is still
        -- there.
        CREATE TRIGGER task_notes_forbid_delete
        BEFORE DELETE ON task_notes
        WHEN EXISTS (SELECT 1 FROM tasks WHERE id = OLD.task_id)
        BEGIN
            SELECT RAISE(ABORT, 'task_notes is append-only; supersede instead');
        END;

        CREATE TABLE task_blocks (
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            blocked_by BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            reason TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (task_id, blocked_by)
        );
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

    /// Dropping the name must not drop the workspace, or the terminals hanging
    /// off it.
    ///
    /// A `DROP COLUMN` SQLite cannot do in place is a table rebuild, and a
    /// rebuild is where a foreign key to a re-created `workspaces` row goes
    /// wrong quietly: the rows survive, the terminals do not, and what the user
    /// sees is a worktree that has forgotten every agent that ever ran in it.
    #[test]
    fn dropping_the_name_keeps_the_workspace_and_its_terminals() {
        let mut conn = open();
        for m in &MIGRATIONS[..7] {
            let tx = conn.transaction().unwrap();
            m(&tx).unwrap();
            tx.commit().unwrap();
        }
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1);
             INSERT INTO workspaces
                 VALUES (x'04', x'03', 'rate limiting', 'feat/rate-limiting',
                         '/r/wt/rate-limiting', 0, 0, 1, 0, 0);
             INSERT INTO terminals (id, workspace_id, title, command_preset, intent,
                                    runtime_confirmed, columns, rows, resource_version,
                                    lease_generation, epoch)
                 VALUES (x'05', x'04', 'Agent', 'claude', 0, 1, 80, 24, 1, 0, 0);",
        )
        .unwrap();

        migrate(&mut conn, 7).unwrap();

        let path: String = conn
            .query_row("SELECT worktree_path FROM workspaces WHERE id = x'04'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(path, "/r/wt/rate-limiting", "the path IS the name now, so it must survive");

        let terminals: i64 = conn
            .query_row("SELECT count(*) FROM terminals WHERE workspace_id = x'04'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(terminals, 1, "the terminals must still point at the workspace");

        let named: rusqlite::Result<String> = conn
            .query_row("SELECT task_name FROM workspaces WHERE id = x'04'", [], |r| r.get(0));
        assert!(named.is_err(), "the column is gone, not merely ignored");
    }

    /// A database written before 0009 opens, and its workspaces come back in a
    /// fixed order rather than the query plan's.
    ///
    /// Against a hand-built v8 schema for the reason `archived_rows_become_hidden`
    /// gives: the thing under test is that the backfill ranks rows, and a fixture
    /// would only prove the fixture was written correctly.
    ///
    /// The rows go in deliberately scrambled and with the main checkout LAST, so
    /// a migration that merely added the column and left every row at the
    /// default 0 fails here — as does one that ranked by insertion order.
    #[test]
    fn existing_workspaces_are_ranked_main_checkout_first_then_by_path() {
        let mut conn = open();
        for m in &MIGRATIONS[..8] {
            let tx = conn.transaction().unwrap();
            m(&tx).unwrap();
            tx.commit().unwrap();
        }
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1);
             INSERT INTO workspaces
                 VALUES (x'11', x'03', 'feat/zebra', '/r/wt/zebra', 0, 0, 1, 0, 0);
             INSERT INTO workspaces
                 VALUES (x'12', x'03', 'feat/apple', '/r/wt/apple', 0, 0, 1, 0, 0);
             INSERT INTO workspaces
                 VALUES (x'13', x'03', 'main', '/r', 0, 0, 1, 1, 0);
             INSERT INTO workspaces
                 VALUES (x'14', x'03', 'feat/mango', '/r/wt/mango', 0, 0, 1, 0, 0);",
        )
        .unwrap();

        migrate(&mut conn, 8).unwrap();

        let mut stmt = conn
            .prepare("SELECT branch FROM workspaces ORDER BY ordinal, worktree_path")
            .unwrap();
        let order: Vec<String> =
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(
            order,
            vec!["main", "feat/apple", "feat/mango", "feat/zebra"],
            "the repository's own checkout first, then alphabetically by worktree path"
        );

        // Dense and distinct, so the next create can take MAX + 1 and land
        // after everything rather than on top of something.
        let mut stmt = conn.prepare("SELECT ordinal FROM workspaces ORDER BY ordinal").unwrap();
        let ranks: Vec<i64> =
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(ranks, vec![0, 1, 2, 3], "every row gets its own rank, not the default 0");
    }

    /// One path, one row. The reconciler and `create_workspace` can race, and
    /// the index is what turns that into an error instead of a duplicate.
    #[test]
    fn one_row_per_worktree_path() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, '');
             INSERT INTO workspaces VALUES (x'04', x'03', 'main', '/r/wt', 0, 0, 1, 0, 0, 0);",
        )
        .unwrap();

        let second = conn.execute_batch(
            "INSERT INTO workspaces VALUES (x'05', x'03', 'main', '/r/wt', 0, 0, 1, 0, 0, 0);",
        );
        assert!(second.is_err(), "a second row for the same path is refused");
    }

    #[test]
    fn a_database_from_before_the_board_gains_its_tables() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        for table in ["tasks", "task_notes", "task_blocks"] {
            let found: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "{table} is missing");
        }
    }

    /// A migrated database with one task (`x'02'`, under repository `x'12'`)
    /// for a note to point at.
    ///
    /// `task_notes.task_id` is a foreign key and this build enforces foreign
    /// keys whether or not `Store::init`'s pragma has run (see
    /// `a_note_cannot_be_updated`'s history), so every append-only test needs
    /// a real task rather than a bare `x'02'`. Factored out because three
    /// tests need the identical row.
    fn open_with_a_task() -> Connection {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'10', x'11', '/r', 0, 1);
             INSERT INTO repositories
                 VALUES (x'12', x'11', x'10', 'r', '/r/.git', '', 1, '');
             INSERT INTO tasks (id, repository_id, key, title, status, status_since, created_at, resource_version)
                 VALUES (x'02', x'12', 'fc-1', 'a task', 'backlog', 0, 0, 1);
             INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra)
             VALUES (x'01', x'02', 'decision', 'user', 0, 'because', '{}');",
        )
        .unwrap();
        conn
    }

    /// The body a fresh note in `open_with_a_task` carries, so every refusal
    /// test can assert it is still there rather than merely that some error
    /// came back. An error alone does not distinguish a trigger that aborts
    /// before the write from one that aborts after it — SQLite rolls back
    /// either way, but only the re-read proves the row was never actually
    /// changed by the statement under test.
    const THE_ORIGINAL_BODY: &str = "because";

    fn note_body(conn: &Connection) -> String {
        conn.query_row("SELECT body FROM task_notes WHERE id = x'01'", [], |r| r.get(0)).unwrap()
    }

    /// A note is a record, and a record that can be rewritten is not one.
    ///
    /// Enforced in the schema rather than left to the code, because "the code
    /// never does that" is exactly the guarantee that decays. The whole value
    /// of the decision log is that a reader can trust it was not edited after
    /// the fact.
    #[test]
    fn a_note_cannot_be_updated() {
        let conn = open_with_a_task();
        let err = conn
            .execute("UPDATE task_notes SET body = 'rewritten' WHERE id = x'01'", [])
            .expect_err("a note must not be rewritable");
        assert!(
            err.to_string().contains("append-only") || err.to_string().contains("trigger"),
            "the schema itself refuses, not a comment asking nicely: {err}"
        );
        assert_eq!(
            note_body(&conn),
            THE_ORIGINAL_BODY,
            "the error must mean the write never happened, not merely that one was reported"
        );
    }

    /// The other half of append-only: erasing a note is exactly as forbidden
    /// as rewriting it. A record you can delete is a record you can make
    /// disappear the moment it becomes inconvenient, which is the same
    /// failure the UPDATE trigger exists to prevent.
    #[test]
    fn a_note_cannot_be_deleted() {
        let conn = open_with_a_task();
        let err = conn
            .execute("DELETE FROM task_notes WHERE id = x'01'", [])
            .expect_err("a note must not be erasable");
        assert!(
            err.to_string().contains("append-only") || err.to_string().contains("trigger"),
            "the schema itself refuses, not a comment asking nicely: {err}"
        );
        assert_eq!(note_body(&conn), THE_ORIGINAL_BODY, "the row must still be there at all");
    }

    /// `INSERT OR REPLACE` is the case that matters most: it is an ordinary
    /// upsert idiom, it keeps the row's id, and a `BEFORE DELETE` trigger
    /// alone does NOT stop it — SQLite only runs delete triggers for
    /// REPLACE's implicit delete when `recursive_triggers` is on. This test
    /// turns that pragma on itself (mirroring what `Store::init` does) so it
    /// is exercising the same configuration a real `Store` runs with, not a
    /// looser one that happens to also refuse the write for an unrelated
    /// reason.
    #[test]
    fn a_note_cannot_be_replaced() {
        let conn = open_with_a_task();
        conn.execute_batch("PRAGMA recursive_triggers = ON;").unwrap();
        let err = conn
            .execute(
                "INSERT OR REPLACE INTO task_notes (id, task_id, kind, actor, at, body, extra)
                 VALUES (x'01', x'02', 'decision', 'user', 0, 'rewritten, same id', '{}')",
                [],
            )
            .expect_err("a note must not be replaceable, same id or not");
        assert!(
            err.to_string().contains("append-only") || err.to_string().contains("trigger"),
            "the schema itself refuses, not a comment asking nicely: {err}"
        );
        assert_eq!(
            note_body(&conn),
            THE_ORIGINAL_BODY,
            "REPLACE keeps the id, so a body check is the only way to tell a rewrite happened"
        );
    }

    #[test]
    fn a_repository_carries_the_prefix_its_task_keys_use() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('repositories') WHERE name='task_key_prefix'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    /// Two repositories, no prefix assigned to either: a plain `UNIQUE`
    /// column could never have been added at all, since every pre-existing
    /// row defaults to `''` — this is what the partial index buys instead.
    #[test]
    fn two_repositories_with_no_task_prefix_are_both_fine() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, '');
             INSERT INTO repositories VALUES (x'04', x'02', x'01', 'r2', '/r2/.git', '', 1, '');",
        )
        .unwrap();
    }

    /// Once a prefix is actually assigned, a second repository cannot claim
    /// the same one — the "resolved when the second repository is
    /// registered, once" promise from the design, enforced by the schema
    /// rather than by every caller remembering to check first.
    #[test]
    fn two_repositories_cannot_share_a_task_prefix() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'fc');",
        )
        .unwrap();
        let second = conn.execute_batch(
            "INSERT INTO repositories VALUES (x'04', x'02', x'01', 'r2', '/r2/.git', '', 1, 'fc');",
        );
        assert!(second.is_err(), "a second repository claiming the same prefix is refused");
    }
}
