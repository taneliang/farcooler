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
//! | Waiting for lanes | a lane on it isn't ready (anything but `landing`) |
//! | Integrating | its lanes are ready and no SHA is out |
//! | Gating locally | stored `gating` |
//! | Waiting for CI | stored `pushed` |
//! | CI failed | stored `red` |
//! | Ready to land | stored `green`, not yet on main |
//! | Landed, Dropped | stored `landed`, `dropped` |
//! | Stalled | integrating or gating (or waiting) for 2 h with no open agent |
//!
//! **Held.** A train waiting for lanes is *held* when at least one lane on it
//! is ready, another isn't, and it has waited more than `HELD_AFTER_MS`
//! (45 minutes). `held_by` names the lanes that aren't ready. The runner
//! flags it and tells the orchestrator once per hold; moving a lane off is the
//! orchestrator's call (`plan train move`), never the runner's, because the
//! lane has to rebase.
//!
//! **History.** `train_events` (migration 0048, additive like 0027) records
//! each state change, each lane put on or taken off, and each hold told, so a
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
use crate::trains::{Train, TrainState};

/// A train waiting for lanes this long, with one lane ready, is held.
pub const HELD_AFTER_MS: i64 = 45 * 60_000;

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
            -- `state`, `lane_on`, `lane_off` or `held`.
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
    pub now: i64,
}

/// A train's phase, since when, and the lanes holding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub phase: TrainPhase,
    pub since: i64,
    pub held_by: Vec<Uuid>,
}

/// The phase table in the module docs, as one function of its inputs.
pub fn derive(i: &PhaseInput) -> Derived {
    let settled = |phase| Derived { phase, since: i.state_since, held_by: Vec::new() };
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
    let ready = open.len() - blockers.len();
    // The clock starts at the last move of the train itself or of its
    // membership, and no earlier than the oldest lane still not ready: a lane
    // that went back to building waits from then.
    let floor = i.state_since.max(i.last_lane_on.unwrap_or(0));
    let (phase, since) = if i.state == TrainState::Gating {
        (TrainPhase::GatingLocally, i.state_since)
    } else if !blockers.is_empty() {
        let oldest = blockers.iter().map(|l| l.state_since).min().unwrap_or(floor);
        (TrainPhase::WaitingForLanes, floor.max(oldest))
    } else {
        let started = if i.agent_open { i.agent_started_at.unwrap_or(0) } else { 0 };
        (TrainPhase::Integrating, floor.max(started))
    };
    let held = phase == TrainPhase::WaitingForLanes && ready > 0 && i.now - since > HELD_AFTER_MS;
    let held_by = if held { blockers.iter().map(|l| l.id).collect() } else { Vec::new() };
    let stalled = !i.agent_open && i.now - since >= STALLED_AFTER_MS;
    Derived { phase: if stalled { TrainPhase::Stalled } else { phase }, since, held_by }
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

/// A hold the orchestrator hasn't been told of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldTrain {
    pub train: Train,
    pub held_by: Vec<Uuid>,
    /// When the wait began: what tells one hold from the next.
    pub since: i64,
    /// The names of the blocking lanes, in `held_by`'s order.
    pub names: Vec<String>,
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

    /// `workspace`'s held trains whose hold the orchestrator hasn't been told
    /// of: held now, and no `held` event for this wait (`since`) yet.
    pub fn holds_untold(&self, workspace: Uuid) -> Result<Vec<HeldTrain>> {
        let plan = self.plan(workspace, i64::MAX)?;
        let mut out = Vec::new();
        for view in plan.trains.iter().filter(|t| !t.held_by.is_empty()) {
            let told = self.train_events(view.train.id)?.iter().any(|e| {
                e.kind == "held" && e.extra.get("since").and_then(serde_json::Value::as_i64) == Some(view.phase_since)
            });
            if told {
                continue;
            }
            let names = view
                .held_by
                .iter()
                .map(|id| plan.lanes.iter().find(|l| l.lane.id == *id).map_or_else(|| id.to_string(), |l| l.lane.name.clone()))
                .collect();
            out.push(HeldTrain { train: view.train.clone(), held_by: view.held_by.clone(), since: view.phase_since, names });
        }
        Ok(out)
    }

    /// Record that the orchestrator is being told `held`. Written before the
    /// message is, so a crash between the two loses a notice and never
    /// repeats one. False when this hold was already recorded.
    pub fn record_hold_told(&self, held: &HeldTrain) -> Result<bool> {
        let conn = self.conn();
        let id = held.train.id;
        let again = {
            let mut stmt = conn
                .prepare("SELECT extra FROM train_events WHERE train_id = ?1 AND kind = 'held'")
                .map_err(map_err)?;
            let rows = stmt
                .query_map(params![uuid_blob(id)], |r| r.get::<_, String>(0))
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
            rows.iter().any(|raw| {
                serde_json::from_str::<serde_json::Value>(raw)
                    .ok()
                    .and_then(|v| v.get("since").and_then(serde_json::Value::as_i64))
                    == Some(held.since)
            })
        };
        if again {
            return Ok(false);
        }
        let extra = serde_json::json!({
            "since": held.since,
            "held_by": held.held_by.iter().map(Uuid::to_string).collect::<Vec<_>>(),
            "lanes": held.names,
        });
        train_event(&conn, id, "held", Actor::Runner, None, extra)?;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "train_phase_tests.rs"]
mod tests;
