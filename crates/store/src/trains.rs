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
//!
//! # What removing trains would touch beyond these files
//!
//! Pages' CI references (ov-306) read `board_ci` through `Plan.ci`, so they
//! share this migration. Withdrawing `board_trains` degrades cleanly: such a
//! reference draws as its name. Deleting the code also means editing
//! `rpc_pages.rs` (its `ci_watch::kick` and `ci_subjects`) and keeping
//! `page_doc`'s `ci` target, or pages that name CI stop parsing (review
//! train-1005c L6).

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::plan::{LaneState, Subject, board_exists, check_cards, clean_title, event};
use crate::workers::HARNESSES;
use crate::store::Store;
use crate::tasks::now_millis;
use crate::train_phase::train_event;

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
    /// What the orchestrator wrote for people (ov-462); empty when it hasn't.
    pub title: String,
    /// The card the integrating agent works for the train (ov-461).
    pub card_id: Option<Uuid>,
    /// The agent that integrates it (ov-461), recorded on the train and not as
    /// a lane of the train's own name.
    pub agent: Option<TrainAgent>,
    /// The board's collecting train (ov-520, migration 0051): ready lanes join
    /// it, and it departs by itself. At most one per board. A train started by
    /// hand or given an agent never collects.
    pub collecting: bool,
}

/// The agent integrating a train.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainAgent {
    /// One of `workers::HARNESSES`.
    pub harness: String,
    pub agent_id: String,
    pub model: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// An agent to record on a train, or to fill in what is new about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainAgentRecord {
    pub harness: String,
    pub agent_id: String,
    pub model: Option<String>,
    /// It finished. Recording it again without this reopens it.
    pub ended: bool,
}

/// The number in a name like `train-72` or `integ-72`: what a person calls the
/// train ("Train 72"). `None` for any other name.
pub fn train_number(name: &str) -> Option<u32> {
    let lower = name.trim().to_ascii_lowercase();
    let digits = lower.strip_prefix("train-").or_else(|| lower.strip_prefix("integ-"))?;
    if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
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
    pub title: String,
    pub card: Option<Uuid>,
    pub agent: Option<TrainAgentRecord>,
}

/// What `set_train` changes; each `None` or empty list leaves it alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrainUpdate {
    pub state: Option<TrainState>,
    pub base: Option<String>,
    /// An empty string takes the title away; the plan then reads "Train N".
    pub title: Option<String>,
    pub card: Option<Uuid>,
    pub agent: Option<TrainAgentRecord>,
    /// A new pushed SHA. Without a `state`, a train integrating or gating, or
    /// one that was green or red, moves to `pushed`: CI on it hasn't run yet.
    pub sha: Option<String>,
    /// Lanes to put on it, and lanes to take off.
    pub add_lanes: Vec<Uuid>,
    pub remove_lanes: Vec<Uuid>,
    /// Make the write only if the train is still in this state with this
    /// pushed SHA, else refuse it as `train_changed`: how the runner lands a
    /// train it read as green without landing one that moved since.
    pub expect: Option<(TrainState, String)>,
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

pub(crate) const COLS: &str = "id, workspace_id, name, base, pushed_sha, state, state_since, actor, created_at, landed_at,
     resource_version, title, card_id, agent_harness, agent_id, agent_model, agent_started_at, agent_ended_at, collecting";

pub(crate) fn row_to_train(r: &rusqlite::Row) -> rusqlite::Result<Train> {
    let state: String = r.get(5)?;
    Ok(Train {
        collecting: r.get::<_, i64>(18)? != 0,
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
        title: r.get(11)?,
        card_id: r.get::<_, Option<Vec<u8>>>(12)?.and_then(|b| Uuid::from_slice(&b).ok()),
        agent: {
            let harness: String = r.get(13)?;
            let agent_id: String = r.get(14)?;
            if agent_id.is_empty() {
                None
            } else {
                Some(TrainAgent {
                    harness,
                    agent_id,
                    model: r.get(15)?,
                    started_at: r.get::<_, Option<i64>>(16)?.unwrap_or_default(),
                    ended_at: r.get(17)?,
                })
            }
        },
    })
}

/// A recorded agent, checked: its harness is one the board takes, its id is
/// set.
fn clean_agent(agent: &TrainAgentRecord) -> Result<(String, String, Option<String>)> {
    if !HARNESSES.contains(&agent.harness.as_str()) {
        return Err(invalid("harness"));
    }
    let id = agent.agent_id.trim();
    if id.is_empty() || id.chars().count() > NAME_MAX * 2 {
        return Err(invalid("agent_id"));
    }
    let model = agent.model.as_deref().map(str::trim).map(str::to_string);
    if model.as_ref().is_some_and(|m| m.chars().count() > NAME_MAX) {
        return Err(invalid("model"));
    }
    Ok((agent.harness.clone(), id.to_string(), model))
}

pub(crate) fn train_in(conn: &Connection, id: Uuid) -> Result<Train> {
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

/// Write what a train carries beside its state: its title, its card and the
/// agent integrating it (ov-461, ov-462). Each `None` leaves a field alone. The
/// same agent id again updates it (and `ended` marks it finished, or reopens
/// it); another id replaces it.
fn write_extras(
    conn: &Connection,
    train: Uuid,
    title: Option<&str>,
    card: Option<Uuid>,
    agent: Option<&(String, String, Option<String>)>,
    ended: bool,
    now: i64,
) -> Result<()> {
    if let Some(title) = title {
        conn.execute("UPDATE board_trains SET title = ?2 WHERE id = ?1", params![uuid_blob(train), title]).map_err(map_err)?;
    }
    if let Some(card) = card {
        conn.execute("UPDATE board_trains SET card_id = ?2 WHERE id = ?1", params![uuid_blob(train), uuid_blob(card)])
            .map_err(map_err)?;
    }
    if let Some((harness, id, model)) = agent {
        let known = train_in(conn, train)?.agent;
        match known {
            Some(a) if a.harness == *harness && a.agent_id == *id => {
                let ended_at = if ended { Some(a.ended_at.unwrap_or(now)) } else { None };
                conn.execute(
                    "UPDATE board_trains SET agent_model = coalesce(?2, agent_model), agent_ended_at = ?3 WHERE id = ?1",
                    params![uuid_blob(train), model, ended_at],
                )
                .map_err(map_err)?;
            }
            _ => {
                conn.execute(
                    "UPDATE board_trains SET agent_harness = ?2, agent_id = ?3, agent_model = coalesce(?4, ''),
                            agent_started_at = ?5, agent_ended_at = ?6 WHERE id = ?1",
                    params![uuid_blob(train), harness, id, model, now, ended.then_some(now)],
                )
                .map_err(map_err)?;
            }
        }
    }
    Ok(())
}

/// Put `lanes` on `train`, and take `off` off it (only those that are on it).
/// A lane's train is its own field, so a lane on another train moves to this
/// one. Each change is a row of `train_events` (ov-503): `lane_on` on this
/// train, and `lane_off` on the one a lane left, saying where it went.
pub(crate) fn board_lanes(conn: &Connection, train: &Train, on: &[Uuid], off: &[Uuid], actor: Actor) -> Result<()> {
    let name = train.name.as_str();
    for lane in on {
        let from = train_of_lane(conn, *lane)?;
        if from.as_ref().is_some_and(|f| f.id == train.id) {
            continue;
        }
        conn.execute(
            "UPDATE lanes SET train = ?2, held_off = 0, resource_version = resource_version + 1 WHERE id = ?1",
            params![uuid_blob(*lane), name],
        )
        .map_err(map_err)?;
        if let Some(from) = &from {
            train_event(conn, from.id, "lane_off", actor, Some(*lane), serde_json::json!({ "to": name }))?;
            crate::open_train::drop_if_emptied(conn, from.id, actor)?;
        }
        let extra = serde_json::json!({ "from": from.as_ref().map(|f| f.name.clone()) });
        train_event(conn, train.id, "lane_on", actor, Some(*lane), extra)?;
    }
    for lane in off {
        // Taken off by hand: it stays off until it leaves landing and comes back
        // through review, unless it went to another train in the same write.
        let held = i64::from(!on.contains(lane));
        let was = conn
            .execute(
                "UPDATE lanes SET train = NULL, held_off = ?3, resource_version = resource_version + 1
                  WHERE id = ?1 AND train = ?2 COLLATE NOCASE",
                params![uuid_blob(*lane), name, held],
            )
            .map_err(map_err)?;
        if was > 0 {
            train_event(conn, train.id, "lane_off", actor, Some(*lane), serde_json::json!({ "to": null }))?;
            crate::open_train::drop_if_emptied(conn, train.id, actor)?;
        }
    }
    Ok(())
}

/// The train a lane is on, when its label names one on its board.
fn train_of_lane(conn: &Connection, lane: Uuid) -> Result<Option<Train>> {
    conn.query_row(
        &format!(
            "SELECT {COLS} FROM board_trains t WHERE t.workspace_id = (SELECT workspace_id FROM lanes WHERE id = ?1)
               AND t.name = (SELECT train FROM lanes WHERE id = ?1) COLLATE NOCASE"
        ),
        params![uuid_blob(lane)],
        row_to_train,
    )
    .optional()
    .map_err(map_err)
}

/// Every lane in `lanes` that is not already on `train` must be ready to board:
/// reviewed and waiting to land (`landing`). Planning a batch ahead is Next
/// Up's job, so a queued, building or fixing lane is refused, and so is one
/// still in review (the CLI's `--force` moves that one to `landing` first, as a
/// lane write the timeline shows). A closed lane is refused as well.
fn check_ready(conn: &Connection, train: Option<&str>, lanes: &[Uuid]) -> Result<()> {
    for lane in lanes {
        let (state, on): (String, Option<String>) = conn
            .query_row("SELECT state, train FROM lanes WHERE id = ?1", params![uuid_blob(*lane)], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(map_err)?;
        if train.is_some_and(|t| on.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(t))) {
            continue;
        }
        match LaneState::parse(&state) {
            Some(LaneState::Landing) => {}
            Some(LaneState::Review) => return Err(invalid("lane_in_review")),
            Some(s) if s.is_closed() => return Err(invalid("lane_closed")),
            _ => return Err(invalid("lane_not_ready")),
        }
    }
    Ok(())
}

/// A lane may leave a train only while it is `integrating`: from gating on, the
/// train's commits carry the lane, and a train that lands without it leaves the
/// lane open (`train_moving`). Leaving is `remove_lanes`, or boarding another
/// train (`add_lanes` of a lane that is on a different one).
fn check_can_leave(conn: &Connection, train: &Train, remove: &[Uuid], add: &[Uuid]) -> Result<()> {
    for lane in remove {
        let on: Option<String> =
            conn.query_row("SELECT train FROM lanes WHERE id = ?1", params![uuid_blob(*lane)], |r| r.get(0)).map_err(map_err)?;
        if on.is_some_and(|o| o.eq_ignore_ascii_case(&train.name)) && train.state != TrainState::Integrating {
            return Err(invalid("train_moving"));
        }
    }
    for lane in add {
        if let Some(from) = train_of_lane(conn, *lane)?
            && from.id != train.id
            && !from.state.is_settled()
            && from.state != TrainState::Integrating
        {
            return Err(invalid("train_moving"));
        }
    }
    Ok(())
}

/// A lane write that changes the lane's train label (`plan lane set --train`)
/// leaves the train it was on: refused as `train_moving` while that train is
/// past integrating and not settled, as `check_can_leave` does. Landing the lane
/// itself (`--state landed`) is the record of a lane that reached main, and passes.
pub(crate) fn check_label_may_leave(
    conn: &Connection,
    lane: Uuid,
    from: Option<&str>,
    to: Option<&str>,
    landing: bool,
) -> Result<()> {
    let same = match (from, to) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    };
    if same || landing {
        return Ok(());
    }
    if let Some(on) = train_of_lane(conn, lane)?
        && !on.state.is_settled()
        && on.state != TrainState::Integrating
    {
        return Err(invalid("train_moving"));
    }
    Ok(())
}

/// A train landed: land every lane on it that was ready (landing, or in review
/// when forced), with the train's SHA as the lane's `landed_sha` when it has
/// one and the workspace lands direct, and a timeline event on each. A lane on
/// it that never was ready (queued, building or fixing: a stray label, or a
/// lane sent back for a fix) is taken off the train and keeps its state, since
/// a train landing says nothing about work it didn't carry.
///
/// In a pull-request workspace the train is a rehearsal whose SHA was never
/// pushed, so no lane takes it as its `landed_sha`.
fn land_lanes(conn: &Connection, train: &Train, now: i64, actor: Actor) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT id, state FROM lanes
              WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE AND state NOT IN ('landed', 'dropped')",
        )
        .map_err(map_err)?;
    let lanes = stmt
        .query_map(params![uuid_blob(train.workspace_id), train.name], |r| Ok((get_uuid(r, 0)?, r.get::<_, String>(1)?)))
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    let called = train_number(&train.name).map_or_else(|| train.name.clone(), |n| format!("Train {n}"));
    let rehearsal = crate::landing::read(conn, train.workspace_id)?.mode == Some(crate::landing::LandingMode::PullRequests);
    let sha = if rehearsal { None } else { train.pushed_sha.as_deref() };
    for (lane, from) in lanes {
        if !matches!(LaneState::parse(&from), Some(LaneState::Landing | LaneState::Review)) {
            conn.execute(
                "UPDATE lanes SET train = NULL, resource_version = resource_version + 1 WHERE id = ?1",
                params![uuid_blob(lane)],
            )
            .map_err(map_err)?;
            let extra = serde_json::json!({ "train": train.name });
            event(conn, Subject::Lane(lane), "state", actor, &format!("Taken off {called}, which landed without it."), extra)?;
            continue;
        }
        conn.execute(
            "UPDATE lanes SET state = 'landed', state_since = ?2, plan_rank = NULL, landed_sha = coalesce(?3, landed_sha),
                    resource_version = resource_version + 1
              WHERE id = ?1",
            params![uuid_blob(lane), now, sha],
        )
        .map_err(map_err)?;
        let extra = serde_json::json!({ "from": from, "to": "landed", "train": train.name });
        event(conn, Subject::Lane(lane), "state", actor, &format!("Landed with {called}."), extra)?;
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
        check_ready(&tx, None, lanes)?;
        let title = clean_title(&new.title)?;
        let agent = new.agent.as_ref().map(clean_agent).transpose()?;
        if let Some(card) = new.card {
            check_cards(&tx, workspace, &[card])?;
        }
        // The board always has a collecting train (ov-520). A train started by
        // hand adopts it: the same train takes the name and base, carries
        // exactly the lanes listed, and stops collecting. The lanes it held that
        // weren't listed move to a fresh collecting train behind it. A name
        // another train has is refused, as it always was.
        let open = crate::open_train::open_train(&tx, workspace)?;
        let taken = tx
            .query_row(
                "SELECT id FROM board_trains WHERE workspace_id = ?1 AND name = ?2 COLLATE NOCASE",
                params![uuid_blob(workspace), name],
                |r| get_uuid(r, 0),
            )
            .optional()
            .map_err(map_err)?;
        if taken.is_some_and(|t| Some(t) != open.as_ref().map(|o| o.id)) {
            return Err(invalid("name_taken"));
        }
        let now = now_millis();
        let id = match &open {
            Some(open) => {
                if !open.name.eq_ignore_ascii_case(&name) {
                    train_event(&tx, open.id, "renamed", actor, None, serde_json::json!({ "to": name, "from": open.name }))?;
                    tx.execute(
                        "UPDATE lanes SET train = NULL, resource_version = resource_version + 1
                          WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE AND state NOT IN ('landed', 'dropped')",
                        params![uuid_blob(workspace), name],
                    )
                    .map_err(map_err)?;
                    tx.execute(
                        "UPDATE lanes SET train = ?3, resource_version = resource_version + 1
                          WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE",
                        params![uuid_blob(workspace), open.name, name],
                    )
                    .map_err(map_err)?;
                }
                tx.execute(
                    "UPDATE board_trains SET name = ?2, base = ?3, collecting = 0, actor = ?4, resource_version = resource_version + 1
                      WHERE id = ?1",
                    params![uuid_blob(open.id), name, base, actor.to_string()],
                )
                .map_err(map_err)?;
                let mut stmt = tx
                    .prepare(
                        "SELECT id FROM lanes WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE
                            AND state NOT IN ('landed', 'dropped') ORDER BY created_at, id",
                    )
                    .map_err(map_err)?;
                let unlisted: Vec<Uuid> = stmt
                    .query_map(params![uuid_blob(workspace), name], |r| get_uuid(r, 0))
                    .map_err(map_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(map_err)?
                    .into_iter()
                    .filter(|l| !lanes.contains(l))
                    .collect();
                drop(stmt);
                if !unlisted.is_empty() {
                    let fresh = crate::open_train::ensure_open(&tx, workspace, Actor::Runner)?;
                    board_lanes(&tx, &fresh, &unlisted, &[], Actor::Runner)?;
                }
                open.id
            }
            None => {
                let id = Uuid::now_v7();
                tx.execute(
                    "INSERT INTO board_trains (id, workspace_id, name, base, state, state_since, actor, created_at,
                                               resource_version)
                     VALUES (?1, ?2, ?3, ?4, 'integrating', ?5, ?6, ?5, 1)",
                    params![uuid_blob(id), uuid_blob(workspace), name, base, now, actor.to_string()],
                )
                .map_err(map_err)?;
                // A lane labelled with this name before the train existed isn't
                // on it unless it was named here, and passed the gate.
                tx.execute(
                    "UPDATE lanes SET train = NULL, resource_version = resource_version + 1
                      WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE AND state NOT IN ('landed', 'dropped')",
                    params![uuid_blob(workspace), name],
                )
                .map_err(map_err)?;
                train_event(&tx, id, "state", actor, None, serde_json::json!({ "from": null, "to": "integrating" }))?;
                id
            }
        };
        let made = train_in(&tx, id)?;
        board_lanes(&tx, &made, lanes, &[], actor)?;
        write_extras(&tx, id, Some(&title), new.card, agent.as_ref(), new.agent.as_ref().is_some_and(|a| a.ended), now)?;
        crate::open_train::ensure_open(&tx, workspace, Actor::Runner)?;
        let train = train_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(train)
    }

    /// Change a train: move it, rebase it, record what it pushed, and put
    /// lanes on or off it, in one write. A settled train (landed or dropped)
    /// takes no more moves and no new SHA.
    pub fn set_train(&self, train: Uuid, update: &TrainUpdate, actor: Actor) -> Result<Train> {
        let base = update.base.as_deref().map(str::trim).map(str::to_string);
        if base.as_ref().is_some_and(|b| b.chars().count() > BASE_MAX) {
            return Err(invalid("base"));
        }
        let sha = update.sha.as_deref().map(clean_sha).transpose()?;
        let title = update.title.as_deref().map(clean_title).transpose()?;
        let agent = update.agent.as_ref().map(clean_agent).transpose()?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before = train_in(&tx, train)?;
        if let Some((state, sha)) = &update.expect
            && (before.state != *state || before.pushed_sha.as_deref() != Some(sha.as_str()))
        {
            return Err(invalid("train_changed"));
        }
        if let Some(card) = update.card {
            check_cards(&tx, before.workspace_id, &[card])?;
        }
        check_lanes(&tx, before.workspace_id, &update.add_lanes)?;
        check_lanes(&tx, before.workspace_id, &update.remove_lanes)?;
        if before.state.is_settled() && !update.add_lanes.is_empty() {
            return Err(invalid("train_settled"));
        }
        check_ready(&tx, Some(&before.name), &update.add_lanes)?;
        check_can_leave(&tx, &before, &update.remove_lanes, &update.add_lanes)?;
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
        // Collecting is the board's one train that nobody has claimed: starting
        // it, moving it on or giving it an agent claims it. A train moved back to
        // integrating by hand is claimed too, so it is ahead of the collecting one
        // and never leaves on its own.
        let collecting = before.collecting && to == TrainState::Integrating && agent.is_none();
        tx.execute("UPDATE board_trains SET collecting = ?2 WHERE id = ?1", params![uuid_blob(train), i64::from(collecting)])
            .map_err(map_err)?;
        if moved {
            let extra = serde_json::json!({ "from": before.state.as_str(), "to": to.as_str() });
            train_event(&tx, train, "state", actor, None, extra)?;
        }
        board_lanes(&tx, &before, &update.add_lanes, &update.remove_lanes, actor)?;
        write_extras(
            &tx,
            train,
            title.as_deref(),
            update.card,
            agent.as_ref(),
            update.agent.as_ref().is_some_and(|a| a.ended),
            now,
        )?;
        // A dropped train lets its lanes go, so none still says it's in it
        // (review train-1005c L1). A landed one lands them (ov-503) and keeps
        // them on it, as its record.
        if moved && to == TrainState::Landed {
            let landed = train_in(&tx, train)?;
            land_lanes(&tx, &landed, now, actor)?;
        }
        if moved && to == TrainState::Dropped {
            let mut stmt = tx
                .prepare("SELECT id FROM lanes WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE")
                .map_err(map_err)?;
            let released = stmt
                .query_map(params![uuid_blob(before.workspace_id), before.name], |r| get_uuid(r, 0))
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
            drop(stmt);
            for lane in released {
                train_event(&tx, train, "lane_off", actor, Some(lane), serde_json::json!({ "to": null, "dropped": true }))?;
            }
            tx.execute(
                "UPDATE lanes SET train = NULL, resource_version = resource_version + 1
                  WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE",
                params![uuid_blob(before.workspace_id), before.name],
            )
            .map_err(map_err)?;
        }
        // The train ahead is out of the way: the collecting train departs if it
        // can (ov-520). Then the board has a collecting train again, behind it.
        crate::open_train::depart(&tx, before.workspace_id, Actor::Runner)?;
        crate::open_train::ensure_open(&tx, before.workspace_id, Actor::Runner)?;
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
        let extra = serde_json::json!({ "from": train.state.as_str(), "to": to.as_str() });
        train_event(conn, train.id, "state", Actor::Runner, None, extra)?;
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

#[cfg(test)]
#[path = "trains_truth_tests.rs"]
mod truth_tests;
