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
//!
//! **Read state is per person** (ov-740, the owner's ruling on ov-716 rule 1):
//! a member opening a card doesn't clear the owner's Unread. Every device of
//! one person agrees, and nobody else's moves. The owner's marks stay in the
//! two tables every build since ov-113 knows, untouched; each member's are in
//! `member_marks`' copies of them, keyed by handle (`Reader`).

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, ToSql, Transaction, params, params_from_iter};
use uuid::Uuid;

use farcooler_core::Result;

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
            -- Starts at the runner's first look and only ever rises.
            floor_ms INTEGER NOT NULL
        );
        CREATE TABLE task_reads (
            task_id BLOB PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
            opened_ms INTEGER NOT NULL
        );
        "#,
    )
}

/// Whose read state: the runner's owner, or a member by handle (ov-740).
///
/// The owner's lives where it always has, so a runner with no members reads
/// and writes exactly the rows it did, and a build from before ov-740 still
/// finds the owner's. A member's lives in `member_marks`' tables, whose rows
/// go with the member by cascade, so a handle used again starts fresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader<'a> {
    Owner,
    Member(&'a str),
}

impl<'a> Reader<'a> {
    fn floors(self) -> &'static str {
        match self {
            Reader::Owner => "board_read_floors",
            Reader::Member(_) => "member_board_read_floors",
        }
    }

    fn reads(self) -> &'static str {
        match self {
            Reader::Owner => "task_reads",
            Reader::Member(_) => "member_task_reads",
        }
    }

    /// `AND <alias>handle = ?n` for a member, nothing for the owner: the
    /// handle is always a query's last parameter (`bind`), so the owner's SQL
    /// is the SQL it always was.
    pub(crate) fn and_handle(self, n: usize, alias: &str) -> String {
        match self {
            Reader::Owner => String::new(),
            Reader::Member(_) => format!(" AND {alias}handle = ?{n}"),
        }
    }

    /// `, handle` for a member's column list, nothing for the owner.
    pub(crate) fn handle_column(self) -> &'static str {
        match self {
            Reader::Owner => "",
            Reader::Member(_) => ", handle",
        }
    }

    /// `, ?n` for a member's values, nothing for the owner.
    pub(crate) fn handle_value(self, n: usize) -> String {
        match self {
            Reader::Owner => String::new(),
            Reader::Member(_) => format!(", ?{n}"),
        }
    }

    /// `params`, with a member's handle last.
    pub(crate) fn bind<'b>(&'b self, mut params: Vec<&'b dyn ToSql>) -> Vec<&'b dyn ToSql> {
        if let Reader::Member(handle) = self {
            params.push(handle);
        }
        params
    }
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
}

impl Store {
    /// A board's read state, making its first look if it has none yet.
    ///
    /// The first look is written, not recomputed per read, so two devices
    /// reading a minute apart see one floor.
    pub fn board_reads(&self, workspace: Uuid, now_ms: i64) -> Result<BoardReads> {
        self.board_reads_of(Reader::Owner, workspace, now_ms)
    }

    /// `board_reads`, as `reader` has read the board.
    pub fn board_reads_of(&self, reader: Reader, workspace: Uuid, now_ms: i64) -> Result<BoardReads> {
        let mut conn = self.conn();
        // A read first, and a write only for a board's first look: a board
        // read at `read` scope takes no write lock once the row exists.
        let w = uuid_blob(workspace);
        let known: Option<i64> = conn
            .query_row(
                &format!("SELECT floor_ms FROM {} WHERE workspace_id = ?1{}", reader.floors(), reader.and_handle(2, "")),
                params_from_iter(reader.bind(vec![&w])),
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?;
        let tx = conn.transaction().map_err(map_err)?;
        if known.is_none() {
            floor_row(&tx, reader, workspace, now_ms)?;
        }
        let reads = load(&tx, reader, workspace)?;
        tx.commit().map_err(map_err)?;
        Ok(reads)
    }

    /// Raise a board's read state and answer what it is now, with whether
    /// anything changed.
    ///
    /// Every value only rises and is merged by max, so the same writes in any
    /// order end in the same state.
    ///
    /// Every time is clamped to `now_ms`, the runner's clock: a device's
    /// clock running ahead must not hide news the runner writes later, on every
    /// device, for good. A mark for a task that is gone, or now on another
    /// board, is skipped: a device's queued write (the one-time upload sends
    /// every local mark) must not be stuck behind it.
    pub fn merge_board_reads(
        &self,
        workspace: Uuid,
        delta: &ReadsDelta,
        now_ms: i64,
    ) -> Result<(BoardReads, bool)> {
        self.merge_board_reads_of(Reader::Owner, workspace, delta, now_ms)
    }

    /// `merge_board_reads`, into `reader`'s read state alone.
    pub fn merge_board_reads_of(
        &self,
        reader: Reader,
        workspace: Uuid,
        delta: &ReadsDelta,
        now_ms: i64,
    ) -> Result<(BoardReads, bool)> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let mut floor = floor_row(&tx, reader, workspace, now_ms)?;
        let before = load(&tx, reader, workspace)?;
        let w = uuid_blob(workspace);

        let mut marks: Vec<(Uuid, i64)> = Vec::new();
        for (task, ms) in &delta.opened {
            let on: Option<Vec<u8>> = tx
                .query_row("SELECT workspace_id FROM tasks WHERE id = ?1", params![uuid_blob(*task)], |r| r.get(0))
                .optional()
                .map_err(map_err)?;
            match on {
                None => {}
                Some(w) if w == uuid_blob(workspace) => marks.push((*task, (*ms).min(now_ms))),
                Some(_) => {}
            }
        }

        if let Some(raised) = delta.floor_ms.map(|f| f.min(now_ms)).filter(|f| *f > floor) {
            floor = raised;
            tx.execute(
                &format!("UPDATE {} SET floor_ms = ?2 WHERE workspace_id = ?1{}", reader.floors(), reader.and_handle(3, "")),
                params_from_iter(reader.bind(vec![&w, &floor])),
            )
            .map_err(map_err)?;
        }
        for (task, ms) in marks {
            let t = uuid_blob(task);
            tx.execute(
                &format!(
                    "INSERT INTO {} (task_id, opened_ms{}) VALUES (?1, ?2{})
                 ON CONFLICT(task_id{}) DO UPDATE SET opened_ms = MAX(opened_ms, excluded.opened_ms)",
                    reader.reads(),
                    reader.handle_column(),
                    reader.handle_value(3),
                    reader.handle_column(),
                ),
                params_from_iter(reader.bind(vec![&t, &ms])),
            )
            .map_err(map_err)?;
        }
        tx.execute(
            &format!(
                "DELETE FROM {} WHERE opened_ms <= ?2
               AND task_id IN (SELECT id FROM tasks WHERE workspace_id = ?1){}",
                reader.reads(),
                reader.and_handle(3, ""),
            ),
            params_from_iter(reader.bind(vec![&w, &floor])),
        )
        .map_err(map_err)?;

        let after = load(&tx, reader, workspace)?;
        tx.commit().map_err(map_err)?;
        let changed = after != before;
        Ok((after, changed))
    }
}

/// The board's floor, making the first look if there is none.
///
/// A member's first look is their own, as any device's that had never been
/// here: what the owner has read says nothing of what they have.
fn floor_row(tx: &Transaction, reader: Reader, workspace: Uuid, now_ms: i64) -> Result<i64> {
    let (w, first) = (uuid_blob(workspace), now_ms.saturating_sub(FIRST_LOOK_MS));
    tx.execute(
        &format!(
            "INSERT OR IGNORE INTO {} (workspace_id, floor_ms{}) VALUES (?1, ?2{})",
            reader.floors(),
            reader.handle_column(),
            reader.handle_value(3),
        ),
        params_from_iter(reader.bind(vec![&w, &first])),
    )
    .map_err(map_err)?;
    floor_of(tx, reader, workspace)
}

fn floor_of(tx: &Transaction, reader: Reader, workspace: Uuid) -> Result<i64> {
    let w = uuid_blob(workspace);
    tx.query_row(
        &format!("SELECT floor_ms FROM {} WHERE workspace_id = ?1{}", reader.floors(), reader.and_handle(2, "")),
        params_from_iter(reader.bind(vec![&w])),
        |r| r.get(0),
    )
    .map_err(map_err)
}

fn load(tx: &Transaction, reader: Reader, workspace: Uuid) -> Result<BoardReads> {
    let w = uuid_blob(workspace);
    let floor_ms = floor_of(tx, reader, workspace)?;
    let mut stmt = tx
        .prepare(&format!(
            "SELECT r.task_id, r.opened_ms FROM {} r JOIN tasks t ON t.id = r.task_id
              WHERE t.workspace_id = ?1 AND r.opened_ms > ?2{}",
            reader.reads(),
            reader.and_handle(3, "r."),
        ))
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params_from_iter(reader.bind(vec![&w, &floor_ms])), |r| Ok((get_uuid(r, 0)?, r.get::<_, i64>(1)?)))
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
