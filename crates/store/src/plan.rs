//! The plan layer (ov-268): themes, lanes and the plan, beside the board.
//!
//! A **theme** is why a group of cards exists. A **lane** is one unit of
//! execution: an agent, or a chain of them, in one worktree on one branch,
//! working whole cards or slices of them. The **plan** is the board's ordered
//! list of queued lanes.
//!
//! # Additive and removable
//!
//! The layer is an experiment, and the one rule that keeps it one is that
//! tasks never reference it. Six tables of its own, every one pointing at
//! `tasks`, `workspaces` or `worktrees` by cascade (or `SET NULL`), and none
//! of those gaining a column, a trigger, a status or a note kind. Nothing in
//! `tasks.rs`, `waits.rs`, `workers.rs`, `workspaces.rs` or the daemon's board
//! code names these tables (`scripts/plan-layer-lint.py` fails CI if one does),
//! and the drill in `plan_tests.rs` drops all six from a populated database
//! and checks the board reads the same bytes.
//!
//! Lane and theme events are kept in `plan_events`, never in `task_notes`:
//! that table is append-only by trigger, so a note kind written there could
//! not be taken out again with the layer.
//!
//! What can go stale is derived on read (`plan_read.rs`): a theme's progress,
//! a card's coverage, a lane's spend and whether it has sat too long.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;
use crate::workers::HARNESSES;

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

/// How long a name, a reason or a line may be. Generous for a person, and a
/// ceiling for a script that has lost its mind.
const NAME_MAX: usize = 60;
const LINE_MAX: usize = 300;
const STORY_MAX: usize = 4000;
const PATH_MAX: usize = 512;

/// Where a theme stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeState {
    /// Being worked.
    Active,
    /// Set aside on purpose.
    Paused,
    /// Its outcome holds.
    Done,
    /// Given up, and its name is free again.
    Dropped,
}

impl ThemeState {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeState::Active => "active",
            ThemeState::Paused => "paused",
            ThemeState::Done => "done",
            ThemeState::Dropped => "dropped",
        }
    }

    /// The state a stored word names.
    pub fn parse(raw: &str) -> Option<ThemeState> {
        Some(match raw {
            "active" => ThemeState::Active,
            "paused" => ThemeState::Paused,
            "done" => ThemeState::Done,
            "dropped" => ThemeState::Dropped,
            _ => return None,
        })
    }
}

/// Where a lane is. States move forward, with two loops: review to fixing and
/// back, and landing to fixing (a train that fails a lane).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneState {
    /// In the plan or waiting for one; nothing is running.
    Queued,
    /// An agent is building.
    Building,
    /// A reviewer has it.
    Review,
    /// A fix round after a review or a failed train.
    Fixing,
    /// Picked into a train, or being landed on its own.
    Landing,
    /// On main.
    Landed,
    /// Given up.
    Dropped,
}

impl LaneState {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            LaneState::Queued => "queued",
            LaneState::Building => "building",
            LaneState::Review => "review",
            LaneState::Fixing => "fixing",
            LaneState::Landing => "landing",
            LaneState::Landed => "landed",
            LaneState::Dropped => "dropped",
        }
    }

    /// The state a stored word names.
    pub fn parse(raw: &str) -> Option<LaneState> {
        Some(match raw {
            "queued" => LaneState::Queued,
            "building" => LaneState::Building,
            "review" => LaneState::Review,
            "fixing" => LaneState::Fixing,
            "landing" => LaneState::Landing,
            "landed" => LaneState::Landed,
            "dropped" => LaneState::Dropped,
            _ => return None,
        })
    }

    /// Landed and dropped lanes are finished: they take no more moves.
    pub fn is_closed(self) -> bool {
        matches!(self, LaneState::Landed | LaneState::Dropped)
    }

    /// Whether a lane may move from `self` to `to`. Staying put is always
    /// allowed (a reason or a train can change without a move).
    pub fn can_move_to(self, to: LaneState) -> bool {
        use LaneState::*;
        self == to
            || matches!(
                (self, to),
                (Queued, Building)
                    | (Building, Review)
                    | (Review, Landing)
                    | (Review, Landed)
                    | (Landing, Landed)
                    | (Review, Fixing)
                    | (Fixing, Review)
                    | (Landing, Fixing)
            )
            || (!self.is_closed() && to == Dropped)
    }

    /// The states a lane in `self` may move to, other than staying put, in the
    /// order a person reads them. For a refusal that says where to go next.
    pub fn moves(self) -> Vec<LaneState> {
        use LaneState::*;
        [Queued, Building, Review, Fixing, Landing, Landed, Dropped]
            .into_iter()
            .filter(|to| *to != self && self.can_move_to(*to))
            .collect()
    }
}

/// What an agent did in a lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    /// The lane's builder.
    Build,
    /// A reviewer.
    Review,
    /// A fix round.
    Fix,
}

impl AgentRole {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentRole::Build => "build",
            AgentRole::Review => "review",
            AgentRole::Fix => "fix",
        }
    }

    /// The role a stored word names.
    pub fn parse(raw: &str) -> Option<AgentRole> {
        Some(match raw {
            "build" => AgentRole::Build,
            "review" => AgentRole::Review,
            "fix" => AgentRole::Fix,
            _ => return None,
        })
    }
}

/// One theme on one workspace's board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardTheme {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// One sentence: the world when it's done.
    pub outcome: String,
    /// Where it stands, rewritten at checkpoints; the old one is kept in
    /// `plan_events`.
    pub story: String,
    /// What happens next, in one line.
    pub next: String,
    /// What needs the owner, in one line; empty when nothing does.
    pub owner_ask: String,
    pub state: ThemeState,
    pub ordinal: i64,
    /// When `story` was last written, in ms; 0 if never.
    pub story_at: i64,
    pub created_at: i64,
    pub resource_version: u64,
}

/// One lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub state: LaneState,
    /// One line: why it's in the plan, or why it's where it is now.
    pub reason: String,
    /// 1 for next up; `None` outside the plan.
    pub plan_rank: Option<u32>,
    pub worktree_id: Option<Uuid>,
    pub worktree_path: String,
    pub branch: String,
    pub harness: String,
    pub model: String,
    pub train: Option<String>,
    pub landed_sha: Option<String>,
    pub state_since: i64,
    pub created_at: i64,
    pub resource_version: u64,
}

/// A card a lane works, or one slice of it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LaneCard {
    pub task_id: Uuid,
    /// Empty for the whole card.
    pub slice: String,
}

/// An agent that worked in a lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneAgent {
    pub harness: String,
    pub agent_id: String,
    pub role: AgentRole,
    pub model: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// An agent to record on a lane, or to fill in what's new about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRecord {
    /// One of `workers::HARNESSES`.
    pub harness: String,
    pub agent_id: String,
    pub role: AgentRole,
    pub model: Option<String>,
    /// The agent has finished. Recording it again without this reopens it.
    pub ended: bool,
}

/// A theme to create.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTheme {
    pub name: String,
    pub outcome: String,
}

/// What `update_theme` changes; each `None` leaves a field alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeUpdate {
    pub name: Option<String>,
    pub outcome: Option<String>,
    pub story: Option<String>,
    pub next: Option<String>,
    /// An empty string clears it.
    pub owner_ask: Option<String>,
    pub state: Option<ThemeState>,
    pub ordinal: Option<i64>,
}

/// A lane to create.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewLane {
    pub name: String,
    pub reason: String,
    pub worktree_id: Option<Uuid>,
    pub worktree_path: String,
    pub branch: String,
    pub harness: String,
    pub model: String,
}

/// What `update_lane` changes; each `None` leaves a field alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaneUpdate {
    pub state: Option<LaneState>,
    pub reason: Option<String>,
    /// An empty string takes the lane out of its train.
    pub train: Option<String>,
    pub landed_sha: Option<String>,
    pub worktree_id: Option<Uuid>,
    pub worktree_path: Option<String>,
    pub branch: Option<String>,
    /// An agent to record in the same write, so a reviewer and the move to
    /// review are one change.
    pub agent: Option<AgentRecord>,
}

/// One entry in a theme's or lane's timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEvent {
    pub id: Uuid,
    pub at: i64,
    pub actor: String,
    pub theme_id: Option<Uuid>,
    pub lane_id: Option<Uuid>,
    /// `story`, `state`, `plan`, `cards` or `agent`.
    pub kind: String,
    pub body: String,
    pub extra: serde_json::Value,
}

/// A theme or a lane: what an event is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Theme(Uuid),
    Lane(Uuid),
}

fn invalid(what: &'static str) -> DomainError {
    DomainError::InvalidArgument { what }
}

/// `value` trimmed, refused if it is longer than `max` characters, or empty
/// where `required`.
fn clean(value: &str, max: usize, required: bool, what: &'static str) -> Result<String> {
    let value = value.trim();
    if value.chars().count() > max || (required && value.is_empty()) {
        return Err(invalid(what));
    }
    Ok(value.to_string())
}

fn lane_name(value: &str) -> Result<String> {
    let name = clean(value, NAME_MAX, true, "name")?;
    if name.chars().any(char::is_whitespace) {
        return Err(invalid("name"));
    }
    Ok(name)
}

pub(crate) fn board_exists(conn: &Connection, workspace: Uuid) -> Result<()> {
    conn.query_row("SELECT 1 FROM workspaces WHERE id = ?1", params![uuid_blob(workspace)], |_| Ok(()))
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

/// Each card must exist and be on `workspace`'s own board: a theme and a lane
/// belong to one board, as a line does.
fn check_cards(conn: &Connection, workspace: Uuid, tasks: &[Uuid]) -> Result<()> {
    for task in tasks {
        let board: Option<Vec<u8>> = conn
            .query_row("SELECT workspace_id FROM tasks WHERE id = ?1", params![uuid_blob(*task)], |r| r.get(0))
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

fn task_key(conn: &Connection, task: Uuid) -> String {
    conn.query_row("SELECT key FROM tasks WHERE id = ?1", params![uuid_blob(task)], |r| r.get(0))
        .unwrap_or_else(|_| "a card".to_string())
}

/// Drop rows that point at a card now on another board. A moved card leaves
/// its theme and lanes; a read already ignores those rows, and the next write
/// on the board removes them, so `task move` needs no hook.
pub(crate) fn prune_moved(tx: &Connection, workspace: Uuid) -> Result<()> {
    let ws = uuid_blob(workspace);
    tx.execute(
        "DELETE FROM board_theme_tasks
          WHERE theme_id IN (SELECT id FROM board_themes WHERE workspace_id = ?1)
            AND task_id NOT IN (SELECT id FROM tasks WHERE workspace_id = ?1)",
        params![ws],
    )
    .map_err(map_err)?;
    tx.execute(
        "DELETE FROM lane_tasks
          WHERE lane_id IN (SELECT id FROM lanes WHERE workspace_id = ?1)
            AND task_id NOT IN (SELECT id FROM tasks WHERE workspace_id = ?1)",
        params![ws],
    )
    .map_err(map_err)?;
    Ok(())
}

pub(crate) fn event(
    conn: &Connection,
    subject: Subject,
    kind: &str,
    actor: Actor,
    body: &str,
    extra: serde_json::Value,
) -> Result<()> {
    let (theme, lane) = match subject {
        Subject::Theme(id) => (Some(uuid_blob(id)), None),
        Subject::Lane(id) => (None, Some(uuid_blob(id))),
    };
    conn.execute(
        "INSERT INTO plan_events (id, at, actor, theme_id, lane_id, kind, body, extra)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![uuid_blob(Uuid::now_v7()), now_millis(), actor.to_string(), theme, lane, kind, body, extra.to_string()],
    )
    .map_err(map_err)?;
    Ok(())
}

pub(crate) const THEME_COLS: &str =
    "id, workspace_id, name, outcome, story, next, owner_ask, state, ordinal, story_at, created_at, resource_version";

pub(crate) fn row_to_theme(r: &rusqlite::Row) -> rusqlite::Result<BoardTheme> {
    let state: String = r.get(7)?;
    Ok(BoardTheme {
        id: get_uuid(r, 0)?,
        workspace_id: get_uuid(r, 1)?,
        name: r.get(2)?,
        outcome: r.get(3)?,
        story: r.get(4)?,
        next: r.get(5)?,
        owner_ask: r.get(6)?,
        state: ThemeState::parse(&state).unwrap_or(ThemeState::Active),
        ordinal: r.get(8)?,
        story_at: r.get(9)?,
        created_at: r.get(10)?,
        resource_version: r.get::<_, i64>(11)?.max(0) as u64,
    })
}

pub(crate) const LANE_COLS: &str = "id, workspace_id, name, state, reason, plan_rank, worktree_id, worktree_path, branch,
     harness, model, train, landed_sha, state_since, created_at, resource_version";

pub(crate) fn row_to_lane(r: &rusqlite::Row) -> rusqlite::Result<Lane> {
    let state: String = r.get(3)?;
    let worktree: Option<Vec<u8>> = r.get(6)?;
    Ok(Lane {
        id: get_uuid(r, 0)?,
        workspace_id: get_uuid(r, 1)?,
        name: r.get(2)?,
        state: LaneState::parse(&state).unwrap_or(LaneState::Queued),
        reason: r.get(4)?,
        plan_rank: r.get::<_, Option<i64>>(5)?.map(|n| n.max(0) as u32),
        worktree_id: worktree.and_then(|b| Uuid::from_slice(&b).ok()),
        worktree_path: r.get(7)?,
        branch: r.get(8)?,
        harness: r.get(9)?,
        model: r.get(10)?,
        train: r.get(11)?,
        landed_sha: r.get(12)?,
        state_since: r.get(13)?,
        created_at: r.get(14)?,
        resource_version: r.get::<_, i64>(15)?.max(0) as u64,
    })
}

fn theme_in(conn: &Connection, id: Uuid) -> Result<BoardTheme> {
    conn.query_row(&format!("SELECT {THEME_COLS} FROM board_themes WHERE id = ?1"), params![uuid_blob(id)], row_to_theme)
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

fn lane_in(conn: &Connection, id: Uuid) -> Result<Lane> {
    conn.query_row(&format!("SELECT {LANE_COLS} FROM lanes WHERE id = ?1"), params![uuid_blob(id)], row_to_lane)
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

fn theme_name_free(conn: &Connection, workspace: Uuid, name: &str, except: Option<Uuid>) -> Result<()> {
    let taken: bool = conn
        .query_row(
            "SELECT 1 FROM board_themes WHERE workspace_id = ?1 AND name = ?2 COLLATE NOCASE
                AND state <> 'dropped' AND id <> ?3",
            params![uuid_blob(workspace), name, uuid_blob(except.unwrap_or_else(Uuid::nil))],
            |_| Ok(()),
        )
        .optional()
        .map_err(map_err)?
        .is_some();
    if taken { Err(invalid("name_taken")) } else { Ok(()) }
}

fn lane_name_free(conn: &Connection, workspace: Uuid, name: &str) -> Result<()> {
    let taken: bool = conn
        .query_row(
            "SELECT 1 FROM lanes WHERE workspace_id = ?1 AND name = ?2 COLLATE NOCASE
                AND state NOT IN ('landed', 'dropped')",
            params![uuid_blob(workspace), name],
            |_| Ok(()),
        )
        .optional()
        .map_err(map_err)?
        .is_some();
    if taken { Err(invalid("name_taken")) } else { Ok(()) }
}

fn ordinal_word(n: u32) -> String {
    match n {
        1 => "next up".to_string(),
        2 => "2nd".to_string(),
        3 => "3rd".to_string(),
        n => format!("{n}th"),
    }
}

impl Store {
    /// Make a theme on `workspace`'s board, with `tasks` in it.
    ///
    /// A card already in another theme moves here: a card is in one theme.
    pub fn create_theme(&self, workspace: Uuid, new: &NewTheme, tasks: &[Uuid], actor: Actor) -> Result<BoardTheme> {
        let name = clean(&new.name, NAME_MAX, true, "name")?;
        let outcome = clean(&new.outcome, LINE_MAX, false, "outcome")?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_exists(&tx, workspace)?;
        check_cards(&tx, workspace, tasks)?;
        theme_name_free(&tx, workspace, &name, None)?;
        prune_moved(&tx, workspace)?;
        let id = Uuid::now_v7();
        let now = now_millis();
        let ordinal: i64 = tx
            .query_row(
                "SELECT coalesce(max(ordinal), 0) + 1 FROM board_themes WHERE workspace_id = ?1",
                params![uuid_blob(workspace)],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        tx.execute(
            "INSERT INTO board_themes (id, workspace_id, name, outcome, state, ordinal, created_at, resource_version)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, 1)",
            params![uuid_blob(id), uuid_blob(workspace), name, outcome, ordinal, now],
        )
        .map_err(map_err)?;
        event(&tx, Subject::Theme(id), "state", actor, "Created.", serde_json::json!({}))?;
        add_theme_cards(&tx, id, tasks, actor)?;
        let theme = theme_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(theme)
    }

    /// Change a theme. Writing `story` keeps the previous one as a `story`
    /// event, so "what changed since I last looked" has both halves.
    pub fn update_theme(&self, theme: Uuid, update: &ThemeUpdate, actor: Actor) -> Result<BoardTheme> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before = theme_in(&tx, theme)?;
        let name = match &update.name {
            Some(n) => clean(n, NAME_MAX, true, "name")?,
            None => before.name.clone(),
        };
        let pick = |new: &Option<String>, old: &str, max: usize, what: &'static str| match new {
            Some(v) => clean(v, max, false, what),
            None => Ok(old.to_string()),
        };
        let outcome = pick(&update.outcome, &before.outcome, LINE_MAX, "outcome")?;
        let story = pick(&update.story, &before.story, STORY_MAX, "story")?;
        let next = pick(&update.next, &before.next, LINE_MAX, "next")?;
        let ask = pick(&update.owner_ask, &before.owner_ask, LINE_MAX, "owner_ask")?;
        let state = update.state.unwrap_or(before.state);
        if (name != before.name || state != before.state) && state != ThemeState::Dropped {
            theme_name_free(&tx, before.workspace_id, &name, Some(theme))?;
        }
        let story_changed = story != before.story;
        let now = now_millis();
        tx.execute(
            "UPDATE board_themes SET name = ?2, outcome = ?3, story = ?4, next = ?5, owner_ask = ?6, state = ?7,
                    ordinal = ?8, story_at = ?9, resource_version = resource_version + 1
              WHERE id = ?1",
            params![
                uuid_blob(theme),
                name,
                outcome,
                story,
                next,
                ask,
                state.as_str(),
                update.ordinal.unwrap_or(before.ordinal),
                if story_changed { now } else { before.story_at },
            ],
        )
        .map_err(map_err)?;
        if story_changed {
            event(&tx, Subject::Theme(theme), "story", actor, &before.story, serde_json::json!({ "to": story }))?;
        }
        if state != before.state {
            let said = format!("Moved to {}.", state.as_str());
            event(&tx, Subject::Theme(theme), "state", actor, &said, serde_json::json!({ "from": before.state.as_str() }))?;
        }
        let after = theme_in(&tx, theme)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// Add cards to a theme and take others out. An added card in another
    /// theme moves; a removed card that isn't in this one is skipped.
    pub fn theme_cards(&self, theme: Uuid, add: &[Uuid], remove: &[Uuid], actor: Actor) -> Result<BoardTheme> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let found = theme_in(&tx, theme)?;
        check_cards(&tx, found.workspace_id, add)?;
        prune_moved(&tx, found.workspace_id)?;
        add_theme_cards(&tx, theme, add, actor)?;
        for task in remove {
            let gone = tx
                .execute(
                    "DELETE FROM board_theme_tasks WHERE task_id = ?1 AND theme_id = ?2",
                    params![uuid_blob(*task), uuid_blob(theme)],
                )
                .map_err(map_err)?;
            if gone > 0 {
                let said = format!("Removed {}.", task_key(&tx, *task));
                event(&tx, Subject::Theme(theme), "cards", actor, &said, serde_json::json!({}))?;
            }
        }
        tx.execute("UPDATE board_themes SET resource_version = resource_version + 1 WHERE id = ?1", params![uuid_blob(theme)])
            .map_err(map_err)?;
        let after = theme_in(&tx, theme)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// Make a lane on `workspace`'s board, working `cards`, with an agent
    /// recorded in the same write when one is given. A lane with an agent
    /// starts building; one without starts queued.
    pub fn create_lane(
        &self,
        workspace: Uuid,
        new: &NewLane,
        cards: &[LaneCard],
        agent: Option<&AgentRecord>,
        actor: Actor,
    ) -> Result<Lane> {
        let name = lane_name(&new.name)?;
        let reason = clean(&new.reason, LINE_MAX, false, "reason")?;
        let path = clean(&new.worktree_path, PATH_MAX, false, "path")?;
        let branch = clean(&new.branch, NAME_MAX * 4, false, "branch")?;
        let model = clean(&new.model, NAME_MAX, false, "model")?;
        let harness = clean(&new.harness, NAME_MAX, false, "harness")?;
        if !harness.is_empty() && !HARNESSES.contains(&harness.as_str()) {
            return Err(invalid("harness"));
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_exists(&tx, workspace)?;
        let tasks: Vec<Uuid> = cards.iter().map(|c| c.task_id).collect();
        check_cards(&tx, workspace, &tasks)?;
        lane_name_free(&tx, workspace, &name)?;
        prune_moved(&tx, workspace)?;
        let id = Uuid::now_v7();
        let now = now_millis();
        let state = if agent.is_some() { LaneState::Building } else { LaneState::Queued };
        tx.execute(
            "INSERT INTO lanes (id, workspace_id, name, state, reason, worktree_id, worktree_path, branch, harness,
                                model, state_since, created_at, resource_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, 1)",
            params![
                uuid_blob(id),
                uuid_blob(workspace),
                name,
                state.as_str(),
                reason,
                new.worktree_id.map(uuid_blob),
                path,
                branch,
                harness,
                model,
                now,
            ],
        )
        .map_err(map_err)?;
        let said = if agent.is_some() { "Started building." } else { "Queued." };
        event(&tx, Subject::Lane(id), "state", actor, said, serde_json::json!({}))?;
        add_lane_cards(&tx, id, cards, actor)?;
        if let Some(agent) = agent {
            record_agent(&tx, id, agent, actor)?;
        }
        let lane = lane_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(lane)
    }

    /// Change a lane: move it (along the arrows `LaneState::can_move_to`
    /// allows), reword it, put it on a train, record how it landed, and record
    /// an agent, all in one write.
    ///
    /// A lane that leaves `queued` loses its place in the plan in the same
    /// write.
    pub fn update_lane(&self, lane: Uuid, update: &LaneUpdate, actor: Actor) -> Result<Lane> {
        let reason = update.reason.as_deref().map(|v| clean(v, LINE_MAX, false, "reason")).transpose()?;
        let train = update.train.as_deref().map(|v| clean(v, NAME_MAX, false, "train")).transpose()?;
        let sha = update.landed_sha.as_deref().map(|v| clean(v, 64, false, "sha")).transpose()?;
        let path = update.worktree_path.as_deref().map(|v| clean(v, PATH_MAX, false, "path")).transpose()?;
        let branch = update.branch.as_deref().map(|v| clean(v, NAME_MAX * 4, false, "branch")).transpose()?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before = lane_in(&tx, lane)?;
        let to = update.state.unwrap_or(before.state);
        if before.state.is_closed() && (update.state.is_some_and(|s| s != before.state) || update.agent.is_some()) {
            return Err(invalid("lane_closed"));
        }
        if !before.state.can_move_to(to) {
            return Err(invalid("lane_state"));
        }
        let now = now_millis();
        let moved = to != before.state;
        tx.execute(
            "UPDATE lanes SET state = ?2, reason = ?3, train = ?4, landed_sha = ?5, worktree_id = ?6,
                    worktree_path = ?7, branch = ?8, state_since = ?9,
                    plan_rank = CASE WHEN ?2 = 'queued' THEN plan_rank ELSE NULL END,
                    resource_version = resource_version + 1
              WHERE id = ?1",
            params![
                uuid_blob(lane),
                to.as_str(),
                reason.unwrap_or_else(|| before.reason.clone()),
                match train {
                    Some(t) if t.is_empty() => None,
                    Some(t) => Some(t),
                    None => before.train.clone(),
                },
                sha.filter(|s| !s.is_empty()).or_else(|| before.landed_sha.clone()),
                update.worktree_id.or(before.worktree_id).map(uuid_blob),
                path.unwrap_or_else(|| before.worktree_path.clone()),
                branch.unwrap_or_else(|| before.branch.clone()),
                if moved { now } else { before.state_since },
            ],
        )
        .map_err(map_err)?;
        if moved {
            let said = match to {
                LaneState::Building => "Started building.".to_string(),
                LaneState::Review => "Moved to review.".to_string(),
                LaneState::Fixing => "Started a fix round.".to_string(),
                LaneState::Landing => "Moved to landing.".to_string(),
                LaneState::Landed => "Landed.".to_string(),
                LaneState::Dropped => "Dropped.".to_string(),
                LaneState::Queued => "Queued.".to_string(),
            };
            let said = match update.reason.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
                Some(why) => format!("{said} {why}"),
                None => said,
            };
            // Landing from review is one write that passes through landing, and
            // the timeline says so, as it would for the two writes.
            let from = if before.state == LaneState::Review && to == LaneState::Landed {
                let via = serde_json::json!({ "from": "review", "to": "landing" });
                event(&tx, Subject::Lane(lane), "state", actor, "Moved to landing.", via)?;
                LaneState::Landing
            } else {
                before.state
            };
            event(&tx, Subject::Lane(lane), "state", actor, &said, serde_json::json!({ "from": from.as_str(), "to": to.as_str() }))?;
        }
        if let Some(agent) = &update.agent {
            record_agent(&tx, lane, agent, actor)?;
        }
        let after = lane_in(&tx, lane)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// Add cards to a lane and take others off it. The same card with two
    /// slices is two links; a slice names what this lane does of it.
    pub fn lane_cards(&self, lane: Uuid, add: &[LaneCard], remove: &[LaneCard], actor: Actor) -> Result<Lane> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let found = lane_in(&tx, lane)?;
        let tasks: Vec<Uuid> = add.iter().map(|c| c.task_id).collect();
        check_cards(&tx, found.workspace_id, &tasks)?;
        prune_moved(&tx, found.workspace_id)?;
        add_lane_cards(&tx, lane, add, actor)?;
        for card in remove {
            let gone = tx
                .execute(
                    "DELETE FROM lane_tasks WHERE lane_id = ?1 AND task_id = ?2 AND slice = ?3",
                    params![uuid_blob(lane), uuid_blob(card.task_id), card.slice.trim()],
                )
                .map_err(map_err)?;
            if gone > 0 {
                let said = format!("Removed {}.", task_key(&tx, card.task_id));
                event(&tx, Subject::Lane(lane), "cards", actor, &said, serde_json::json!({}))?;
            }
        }
        tx.execute("UPDATE lanes SET resource_version = resource_version + 1 WHERE id = ?1", params![uuid_blob(lane)])
            .map_err(map_err)?;
        let after = lane_in(&tx, lane)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// Record an agent on a lane, or fill in what's new about one already
    /// there. Recording an ended agent again without `ended` reopens it.
    pub fn record_lane_agent(&self, lane: Uuid, agent: &AgentRecord, actor: Actor) -> Result<Lane> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before = lane_in(&tx, lane)?;
        if before.state.is_closed() {
            return Err(invalid("lane_closed"));
        }
        record_agent(&tx, lane, agent, actor)?;
        tx.execute("UPDATE lanes SET resource_version = resource_version + 1 WHERE id = ?1", params![uuid_blob(lane)])
            .map_err(map_err)?;
        let after = lane_in(&tx, lane)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// Replace the plan whole: these queued lanes, in this order, and every
    /// other lane out of it.
    ///
    /// Only a queued lane on the board can be in the plan, and each is named
    /// once. A lane whose place changed gets a `plan` event.
    pub fn set_plan(&self, workspace: Uuid, lanes: &[Uuid], actor: Actor) -> Result<Vec<Lane>> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_exists(&tx, workspace)?;
        let mut seen = std::collections::HashSet::new();
        for id in lanes {
            if !seen.insert(*id) {
                return Err(invalid("lane_twice"));
            }
            let lane = lane_in(&tx, *id)?;
            if lane.workspace_id != workspace {
                return Err(invalid("other_board"));
            }
            if lane.state != LaneState::Queued {
                return Err(invalid("plan_state"));
            }
        }
        let mut old = std::collections::HashMap::new();
        {
            let mut stmt = tx
                .prepare("SELECT id, plan_rank FROM lanes WHERE workspace_id = ?1 AND plan_rank IS NOT NULL")
                .map_err(map_err)?;
            let rows = stmt
                .query_map(params![uuid_blob(workspace)], |r| Ok((get_uuid(r, 0)?, r.get::<_, i64>(1)?)))
                .map_err(map_err)?;
            for row in rows {
                let (id, rank) = row.map_err(map_err)?;
                old.insert(id, rank as u32);
            }
        }
        tx.execute("UPDATE lanes SET plan_rank = NULL WHERE workspace_id = ?1 AND plan_rank IS NOT NULL", params![uuid_blob(workspace)])
            .map_err(map_err)?;
        for (i, id) in lanes.iter().enumerate() {
            let rank = i as u32 + 1;
            tx.execute(
                "UPDATE lanes SET plan_rank = ?2, resource_version = resource_version + 1 WHERE id = ?1",
                params![uuid_blob(*id), rank],
            )
            .map_err(map_err)?;
            if old.get(id) != Some(&rank) {
                let said = format!("In the plan: {}.", ordinal_word(rank));
                event(&tx, Subject::Lane(*id), "plan", actor, &said, serde_json::json!({ "rank": rank }))?;
            }
        }
        for id in old.keys().filter(|id| !seen.contains(id)) {
            tx.execute("UPDATE lanes SET resource_version = resource_version + 1 WHERE id = ?1", params![uuid_blob(*id)])
                .map_err(map_err)?;
            event(&tx, Subject::Lane(*id), "plan", actor, "Out of the plan.", serde_json::json!({}))?;
        }
        let mut out = Vec::with_capacity(lanes.len());
        for id in lanes {
            out.push(lane_in(&tx, *id)?);
        }
        tx.commit().map_err(map_err)?;
        Ok(out)
    }

    /// A theme by id.
    pub fn board_theme(&self, theme: Uuid) -> Result<BoardTheme> {
        theme_in(&self.conn(), theme)
    }

    /// A lane by id.
    pub fn lane(&self, lane: Uuid) -> Result<Lane> {
        lane_in(&self.conn(), lane)
    }
}

fn add_theme_cards(tx: &Connection, theme: Uuid, tasks: &[Uuid], actor: Actor) -> Result<()> {
    for task in tasks {
        let from: Option<Vec<u8>> = tx
            .query_row("SELECT theme_id FROM board_theme_tasks WHERE task_id = ?1", params![uuid_blob(*task)], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        if from.as_ref().is_some_and(|id| *id == uuid_blob(theme)) {
            continue;
        }
        tx.execute(
            "INSERT INTO board_theme_tasks (task_id, theme_id, added_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(task_id) DO UPDATE SET theme_id = excluded.theme_id, added_at = excluded.added_at",
            params![uuid_blob(*task), uuid_blob(theme), now_millis()],
        )
        .map_err(map_err)?;
        let key = task_key(tx, *task);
        event(tx, Subject::Theme(theme), "cards", actor, &format!("Added {key}."), serde_json::json!({}))?;
        if let Some(old) = from.and_then(|b| Uuid::from_slice(&b).ok()) {
            let to: String = tx
                .query_row("SELECT name FROM board_themes WHERE id = ?1", params![uuid_blob(theme)], |r| r.get(0))
                .unwrap_or_default();
            event(tx, Subject::Theme(old), "cards", actor, &format!("Removed {key}: it moved to {to}."), serde_json::json!({}))?;
        }
    }
    Ok(())
}

fn add_lane_cards(tx: &Connection, lane: Uuid, cards: &[LaneCard], actor: Actor) -> Result<()> {
    for card in cards {
        let slice = clean(&card.slice, NAME_MAX, false, "slice")?;
        let added = tx
            .execute(
                "INSERT OR IGNORE INTO lane_tasks (lane_id, task_id, slice) VALUES (?1, ?2, ?3)",
                params![uuid_blob(lane), uuid_blob(card.task_id), slice],
            )
            .map_err(map_err)?;
        if added > 0 {
            let key = task_key(tx, card.task_id);
            let said = if slice.is_empty() { format!("Added {key}.") } else { format!("Added {key}, {slice}.") };
            event(tx, Subject::Lane(lane), "cards", actor, &said, serde_json::json!({}))?;
        }
    }
    Ok(())
}

fn record_agent(tx: &Connection, lane: Uuid, agent: &AgentRecord, actor: Actor) -> Result<()> {
    if !HARNESSES.contains(&agent.harness.as_str()) {
        return Err(invalid("harness"));
    }
    let id = clean(&agent.agent_id, NAME_MAX * 2, true, "agent_id")?;
    let model = agent.model.as_deref().map(|m| clean(m, NAME_MAX, false, "model")).transpose()?;
    let now = now_millis();
    let known: Option<Option<i64>> = tx
        .query_row(
            "SELECT ended_at FROM lane_agents WHERE lane_id = ?1 AND harness = ?2 AND agent_id = ?3",
            params![uuid_blob(lane), agent.harness, id],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_err)?;
    match known {
        None => {
            tx.execute(
                "INSERT INTO lane_agents (lane_id, harness, agent_id, role, model, started_at, ended_at)
                 VALUES (?1, ?2, ?3, ?4, coalesce(?5, ''), ?6, ?7)",
                params![uuid_blob(lane), agent.harness, id, agent.role.as_str(), model, now, agent.ended.then_some(now)],
            )
            .map_err(map_err)?;
            let said = format!("Started a {} agent.", agent.role.as_str());
            event(tx, Subject::Lane(lane), "agent", actor, &said, serde_json::json!({ "agent_id": id }))?;
        }
        Some(ended_at) => {
            let ended = if agent.ended { Some(ended_at.unwrap_or(now)) } else { None };
            tx.execute(
                "UPDATE lane_agents SET role = ?4, model = coalesce(?5, model), ended_at = ?6
                  WHERE lane_id = ?1 AND harness = ?2 AND agent_id = ?3",
                params![uuid_blob(lane), agent.harness, id, agent.role.as_str(), model, ended],
            )
            .map_err(map_err)?;
            if agent.ended && ended_at.is_none() {
                let said = format!("A {} agent finished.", agent.role.as_str());
                event(tx, Subject::Lane(lane), "agent", actor, &said, serde_json::json!({ "agent_id": id }))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
