//! Agents (ov-872): one row per agent the plan shows, recorded when it starts
//! rather than derived from lanes, trains and worker records.
//!
//! An agent has a **role** (orchestrator, builder, reviewer or lander), a
//! **kind** (a pane, or a subagent), a **harness** (any word: claude, codex,
//! cursor) and a place: a pane's terminal, or a subagent's own id and the pane
//! it runs under. What it is doing now, whether it is working, and what it
//! spent are observed, so they are derived on read and never stored here.
//!
//! The tickets an agent works are `ticket_work` rows (`ticket_work.rs`): a
//! builder or a reviewer may hold any number of them.
//!
//! The table is made by `line_migration.rs`, which also moves today's lane
//! agents, train agents and worker records into it.

use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, get_optional_uuid, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;
use crate::ticket_work::Verdict;

/// How long a harness word, a model or a session id may be.
const WORD_MAX: usize = 60;
/// A subagent's own id: Claude's `agentId`, or codex's agent path.
const ID_MAX: usize = 200;
/// A title is a short line a person reads in a row.
const TITLE_MAX: usize = 80;
const PATH_MAX: usize = 512;

/// What an agent is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    /// Turns what you want into tickets, plans them and dispatches the others.
    Orchestrator,
    /// Builds tickets, and fixes them after a review.
    Builder,
    /// Reviews tickets, from any builder.
    Reviewer,
    /// Puts finished tickets into the product.
    Lander,
}

impl AgentRole {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentRole::Orchestrator => "orchestrator",
            AgentRole::Builder => "builder",
            AgentRole::Reviewer => "reviewer",
            AgentRole::Lander => "lander",
        }
    }

    /// The role a stored word names.
    pub fn parse(raw: &str) -> Option<AgentRole> {
        Some(match raw {
            "orchestrator" => AgentRole::Orchestrator,
            "builder" => AgentRole::Builder,
            "reviewer" => AgentRole::Reviewer,
            "lander" => AgentRole::Lander,
            _ => return None,
        })
    }
}

/// Where an agent runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPlace {
    /// A pane: a terminal of its own, which a person can open and type into.
    Pane { terminal: Uuid },
    /// A subagent: the harness's own id for it, and the pane it runs under
    /// when that is known.
    Subagent { id: String, parent: Option<Uuid> },
}

impl AgentPlace {
    /// The stored word for the kind.
    pub fn kind(&self) -> &'static str {
        match self {
            AgentPlace::Pane { .. } => "pane",
            AgentPlace::Subagent { .. } => "subagent",
        }
    }
}

/// One agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub role: AgentRole,
    pub place: AgentPlace,
    pub harness: String,
    /// The session it belongs to (a subagent's parent session), when known.
    pub session_id: Option<String>,
    pub session_cwd: Option<String>,
    /// Empty when no model was named.
    pub model: String,
    /// What the orchestrator called it; empty when it said nothing.
    pub title: String,
    /// Unix milliseconds.
    pub started_at: i64,
    /// Unix milliseconds; `None` while it runs.
    pub ended_at: Option<i64>,
    pub end_reason: Option<Verdict>,
    /// Who asked it to stop, in `task_notes.actor`'s words, and when. An agent
    /// asked to stop reads "Stopping" until it ends: a tool call already
    /// running finishes first.
    pub stop_asked_by: Option<String>,
    pub stop_asked_at: Option<i64>,
}

/// An agent to record, or to fill in what is new about. The `Option`s fill in
/// what is known and leave alone what was recorded before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAgent {
    pub role: AgentRole,
    pub place: AgentPlace,
    pub harness: String,
    pub session_id: Option<String>,
    pub session_cwd: Option<String>,
    pub model: Option<String>,
    pub title: Option<String>,
}

fn invalid(what: &'static str) -> DomainError {
    DomainError::InvalidArgument { what }
}

/// `value` trimmed, refused if longer than `max` characters, holding a control
/// character, or empty where `required`.
pub(crate) fn clean(value: &str, max: usize, required: bool, what: &'static str) -> Result<String> {
    let value = value.trim();
    if value.chars().count() > max || (required && value.is_empty()) || value.chars().any(char::is_control) {
        return Err(invalid(what));
    }
    Ok(value.to_string())
}

/// A harness is any short word: `claude`, `codex`, `cursor`, or one an ACP
/// adapter names.
fn harness_word(value: &str) -> Result<String> {
    let harness = clean(value, WORD_MAX, true, "harness")?.to_ascii_lowercase();
    if harness.chars().any(char::is_whitespace) {
        return Err(invalid("harness"));
    }
    Ok(harness)
}

/// `workspace` exists.
pub(crate) fn board_exists(conn: &Connection, workspace: Uuid) -> Result<()> {
    conn.query_row("SELECT 1 FROM workspaces WHERE id = ?1", params![uuid_blob(workspace)], |_| Ok(()))
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

pub(crate) const AGENT_COLS: &str = "id, workspace_id, role, kind, harness, terminal_id, subagent_id, parent_terminal_id, \
     session_id, session_cwd, model, title, started_at, ended_at, end_reason, stop_asked_by, stop_asked_at";

pub(crate) fn row_to_agent(r: &Row) -> rusqlite::Result<Agent> {
    let role: String = r.get(2)?;
    let kind: String = r.get(3)?;
    let place = if kind == "pane" {
        AgentPlace::Pane { terminal: get_uuid(r, 5)? }
    } else {
        AgentPlace::Subagent { id: r.get(6)?, parent: get_optional_uuid(r, 7)? }
    };
    let reason: Option<String> = r.get(14)?;
    Ok(Agent {
        id: get_uuid(r, 0)?,
        workspace_id: get_uuid(r, 1)?,
        // A word from a newer build reads as a builder: an agent doing work.
        role: AgentRole::parse(&role).unwrap_or(AgentRole::Builder),
        place,
        harness: r.get(4)?,
        session_id: r.get(8)?,
        session_cwd: r.get(9)?,
        model: r.get(10)?,
        title: r.get(11)?,
        started_at: r.get(12)?,
        ended_at: r.get(13)?,
        end_reason: reason.map(|w| Verdict::parse(&w).unwrap_or(Verdict::Stopped)),
        stop_asked_by: r.get(15)?,
        stop_asked_at: r.get(16)?,
    })
}

pub(crate) fn agent_in(conn: &Connection, id: Uuid) -> Result<Agent> {
    conn.query_row(&format!("SELECT {AGENT_COLS} FROM agents WHERE id = ?1"), params![uuid_blob(id)], row_to_agent)
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

/// The agent already recorded for `place` on `workspace`: a subagent by its id
/// (open or not: an id is never reused), a pane by its open row.
fn existing(conn: &Connection, workspace: Uuid, harness: &str, place: &AgentPlace) -> Result<Option<Uuid>> {
    let found: Option<Vec<u8>> = match place {
        AgentPlace::Subagent { id, .. } => conn.query_row(
            "SELECT id FROM agents WHERE workspace_id = ?1 AND harness = ?2 AND subagent_id = ?3",
            params![uuid_blob(workspace), harness, id],
            |r| r.get(0),
        ),
        AgentPlace::Pane { terminal } => conn.query_row(
            "SELECT id FROM agents WHERE workspace_id = ?1 AND terminal_id = ?2 AND ended_at IS NULL
              ORDER BY started_at DESC LIMIT 1",
            params![uuid_blob(workspace), uuid_blob(*terminal)],
            |r| r.get(0),
        ),
    }
    .optional()
    .map_err(map_err)?;
    Ok(found.and_then(|b| Uuid::from_slice(&b).ok()))
}

impl Store {
    /// Record an agent on `workspace`'s board when it starts.
    ///
    /// Recording the same subagent again (same harness and id), or a pane that
    /// already has an open agent, fills in what is new and keeps the rest: the
    /// orchestrator's dispatch and the harness's start hook may both record
    /// one agent. A pane whose agent ended starts a new one.
    pub fn record_agent(&self, workspace: Uuid, new: &NewAgent) -> Result<Agent> {
        let harness = harness_word(&new.harness)?;
        let place = match &new.place {
            AgentPlace::Pane { terminal } => AgentPlace::Pane { terminal: *terminal },
            AgentPlace::Subagent { id, parent } => {
                AgentPlace::Subagent { id: clean(id, ID_MAX, true, "agent_id")?, parent: *parent }
            }
        };
        let session = new.session_id.as_deref().map(|v| clean(v, ID_MAX, false, "session")).transpose()?;
        let cwd = new.session_cwd.as_deref().map(|v| clean(v, PATH_MAX, false, "cwd")).transpose()?;
        let model = new.model.as_deref().map(|v| clean(v, WORD_MAX, false, "model")).transpose()?;
        let title = new.title.as_deref().map(|v| clean(v, TITLE_MAX, false, "title")).transpose()?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_exists(&tx, workspace)?;
        let id = match existing(&tx, workspace, &harness, &place)? {
            Some(id) => {
                let parent = match &place {
                    AgentPlace::Subagent { parent, .. } => parent.map(uuid_blob),
                    AgentPlace::Pane { .. } => None,
                };
                tx.execute(
                    "UPDATE agents SET role = ?2,
                            parent_terminal_id = coalesce(?3, parent_terminal_id),
                            session_id = coalesce(?4, session_id), session_cwd = coalesce(?5, session_cwd),
                            model = coalesce(nullif(?6, ''), model), title = coalesce(nullif(?7, ''), title)
                      WHERE id = ?1",
                    params![uuid_blob(id), new.role.as_str(), parent, session, cwd, model, title],
                )
                .map_err(map_err)?;
                id
            }
            None => {
                let id = Uuid::now_v7();
                let (terminal, subagent, parent) = match &place {
                    AgentPlace::Pane { terminal } => (Some(uuid_blob(*terminal)), None, None),
                    AgentPlace::Subagent { id, parent } => (None, Some(id.clone()), parent.map(uuid_blob)),
                };
                tx.execute(
                    "INSERT INTO agents (id, workspace_id, role, kind, harness, terminal_id, subagent_id,
                                         parent_terminal_id, session_id, session_cwd, model, title, started_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        uuid_blob(id),
                        uuid_blob(workspace),
                        new.role.as_str(),
                        place.kind(),
                        harness,
                        terminal,
                        subagent,
                        parent,
                        session,
                        cwd,
                        model.unwrap_or_default(),
                        title.unwrap_or_default(),
                        now_millis(),
                    ],
                )
                .map_err(map_err)?;
                id
            }
        };
        let agent = agent_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(agent)
    }

    /// An agent by id.
    pub fn agent(&self, id: Uuid) -> Result<Agent> {
        agent_in(&self.conn(), id)
    }

    /// `workspace`'s agents, oldest first: the open ones, and with `ended` the
    /// finished ones as well.
    pub fn agents(&self, workspace: Uuid, ended: bool) -> Result<Vec<Agent>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {AGENT_COLS} FROM agents WHERE workspace_id = ?1 AND (?2 OR ended_at IS NULL)
                  ORDER BY started_at, id"
            ))
            .map_err(map_err)?;
        let rows = stmt.query_map(params![uuid_blob(workspace), ended], row_to_agent).map_err(map_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(map_err)
    }

    /// The agent a harness's subagent id names, on any board: what a hook
    /// carrying only `agent_id` is about.
    pub fn subagent(&self, harness: &str, id: &str) -> Result<Option<Agent>> {
        let conn = self.conn();
        conn.query_row(
            &format!("SELECT {AGENT_COLS} FROM agents WHERE harness = ?1 AND subagent_id = ?2 ORDER BY started_at DESC LIMIT 1"),
            params![harness.trim().to_ascii_lowercase(), id],
            row_to_agent,
        )
        .optional()
        .map_err(map_err)
    }

    /// The open agent in pane `terminal`, if one is recorded.
    pub fn pane_agent(&self, terminal: Uuid) -> Result<Option<Agent>> {
        let conn = self.conn();
        conn.query_row(
            &format!(
                "SELECT {AGENT_COLS} FROM agents WHERE terminal_id = ?1 AND ended_at IS NULL ORDER BY started_at DESC LIMIT 1"
            ),
            params![uuid_blob(terminal)],
            row_to_agent,
        )
        .optional()
        .map_err(map_err)
    }

    /// Ask an agent to stop: the orchestrator's `agent stop`. Asking again
    /// keeps the first ask. An agent that already ended is refused.
    pub fn ask_agent_stop(&self, id: Uuid, actor: Actor) -> Result<Agent> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let agent = agent_in(&tx, id)?;
        if agent.ended_at.is_some() {
            return Err(invalid("agent_ended"));
        }
        tx.execute(
            "UPDATE agents SET stop_asked_by = coalesce(stop_asked_by, ?2), stop_asked_at = coalesce(stop_asked_at, ?3)
              WHERE id = ?1",
            params![uuid_blob(id), actor.to_string(), now_millis()],
        )
        .map_err(map_err)?;
        let agent = agent_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(agent)
    }

    /// End an agent, and every ticket it still works with the same reason.
    /// Ending one already ended keeps when and why it first ended.
    pub fn end_agent(&self, id: Uuid, reason: Verdict) -> Result<Agent> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let agent = agent_in(&tx, id)?;
        if agent.ended_at.is_none() {
            let now = now_millis();
            tx.execute(
                "UPDATE agents SET ended_at = ?2, end_reason = ?3 WHERE id = ?1",
                params![uuid_blob(id), now, reason.as_str()],
            )
            .map_err(map_err)?;
            tx.execute(
                "UPDATE ticket_work SET ended_at = ?2, verdict = coalesce(verdict, ?3)
                  WHERE agent_id = ?1 AND ended_at IS NULL",
                params![uuid_blob(id), now, reason.as_str()],
            )
            .map_err(map_err)?;
        }
        let agent = agent_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(agent)
    }
}

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
