//! What a train is doing, in one word every surface reads (ov-503), and the
//! record of how it got there.
//!
//! A train's stored `state` was written by hand and says little: a train that
//! was `integrating` could be waiting on one lane, working with an agent, or
//! forgotten. The **phase** is derived on the runner, on every plan read, from
//! the state, the lanes on the train, its agent and the clock, so the Mac, the
//! phones and the CLI agree. Nothing here is stored but the history.
//!
//! | Phase | When |
//! |---|---|
//! | Waiting for the earlier landing | the collecting train, while a train ahead of it is still on its way to main |
//! | Waiting for main | the collecting train with a lane ready, nothing ahead and main's CI red |
//! | Waiting for lanes | the collecting train with nothing ahead and no lane ready, or a lane on a train started by hand isn't ready (anything but `landing`) |
//! | Integrating | its lanes are ready and no SHA is out |
//! | Gating locally | stored `gating` |
//! | Waiting for CI | stored `pushed` |
//! | CI failed | stored `red` |
//! | Ready to land | stored `green`, not yet on main |
//! | Landed, Dropped | stored `landed`, `dropped` |
//! | Stalled | integrating or gating (or waiting) for 2 h with no open agent |
//!
//! **Never held (ov-523).** A train isn't held for a lane. A lane that leaves
//! `landing` while on an unlanded train comes off it in the same write
//! (`open_train.rs`), so no lane on the open train is ever a reason to wait,
//! and nobody is told that a train is held. (Trains 86 and 89 were held for
//! two hours before this.)
//!
//! **History.** `train_events` (migration 0048, additive like 0027) records
//! each state change and each lane put on or taken off, so a
//! lane that moved can say "from Train 86 · 40 min ago". `plan_events` can't
//! take a train because of its theme-xor-lane CHECK.
//!
//! Part of the plan layer and as removable: the table points at `board_trains`
//! by cascade, nothing points at it, and no old table gains a column.

use rusqlite::{Connection, Transaction, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::plan::LaneState;
use crate::store::Store;
use crate::tasks::now_millis;
use crate::trains::TrainState;

/// A train with no open agent and no move for this long is stalled.
pub const STALLED_AFTER_MS: i64 = 2 * 60 * 60_000;

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

/// What a train is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainPhase {
    /// The open train, collecting ready lanes while the train ahead is still
    /// on its way to main (ov-520).
    WaitingForPreviousTrain,
    /// The collecting train, with a lane ready and nothing ahead, while main's
    /// CI is red.
    WaitingForMain,
    WaitingForLanes,
    Integrating,
    GatingLocally,
    WaitingForCi,
    CiFailed,
    /// Green, and not (yet) on main: a pull-request workspace's train, which
    /// the orchestrator lands by hand.
    ReadyToLand,
    Landed,
    Stalled,
    Dropped,
}

/// A lane on a train, as the phase reads it.
#[derive(Debug, Clone, Copy)]
pub struct LaneOn {
    pub id: Uuid,
    pub state: LaneState,
    pub state_since: i64,
    /// When it last left `landing` (a lane's own timeline says), if it has:
    /// the moment a lane that blocks the train stopped being ready, which its
    /// later steps (fixing to review and back) do not move.
    pub left_ready_at: Option<i64>,
}

/// What the phase of a train is read from.
#[derive(Debug, Clone)]
pub struct PhaseInput<'a> {
    pub state: TrainState,
    pub state_since: i64,
    /// An agent is recorded on the train and hasn't finished.
    pub agent_open: bool,
    pub agent_started_at: Option<i64>,
    /// When a lane was last put on the train.
    pub last_lane_on: Option<i64>,
    pub lanes: &'a [LaneOn],
    /// The board's collecting train (ov-520), not one started by hand.
    pub collecting: bool,
    /// Main's CI reads failed.
    pub main_red: bool,
    /// Another train on the board is ahead of the collecting one: gating,
    /// pushed, green or red, or one started by hand and still integrating.
    pub ahead: bool,
    pub now: i64,
}

/// A train's phase, and since when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub phase: TrainPhase,
    pub since: i64,
}

/// The phase table in the module docs, as one function of its inputs.
pub fn derive(i: &PhaseInput) -> Derived {
    let settled = |phase| Derived { phase, since: i.state_since };
    match i.state {
        TrainState::Landed => return settled(TrainPhase::Landed),
        TrainState::Dropped => return settled(TrainPhase::Dropped),
        TrainState::Pushed => return settled(TrainPhase::WaitingForCi),
        TrainState::Red => return settled(TrainPhase::CiFailed),
        TrainState::Green => return settled(TrainPhase::ReadyToLand),
        TrainState::Integrating | TrainState::Gating => {}
    }
    let open: Vec<&LaneOn> = i.lanes.iter().filter(|l| !l.state.is_closed()).collect();
    let blockers: Vec<&LaneOn> = open.iter().copied().filter(|l| l.state != LaneState::Landing).collect();
    // The clock starts at the last move of the train itself or of its
    // membership, and no earlier than the first moment a lane still blocking
    // it stopped being ready. A blocker that goes on from fixing to review
    // and back is the same wait. Once every lane is ready, the last one to
    // become ready starts Integrating's clock.
    let floor = i.state_since.max(i.last_lane_on.unwrap_or(0));
    // The collecting train waits behind the train ahead, or for a lane to
    // depart with (it leaves at once when nothing is ahead, so waiting with
    // nothing ahead means no lane is ready, or main is red). It is waiting by
    // design, so it is never stalled.
    if i.collecting && i.state == TrainState::Integrating && !i.agent_open {
        let ready = open.iter().any(|l| l.state == LaneState::Landing);
        let phase = match (i.ahead, ready && i.main_red) {
            (true, _) => TrainPhase::WaitingForPreviousTrain,
            (false, true) => TrainPhase::WaitingForMain,
            (false, false) => TrainPhase::WaitingForLanes,
        };
        return Derived { phase, since: floor.max(open.iter().map(|l| l.state_since).max().unwrap_or(0)) };
    }
    let (phase, since) = if i.state == TrainState::Gating {
        (TrainPhase::GatingLocally, i.state_since)
    } else if !blockers.is_empty() {
        let oldest = blockers.iter().map(|l| l.left_ready_at.unwrap_or(l.state_since)).min().unwrap_or(floor);
        (TrainPhase::WaitingForLanes, floor.max(oldest))
    } else {
        let started = if i.agent_open { i.agent_started_at.unwrap_or(0) } else { 0 };
        let latest_ready = open.iter().map(|l| l.state_since).max().unwrap_or(0);
        (TrainPhase::Integrating, floor.max(started).max(latest_ready))
    };
    let stalled = !i.agent_open && i.now - since >= STALLED_AFTER_MS;
    Derived { phase: if stalled { TrainPhase::Stalled } else { phase }, since }
}

/// Write one history row for `train`.
pub(crate) fn train_event(
    conn: &Connection,
    train: Uuid,
    kind: &str,
    actor: Actor,
    lane: Option<Uuid>,
    extra: serde_json::Value,
) -> Result<()> {
    conn.execute(
        "INSERT INTO train_events (id, train_id, at, actor, kind, lane_id, extra) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            uuid_blob(Uuid::now_v7()),
            uuid_blob(train),
            now_millis(),
            actor.to_string(),
            kind,
            lane.map(uuid_blob),
            extra.to_string()
        ],
    )
    .map_err(map_err)?;
    Ok(())
}

/// When `lane` last left `landing`, from its own timeline.
pub(crate) fn left_ready_at(conn: &Connection, lane: Uuid) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT max(at) FROM plan_events WHERE lane_id = ?1 AND kind = 'state' AND json_extract(extra, '$.from') = 'landing'",
        params![uuid_blob(lane)],
        |r| r.get(0),
    )
    .map_err(map_err)
}

/// A lane's `train` label changed from `from` to `to` by a lane write (`plan
/// lane set --train`): the same history as the train verbs write.
pub(crate) fn label_moved(
    conn: &Connection,
    workspace: Uuid,
    lane: Uuid,
    from: Option<&str>,
    to: Option<&str>,
    actor: Actor,
) -> Result<()> {
    let id_of = |name: &str| -> Result<Option<Uuid>> {
        use rusqlite::OptionalExtension;
        conn.query_row(
            "SELECT id FROM board_trains WHERE workspace_id = ?1 AND name = ?2 COLLATE NOCASE",
            params![uuid_blob(workspace), name],
            |r| get_uuid(r, 0),
        )
        .optional()
        .map_err(map_err)
    };
    let same = match (from, to) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    };
    if same {
        return Ok(());
    }
    if let Some(train) = from.filter(|n| !n.is_empty()).map(id_of).transpose()?.flatten() {
        train_event(conn, train, "lane_off", actor, Some(lane), serde_json::json!({ "to": to }))?;
    }
    if let Some(train) = to.filter(|n| !n.is_empty()).map(id_of).transpose()?.flatten() {
        train_event(conn, train, "lane_on", actor, Some(lane), serde_json::json!({ "from": from }))?;
    }
    Ok(())
}

/// When a lane was last put on `train`, from its history.
pub(crate) fn last_lane_on(conn: &Connection, train: Uuid) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT max(at) FROM train_events WHERE train_id = ?1 AND kind = 'lane_on'",
        params![uuid_blob(train)],
        |r| r.get(0),
    )
    .map_err(map_err)
}

/// One row of a train's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainEvent {
    pub train_id: Uuid,
    pub at: i64,
    pub actor: String,
    pub kind: String,
    pub lane_id: Option<Uuid>,
    pub extra: serde_json::Value,
}

impl Store {
    /// A train's history, oldest first.
    pub fn train_events(&self, train: Uuid) -> Result<Vec<TrainEvent>> {
        self.events_where("train_id = ?1", train)
    }

    /// The history rows that name `lane`, oldest first: where it was put on a
    /// train and where it came off.
    pub fn lane_train_events(&self, lane: Uuid) -> Result<Vec<TrainEvent>> {
        self.events_where("lane_id = ?1", lane)
    }

    fn events_where(&self, clause: &str, id: Uuid) -> Result<Vec<TrainEvent>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT train_id, at, actor, kind, lane_id, extra FROM train_events WHERE {clause} ORDER BY at, rowid"
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![uuid_blob(id)], |r| {
                Ok(TrainEvent {
                    train_id: get_uuid(r, 0)?,
                    at: r.get(1)?,
                    actor: r.get(2)?,
                    kind: r.get(3)?,
                    lane_id: r.get::<_, Option<Vec<u8>>>(4)?.and_then(|b| Uuid::from_slice(&b).ok()),
                    extra: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                })
            })
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        Ok(rows)
    }
}

#[cfg(test)]
#[path = "train_phase_tests.rs"]
mod tests;
