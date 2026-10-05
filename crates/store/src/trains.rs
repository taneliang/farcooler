//! Trains (ov-309): the plan layer's record of a batch of lanes landing
//! together, with the CI the runner reads for it.
//!
//! A **train** has a name (`integ-14`), a base it was cut from, the SHA it
//! pushed, and where it stands: `integrating` while lanes are picked into it,
//! `gating` while its local gates run, `pushed` once a SHA is out, then
//! `green` or `red` as that SHA's CI says, and `landed` (or `dropped`). Its
//! lanes are the lanes whose `train` names it, so "in integ-14" on a lane and
//! the train's own list can't disagree.
//!
//! The runner reads the pushed SHA's CI through `gh` (`board_ci.rs` keeps what
//! it read) and moves a pushed train to green or red, and back to pushed when a
//! run starts again, without the orchestrator writing anything. That's what
//! retires the hand-kept trains page.
//!
//! # Additive and removable, as the plan layer is
//!
//! Two tables of their own (migration 0027, `Older::Welcome`) pointing at
//! `workspaces` by cascade. Nothing old gains a column, a trigger, a status or
//! a note kind, and nothing the board runs on names these tables
//! (`scripts/plan-layer-lint.py` checks). The plan read carries them
//! (`plan_read.rs`), so the removal drill in `plan_tests.rs` drops them with
//! the rest of the layer.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::plan::board_exists;
use crate::store::Store;
use crate::tasks::now_millis;

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

/// A train's name is short, like a lane's; its base is a ref or a SHA.
const NAME_MAX: usize = 60;
const BASE_MAX: usize = 200;

/// Where a train stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainState {
    /// Lanes are being picked into it.
    Integrating,
    /// Its local gates are running.
    Gating,
    /// A SHA is out and CI hasn't finished on it.
    Pushed,
    /// CI passed on the pushed SHA.
    Green,
    /// CI failed on the pushed SHA.
    Red,
    /// On main.
    Landed,
    /// Given up; its lanes go on in another train or alone.
    Dropped,
}

impl TrainState {
    /// Every state's stored word, in order.
    pub const WORDS: [&'static str; 7] = ["integrating", "gating", "pushed", "green", "red", "landed", "dropped"];

    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            TrainState::Integrating => "integrating",
            TrainState::Gating => "gating",
            TrainState::Pushed => "pushed",
            TrainState::Green => "green",
            TrainState::Red => "red",
            TrainState::Landed => "landed",
            TrainState::Dropped => "dropped",
        }
    }

    /// The state a stored word names.
    pub fn parse(raw: &str) -> Option<TrainState> {
        Some(match raw {
            "integrating" => TrainState::Integrating,
            "gating" => TrainState::Gating,
            "pushed" => TrainState::Pushed,
            "green" => TrainState::Green,
            "red" => TrainState::Red,
            "landed" => TrainState::Landed,
            "dropped" => TrainState::Dropped,
            _ => return None,
        })
    }

    /// Landed or dropped: nothing moves it again, and the runner stops reading
    /// its CI.
    pub fn is_settled(self) -> bool {
        matches!(self, TrainState::Landed | TrainState::Dropped)
    }

    /// Whether the runner moves it with its CI: once a SHA is out and until it
    /// settles.
    pub fn follows_ci(self) -> bool {
        matches!(self, TrainState::Pushed | TrainState::Green | TrainState::Red)
    }
}

/// One train.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Train {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub base: String,
    pub pushed_sha: Option<String>,
    pub state: TrainState,
    pub state_since: i64,
    /// Who started it, in `task_notes.actor`'s words.
    pub actor: String,
    pub created_at: i64,
    pub landed_at: Option<i64>,
    pub resource_version: u64,
}

impl Train {
    /// The CI subject its pushed SHA is read under, once it has one.
    pub fn ci_subject(&self) -> Option<String> {
        self.pushed_sha.as_deref().map(crate::board_ci::sha_subject)
    }
}

/// A train to start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTrain {
    pub name: String,
    pub base: String,
}

/// What `set_train` changes; each `None` or empty list leaves it alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrainUpdate {
    pub state: Option<TrainState>,
    pub base: Option<String>,
    /// A new pushed SHA. Without a `state`, a train integrating or gating, or
    /// one that was green or red, moves to `pushed`: CI on it hasn't run yet.
    pub sha: Option<String>,
    /// Lanes to put on it, and lanes to take off.
    pub add_lanes: Vec<Uuid>,
    pub remove_lanes: Vec<Uuid>,
}

fn invalid(what: &'static str) -> DomainError {
    DomainError::InvalidArgument { what }
}

/// A train's name: required, short, no whitespace (it's typed in a command).
fn train_name(value: &str) -> Result<String> {
    let name = value.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_whitespace) {
        return Err(invalid("name"));
    }
    Ok(name.to_string())
}

/// A SHA as given, lowercased: 7 to 40 hex digits.
pub fn clean_sha(value: &str) -> Result<String> {
    let sha = value.trim().to_ascii_lowercase();
    if !(7..=40).contains(&sha.len()) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("sha"));
    }
    Ok(sha)
}

const COLS: &str =
    "id, workspace_id, name, base, pushed_sha, state, state_since, actor, created_at, landed_at, resource_version";

fn row_to_train(r: &rusqlite::Row) -> rusqlite::Result<Train> {
    let state: String = r.get(5)?;
    Ok(Train {
        id: get_uuid(r, 0)?,
        workspace_id: get_uuid(r, 1)?,
        name: r.get(2)?,
        base: r.get(3)?,
        pushed_sha: r.get(4)?,
        state: TrainState::parse(&state).unwrap_or(TrainState::Integrating),
        state_since: r.get(6)?,
        actor: r.get(7)?,
        created_at: r.get(8)?,
        landed_at: r.get(9)?,
        resource_version: r.get::<_, i64>(10)?.max(0) as u64,
    })
}

fn train_in(conn: &Connection, id: Uuid) -> Result<Train> {
    conn.query_row(&format!("SELECT {COLS} FROM board_trains WHERE id = ?1"), params![uuid_blob(id)], row_to_train)
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

/// Each lane must exist and be on `workspace`'s board.
fn check_lanes(conn: &Connection, workspace: Uuid, lanes: &[Uuid]) -> Result<()> {
    for lane in lanes {
        let board: Option<Vec<u8>> = conn
            .query_row("SELECT workspace_id FROM lanes WHERE id = ?1", params![uuid_blob(*lane)], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        match board {
            None => return Err(DomainError::NotFound),
            Some(b) if b != uuid_blob(workspace) => return Err(invalid("other_board")),
            Some(_) => {}
        }
    }
    Ok(())
}

/// Put `lanes` on the train named `name`, and take `off` off it (only those
/// that are on it). A lane's train is its own field, so a lane on another
/// train moves to this one.
fn board_lanes(conn: &Connection, name: &str, on: &[Uuid], off: &[Uuid]) -> Result<()> {
    for lane in on {
        conn.execute(
            "UPDATE lanes SET train = ?2, resource_version = resource_version + 1 WHERE id = ?1",
            params![uuid_blob(*lane), name],
        )
        .map_err(map_err)?;
    }
    for lane in off {
        conn.execute(
            "UPDATE lanes SET train = NULL, resource_version = resource_version + 1
              WHERE id = ?1 AND train = ?2 COLLATE NOCASE",
            params![uuid_blob(*lane), name],
        )
        .map_err(map_err)?;
    }
    Ok(())
}

impl Store {
    /// Start a train on `workspace`'s board, integrating, with `lanes` on it.
    /// A name is used once per board: `integ-14` names one train for good.
    pub fn start_train(&self, workspace: Uuid, new: &NewTrain, lanes: &[Uuid], actor: Actor) -> Result<Train> {
        let name = train_name(&new.name)?;
        let base = new.base.trim();
        if base.chars().count() > BASE_MAX {
            return Err(invalid("base"));
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_exists(&tx, workspace)?;
        check_lanes(&tx, workspace, lanes)?;
        let taken = tx
            .query_row(
                "SELECT 1 FROM board_trains WHERE workspace_id = ?1 AND name = ?2 COLLATE NOCASE",
                params![uuid_blob(workspace), name],
                |_| Ok(()),
            )
            .optional()
            .map_err(map_err)?
            .is_some();
        if taken {
            return Err(invalid("name_taken"));
        }
        let id = Uuid::now_v7();
        let now = now_millis();
        tx.execute(
            "INSERT INTO board_trains (id, workspace_id, name, base, state, state_since, actor, created_at,
                                       resource_version)
             VALUES (?1, ?2, ?3, ?4, 'integrating', ?5, ?6, ?5, 1)",
            params![uuid_blob(id), uuid_blob(workspace), name, base, now, actor.to_string()],
        )
        .map_err(map_err)?;
        board_lanes(&tx, &name, lanes, &[])?;
        let train = train_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(train)
    }

    /// Change a train: move it, rebase it, record what it pushed, and put
    /// lanes on or off it, in one write. A settled train (landed or dropped)
    /// takes no more moves and no new SHA.
    pub fn set_train(&self, train: Uuid, update: &TrainUpdate, _actor: Actor) -> Result<Train> {
        let base = update.base.as_deref().map(str::trim).map(str::to_string);
        if base.as_ref().is_some_and(|b| b.chars().count() > BASE_MAX) {
            return Err(invalid("base"));
        }
        let sha = update.sha.as_deref().map(clean_sha).transpose()?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before = train_in(&tx, train)?;
        check_lanes(&tx, before.workspace_id, &update.add_lanes)?;
        check_lanes(&tx, before.workspace_id, &update.remove_lanes)?;
        let to = match (update.state, &sha) {
            (Some(state), _) => state,
            (None, Some(_)) => TrainState::Pushed,
            (None, None) => before.state,
        };
        if before.state.is_settled() && (to != before.state || sha.is_some()) {
            return Err(invalid("train_settled"));
        }
        if matches!(to, TrainState::Pushed | TrainState::Green | TrainState::Red)
            && sha.is_none()
            && before.pushed_sha.is_none()
        {
            return Err(invalid("sha"));
        }
        let now = now_millis();
        let moved = to != before.state;
        tx.execute(
            "UPDATE board_trains SET state = ?2, base = ?3, pushed_sha = ?4, state_since = ?5,
                    landed_at = CASE WHEN ?2 = 'landed' THEN coalesce(landed_at, ?6) ELSE landed_at END,
                    resource_version = resource_version + 1
              WHERE id = ?1",
            params![
                uuid_blob(train),
                to.as_str(),
                base.unwrap_or_else(|| before.base.clone()),
                sha.or_else(|| before.pushed_sha.clone()),
                if moved { now } else { before.state_since },
                now,
            ],
        )
        .map_err(map_err)?;
        board_lanes(&tx, &before.name, &update.add_lanes, &update.remove_lanes)?;
        // A dropped train lets its lanes go, so none still says it's in it
        // (review train-1005c L1). A landed one keeps them, as its record.
        if moved && to == TrainState::Dropped {
            tx.execute(
                "UPDATE lanes SET train = NULL, resource_version = resource_version + 1
                  WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE",
                params![uuid_blob(before.workspace_id), before.name],
            )
            .map_err(map_err)?;
        }
        let after = train_in(&tx, train)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// A train by id.
    pub fn train(&self, train: Uuid) -> Result<Train> {
        train_in(&self.conn(), train)
    }

    /// A board's train by name, ignoring case.
    pub fn train_named(&self, workspace: Uuid, name: &str) -> Result<Train> {
        self.conn()
            .query_row(
                &format!("SELECT {COLS} FROM board_trains WHERE workspace_id = ?1 AND name = ?2 COLLATE NOCASE"),
                params![uuid_blob(workspace), name.trim()],
                row_to_train,
            )
            .optional()
            .map_err(map_err)?
            .ok_or(DomainError::NotFound)
    }

    /// Every board's trains whose CI the runner reads: pushed, green or red,
    /// with a SHA. What the runner's CI watch walks (`daemon/src/ci_watch.rs`).
    pub fn trains_following_ci(&self) -> Result<Vec<Train>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {COLS} FROM board_trains
                  WHERE state IN ('pushed', 'green', 'red') AND pushed_sha IS NOT NULL ORDER BY created_at, id"
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], row_to_train)
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        Ok(rows)
    }
}

/// Move `workspace`'s trains that follow CI on `subject` to what it now says:
/// green when it passed, red when it failed, pushed while it runs again. The
/// trains moved, by id.
pub(crate) fn follow_ci(conn: &Connection, workspace: Uuid, subject: &str, status: crate::board_ci::CiStatus) -> Result<Vec<Uuid>> {
    use crate::board_ci::CiStatus;
    let to = match status {
        CiStatus::Passed => TrainState::Green,
        CiStatus::Failed => TrainState::Red,
        CiStatus::Running | CiStatus::Queued => TrainState::Pushed,
        // Nothing ran yet, GitHub couldn't be asked, or a newer push canceled
        // its run: the train stays put.
        CiStatus::None | CiStatus::Unknown | CiStatus::Superseded => return Ok(Vec::new()),
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM board_trains
              WHERE workspace_id = ?1 AND state IN ('pushed', 'green', 'red') AND pushed_sha IS NOT NULL"
        ))
        .map_err(map_err)?;
    let trains = stmt
        .query_map(params![uuid_blob(workspace)], row_to_train)
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    let mut moved = Vec::new();
    for train in trains.into_iter().filter(|t| t.ci_subject().as_deref() == Some(subject) && t.state != to) {
        conn.execute(
            "UPDATE board_trains SET state = ?2, state_since = ?3, resource_version = resource_version + 1
              WHERE id = ?1",
            params![uuid_blob(train.id), to.as_str(), now_millis()],
        )
        .map_err(map_err)?;
        moved.push(train.id);
    }
    Ok(moved)
}

/// A board's trains for the plan read: every one not settled, oldest first,
/// then those settled at or after `settled_since_ms`, most recent first.
pub(crate) fn trains_of(conn: &Connection, workspace: Uuid, settled_since_ms: i64) -> Result<Vec<Train>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM board_trains
              WHERE workspace_id = ?1 AND (state NOT IN ('landed', 'dropped') OR state_since >= ?2)
              ORDER BY state IN ('landed', 'dropped'),
                       CASE WHEN state IN ('landed', 'dropped') THEN -state_since ELSE created_at END, id"
        ))
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(workspace), settled_since_ms], row_to_train)
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    Ok(rows)
}

#[cfg(test)]
#[path = "trains_tests.rs"]
mod tests;
