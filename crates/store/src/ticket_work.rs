//! Who works which ticket (ov-872): one row per agent, ticket and role.
//!
//! It replaces three records that said the same thing three ways: a lane's
//! agents (`lane_agents`), a subagent on a card (`task_workers`), and the
//! agent columns on a train. A builder or a reviewer may hold any number of
//! tickets, and a reviewer's tickets may come from any builder.
//!
//! The table is made by `line_migration.rs`.

use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::agents::{AgentRole, agent_in};
use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;
use crate::ticket_lines::{LineSubject, line_event, task_key};

/// What an agent does for a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkRole {
    /// Builds it.
    Build,
    /// A fix round after a review or a failed landing.
    Fix,
    /// Reviews it.
    Review,
    /// Lands it.
    Land,
}

impl WorkRole {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            WorkRole::Build => "build",
            WorkRole::Fix => "fix",
            WorkRole::Review => "review",
            WorkRole::Land => "land",
        }
    }

    /// The role a stored word names.
    pub fn parse(raw: &str) -> Option<WorkRole> {
        Some(match raw {
            "build" => WorkRole::Build,
            "fix" => WorkRole::Fix,
            "review" => WorkRole::Review,
            "land" => WorkRole::Land,
            _ => return None,
        })
    }

    /// Whether an agent in `role` may do this. The orchestrator may do any of
    /// it itself.
    pub fn fits(self, role: AgentRole) -> bool {
        match role {
            AgentRole::Orchestrator => true,
            AgentRole::Builder => matches!(self, WorkRole::Build | WorkRole::Fix),
            AgentRole::Reviewer => self == WorkRole::Review,
            AgentRole::Lander => self == WorkRole::Land,
        }
    }
}

/// How an agent's work on a ticket ended, or why the agent ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Finished,
    Failed,
    /// Killed or stopped.
    Stopped,
    /// Its work went back to the orchestrator.
    HandedBack,
    /// Its ticket was done or cancelled while it worked it.
    TaskClosed,
    /// Linked from its description, then recorded on another ticket.
    Relinked,
    /// A reviewer passed it.
    Approved,
    /// A reviewer asked for changes.
    ChangesAsked,
}

impl Verdict {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Finished => "finished",
            Verdict::Failed => "failed",
            Verdict::Stopped => "stopped",
            Verdict::HandedBack => "handed_back",
            Verdict::TaskClosed => "task_closed",
            Verdict::Relinked => "relinked",
            Verdict::Approved => "approved",
            Verdict::ChangesAsked => "changes_asked",
        }
    }

    /// The verdict a stored word names.
    pub fn parse(raw: &str) -> Option<Verdict> {
        Some(match raw {
            "finished" => Verdict::Finished,
            "failed" => Verdict::Failed,
            "stopped" => Verdict::Stopped,
            "handed_back" => Verdict::HandedBack,
            "task_closed" => Verdict::TaskClosed,
            "relinked" => Verdict::Relinked,
            "approved" => Verdict::Approved,
            "changes_asked" => Verdict::ChangesAsked,
            _ => return None,
        })
    }
}

/// Who linked an agent to a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedBy {
    /// The orchestrator's dispatch.
    Orchestrator,
    /// The runner, from a key at the start of a subagent's description. The
    /// orchestrator recording it on another ticket ends this link as relinked.
    Description,
}

impl LinkedBy {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            LinkedBy::Orchestrator => "orchestrator",
            LinkedBy::Description => "description",
        }
    }
}

/// One agent's work on one ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketWork {
    pub agent_id: Uuid,
    pub task_id: Uuid,
    pub role: WorkRole,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub verdict: Option<Verdict>,
    pub linked_by: LinkedBy,
}

fn invalid(what: &'static str) -> DomainError {
    DomainError::InvalidArgument { what }
}

const WORK_COLS: &str = "agent_id, task_id, role, started_at, ended_at, verdict, linked_by";

pub(crate) fn row_to_work(r: &Row) -> rusqlite::Result<TicketWork> {
    let role: String = r.get(2)?;
    let verdict: Option<String> = r.get(5)?;
    let linked: String = r.get(6)?;
    Ok(TicketWork {
        agent_id: get_uuid(r, 0)?,
        task_id: get_uuid(r, 1)?,
        role: WorkRole::parse(&role).unwrap_or(WorkRole::Build),
        started_at: r.get(3)?,
        ended_at: r.get(4)?,
        verdict: verdict.map(|w| Verdict::parse(&w).unwrap_or(Verdict::Stopped)),
        linked_by: if linked == "description" { LinkedBy::Description } else { LinkedBy::Orchestrator },
    })
}

fn work_in(conn: &Connection, agent: Uuid, task: Uuid, role: WorkRole) -> Result<TicketWork> {
    conn.query_row(
        &format!("SELECT {WORK_COLS} FROM ticket_work WHERE agent_id = ?1 AND task_id = ?2 AND role = ?3"),
        params![uuid_blob(agent), uuid_blob(task), role.as_str()],
        row_to_work,
    )
    .optional()
    .map_err(map_err)?
    .ok_or(DomainError::NotFound)
}

pub(crate) fn works(conn: &Connection, column: &str, id: Uuid) -> Result<Vec<TicketWork>> {
    let mut stmt = conn
        .prepare(&format!("SELECT {WORK_COLS} FROM ticket_work WHERE {column} = ?1 ORDER BY started_at, agent_id, role"))
        .map_err(map_err)?;
    let rows = stmt.query_map(params![uuid_blob(id)], row_to_work).map_err(map_err)?;
    rows.collect::<rusqlite::Result<_>>().map_err(map_err)
}

/// "Builder", "Reviewer": how a ticket's timeline names an agent.
fn said(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Orchestrator => "The orchestrator",
        AgentRole::Builder => "A builder",
        AgentRole::Reviewer => "A reviewer",
        AgentRole::Lander => "The lander",
    }
}

impl Store {
    /// Record that `agent` works `task` as `role`. The ticket must be on the
    /// agent's board and the role must fit the agent's. Recording it again
    /// reopens it.
    ///
    /// Linked by the orchestrator, it ends any link the runner made from the
    /// agent's description to another ticket, as relinked.
    pub fn start_work(&self, agent: Uuid, task: Uuid, role: WorkRole, linked_by: LinkedBy, actor: Actor) -> Result<TicketWork> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let who = agent_in(&tx, agent)?;
        if who.ended_at.is_some() {
            return Err(invalid("agent_ended"));
        }
        if !role.fits(who.role) {
            return Err(invalid("role"));
        }
        crate::ticket_lines::check_tickets(&tx, who.workspace_id, &[task])?;
        let now = now_millis();
        if linked_by == LinkedBy::Orchestrator {
            tx.execute(
                "UPDATE ticket_work SET ended_at = ?3, verdict = 'relinked'
                  WHERE agent_id = ?1 AND task_id <> ?2 AND linked_by = 'description' AND ended_at IS NULL",
                params![uuid_blob(agent), uuid_blob(task), now],
            )
            .map_err(map_err)?;
        }
        let fresh = tx
            .execute(
                "INSERT INTO ticket_work (agent_id, task_id, role, started_at, linked_by) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(agent_id, task_id, role) DO UPDATE SET ended_at = NULL, verdict = NULL
                  WHERE ended_at IS NOT NULL",
                params![uuid_blob(agent), uuid_blob(task), role.as_str(), now, linked_by.as_str()],
            )
            .map_err(map_err)?;
        if fresh > 0 {
            let body = format!("{} started on {}.", said(who.role), task_key(&tx, task));
            let extra = serde_json::json!({ "agent": agent.to_string(), "role": role.as_str() });
            line_event(&tx, LineSubject::Ticket(task), "agent", actor, &body, extra)?;
        }
        let work = work_in(&tx, agent, task, role)?;
        tx.commit().map_err(map_err)?;
        Ok(work)
    }

    /// End `agent`'s work on `task` as `role` with `verdict`. Ending it again
    /// keeps the first ending.
    pub fn end_work(&self, agent: Uuid, task: Uuid, role: WorkRole, verdict: Verdict, actor: Actor) -> Result<TicketWork> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let who = agent_in(&tx, agent)?;
        work_in(&tx, agent, task, role)?;
        let ended = tx
            .execute(
                "UPDATE ticket_work SET ended_at = ?4, verdict = ?5
                  WHERE agent_id = ?1 AND task_id = ?2 AND role = ?3 AND ended_at IS NULL",
                params![uuid_blob(agent), uuid_blob(task), role.as_str(), now_millis(), verdict.as_str()],
            )
            .map_err(map_err)?;
        if ended > 0 {
            let body = format!("{} {}.", said(who.role), verdict_said(verdict));
            let extra = serde_json::json!({ "agent": agent.to_string(), "role": role.as_str(), "verdict": verdict.as_str() });
            line_event(&tx, LineSubject::Ticket(task), "agent", actor, &body, extra)?;
        }
        let work = work_in(&tx, agent, task, role)?;
        tx.commit().map_err(map_err)?;
        Ok(work)
    }

    /// Everyone who worked `task`, oldest first.
    pub fn work_on(&self, task: Uuid) -> Result<Vec<TicketWork>> {
        works(&self.conn(), "task_id", task)
    }

    /// Every ticket `agent` worked, oldest first.
    pub fn work_of(&self, agent: Uuid) -> Result<Vec<TicketWork>> {
        works(&self.conn(), "agent_id", agent)
    }
}

/// The end of the timeline's sentence: "A reviewer asked for changes."
fn verdict_said(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Finished => "finished",
        Verdict::Failed => "failed",
        Verdict::Stopped => "stopped",
        Verdict::HandedBack => "handed its work back",
        Verdict::TaskClosed => "closed with its ticket",
        Verdict::Relinked => "was recorded on another ticket",
        Verdict::Approved => "approved it",
        Verdict::ChangesAsked => "asked for changes",
    }
}

#[cfg(test)]
#[path = "ticket_work_tests.rs"]
mod tests;
