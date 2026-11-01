//! The migrations that made lanes and trains, kept as they ran (ov-268,
//! ov-309, ov-461, ov-503, ov-520).
//!
//! A database is migrated through every step in order, so these stay exactly
//! as they shipped, though nothing reads their tables any more:
//! `line_migration.rs` (migration 0067, ov-872) moves their rows into agents,
//! the line and landings and drops them.

use rusqlite::{Transaction, params};

use crate::tasks::now_millis;

/// Six new tables that only this file and `plan_read.rs` touch.
///
/// `Older::Welcome`: a build from before them reads every table it knows
/// exactly as it did. Their rows go with their workspace or task by cascade,
/// whichever build deletes it. There is no trigger on an existing table and
/// no new column on one.
pub(crate) fn migration_0023_plan_layer(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE board_themes (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            outcome TEXT NOT NULL DEFAULT '',
            story TEXT NOT NULL DEFAULT '',
            next TEXT NOT NULL DEFAULT '',
            owner_ask TEXT NOT NULL DEFAULT '',
            state TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            -- When `story` was last written; 0 until it first is.
            story_at INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX board_themes_live_name
            ON board_themes (workspace_id, name COLLATE NOCASE) WHERE state <> 'dropped';

        -- A card is in at most one theme: the task is the key.
        CREATE TABLE board_theme_tasks (
            task_id BLOB PRIMARY KEY NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            theme_id BLOB NOT NULL REFERENCES board_themes(id) ON DELETE CASCADE,
            added_at INTEGER NOT NULL
        );
        CREATE INDEX board_theme_tasks_by_theme ON board_theme_tasks (theme_id);

        CREATE TABLE lanes (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            state TEXT NOT NULL,
            reason TEXT NOT NULL DEFAULT '',
            -- Set only by `plan set`, and only on a queued lane.
            plan_rank INTEGER,
            worktree_id BLOB REFERENCES worktrees(id) ON DELETE SET NULL,
            worktree_path TEXT NOT NULL DEFAULT '',
            branch TEXT NOT NULL DEFAULT '',
            harness TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            train TEXT,
            landed_sha TEXT,
            state_since INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX lanes_live_name
            ON lanes (workspace_id, name COLLATE NOCASE) WHERE state NOT IN ('landed', 'dropped');

        CREATE TABLE lane_tasks (
            lane_id BLOB NOT NULL REFERENCES lanes(id) ON DELETE CASCADE,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            -- Empty when the lane works the whole card.
            slice TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (lane_id, task_id, slice)
        );
        CREATE INDEX lane_tasks_by_task ON lane_tasks (task_id);

        CREATE TABLE lane_agents (
            lane_id BLOB NOT NULL REFERENCES lanes(id) ON DELETE CASCADE,
            harness TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            role TEXT NOT NULL,
            model TEXT NOT NULL DEFAULT '',
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            PRIMARY KEY (lane_id, harness, agent_id)
        );

        CREATE TABLE plan_events (
            id BLOB PRIMARY KEY NOT NULL,
            at INTEGER NOT NULL,
            actor TEXT NOT NULL,
            theme_id BLOB REFERENCES board_themes(id) ON DELETE CASCADE,
            lane_id BLOB REFERENCES lanes(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            body TEXT NOT NULL,
            extra TEXT NOT NULL DEFAULT '{}',
            CHECK ((theme_id IS NULL) <> (lane_id IS NULL))
        );
        CREATE INDEX plan_events_by_theme ON plan_events (theme_id, at) WHERE theme_id IS NOT NULL;
        CREATE INDEX plan_events_by_lane ON plan_events (lane_id, at) WHERE lane_id IS NOT NULL;
        "#,
    )
}

/// Two columns and a few more, all on this layer's own tables (ov-462, ov-461):
/// a lane's and a train's human title, and a train's integrating agent and
/// card. Nullable or defaulted, so a build from before reads every table as it
/// did (`Older::Welcome`): its lane and train inserts name no new column, and
/// its reads select the old ones.
pub(crate) fn migration_0044_plan_titles(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE lanes ADD COLUMN title TEXT NOT NULL DEFAULT '';
        ALTER TABLE board_trains ADD COLUMN title TEXT NOT NULL DEFAULT '';
        -- The card the integrating agent works for the train, if it has one.
        ALTER TABLE board_trains ADD COLUMN card_id BLOB REFERENCES tasks(id) ON DELETE SET NULL;
        -- The agent that integrates it: empty harness and id when none is recorded.
        ALTER TABLE board_trains ADD COLUMN agent_harness TEXT NOT NULL DEFAULT '';
        ALTER TABLE board_trains ADD COLUMN agent_id TEXT NOT NULL DEFAULT '';
        ALTER TABLE board_trains ADD COLUMN agent_model TEXT NOT NULL DEFAULT '';
        ALTER TABLE board_trains ADD COLUMN agent_started_at INTEGER;
        ALTER TABLE board_trains ADD COLUMN agent_ended_at INTEGER;
        "#,
    )
}

/// Two new tables: trains, and what the runner last read of CI for the SHAs,
/// runs and default branch a board names (`board_ci.rs`).
///
/// `Older::Welcome`: a build from before them reads every table it knows as it
/// did. Their rows go with their workspace by cascade, whichever build deletes
/// it. No trigger on an existing table, no new column on one.
pub(crate) fn migration_0027_trains(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE board_trains (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            -- What it was cut from: `origin/main`, or a SHA.
            base TEXT NOT NULL DEFAULT '',
            pushed_sha TEXT,
            state TEXT NOT NULL,
            state_since INTEGER NOT NULL,
            actor TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            landed_at INTEGER,
            resource_version INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX board_trains_name ON board_trains (workspace_id, name COLLATE NOCASE);

        -- What the runner last read of CI, one row per subject a board names:
        -- `sha:<sha>`, `run:<id>` or `main`. Replaced whole on each read.
        CREATE TABLE board_ci (
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            subject TEXT NOT NULL,
            -- The commit the runs are for, in full, once a run names it.
            sha TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL,
            url TEXT NOT NULL DEFAULT '',
            -- JSON: [{"name", "state", "url"}], in the order GitHub lists them.
            jobs TEXT NOT NULL DEFAULT '[]',
            -- When a read last worked; 0 if none has.
            fetched_at INTEGER NOT NULL,
            changed_at INTEGER NOT NULL,
            -- When the runner last asked, whether or not GitHub answered.
            asked_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (workspace_id, subject)
        );
        "#,
    )
}

/// A one-time repair of what the hand-kept train states left wrong (ov-503):
///
/// - A lane still open on a train that landed. Landing a train never landed its
///   lanes, so they read queued, in review or building long after their commits
///   reached main. Each lands, with the train's SHA when the train recorded one
///   (trains 81 to 87 landed with none), and a timeline event says why.
/// - A train still `integrating` whose lanes are all queued, which is a batch
///   planned ahead and then forgotten. It is dropped, which releases its lanes
///   to wait in the queue, as a dropped train always has.
///
/// `Older::Welcome`: it adds no table, column, trigger or constraint and writes
/// only rows and words (`landed`, `dropped`, a `state` event) an older build
/// already reads, so the build before it opens the file and sees the repaired
/// lanes as any lane the orchestrator closed by hand.
pub(crate) fn migration_0047_train_truth(tx: &Transaction) -> rusqlite::Result<()> {
    let now = now_millis();
    tx.execute(
        "INSERT INTO plan_events (id, at, actor, lane_id, kind, body, extra)
         SELECT randomblob(16), ?1, 'runner', l.id, 'state', 'Repaired: its train had landed.',
                json_object('from', l.state, 'to', 'landed', 'train', l.train, 'repaired', 1)
           FROM lanes l JOIN board_trains t ON t.workspace_id = l.workspace_id AND l.train = t.name COLLATE NOCASE
          WHERE t.state = 'landed' AND l.state NOT IN ('landed', 'dropped')",
        params![now],
    )?;
    tx.execute(
        "UPDATE lanes SET state = 'landed', state_since = ?1, plan_rank = NULL,
                landed_sha = coalesce((SELECT t.pushed_sha FROM board_trains t
                                        WHERE t.workspace_id = lanes.workspace_id AND lanes.train = t.name COLLATE NOCASE),
                                      landed_sha),
                resource_version = resource_version + 1
          WHERE state NOT IN ('landed', 'dropped')
            AND EXISTS (SELECT 1 FROM board_trains t
                         WHERE t.workspace_id = lanes.workspace_id AND lanes.train = t.name COLLATE NOCASE
                           AND t.state = 'landed')",
        params![now],
    )?;
    // Integrating, with lanes, and every one of them still queued.
    let forgotten: Vec<(Vec<u8>, Vec<u8>, String)> = {
        let mut stmt = tx.prepare(
            "SELECT t.id, t.workspace_id, t.name FROM board_trains t
              WHERE t.state = 'integrating'
                AND EXISTS (SELECT 1 FROM lanes l WHERE l.workspace_id = t.workspace_id AND l.train = t.name COLLATE NOCASE)
                AND NOT EXISTS (SELECT 1 FROM lanes l WHERE l.workspace_id = t.workspace_id AND l.train = t.name COLLATE NOCASE
                                   AND l.state <> 'queued')",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?
    };
    for (id, workspace, name) in forgotten {
        tx.execute(
            "INSERT INTO plan_events (id, at, actor, lane_id, kind, body, extra)
             SELECT randomblob(16), ?3, 'runner', id, 'state', 'Repaired: its train never started, so it was dropped.',
                    json_object('train', ?2, 'repaired', 1)
               FROM lanes WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE",
            params![workspace, name, now],
        )?;
        tx.execute(
            "UPDATE lanes SET train = NULL, resource_version = resource_version + 1
              WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE",
            params![workspace, name],
        )?;
        tx.execute(
            "UPDATE board_trains SET state = 'dropped', state_since = ?2, resource_version = resource_version + 1
              WHERE id = ?1",
            params![id, now],
        )?;
    }
    Ok(())
}

/// One new table. `Older::Welcome`: a build from before it never reads it, and
/// its rows go with their train by cascade, whichever build deletes the train.
pub(crate) fn migration_0048_train_events(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE train_events (
            id BLOB PRIMARY KEY NOT NULL,
            train_id BLOB NOT NULL REFERENCES board_trains(id) ON DELETE CASCADE,
            at INTEGER NOT NULL,
            actor TEXT NOT NULL,
            -- `state`, `lane_on` or `lane_off` (and `held`, which older builds wrote).
            kind TEXT NOT NULL,
            -- The lane a `lane_on` or `lane_off` is about.
            lane_id BLOB,
            extra TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX train_events_by_train ON train_events (train_id, at);
        CREATE INDEX train_events_by_lane ON train_events (lane_id, at) WHERE lane_id IS NOT NULL;
        "#,
    )
}

/// Migration 0051 (ov-520): which train collects, and which lanes were taken off
/// by hand. Two defaulted columns on the layer's own tables, so a build from
/// before reads every table as it did (`Older::Welcome`). The oldest train
/// still integrating on each board becomes its collecting one.
pub(crate) fn migration_0051_collecting_train(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE board_trains ADD COLUMN collecting INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE lanes ADD COLUMN held_off INTEGER NOT NULL DEFAULT 0;
        UPDATE board_trains SET collecting = 1
         WHERE state = 'integrating'
           AND id = (SELECT t.id FROM board_trains t
                      WHERE t.workspace_id = board_trains.workspace_id AND t.state = 'integrating'
                      ORDER BY t.created_at, t.id LIMIT 1);
        "#,
    )
}
