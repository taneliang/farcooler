//! Forward-only migrations within a major version.
//!
//! Each migration takes the schema from version N to N+1. `schema_version` in
//! `meta` is the durable watermark: reopening an already-current database is a
//! no-op rather than reapplying DDL, which is what makes running migrations
//! twice safe.

use rusqlite::{Connection, Transaction};

use crate::error::map_err;

type Migration = fn(&Transaction) -> rusqlite::Result<()>;

/// Every migration, in order, each with whether a build from before it may
/// still open the database after it.
///
/// The second half is required, not defaulted, so a migration can't arrive
/// without someone deciding. See `Older` and `COMPATIBLE_DOWN_TO`.
pub(crate) const MIGRATIONS: &[(Migration, Older)] = &[
    (migration_0001_initial_schema, Older::Refused),
    (migration_0002_pane_groups, Older::Refused),
    (migration_0003_drop_pane_groups, Older::Refused),
    // Two columns old code never names: one with a default, one nullable.
    (migration_0004_pane_mode, Older::Welcome),
    (migration_0005_drop_loss_dismissed, Older::Refused),
    (migration_0006_worktrees_are_managed, Older::Refused),
    (migration_0007_review, Older::Refused),
    (migration_0008_drop_task_name, Older::Refused),
    (migration_0009_workspace_order, Older::Refused),
    (migration_0010_the_board, Older::Refused),
    // A nullable column; deleting a task sets it NULL, whoever deletes.
    (migration_0011_terminal_task, Older::Welcome),
    (migration_0012_every_board_has_a_prefix, Older::Refused),
    // A nullable column, already read as "no revision we know of".
    (migration_0013_task_edited_at, Older::Welcome),
    (migration_0014_worktrees, Older::Refused),
    (migration_0015_workspaces, Older::Refused),
    // An index.
    (migration_0016_tasks_by_worktree, Older::Welcome),
    // Nullable, and NULL is already "nobody wrote down where it came from".
    (migration_0017_terminal_split_of, Older::Welcome),
    // The same, for whether that origin was the orchestrator.
    (migration_0018_terminal_split_of_orchestrator, Older::Welcome),
    // A column with a default and a table old code never touches, whose rows
    // go with their task by cascade. An older build enqueues no wakes; an
    // answer given while it runs wakes nobody, which is what it did anyway.
    // Refused after all (ov-212 review): the answers it queues are noted by
    // the runner (`actor = 'runner'`), which a build before it can't parse,
    // failing every read of that card's notes.
    (migration_0019_wake_on_answer, Older::Refused),
    // Two new tables (ov-194) that only usage.rs touches, with no key into
    // any table old code writes and no trigger. An older build records no
    // turns; the ones already there wait for a newer build.
    (crate::usage::migration_0020_agent_turns, Older::Welcome),
    // Refused: the schema is additive, but the `wait` and `worker` notes
    // written into it fail an older build's reads. See its doc.
    (crate::waits::migration_0021_waits_and_workers, Older::Refused),
    // Two new tables (ov-113) only board_reads.rs touches, whose rows go with
    // their workspace or task by cascade, whichever build deletes it. An older
    // build never reads or writes read state.
    (crate::board_reads::migration_0022_board_reads, Older::Welcome),
    // Six new tables (ov-268) only plan.rs and plan_read.rs touch, whose rows
    // go with their workspace, task or lane by cascade, or lose a worktree
    // link (SET NULL). No column on a table old code writes, and no trigger.
    // Older builds never read them, so a rollback past the experiment keeps a
    // working database.
    (crate::plan::migration_0023_plan_layer, Older::Welcome),
    // One new table (ov-199) only lfs_pointers.rs touches, whose rows go with
    // their worktree by cascade, whichever build deletes it. An older build
    // never reads or writes which large files weren't downloaded.
    (crate::lfs_pointers::migration_0024_lfs_pointers, Older::Welcome),
    // Two new tables (ov-269) only pages.rs touches, whose rows go with their
    // workspace by cascade, whichever build deletes it. No column on a table
    // old code writes, no trigger, and no key into the plan layer. An older
    // build never reads them, so a rollback past the experiment keeps a
    // working database.
    (crate::pages::migration_0025_pages, Older::Welcome),
    (crate::rulings::migration_0026_rulings, Older::Welcome), // ov-304: new tables only (rulings.rs says why)
    (crate::trains::migration_0027_trains, Older::Welcome), // ov-309: new tables only (trains.rs says why)
    (crate::plan_cost::migration_0028_plan_budgets, Older::Welcome), // ov-307: one new table (plan_cost.rs says why)
    (crate::landing::migration_0029_workspace_landing, Older::Welcome), // ov-313: one new table (landing.rs says why)
    (crate::rulings::migration_0030_ruling_reversals, Older::Welcome), // ov-333: one column on 0026's own table (rulings.rs says why)
    (crate::wakes::migration_0038_wake_pasted, Older::Welcome), // ov-385: one nullable column per wake queue (wakes.rs says why)
    (crate::web_panes::migration_0040_web_url, Older::Welcome), // ov-435: one nullable column (web_panes.rs says why)
    (crate::usage::migration_0042_cache_write_1h, Older::Welcome), // ov-460: one nullable column (usage.rs says why)
    (crate::messages::migration_0043_message_wakes, Older::Welcome), // ov-455: one new table (messages.rs says why)
    (crate::plan::migration_0043_plan_titles, Older::Welcome), // ov-461, ov-462: titles, and a train's agent, on the layer's own tables (plan.rs says why)
];

pub(crate) const CURRENT_SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub(crate) fn read_schema_version(conn: &Connection) -> farcooler_core::Result<u32> {
    Ok(read_meta_u32(conn, "schema_version")?.unwrap_or(0))
}

pub(crate) use crate::compat::{Older, read_compatible_down_to, read_meta_u32, stamp_compatible_down_to};

/// Apply every migration from `from_version` up to `CURRENT_SCHEMA_VERSION` in
/// one transaction, then advance the watermark. A no-op when already current.
pub(crate) fn migrate(conn: &mut Connection, from_version: u32) -> farcooler_core::Result<()> {
    if from_version >= CURRENT_SCHEMA_VERSION {
        return Ok(());
    }

    let tx = conn.transaction().map_err(map_err)?;
    for (m, _) in &MIGRATIONS[from_version as usize..] {
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

/// The task a terminal was opened for, when it was opened for one.
///
/// A column rather than an answer derived from the workspace: a task names a
/// workspace, but several tasks can share one lane, so "the task in this
/// workspace" has no single answer. A restart has to export the key the first
/// launch did, so the first launch has to write it down.
///
/// `ON DELETE SET NULL`: a terminal outlives the ticket it was opened for.
/// Deleting the task (by hand, or with its repository) forgets the link and
/// leaves the pane alone, rather than refusing the delete or taking a running
/// agent's record with it.
fn migration_0011_terminal_task(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "ALTER TABLE terminals ADD COLUMN task_id BLOB REFERENCES tasks(id) ON DELETE SET NULL;",
    )
}

/// A repository registered before the board existed gets its task key
/// prefix now, and its tasks the keys that go with it.
///
/// `migration_0010_the_board` added `task_key_prefix` with a `''` default and
/// no backfill, and the only thing that ever assigns one is repository
/// registration. Every repository registered before migration 10 therefore
/// kept `''` for good and minted `-1`, `-2`: keys a command line reads as
/// flags. This gives each such repository the prefix registration would have,
/// by the same `claim_task_key_prefix` (derivation and collision rule
/// both), in registration order. Among repositories that had no prefix, the
/// older one wins a contested prefix (`overnight` gets `ov`, `Ovation`
/// registered after it `ov2`); a prefix a repository already holds is never
/// taken back, so a prefixless `Far Cry` beside `Far Cooler`'s `fc` gets
/// `fc2` however much older it is.
///
/// Its tasks are renamed `-3` to `ov-3`, and the old key is kept in
/// `former_key`, which `Store::tasks_with_key` also answers to. Renaming is
/// what stops the board printing a key that reads as a flag; keeping the
/// old one is what makes the rename safe without touching anything else.
/// Nothing that refers to a task by id changes (`task_blocks`,
/// `terminals.task_id`, a note's `task_id`), and nothing that refers to one
/// by its old key has to: a note's body mentioning `-3` cannot be rewritten
/// anyway (`task_notes` is append-only, and this migration would abort on
/// its trigger if it tried), and an agent pane launched before the upgrade
/// carries `FARCOOLER_TASK=-3` in its environment until it restarts.
fn migration_0012_every_board_has_a_prefix(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE tasks ADD COLUMN former_key TEXT;")?;

    let prefixless: Vec<(Vec<u8>, String)> = {
        let mut stmt = tx.prepare(
            "SELECT id, display_name FROM repositories WHERE task_key_prefix = '' ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (repo, name) in prefixless {
        let prefix = crate::tasks::claim_task_key_prefix(tx, &repo, &name)?;
        // A prefixless board's keys are `'' || '-' || n`, so every one of them
        // starts with `-`, and after this none does.
        tx.execute(
            "UPDATE tasks SET former_key = key, key = ?1 || key,
                              resource_version = resource_version + 1
              WHERE repository_id = ?2 AND key LIKE '-%'",
            rusqlite::params![prefix, repo],
        )?;
    }
    Ok(())
}

/// When a task's understanding was last revised, which nothing recorded.
///
/// A card's "Updated 2h ago" is the latest of four things, and three of them
/// were already on disk: `created_at`, `status_since`, and every note's `at`
/// (a status move writes a `status_change` note too). The fourth, a revision
/// through `Store::update_task`, wrote no note on purpose and no time at all
/// -- only `resource_version`, which counts and does not date. This is that
/// time, and ONLY that time: "last updated" stays derived in the query
/// (`TASK_COLUMNS` in tasks.rs) rather than kept here, so a note or a move
/// can never be written without moving it.
///
/// Nullable with no backfill. A revision made before this migration left no
/// trace of when, and the honest value for "we do not know" is NULL, which
/// the derivation skips -- a card revised last month then falls back to its
/// last note or move, not to "just now".
fn migration_0013_task_edited_at(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE tasks ADD COLUMN edited_at INTEGER;")
}

/// `workspaces` becomes `worktrees`, and every `workspace_id` that means the
/// worktree becomes `worktree_id`.
///
/// The word "workspace" moves up a level to mean a workstream (migration 0015
/// creates that table). A native rename with `legacy_alter_table` off rewrites
/// every foreign key that names the table, so `terminals`, `tasks`,
/// `review_bases` and `review_reviewed` follow without a rebuild. SQLite has no
/// `ALTER INDEX`, so the two indexes are dropped and made again under their new
/// names, each on exactly the columns it had.
fn migration_0014_worktrees(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE workspaces RENAME TO worktrees;
        ALTER TABLE terminals RENAME COLUMN workspace_id TO worktree_id;
        ALTER TABLE tasks RENAME COLUMN workspace_id TO worktree_id;
        ALTER TABLE review_bases RENAME COLUMN workspace_id TO worktree_id;
        ALTER TABLE review_reviewed RENAME COLUMN workspace_id TO worktree_id;
        DROP INDEX workspaces_one_per_path;
        CREATE UNIQUE INDEX worktrees_one_per_path ON worktrees (repository_id, worktree_path);
        DROP INDEX workspaces_by_ordinal;
        CREATE INDEX worktrees_by_ordinal ON worktrees (ordinal);
        "#,
    )
}

/// Workspaces: workstreams that own tasks and, once claimed, worktrees.
///
/// Every repository gets a workspace called Main that takes its prefix, its
/// whole board, every worktree and every terminal. A task's workspace is
/// required, but SQLite will not `ADD COLUMN … NOT NULL` without a default, and
/// rebuilding `tasks` inside this transaction would run `task_notes`'s cascade
/// and its append-only triggers (foreign keys cannot be switched off inside a
/// transaction). So the column is added nullable, filled here, and held
/// non-null from now on by two triggers.
///
/// **Prefixes already held are placed first.** A repository with no prefix
/// (registered before the board and never given one) derives one, and it must
/// not derive a prefix another repository already holds: placing it first
/// would take that prefix, and the holder's own Main would then fail the
/// unique index and the store would not open. So every held prefix goes in
/// before any is derived.
///
/// **What deleting does.** The three new references are plain (`NO ACTION`),
/// which SQLite checks at the end of each statement. Deleting a workspace that
/// a task, a worktree or a terminal still names fails: that is the refusal
/// the design wants. Deleting a repository cascades to its workspaces and to
/// its tasks (and so their notes) in the same statement, so nothing is left
/// naming a deleted workspace when the check runs. It does NOT cascade to
/// worktrees or terminals: `worktrees.repository_id` and
/// `terminals.worktree_id` have never cascaded, and a repository with rows in
/// either still refuses to be deleted, exactly as before this migration.
/// `Service` removes terminals and worktrees itself before it deletes a
/// repository.
///
/// Roles: a terminal running the `shell` preset is a shell, and every other
/// terminal is an agent. Nothing is inferred to be an orchestrator; one running
/// today is re-tagged by hand or by restarting it as one.
///
/// `repositories.task_key_prefix` stays in the table, since migrations 0010 and
/// 0012 and their tests still write it, but nothing reads it after this: the
/// prefix lives on the workspace.
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

        -- One prefix per runner, ignoring case: keys are looked up ignoring
        -- case (see `Store::tasks_with_key`), so `bil` and `BIL` would mint
        -- keys that resolve to each other's tasks.
        CREATE UNIQUE INDEX workspaces_one_prefix ON workspaces (task_prefix COLLATE NOCASE);
        CREATE UNIQUE INDEX workspaces_one_main ON workspaces (repository_id) WHERE is_main = 1;
        CREATE INDEX workspaces_by_repository ON workspaces (repository_id, ordinal);

        ALTER TABLE worktrees ADD COLUMN workspace_id BLOB REFERENCES workspaces(id);
        ALTER TABLE worktrees ADD COLUMN claim_source TEXT;
        ALTER TABLE terminals ADD COLUMN workspace_id BLOB REFERENCES workspaces(id);
        ALTER TABLE terminals ADD COLUMN role INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE tasks ADD COLUMN workspace_id BLOB REFERENCES workspaces(id);
        CREATE INDEX tasks_by_workspace ON tasks (workspace_id, status);
        "#,
    )?;

    let repositories: Vec<(Vec<u8>, String, String)> = {
        let mut stmt =
            tx.prepare("SELECT id, display_name, task_key_prefix FROM repositories ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let now = crate::tasks::now_millis();
    let (held, prefixless): (Vec<_>, Vec<_>) =
        repositories.into_iter().partition(|(_, _, prefix)| !prefix.is_empty());
    for (repository, name, prefix) in held.into_iter().chain(prefixless) {
        // A held prefix is its own base, lowercased as every prefix since is,
        // and comes back unchanged unless an earlier repository holds the
        // same letters in another case, which `repositories_one_task_prefix`
        // (case-sensitive) never refused. Then it takes a digit. Keys minted
        // under it in another case still resolve: a key is looked up
        // ignoring case.
        let base = if prefix.is_empty() {
            crate::tasks::derive_prefix(&name)
        } else {
            prefix.to_ascii_lowercase()
        };
        let prefix = crate::workspaces::free_prefix(tx, &base)?;
        let main = crate::models::uuid_blob(uuid::Uuid::now_v7());
        tx.execute(
            "INSERT INTO workspaces (id, repository_id, name, task_prefix, is_main, ordinal, created_at)
             VALUES (?1, ?2, 'Main', ?3, 1, 0, ?4)",
            rusqlite::params![main, repository, prefix, now],
        )?;
        tx.execute(
            "UPDATE worktrees SET workspace_id = ?1, claim_source = 'migration' WHERE repository_id = ?2",
            rusqlite::params![main, repository],
        )?;
        tx.execute(
            "UPDATE tasks SET workspace_id = ?1 WHERE repository_id = ?2",
            rusqlite::params![main, repository],
        )?;
    }

    tx.execute_batch(
        r#"
        UPDATE terminals
           SET workspace_id = (SELECT w.workspace_id FROM worktrees w WHERE w.id = terminals.worktree_id);
        UPDATE terminals SET role = 0 WHERE command_preset = 'shell';

        CREATE TRIGGER tasks_need_a_workspace
        BEFORE INSERT ON tasks WHEN NEW.workspace_id IS NULL
        BEGIN
            SELECT RAISE(ABORT, 'a task needs a workspace');
        END;

        CREATE TRIGGER tasks_keep_a_workspace
        BEFORE UPDATE OF workspace_id ON tasks WHEN NEW.workspace_id IS NULL
        BEGIN
            SELECT RAISE(ABORT, 'a task needs a workspace');
        END;
        "#,
    )
}

/// An index on a task's lane, for `Worktree.open_tasks`: the fleet read asks
/// every worktree for its open tasks, and without this each ask scanned the
/// whole table.
fn migration_0016_tasks_by_worktree(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE INDEX tasks_by_worktree ON tasks (worktree_id, status);")
}

/// Where a terminal came from: the terminal it was split from, when a split
/// made it (`Service::split_terminal_with_prompt`, which the app's split, ⌃B %
/// and ⌃B ", and `terminal.create`'s join-the-layout all come through). NULL
/// for everything else, and for every row from before this column: a row
/// that predates it has an origin nobody wrote down, and a guess is worse
/// than none.
///
/// No foreign key, on purpose. A split that outlives the terminal it was
/// split from keeps the id it came with; that id names no terminal, which is
/// the same answer to "was this split from one in my window?" as a NULL, with
/// no cascade to write on every terminal removal.
fn migration_0017_terminal_split_of(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE terminals ADD COLUMN split_of BLOB;")
}

/// Whether the terminal a split was made from (`split_of`) was the
/// orchestrator at the time: 1 or 0, written in the split's own INSERT. NULL
/// for every row that isn't a split and for every split from before this
/// column, whose answer nobody wrote down. `split_of` alone can't tell a pane
/// split beside the orchestrator from one split beside a terminal that was
/// made the orchestrator afterwards, and only the first is meant to be there.
fn migration_0018_terminal_split_of_orchestrator(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE terminals ADD COLUMN split_of_orchestrator INTEGER;")
}

/// Waking the agent when somebody answers its task's decision.
///
/// `workspaces.wake_on_answer` is the workspace's switch, on (1) for every
/// workspace, old and new: the default the owner asked for.
///
/// `answer_wakes` is the queue, one row per ANSWER note, keyed by the note so
/// an answer is told at most once however often the daemon restarts. A row
/// stays after it is told, with `done_at` set, rather than being deleted: the
/// key is what keeps a second enqueue of the same note from telling it again.
/// It goes with its task.
///
/// `claimed_at` is set before the first byte is typed; a row claimed and not
/// done is never typed again (0038's `pasted_at` aside, `wakes.rs`).
fn migration_0019_wake_on_answer(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE workspaces ADD COLUMN wake_on_answer INTEGER NOT NULL DEFAULT 1;

        CREATE TABLE answer_wakes (
            note_id BLOB PRIMARY KEY NOT NULL,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            enqueued_at INTEGER NOT NULL,
            claimed_at INTEGER,
            done_at INTEGER
        );
        CREATE INDEX answer_wakes_pending ON answer_wakes (enqueued_at) WHERE done_at IS NULL;
        "#,
    )
}

/// Every migration below `version`, applied in one transaction, with the
/// watermark set to it: a database exactly as a build that stopped at
/// `version` left it, for a test to seed and then migrate forward.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn migrate_only_to(conn: &mut Connection, version: u32) {
    let tx = conn.transaction().unwrap();
    for (m, _) in &MIGRATIONS[..version as usize] {
        m(&tx).unwrap();
    }
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
        [version.to_string()],
    )
    .unwrap();
    tx.commit().unwrap();
}

#[cfg(test)]
#[path = "migrate_tests.rs"]
mod more_tests;

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
            "worktrees",
            "terminals",
            "idempotency",
            "meta",
            "board_read_floors",
            "task_reads",
            "worktree_lfs_pointers",
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
        for (m, _) in &MIGRATIONS[..5] {
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
            .query_row("SELECT hidden FROM worktrees WHERE id = x'04'", [], |r| r.get(0))
            .unwrap();
        assert!(hidden, "an archived workspace is a hidden one");

        let main: bool = conn
            .query_row("SELECT is_main_checkout FROM worktrees WHERE id = x'04'", [], |r| r.get(0))
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
        for (m, _) in &MIGRATIONS[..7] {
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
            .query_row("SELECT worktree_path FROM worktrees WHERE id = x'04'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(path, "/r/wt/rate-limiting", "the path IS the name now, so it must survive");

        let terminals: i64 = conn
            .query_row("SELECT count(*) FROM terminals WHERE worktree_id = x'04'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(terminals, 1, "the terminals must still point at the workspace");

        let named: rusqlite::Result<String> = conn
            .query_row("SELECT task_name FROM worktrees WHERE id = x'04'", [], |r| r.get(0));
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
        for (m, _) in &MIGRATIONS[..8] {
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
            .prepare("SELECT branch FROM worktrees ORDER BY ordinal, worktree_path")
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
        let mut stmt = conn.prepare("SELECT ordinal FROM worktrees ORDER BY ordinal").unwrap();
        let ranks: Vec<i64> =
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(ranks, vec![0, 1, 2, 3], "every row gets its own rank, not the default 0");
    }

    /// One path, one row. The reconciler and `create_worktree` can race, and
    /// the index is what turns that into an error instead of a duplicate.
    #[test]
    fn one_row_per_worktree_path() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, '');
             INSERT INTO worktrees (id, repository_id, branch, worktree_path, hidden, creation_failed,
                                    resource_version)
                 VALUES (x'04', x'03', 'main', '/r/wt', 0, 0, 1);",
        )
        .unwrap();

        let second = conn.execute_batch(
            "INSERT INTO worktrees (id, repository_id, branch, worktree_path, hidden, creation_failed,
                                    resource_version)
                 VALUES (x'05', x'03', 'main', '/r/wt', 0, 0, 1);",
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
             INSERT INTO workspaces (id, repository_id, name, task_prefix, is_main, ordinal, created_at)
                 VALUES (x'13', x'12', 'Main', 'fc', 1, 0, 0);
             INSERT INTO tasks (id, repository_id, workspace_id, key, title, status, status_since,
                                created_at, resource_version)
                 VALUES (x'02', x'12', x'13', 'fc-1', 'a task', 'backlog', 0, 0, 1);
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

    /// A database written before dispatch opens, gains the column, and keeps
    /// its terminals, each opened for no task.
    #[test]
    fn a_database_from_before_dispatch_gains_terminal_task_id() {
        let mut conn = open();
        {
            let tx = conn.transaction().unwrap();
            for (m, _) in &MIGRATIONS[..10] {
                m(&tx).unwrap();
            }
            tx.execute_batch(
                "INSERT INTO meta (key, value) VALUES ('schema_version', '10');
                 INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
                 INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, '');
                 INSERT INTO workspaces (id, repository_id, branch, worktree_path, hidden, creation_failed, resource_version)
                     VALUES (x'05', x'03', 'main', '/r', 0, 0, 1);
                 INSERT INTO terminals (id, workspace_id, title, command_preset, intent, runtime_confirmed,
                     lease_generation, epoch, \"columns\", \"rows\", resource_version)
                     VALUES (x'06', x'05', 'old', 'claude', 1, 0, 0, 0, 80, 24, 1);",
            )
            .unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(read_schema_version(&conn).unwrap(), 10);

        migrate(&mut conn, 10).unwrap();

        let columns: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('terminals') WHERE name='task_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(columns, 1, "terminals gained task_id");
        let task: Option<Vec<u8>> = conn
            .query_row("SELECT task_id FROM terminals WHERE id = x'06'", [], |r| r.get(0))
            .expect("the old terminal is still there");
        assert_eq!(task, None, "a terminal from before dispatch was opened for no task");
    }

    /// Exactly the migrations from `from` up to `to`, in one transaction, with
    /// the watermark moved to `to`: a database as a build that stopped at `to`
    /// would leave it.
    ///
    /// For a test that has to hold at one version however many come after it.
    /// `migrate` always runs to `CURRENT_SCHEMA_VERSION`, so a test that
    /// asserts the word `workspace` is gone would go red the day a later
    /// migration gives the word a meaning again.
    fn migrate_between(conn: &mut Connection, from: u32, to: u32) {
        let tx = conn.transaction().unwrap();
        for (m, _) in &MIGRATIONS[from as usize..to as usize] {
            m(&tx).unwrap();
        }
        tx.execute("UPDATE meta SET value = ?1 WHERE key = 'schema_version'", [to.to_string()])
            .unwrap();
        tx.commit().unwrap();
    }

    /// 0014 is a rename and nothing else: every row survives, every reference
    /// follows the table, and at 14 no column is still called `workspace_id`.
    ///
    /// Held at 14 rather than run to current, because 0015 brings the word
    /// back with its new meaning: a `workspaces` table for workstreams, and a
    /// `workspace_id` on worktrees, terminals and tasks naming one.
    #[test]
    fn a_database_from_before_the_rename_calls_every_worktree_a_worktree() {
        let mut conn = open();
        migrate_only_to(&mut conn, 13);
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'r');
             INSERT INTO workspaces (id, repository_id, branch, worktree_path, hidden, creation_failed,
                                     resource_version, is_main_checkout, ordinal)
                 VALUES (x'04', x'03', 'main', '/r', 0, 0, 1, 1, 0);
             INSERT INTO tasks (id, repository_id, key, title, status, status_since, workspace_id,
                                created_at, resource_version)
                 VALUES (x'06', x'03', 'r-1', 'a task', 'active', 0, x'04', 0, 1);
             INSERT INTO terminals (id, workspace_id, title, command_preset, intent, runtime_confirmed,
                                    lease_generation, epoch, \"columns\", \"rows\", resource_version, task_id)
                 VALUES (x'05', x'04', 't', 'shell', 1, 0, 0, 0, 80, 24, 1, x'06');
             INSERT INTO review_bases (workspace_id, base_ref) VALUES (x'04', 'release/2');
             INSERT INTO review_reviewed (workspace_id, branch, head_commit, worktree_digest, marked_at)
                 VALUES (x'04', 'main', 'head', 'digest', 9);",
        )
        .unwrap();

        migrate_between(&mut conn, 13, 14);
        assert_eq!(read_schema_version(&conn).unwrap(), 14);

        let path: String = conn
            .query_row("SELECT worktree_path FROM worktrees WHERE id = x'04'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(path, "/r", "the row survives the rename");
        for table in ["terminals", "tasks", "review_bases", "review_reviewed"] {
            let on: Vec<u8> = conn
                .query_row(&format!("SELECT worktree_id FROM {table}"), [], |r| r.get(0))
                .unwrap_or_else(|e| panic!("{table} has no worktree_id: {e}"));
            assert_eq!(on, vec![4], "{table} still points at the worktree");
            let target: String = conn
                .query_row(
                    "SELECT \"table\" FROM pragma_foreign_key_list(?1) WHERE \"from\" = 'worktree_id'",
                    [table],
                    |r| r.get(0),
                )
                .unwrap_or_else(|e| panic!("{table}.worktree_id is not a foreign key: {e}"));
            assert_eq!(target, "worktrees", "{table}'s foreign key follows the table");
        }

        let stragglers: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT m.name FROM sqlite_master m, pragma_table_info(m.name) c
                     WHERE m.type = 'table' AND c.name = 'workspace_id'",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
        };
        assert!(stragglers.is_empty(), "a column still says workspace_id in {stragglers:?}");
        let old: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE name LIKE 'workspaces%'
                     UNION ALL
                     SELECT m.name FROM sqlite_master m, pragma_foreign_key_list(m.name) f
                     WHERE m.type = 'table' AND f.\"table\" = 'workspaces'",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
        };
        assert!(old.is_empty(), "still named for, or pointing at, workspaces: {old:?}");

        // The indexes come back under their new names and still do their jobs.
        for (index, unique) in [("worktrees_one_per_path", 1), ("worktrees_by_ordinal", 0)] {
            let found: i64 = conn
                .query_row(
                    "SELECT \"unique\" FROM pragma_index_list('worktrees') WHERE name = ?1",
                    [index],
                    |r| r.get(0),
                )
                .unwrap_or_else(|e| panic!("{index} is missing: {e}"));
            assert_eq!(found, unique, "{index} keeps its uniqueness");
        }
        let second = conn.execute_batch(
            "INSERT INTO worktrees (id, repository_id, branch, worktree_path, hidden, creation_failed,
                                    resource_version)
                 VALUES (x'07', x'03', 'main', '/r', 0, 0, 1);",
        );
        assert!(second.is_err(), "a second row for the same path is still refused");

        let broken: i64 =
            conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
        assert_eq!(broken, 0, "every reference still resolves");
    }

    /// Every row of `query`, each column rendered as text, for comparing a
    /// table before and after a migration without naming its columns twice.
    fn snapshot(conn: &Connection, query: &str) -> Vec<Vec<String>> {
        let mut stmt = conn.prepare(query).unwrap();
        let width = stmt.column_count();
        stmt.query_map([], |r| {
            (0..width)
                .map(|i| {
                    Ok(match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => "NULL".to_string(),
                        rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                        rusqlite::types::ValueRef::Real(f) => f.to_string(),
                        rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                        rusqlite::types::ValueRef::Blob(b) => format!("{b:02x?}"),
                    })
                })
                .collect()
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    }

    /// Main's id in `repository`.
    fn main_of(conn: &Connection, repository: u8) -> Vec<u8> {
        conn.query_row(
            "SELECT id FROM workspaces WHERE repository_id = ?1 AND is_main = 1",
            [vec![repository]],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("repository {repository:#x} has no Main: {e}"))
    }

    /// The live board this must carry: keys, former keys from the prefixless
    /// era, and notes, all landing on their repository's Main.
    ///
    /// Two repositories, and the one registered FIRST has no prefix and a name
    /// that derives the prefix the second already holds (`overcast` and
    /// `overnight` both give `ov`). A migration that walked repositories in
    /// order and derived a free prefix for each as it went would hand `ov` to
    /// `overcast`, then fail on `overnight`'s own `ov` and refuse to open the
    /// store. Every prefix already held is placed first.
    #[test]
    fn a_board_with_renamed_keys_survives_into_main() {
        let mut conn = open();
        migrate_only_to(&mut conn, 14);
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'0a', x'02', x'01', 'overcast', '/a/.git', '', 1, '');
             INSERT INTO repositories VALUES (x'0b', x'02', x'01', 'overnight', '/b/.git', '', 3, 'ov');
             INSERT INTO worktrees (id, repository_id, branch, worktree_path, hidden, creation_failed,
                                    resource_version, is_main_checkout, worktree_missing, ordinal)
                 VALUES (x'a1', x'0a', 'main', '/a', 0, 0, 1, 1, 0, 0),
                        (x'b1', x'0b', 'main', '/b', 0, 0, 4, 1, 0, 1),
                        (x'b2', x'0b', 'feat/x', '/b/.worktrees/x', 1, 0, 2, 0, 1, 2);
             INSERT INTO tasks (id, repository_id, key, former_key, title, status, status_since,
                                intent, acceptance, constraints, labels, worktree_id, created_at,
                                resource_version, edited_at)
                 VALUES (x'd1', x'0b', 'ov-1', '-1', 'first', 'in_progress', 5, 'why', '[]', '[\"c\"]',
                         '[\"l\"]', x'b2', 1, 3, 7),
                        (x'd2', x'0b', 'ov-2', '-2', 'second', 'done', 6, '', '[]', '[]', '[]', NULL, 2,
                         2, NULL),
                        (x'd3', x'0b', 'ov-3', NULL, 'third', 'todo', 8, '', '[]', '[]', '[]', NULL, 8,
                         1, NULL),
                        (x'd4', x'0a', '-1', NULL, 'unkeyed', 'backlog', 9, '', '[]', '[]', '[]', NULL, 9,
                         1, NULL);
             INSERT INTO terminals (id, worktree_id, title, command_preset, intent, runtime_confirmed,
                                    exit_code, exit_signal, lease_generation, epoch, \"columns\", \"rows\",
                                    resource_version, pane_mode, agent_session_id, task_id)
                 VALUES (x'c1', x'b1', 'agent', 'claude', 1, 1, NULL, NULL, 0, 0, 80, 24, 1, 0, 's', NULL),
                        (x'c2', x'b1', 'shell', 'shell', 1, 1, NULL, NULL, 0, 0, 80, 24, 1, 0, NULL, NULL),
                        (x'c3', x'a1', 'codex', 'codex', 2, 1, 0, NULL, 0, 0, 80, 24, 1, 0, NULL, NULL),
                        (x'c4', x'b2', 'worker', 'claude:opus', 1, 1, NULL, NULL, 0, 0, 80, 24, 1, 1,
                         NULL, x'd1');
             INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra, supersedes)
                 VALUES (x'e1', x'd1', 'created', 'user', 1, 'first', '{}', NULL),
                        (x'e2', x'd1', 'decision', 'manager', 2, 'use sqlite', '{\"rejected\":[\"files\"]}', NULL),
                        (x'e3', x'd1', 'decision', 'manager', 3, 'use files after all', '{}', x'e2'),
                        (x'e4', x'd2', 'status_change', 'user', 4, '', '{\"from\":\"todo\",\"to\":\"done\"}', NULL),
                        (x'e5', x'd4', 'comment', 'user', 5, 'still here', '{}', NULL);",
        )
        .unwrap();
        // Every column each table had at 14, which 0015 must not change.
        let tasks_at_14 = "SELECT id, repository_id, key, former_key, title, status, status_since, intent,
                                  acceptance, constraints, labels, worktree_id, created_at,
                                  resource_version, edited_at
                             FROM tasks ORDER BY id";
        let notes_at_14 = "SELECT * FROM task_notes ORDER BY id";
        let worktrees_at_14 = "SELECT id, repository_id, branch, worktree_path, hidden, creation_failed,
                                      resource_version, is_main_checkout, worktree_missing, ordinal
                                 FROM worktrees ORDER BY id";
        let terminals_at_14 = "SELECT id, worktree_id, title, command_preset, intent, runtime_confirmed,
                                      exit_code, exit_signal, lease_generation, epoch, \"columns\", \"rows\",
                                      resource_version, pane_mode, agent_session_id, task_id
                                 FROM terminals ORDER BY id";
        let before: Vec<_> = [tasks_at_14, notes_at_14, worktrees_at_14, terminals_at_14]
            .iter()
            .map(|q| snapshot(&conn, q))
            .collect();

        migrate(&mut conn, 14).unwrap();

        let after: Vec<_> = [tasks_at_14, notes_at_14, worktrees_at_14, terminals_at_14]
            .iter()
            .map(|q| snapshot(&conn, q))
            .collect();
        assert_eq!(before, after, "every task, key, former key, note, worktree and terminal as it was");

        let workspaces = snapshot(
            &conn,
            "SELECT hex(repository_id), name, task_prefix, is_main, ordinal, resource_version
               FROM workspaces ORDER BY repository_id",
        );
        assert_eq!(
            workspaces,
            vec![
                vec!["0A", "Main", "ov2", "1", "0", "1"],
                vec!["0B", "Main", "ov", "1", "0", "1"],
            ],
            "one Main per repository; a prefix already held stays with its holder"
        );
        let (a, b) = (main_of(&conn, 0x0a), main_of(&conn, 0x0b));

        let tasks = snapshot(&conn, "SELECT hex(id), hex(workspace_id) FROM tasks ORDER BY id");
        let (a_hex, b_hex) = (hex(&a), hex(&b));
        assert_eq!(
            tasks,
            vec![
                vec!["D1".to_string(), b_hex.clone()],
                vec!["D2".to_string(), b_hex.clone()],
                vec!["D3".to_string(), b_hex.clone()],
                vec!["D4".to_string(), a_hex.clone()],
            ],
            "every task is on its own repository's Main"
        );
        let worktrees =
            snapshot(&conn, "SELECT hex(id), hex(workspace_id), claim_source FROM worktrees ORDER BY id");
        assert_eq!(
            worktrees,
            vec![
                vec!["A1".to_string(), a_hex.clone(), "migration".to_string()],
                vec!["B1".to_string(), b_hex.clone(), "migration".to_string()],
                vec!["B2".to_string(), b_hex.clone(), "migration".to_string()],
            ],
            "every worktree, hidden and missing ones too, belongs to its repository's Main"
        );
        let terminals =
            snapshot(&conn, "SELECT hex(id), hex(workspace_id), role FROM terminals ORDER BY id");
        assert_eq!(
            terminals,
            vec![
                vec!["C1".to_string(), b_hex.clone(), "1".to_string()],
                vec!["C2".to_string(), b_hex.clone(), "0".to_string()],
                vec!["C3".to_string(), a_hex.clone(), "1".to_string()],
                vec!["C4".to_string(), b_hex.clone(), "1".to_string()],
            ],
            "a shell is a shell, everything else an agent, and nothing is guessed to be an orchestrator"
        );

        let broken: i64 =
            conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
        assert_eq!(broken, 0, "every reference resolves");
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02X}")).collect()
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
        let prefix: String =
            conn.query_row("SELECT task_prefix FROM workspaces", [], |r| r.get(0)).unwrap();
        assert_eq!(prefix, "fc");
    }

    /// A held prefix is stored lowercase, and one that differs from an
    /// earlier holder's only by case takes a digit, as any other taken one
    /// does. Unreachable from a real database, since every prefix before 0015
    /// was `derive_prefix`'s, which lowercases; the old index was
    /// case-sensitive, so nothing but that ever kept `BIL` out.
    #[test]
    fn a_held_prefix_is_lowercased_and_one_differing_only_by_case_takes_a_digit() {
        let mut conn = open();
        migrate_only_to(&mut conn, 14);
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'0a', x'02', x'01', 'billing', '/a/.git', '', 1, 'bil');
             INSERT INTO repositories VALUES (x'0b', x'02', x'01', 'bills', '/b/.git', '', 1, 'BIL');
             INSERT INTO repositories VALUES (x'0c', x'02', x'01', 'ops', '/c/.git', '', 1, 'Ops');",
        )
        .unwrap();
        migrate(&mut conn, 14).unwrap();
        let prefixes = snapshot(&conn, "SELECT task_prefix FROM workspaces ORDER BY repository_id");
        assert_eq!(prefixes, vec![vec!["bil"], vec!["bil2"], vec!["ops"]]);
    }

    /// Deleting a workspace that still holds a task is refused by the schema
    /// itself, a task cannot be left without a workspace, and deleting the
    /// repository still takes its workspaces and tasks with it.
    #[test]
    fn a_workspace_with_tasks_cannot_be_deleted_but_its_repository_can() {
        let mut conn = open();
        migrate_only_to(&mut conn, 14);
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'r');
             INSERT INTO tasks (id, repository_id, key, title, status, status_since, created_at,
                                resource_version)
                 VALUES (x'07', x'03', 'r-1', 't', 'todo', 0, 0, 1);
             INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra)
                 VALUES (x'08', x'07', 'decision', 'manager', 0, 'why', '{}');",
        )
        .unwrap();
        migrate(&mut conn, 14).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        assert!(conn.execute("DELETE FROM workspaces", []).is_err(), "Main still holds a task");
        assert!(
            conn.execute("UPDATE tasks SET workspace_id = NULL", []).is_err(),
            "a task needs a workspace"
        );
        assert!(
            conn.execute(
                "INSERT INTO tasks (id, repository_id, key, title, status, status_since, created_at,
                                    resource_version)
                     VALUES (x'09', x'03', 'r-2', 't', 'todo', 0, 0, 1)",
                [],
            )
            .is_err(),
            "nor can one be filed without one"
        );

        conn.execute("DELETE FROM repositories", []).unwrap();
        let left: i64 = conn
            .query_row(
                "SELECT (SELECT count(*) FROM tasks) + (SELECT count(*) FROM workspaces)
                      + (SELECT count(*) FROM task_notes)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0, "the repository took its workspaces, tasks and notes with it");
    }

    /// `bil` and `BIL` are one prefix: a key is looked up ignoring case, so
    /// two workspaces whose prefixes differ only by case would mint keys that
    /// resolve to each other's tasks. Held by the index, not only by the
    /// store's own check.
    #[test]
    fn the_prefix_index_ignores_case() {
        let mut conn = open();
        migrate(&mut conn, 0).unwrap();
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, '');
             INSERT INTO workspaces (id, repository_id, name, task_prefix, ordinal, created_at)
                 VALUES (x'04', x'03', 'Billing', 'bil', 1, 0);",
        )
        .unwrap();
        let second = conn.execute(
            "INSERT INTO workspaces (id, repository_id, name, task_prefix, ordinal, created_at)
                 VALUES (x'05', x'03', 'Other', 'BIL', 2, 0)",
            [],
        );
        assert!(second.is_err(), "a prefix differing only by case is the same prefix");
    }

    /// A database from before a terminal recorded where it came from gains
    /// the column, and its terminals read as unknown: NULL, never a guess.
    #[test]
    fn a_database_from_before_splits_were_recorded_reads_every_terminal_as_unknown() {
        let mut conn = open();
        migrate_only_to(&mut conn, 16);
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'r');
             INSERT INTO worktrees (id, repository_id, branch, worktree_path, hidden, creation_failed, resource_version)
                 VALUES (x'05', x'03', 'main', '/r', 0, 0, 1);
             INSERT INTO terminals (id, worktree_id, title, command_preset, intent, runtime_confirmed,
                 lease_generation, epoch, \"columns\", \"rows\", resource_version)
                 VALUES (x'06', x'05', 'old', 'shell', 1, 0, 0, 0, 80, 24, 1);",
        )
        .unwrap();

        migrate(&mut conn, 16).unwrap();

        let split_of: Option<Vec<u8>> = conn
            .query_row("SELECT split_of FROM terminals WHERE id = x'06'", [], |r| r.get(0))
            .expect("the old terminal is still there, with the column");
        assert_eq!(split_of, None, "an old row's origin is unknown");
    }

    /// A split recorded before the source's role was gains the column, and
    /// reads as not known: NULL, never a guess either way.
    #[test]
    fn a_split_from_before_the_sources_role_was_recorded_reads_as_unknown() {
        let mut conn = open();
        migrate_only_to(&mut conn, 17);
        conn.execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'r');
             INSERT INTO worktrees (id, repository_id, branch, worktree_path, hidden, creation_failed, resource_version)
                 VALUES (x'05', x'03', 'main', '/r', 0, 0, 1);
             INSERT INTO terminals (id, worktree_id, title, command_preset, intent, runtime_confirmed,
                 lease_generation, epoch, \"columns\", \"rows\", resource_version, split_of)
                 VALUES (x'06', x'05', 'old', 'shell', 1, 0, 0, 0, 80, 24, 1, x'07');",
        )
        .unwrap();

        migrate(&mut conn, 17).unwrap();

        let (split_of, from_orchestrator): (Option<Vec<u8>>, Option<i64>) = conn
            .query_row("SELECT split_of, split_of_orchestrator FROM terminals WHERE id = x'06'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("the old split is still there, with the column");
        assert_eq!(split_of, Some(vec![7]), "its origin stands");
        assert_eq!(from_orchestrator, None, "whether that was the orchestrator isn't known");
    }
}
