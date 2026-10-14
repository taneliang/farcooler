//! The collecting train (ov-520, ov-523): one train per board that is always
//! collecting the lanes that are ready to land.
//!
//! **One collecting train, by flag.** A board that uses trains always has
//! exactly one train that is `integrating` and has `collecting` set
//! (migration 0051). A lane that passes review (enters `landing`) joins it in
//! the same write, and when none exists it is made, named `train-N` for the next
//! free number. A train the coordinator starts by hand, gives an agent or moves
//! on stops collecting at once, and a new collecting train opens behind it.
//!
//! **A lane that leaves landing comes off.** A lane that goes back to fixing, or
//! is dropped, comes off a train whose SHA is not out yet (`integrating`,
//! `gating`) in the same write. On a pushed, green or red train it stays, since
//! the push carries its commits, and its timeline says so. The train goes on
//! with the lanes still ready: a train is never held for a lane.
//!
//! **A lane taken off by hand stays off.** `held_off` marks a lane that
//! `--remove-lane`, `plan train move` or Move to Next Train took off a train. The
//! scan and the join skip it until it leaves landing and comes back through
//! review.
//!
//! **Departure retries.** Whenever a lane joins, the runner's scan runs, main's
//! CI is recorded, or a train settles, the collecting train departs (`gating`)
//! if a lane on it is ready, no train is ahead of it, and main's CI isn't red
//! (running or unread doesn't hold it). A new collecting train opens behind it,
//! and the orchestrator is told once (`departures_untold`). A train that isn't
//! collecting never departs on its own.
//!
//! No other table is touched: migration 0051 adds the `collecting` and `held_off`
//! columns, defaulted, to the layer's own tables.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::board_ci::{CiStatus, MAIN_SUBJECT, ci_of};
use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;
use crate::train_phase::train_event;
use crate::trains::{COLS, Train, TrainState, board_lanes, row_to_train, train_in, train_number};

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

/// Lanes in `landing` on no live train and not taken off by hand: unlabeled, or
/// labeled with a train that is gone or settled. Both the gather and the
/// runner's scan read it.
const UNCOLLECTED: &str = "l.state = 'landing' AND l.held_off = 0 AND NOT EXISTS (
        SELECT 1 FROM board_trains t
         WHERE t.workspace_id = l.workspace_id AND t.name = l.train COLLATE NOCASE
           AND t.state NOT IN ('landed', 'dropped'))";

/// The board's collecting train.
pub(crate) fn open_train(conn: &Connection, workspace: Uuid) -> Result<Option<Train>> {
    conn.query_row(
        &format!(
            "SELECT {COLS} FROM board_trains WHERE workspace_id = ?1 AND state = 'integrating' AND collecting = 1
              ORDER BY created_at, id LIMIT 1"
        ),
        params![uuid_blob(workspace)],
        row_to_train,
    )
    .optional()
    .map_err(map_err)
}

/// The next free `train-N`: one past the highest number any train on the board
/// has had, `integ-N` names included.
fn next_name(conn: &Connection, workspace: Uuid) -> Result<String> {
    let mut stmt = conn.prepare("SELECT name FROM board_trains WHERE workspace_id = ?1").map_err(map_err)?;
    let names = stmt
        .query_map(params![uuid_blob(workspace)], |r| r.get::<_, String>(0))
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    let mut n = names.iter().filter_map(|n| train_number(n)).max().unwrap_or(0) + 1;
    while names.iter().any(|x| x.eq_ignore_ascii_case(&format!("train-{n}"))) {
        n += 1;
    }
    Ok(format!("train-{n}"))
}

/// The board's open train, made when it has none.
pub(crate) fn ensure_open(tx: &Transaction, workspace: Uuid, actor: Actor) -> Result<Train> {
    if let Some(open) = open_train(tx, workspace)? {
        return Ok(open);
    }
    let id = Uuid::now_v7();
    let now = now_millis();
    tx.execute(
        "INSERT INTO board_trains (id, workspace_id, name, base, state, state_since, actor, created_at, resource_version, collecting)
         VALUES (?1, ?2, ?3, '', 'integrating', ?4, ?5, ?4, 1, 1)",
        params![uuid_blob(id), uuid_blob(workspace), next_name(tx, workspace)?, now, actor.to_string()],
    )
    .map_err(map_err)?;
    train_event(tx, id, "state", actor, None, serde_json::json!({ "from": null, "to": "integrating", "open": true }))?;
    train_in(tx, id)
}

/// The state of the train `label` names on the board, if it hasn't landed or
/// been dropped.
pub(crate) fn live_train_state(conn: &Connection, workspace: Uuid, label: Option<&str>) -> Result<Option<TrainState>> {
    let Some(label) = label.filter(|l| !l.is_empty()) else { return Ok(None) };
    let state: Option<String> = conn
        .query_row(
            "SELECT state FROM board_trains WHERE workspace_id = ?1 AND name = ?2 COLLATE NOCASE
                AND state NOT IN ('landed', 'dropped')",
            params![uuid_blob(workspace), label],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_err)?;
    Ok(state.as_deref().and_then(TrainState::parse))
}

/// Whether `label` names a train on the board that hasn't landed or been
/// dropped.
pub(crate) fn names_live_train(conn: &Connection, workspace: Uuid, label: Option<&str>) -> Result<bool> {
    live_train_state(conn, workspace, label).map(|s| s.is_some())
}

/// Put `lanes` on the open train, making it if need be.
pub(crate) fn join(tx: &Transaction, workspace: Uuid, lanes: &[Uuid], actor: Actor) -> Result<()> {
    let open = ensure_open(tx, workspace, actor)?;
    board_lanes(tx, &open, lanes, &[], actor)?;
    depart(tx, workspace, actor).map(|_| ())
}

/// A lane left `landing` while on a live train: say so on its timeline. The
/// caller clears the lane's label, and `label_moved` writes the train's side.
pub(crate) fn note_left(conn: &Connection, lane: Uuid, train: &str, to: &str, actor: Actor) -> Result<()> {
    let called = train_number(train).map_or_else(|| train.to_string(), |n| format!("Train {n}"));
    let extra = serde_json::json!({ "train": train, "left_landing": true, "to_state": to });
    crate::plan::event(conn, crate::plan::Subject::Lane(lane), "train", actor, &format!("Taken off {called}: it left landing."), extra)
}

/// A train the runner sent off on its own and that has lost every lane it
/// carried has nothing to check: it is dropped, so it isn't ahead of the
/// collecting train for good. Only a train whose last state event is the
/// runner's departure, with no agent and no SHA: a train the coordinator gated
/// by hand is theirs to finish, and its `--sha` still works. True when it dropped.
pub(crate) fn drop_if_emptied(tx: &Connection, train: Uuid, actor: Actor) -> Result<bool> {
    let t = train_in(tx, train)?;
    if t.state != TrainState::Gating || t.agent.is_some() || t.pushed_sha.is_some() {
        return Ok(false);
    }
    let last: Option<String> = tx
        .query_row(
            "SELECT extra FROM train_events WHERE train_id = ?1 AND kind = 'state' ORDER BY at DESC, rowid DESC LIMIT 1",
            params![uuid_blob(train)],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_err)?;
    let by_runner = last
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .is_some_and(|v| v["departed"] == true);
    if !by_runner {
        return Ok(false);
    }
    let carried: i64 = tx
        .query_row(
            "SELECT count(*) FROM lanes WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE AND state NOT IN ('landed', 'dropped')",
            params![uuid_blob(t.workspace_id), t.name],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    if carried > 0 {
        return Ok(false);
    }
    tx.execute(
        "UPDATE board_trains SET state = 'dropped', collecting = 0, state_since = ?2, resource_version = resource_version + 1 WHERE id = ?1",
        params![uuid_blob(train), now_millis()],
    )
    .map_err(map_err)?;
    train_event(tx, train, "state", actor, None, serde_json::json!({ "from": "gating", "to": "dropped", "empty": true }))?;
    Ok(true)
}

/// A lane left landing but its train's push carries its commits, so it stays on
/// the train: say so on its timeline.
pub(crate) fn note_kept(conn: &Connection, lane: Uuid, train: &str, actor: Actor) -> Result<()> {
    let called = train_number(train).map_or_else(|| train.to_string(), |n| format!("Train {n}"));
    let extra = serde_json::json!({ "train": train, "left_landing": true, "kept": true });
    crate::plan::event(
        conn,
        crate::plan::Subject::Lane(lane),
        "train",
        actor,
        &format!("Stays on {called}: its commits are in the train's push."),
        extra,
    )
}

/// Whether a train on the board is ahead of the collecting one: one with its
/// checks started or out, or one the coordinator started by hand and is still
/// assembling.
pub(crate) fn train_ahead(conn: &Connection, workspace: Uuid) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM board_trains WHERE workspace_id = ?1
            AND (state IN ('gating', 'pushed', 'green', 'red') OR (state = 'integrating' AND collecting = 0))",
        params![uuid_blob(workspace)],
        |_| Ok(()),
    )
    .optional()
    .map_err(map_err)
    .map(|found| found.is_some())
}

/// The collecting train departs for its checks when it can: a lane on it is
/// ready, no train is ahead and main's CI isn't red (running or unread doesn't
/// hold it, since an unreadable CI would hold it for good). A new collecting
/// train opens behind it. Safe to ask again: it does nothing when it can't. The
/// train that left, if one did.
pub(crate) fn depart(tx: &Transaction, workspace: Uuid, actor: Actor) -> Result<Option<Train>> {
    let Some(open) = open_train(tx, workspace)? else { return Ok(None) };
    if train_ahead(tx, workspace)? {
        return Ok(None);
    }
    if ci_of(tx, workspace)?.iter().any(|c| c.subject == MAIN_SUBJECT && c.status == CiStatus::Failed) {
        return Ok(None);
    }
    let ready: i64 = tx
        .query_row(
            "SELECT count(*) FROM lanes WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE AND state = 'landing'",
            params![uuid_blob(workspace), open.name],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    if ready == 0 {
        return Ok(None);
    }
    tx.execute(
        "UPDATE board_trains SET state = 'gating', collecting = 0, state_since = ?2, resource_version = resource_version + 1
          WHERE id = ?1",
        params![uuid_blob(open.id), now_millis()],
    )
    .map_err(map_err)?;
    train_event(tx, open.id, "state", actor, None, serde_json::json!({ "from": "integrating", "to": "gating", "departed": true }))?;
    ensure_open(tx, workspace, actor)?;
    Ok(Some(train_in(tx, open.id)?))
}

/// A train that left for its checks and whose orchestrator hasn't been told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departed {
    pub train: Train,
    /// When it left: what tells one departure from the next.
    pub at: i64,
    /// The names of the lanes on it, in lane order.
    pub lanes: Vec<String>,
    /// The cards a notice can be filed on: the train's own, then its lanes'.
    pub cards: Vec<Uuid>,
}

/// What one scan of a board did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    /// The lanes put on the collecting train.
    pub joined: Vec<Uuid>,
    /// The train that left for its checks, if one did.
    pub departed: Option<Uuid>,
}

impl Store {
    /// Put every lane of `workspace` that is ready to land, on no live train and
    /// not taken off by hand on the collecting train, and let it depart if it
    /// can. Lanes that reached `landing` through a build that predates the
    /// collecting train are caught up here, and a departure that couldn't
    /// happen when it was asked for is retried.
    pub fn collect_ready_lanes(&self, workspace: Uuid, actor: Actor) -> Result<Collected> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let lanes = {
            let mut stmt = tx
                .prepare(&format!("SELECT l.id FROM lanes l WHERE l.workspace_id = ?1 AND {UNCOLLECTED} ORDER BY l.state_since, l.id"))
                .map_err(map_err)?;
            stmt.query_map(params![uuid_blob(workspace)], |r| get_uuid(r, 0))
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?
        };
        if !lanes.is_empty() {
            join(&tx, workspace, &lanes, actor)?;
        }
        let departed = depart(&tx, workspace, actor)?;
        tx.commit().map_err(map_err)?;
        Ok(Collected { joined: lanes, departed: departed.map(|t| t.id) })
    }

    /// The boards the runner's scan has work on: a lane ready to land that is on
    /// no live train, or a collecting train with a ready lane on it.
    pub fn workspaces_to_collect(&self) -> Result<Vec<Uuid>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT l.workspace_id FROM lanes l WHERE {UNCOLLECTED}
                 UNION
                 SELECT t.workspace_id FROM board_trains t
                  WHERE t.state = 'integrating' AND t.collecting = 1
                    AND EXISTS (SELECT 1 FROM lanes l WHERE l.workspace_id = t.workspace_id
                                  AND l.train = t.name COLLATE NOCASE AND l.state = 'landing')"
            ))
            .map_err(map_err)?;
        stmt.query_map([], |r| get_uuid(r, 0))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)
    }

    /// `workspace`'s trains that left for their checks by themselves and whose
    /// orchestrator hasn't been told: still gating, with a `departed` event and no
    /// `told` event for it.
    pub fn departures_untold(&self, workspace: Uuid) -> Result<Vec<Departed>> {
        departures_in(&self.conn(), workspace)
    }

    /// Record that the orchestrator is being told of `departed`. Written before the
    /// message is, so a crash between the two loses a notice and never repeats
    /// one. The check and the write are one transaction, so callers that race
    /// get one `true` between them. False when it was already recorded.
    pub fn record_departure_told(&self, departed: &Departed) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let untold = departures_in(&tx, departed.train.workspace_id)?;
        if !untold.iter().any(|d| d.train.id == departed.train.id && d.at == departed.at) {
            return Ok(false);
        }
        train_event(&tx, departed.train.id, "told", Actor::Runner, None, serde_json::json!({ "departed_at": departed.at }))?;
        tx.commit().map_err(map_err)?;
        Ok(true)
    }

    /// The boards with a train that left for its checks.
    pub fn workspaces_with_gating_trains(&self) -> Result<Vec<Uuid>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT workspace_id FROM board_trains WHERE state = 'gating'").map_err(map_err)?;
        stmt.query_map([], |r| get_uuid(r, 0))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)
    }
}

/// The departures of `workspace` not yet told, read on `conn`.
fn departures_in(conn: &Connection, workspace: Uuid) -> Result<Vec<Departed>> {
    {
        let mut stmt = conn
            .prepare(&format!("SELECT {COLS} FROM board_trains WHERE workspace_id = ?1 AND state = 'gating' ORDER BY created_at, id"))
            .map_err(map_err)?;
        let trains = stmt
            .query_map(params![uuid_blob(workspace)], row_to_train)
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        drop(stmt);
        let mut out = Vec::new();
        for train in trains {
            let (mut left, mut told) = (None, Vec::new());
            let mut ev = conn
                .prepare("SELECT at, kind, extra FROM train_events WHERE train_id = ?1 AND kind IN ('state', 'told') ORDER BY at, rowid")
                .map_err(map_err)?;
            let rows = ev
                .query_map(params![uuid_blob(train.id)], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
            for (at, kind, extra) in rows {
                let extra: serde_json::Value = serde_json::from_str(&extra).unwrap_or_default();
                match kind.as_str() {
                    "state" if extra["departed"] == true => left = Some(at),
                    "told" => told.push(extra["departed_at"].as_i64().unwrap_or(0)),
                    _ => {}
                }
            }
            let Some(at) = left.filter(|at| !told.contains(at)) else { continue };
            let mut ls = conn
                .prepare("SELECT id, name FROM lanes WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE AND state NOT IN ('landed', 'dropped') ORDER BY created_at, id")
                .map_err(map_err)?;
            let lanes: Vec<(Uuid, String)> = ls
                .query_map(params![uuid_blob(workspace), train.name], |r| Ok((get_uuid(r, 0)?, r.get(1)?)))
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
            let mut cards: Vec<Uuid> = train.card_id.into_iter().collect();
            for (id, _) in &lanes {
                let mut cs = conn.prepare("SELECT task_id FROM lane_tasks WHERE lane_id = ?1").map_err(map_err)?;
                let found = cs
                    .query_map(params![uuid_blob(*id)], |r| get_uuid(r, 0))
                    .map_err(map_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(map_err)?;
                for c in found {
                    if !cards.contains(&c) {
                        cards.push(c);
                    }
                }
            }
            out.push(Departed { train, at, lanes: lanes.into_iter().map(|(_, n)| n).collect(), cards });
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "open_train_tests.rs"]
mod tests;
