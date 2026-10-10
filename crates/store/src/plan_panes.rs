//! A lane's agent pane (ov-457): the pane a lane dispatch opens, recorded on
//! the lane as its build agent, so the lane knows its agent and goes to
//! building with nobody recording either by hand.
//!
//! The link lives in the layer, never on the terminal: the pane is a lane
//! agent whose id is `pane:<terminal id>` (`pane_agent_id`). A terminal row
//! gains no column, and dropping the layer drops the link with it. What reads
//! it back is the pane's launch, which exports the lane's name
//! (`FARCOOLER_LANE`) on every launch from `lane_of_pane`.

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, uuid_blob};
use crate::plan::{AgentRecord, AgentRole, LANE_COLS, Lane, LaneCard, LaneState, LaneUpdate, row_to_lane};
use crate::store::Store;

/// The agents a lane can record: the board's subagent harnesses and cursor,
/// whose panes a lane dispatch opens too.
pub const LANE_HARNESSES: [&str; 3] = ["claude", "codex", "cursor"];

/// The lane agent id of the pane that is terminal `terminal`.
pub fn pane_agent_id(terminal: Uuid) -> String {
    format!("pane:{terminal}")
}

/// The terminal a lane agent id names, when it names a pane.
pub fn pane_of_agent(agent_id: &str) -> Option<Uuid> {
    agent_id.strip_prefix("pane:").and_then(|id| Uuid::parse_str(id).ok())
}

/// Where a lane's pane is working, written on the lane when it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneStart {
    pub terminal: Uuid,
    /// A preset's agent: `claude`, `codex:gpt-5` and so on.
    pub preset: String,
    pub worktree: Uuid,
    pub worktree_path: String,
    pub branch: String,
}

impl Store {
    /// `workspace`'s lane named `name`, that hasn't landed or been dropped.
    /// Names are matched as the layer keeps them unique: without case.
    pub fn live_lane_named(&self, workspace: Uuid, name: &str) -> Result<Lane> {
        self.conn()
            .query_row(
                &format!(
                    "SELECT {LANE_COLS} FROM lanes WHERE workspace_id = ?1 AND name = ?2 COLLATE NOCASE
                       AND state NOT IN ('landed', 'dropped')"
                ),
                params![uuid_blob(workspace), name.trim()],
                row_to_lane,
            )
            .optional()
            .map_err(map_err)?
            .ok_or(DomainError::NotFound)
    }

    /// A lane's cards in the order they were added: the order its agent
    /// works them.
    pub fn lane_cards_in_order(&self, lane: Uuid) -> Result<Vec<LaneCard>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT task_id, slice FROM lane_tasks WHERE lane_id = ?1 ORDER BY rowid")
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![uuid_blob(lane)], |r| {
                Ok(LaneCard { task_id: crate::models::get_uuid(r, 0)?, slice: r.get(1)? })
            })
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Record `start`'s pane as `lane`'s build agent, put the lane on the
    /// pane's worktree and branch, and move a queued lane to building, in one
    /// write.
    pub fn start_lane_pane(&self, lane: Uuid, start: &PaneStart, actor: Actor) -> Result<Lane> {
        let harness = start.preset.split_once(':').map_or(start.preset.as_str(), |(agent, _)| agent);
        let model = start.preset.split_once(':').map(|(_, model)| model.to_string());
        let before = self.lane(lane)?;
        let update = LaneUpdate {
            state: (before.state == LaneState::Queued).then_some(LaneState::Building),
            worktree_id: Some(start.worktree),
            worktree_path: Some(start.worktree_path.clone()),
            branch: Some(start.branch.clone()),
            agent: Some(AgentRecord {
                harness: harness.to_string(),
                agent_id: pane_agent_id(start.terminal),
                role: AgentRole::Build,
                model,
                ended: false,
            }),
            ..Default::default()
        };
        self.update_lane(lane, &update, actor)
    }

    /// The lane whose agent the pane `terminal` is, while that lane is live
    /// and the pane hasn't been recorded as finished.
    pub fn lane_of_pane(&self, terminal: Uuid) -> Result<Option<Lane>> {
        let cols: String = LANE_COLS.split(',').map(|c| format!("l.{}", c.trim())).collect::<Vec<_>>().join(", ");
        self.conn()
            .query_row(
                &format!(
                    "SELECT {cols} FROM lanes l JOIN lane_agents a ON a.lane_id = l.id
                      WHERE a.agent_id = ?1 AND a.ended_at IS NULL AND l.state NOT IN ('landed', 'dropped')
                      ORDER BY a.started_at DESC LIMIT 1"
                ),
                params![pane_agent_id(terminal)],
                row_to_lane,
            )
            .optional()
            .map_err(map_err)
    }

    /// The live lane that holds `task`, the newest when more than one does.
    pub fn live_lane_of_card(&self, task: Uuid) -> Result<Option<Lane>> {
        let cols: String = LANE_COLS.split(',').map(|c| format!("l.{}", c.trim())).collect::<Vec<_>>().join(", ");
        self.conn()
            .query_row(
                &format!(
                    "SELECT {cols} FROM lanes l JOIN lane_tasks c ON c.lane_id = l.id
                      WHERE c.task_id = ?1 AND l.state NOT IN ('landed', 'dropped')
                      ORDER BY l.created_at DESC LIMIT 1"
                ),
                params![uuid_blob(task)],
                row_to_lane,
            )
            .optional()
            .map_err(map_err)
    }

    /// The pane working `lane` now: its newest pane agent not recorded as
    /// finished (ov-455 addresses a lane through it).
    pub fn lane_pane(&self, lane: Uuid) -> Result<Option<Uuid>> {
        let id: Option<String> = self
            .conn()
            .query_row(
                "SELECT agent_id FROM lane_agents WHERE lane_id = ?1 AND agent_id LIKE 'pane:%' AND ended_at IS NULL
                  ORDER BY started_at DESC LIMIT 1",
                params![uuid_blob(lane)],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?;
        Ok(id.as_deref().and_then(pane_of_agent))
    }

    /// Record the pane `terminal` as finished on every lane it works.
    pub fn end_lane_pane(&self, terminal: Uuid, actor: Actor) -> Result<()> {
        let id = pane_agent_id(terminal);
        let lanes: Vec<(Uuid, String)> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare("SELECT lane_id, harness FROM lane_agents WHERE agent_id = ?1 AND ended_at IS NULL")
                .map_err(map_err)?;
            let rows = stmt
                .query_map(params![id], |r| Ok((crate::models::get_uuid(r, 0)?, r.get(1)?)))
                .map_err(map_err)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)?
        };
        for (lane, harness) in lanes {
            let record = AgentRecord { harness, agent_id: id.clone(), role: AgentRole::Build, model: None, ended: true };
            match self.record_lane_agent(lane, &record, actor) {
                Ok(_) | Err(DomainError::InvalidArgument { what: "lane_closed" }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "plan_panes_tests.rs"]
mod tests;
