//! What has been read on a board, kept on the runner so every device agrees.
//!
//! A board's read state is one floor (everything at or before it counts as
//! read) and the time each ticket above it was last opened. Both only ever
//! rise, so a merge is a max: commutative, associative and idempotent. Two
//! devices writing out of order, or one write retried, end in the same state
//! with nothing to resolve. Nothing here can make a ticket unread again.
//!
//! Kept in two tables of their own, never as columns on `tasks`: a read mark on
//! the task row would bump `resource_version` and break the optimistic
//! `task.update` check, and it would announce `task_changed`, which managers
//! and agents wake on.
//!
//! Every time here is the runner's, Unix milliseconds. A device sends times it
//! was told (a ticket's `lastMoved`, a note's `at`), never its own clock, so a
//! device running ahead cannot hide news the runner writes later.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{get_uuid, uuid_blob};
use crate::store::Store;

/// How far back a board's first look reaches: what a device that has never
/// been here reads as new.
pub const FIRST_LOOK_MS: i64 = 24 * 60 * 60 * 1000;

/// Two new tables that only this file touches, so a build from before them
/// reads every table it knows exactly as it did. Their rows go with their
/// workspace or task by cascade, whichever build deletes it.
pub(crate) fn migration_0022_board_reads(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE board_read_floors (
            workspace_id BLOB PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
            floor_ms INTEGER NOT NULL,
            -- 1 while the floor is the runner's own first-look default: the
            -- first floor a device seeds replaces it outright.
            implicit INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE task_reads (
            task_id BLOB PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
            opened_ms INTEGER NOT NULL
        );
        "#,
    )
}

/// One board's read state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardReads {
    pub workspace_id: Uuid,
    pub floor_ms: i64,
    /// The tickets on the board now opened above the floor, by task id.
    pub opened: Vec<(Uuid, i64)>,
}

/// What a device raises.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadsDelta {
    /// Mark All as Read. Absent for opening a ticket.
    pub floor_ms: Option<i64>,
    pub opened: Vec<(Uuid, i64)>,
    /// The floor is a device's pre-sync one: it replaces the runner's
    /// first-look default once, and is otherwise a plain raise.
    pub seeds_floor: bool,
}

impl Store {
    /// A board's read state, making its first look if it has none yet.
    ///
    /// The first look is written, not recomputed per read, so two devices
    /// reading a minute apart see one floor.
    pub fn board_reads(&self, workspace: Uuid, now_ms: i64) -> Result<BoardReads> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        floor_row(&tx, workspace, now_ms)?;
        let reads = load(&tx, workspace)?;
        tx.commit().map_err(map_err)?;
        Ok(reads)
    }

    /// Raise a board's read state and answer what it is now, with whether
    /// anything changed.
    ///
    /// Refused as `other_board` before any write when a mark names a task on
    /// another board. A mark for a task that no longer exists is skipped: it
    /// was deleted, and a device's queued write must not be stuck behind it.
    pub fn merge_board_reads(
        &self,
        workspace: Uuid,
        delta: &ReadsDelta,
        now_ms: i64,
    ) -> Result<(BoardReads, bool)> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let (mut floor, mut implicit) = floor_row(&tx, workspace, now_ms)?;
        let before = load(&tx, workspace)?;

        let mut marks: Vec<(Uuid, i64)> = Vec::new();
        for (task, ms) in &delta.opened {
            let on: Option<Vec<u8>> = tx
                .query_row("SELECT workspace_id FROM tasks WHERE id = ?1", params![uuid_blob(*task)], |r| r.get(0))
                .optional()
                .map_err(map_err)?;
            match on {
                None => {}
                Some(w) if w == uuid_blob(workspace) => marks.push((*task, *ms)),
                Some(_) => return Err(DomainError::InvalidArgument { what: "other_board" }),
            }
        }

        if let Some(raised) = delta.floor_ms {
            // A seed replaces the first-look default outright, even with a
            // lower floor; every other floor is a plain raise.
            if (implicit && delta.seeds_floor) || raised > floor {
                floor = raised;
                implicit = false;
            }
            tx.execute(
                "UPDATE board_read_floors SET floor_ms = ?2, implicit = ?3 WHERE workspace_id = ?1",
                params![uuid_blob(workspace), floor, implicit as i64],
            )
            .map_err(map_err)?;
        }
        for (task, ms) in marks {
            tx.execute(
                "INSERT INTO task_reads (task_id, opened_ms) VALUES (?1, ?2)
                 ON CONFLICT(task_id) DO UPDATE SET opened_ms = MAX(opened_ms, excluded.opened_ms)",
                params![uuid_blob(task), ms],
            )
            .map_err(map_err)?;
        }
        tx.execute(
            "DELETE FROM task_reads WHERE opened_ms <= ?2
               AND task_id IN (SELECT id FROM tasks WHERE workspace_id = ?1)",
            params![uuid_blob(workspace), floor],
        )
        .map_err(map_err)?;

        let after = load(&tx, workspace)?;
        tx.commit().map_err(map_err)?;
        let changed = after != before;
        Ok((after, changed))
    }
}

/// The board's floor and whether it is still the first-look default, making
/// the default if there is none.
fn floor_row(tx: &Transaction, workspace: Uuid, now_ms: i64) -> Result<(i64, bool)> {
    tx.execute(
        "INSERT OR IGNORE INTO board_read_floors (workspace_id, floor_ms, implicit) VALUES (?1, ?2, 1)",
        params![uuid_blob(workspace), now_ms.saturating_sub(FIRST_LOOK_MS)],
    )
    .map_err(map_err)?;
    tx.query_row(
        "SELECT floor_ms, implicit FROM board_read_floors WHERE workspace_id = ?1",
        params![uuid_blob(workspace)],
        |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
    )
    .map_err(map_err)
}

fn load(tx: &Transaction, workspace: Uuid) -> Result<BoardReads> {
    let floor_ms = tx
        .query_row("SELECT floor_ms FROM board_read_floors WHERE workspace_id = ?1", params![uuid_blob(workspace)], |r| {
            r.get(0)
        })
        .map_err(map_err)?;
    let mut stmt = tx
        .prepare(
            "SELECT r.task_id, r.opened_ms FROM task_reads r JOIN tasks t ON t.id = r.task_id
              WHERE t.workspace_id = ?1 AND r.opened_ms > ?2",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(workspace), floor_ms], |r| Ok((get_uuid(r, 0)?, r.get::<_, i64>(1)?)))
        .map_err(map_err)?;
    let mut opened = BTreeMap::new();
    for row in rows {
        let (task, ms) = row.map_err(map_err)?;
        opened.insert(task, ms);
    }
    Ok(BoardReads { workspace_id: workspace, floor_ms, opened: opened.into_iter().collect() })
}

#[cfg(test)]
#[path = "board_reads_tests.rs"]
mod tests;
