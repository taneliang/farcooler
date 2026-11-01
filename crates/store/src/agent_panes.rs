//! An agent's pane (ov-457, ov-872): the pane `agent dispatch` opens, recorded
//! as an agent working its tickets, so nobody records either by hand.
//!
//! The link lives in `agents` and `ticket_work`, never on the terminal: a
//! terminal row gains no column. What reads it back is the pane's launch,
//! which exports the pane's first open ticket (`FARCOOLER_TICKET`), and a
//! message addressed to a ticket, which goes to the pane working it.

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::agents::{Agent, AgentPlace, AgentRole, NewAgent};
use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::store::Store;
use crate::ticket_lines::{LineUpdate, Stage};
use crate::ticket_work::{LinkedBy, Verdict, WorkRole};

/// Where a pane is working, written on its tickets when it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneStart {
    pub terminal: Uuid,
    /// A preset's agent: `claude`, `codex:gpt-5` and so on.
    pub preset: String,
    pub role: AgentRole,
    /// Its tickets, in the order it works them.
    pub tickets: Vec<Uuid>,
    pub worktree: Option<Uuid>,
    pub worktree_path: String,
    pub branch: String,
    pub title: String,
}

/// The work an agent in `role` does on the tickets it is dispatched to.
pub fn dispatch_role(role: AgentRole) -> WorkRole {
    match role {
        AgentRole::Reviewer => WorkRole::Review,
        AgentRole::Lander => WorkRole::Land,
        AgentRole::Orchestrator | AgentRole::Builder => WorkRole::Build,
    }
}

impl Store {
    /// Record `start`'s pane as an agent on `workspace`'s board working its
    /// tickets: each goes on the line if it isn't on it, and a builder puts
    /// its first ticket on the pane's worktree and branch and moves a waiting
    /// one to building. The agent.
    pub fn start_pane_agent(&self, workspace: Uuid, start: &PaneStart, actor: Actor) -> Result<Agent> {
        let (harness, model) = match start.preset.split_once(':') {
            Some((agent, model)) => (agent.to_string(), Some(model.to_string())),
            None => (start.preset.clone(), None),
        };
        let new = NewAgent {
            role: start.role,
            place: AgentPlace::Pane { terminal: start.terminal },
            harness,
            session_id: None,
            session_cwd: None,
            model,
            title: Some(start.title.clone()).filter(|t| !t.is_empty()),
        };
        let agent = self.record_agent(workspace, &new)?;
        let role = dispatch_role(start.role);
        for (i, task) in start.tickets.iter().enumerate() {
            let line = self.ensure_on_line(*task, actor)?;
            self.start_work(agent.id, *task, role, LinkedBy::Orchestrator, actor)?;
            if role != WorkRole::Build {
                continue;
            }
            let first = i == 0;
            let update = LineUpdate {
                stage: (first && line.stage == Stage::Waiting).then_some(Stage::Building),
                worktree_id: first.then_some(start.worktree).filter(|w| w.is_some()),
                worktree_path: (first && !start.worktree_path.is_empty()).then(|| start.worktree_path.clone()),
                branch: (first && !start.branch.is_empty() && line.branch.is_empty()).then(|| start.branch.clone()),
                ..LineUpdate::default()
            };
            if update != LineUpdate::default() {
                self.update_line(*task, &update, actor)?;
            }
        }
        self.agent(agent.id)
    }

    /// The tickets the open agent in pane `terminal` works now, in the order
    /// it was given them.
    pub fn tickets_of_pane(&self, terminal: Uuid) -> Result<Vec<Uuid>> {
        let Some(agent) = self.pane_agent(terminal)? else { return Ok(Vec::new()) };
        Ok(self.work_of(agent.id)?.into_iter().filter(|w| w.ended_at.is_none()).map(|w| w.task_id).collect())
    }

    /// The pane working `task` now: the newest open pane agent with open work
    /// on it.
    pub fn pane_of_ticket(&self, task: Uuid) -> Result<Option<Uuid>> {
        self.conn()
            .query_row(
                "SELECT a.terminal_id FROM ticket_work w JOIN agents a ON a.id = w.agent_id
                  WHERE w.task_id = ?1 AND w.ended_at IS NULL AND a.kind = 'pane' AND a.ended_at IS NULL
                  ORDER BY w.started_at DESC LIMIT 1",
                params![uuid_blob(task)],
                |r| get_uuid(r, 0),
            )
            .optional()
            .map_err(map_err)
    }

    /// End the agent in pane `terminal`, and its work: its terminal is gone.
    /// Nothing to do for a pane no agent is recorded in.
    pub fn end_pane_agent(&self, terminal: Uuid) -> Result<()> {
        match self.pane_agent(terminal)? {
            Some(agent) => self.end_agent(agent.id, Verdict::Stopped).map(|_| ()),
            None => Ok(()),
        }
    }

    /// The board an agent of pane `terminal` is on, refused as `NotFound` when
    /// the pane has none.
    pub fn board_of_pane_agent(&self, terminal: Uuid) -> Result<Uuid> {
        self.pane_agent(terminal)?.map(|a| a.workspace_id).ok_or(DomainError::NotFound)
    }
}

#[cfg(test)]
#[path = "agent_panes_tests.rs"]
mod tests;
