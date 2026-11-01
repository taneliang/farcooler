//! The agents on the plan (ov-503, ov-518, ov-872): every recorded agent still
//! at work on this board, with the tickets it works and what it spent.
//!
//! # Recorded, then observed
//!
//! An agent is a row of `agents` (`agents.rs`), written when it starts: by the
//! orchestrator's dispatch, a pane's launch, or the runner linking a subagent
//! from its description. What it is doing is observed and never stored:
//! whether it is working, when it last moved, and its spend, from
//! `agent_turns`. What it is doing now (a pane's last line, a subagent's last
//! tool call) is the daemon's to add, since only it watches them live.
//!
//! # Only agents still at work
//!
//! The list holds open agents: a pane while its terminal runs, and a subagent
//! while it runs. A subagent's end is recorded when the runner reads it
//! (`watch::workers`), but a session that died ends nothing, so an open
//! subagent is listed only while it moved in the last `RECENT_MS`, or while the
//! pane that spawned it still runs and it moved within `STALLED_AFTER_MS`: a
//! landing's full test run writes nothing for many minutes. One just
//! dispatched, with no turn yet, is listed for `RECENT_MS`.
//!
//! What every agent of the day spent, listed or not, is `Fleet::spend`, so the
//! header can say what the day cost.

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::agents::{AGENT_COLS, Agent, AgentPlace, AgentRole, row_to_agent};
use crate::error::map_err;
use crate::landing_phase::STALLED_AFTER_MS;
use crate::models::uuid_blob;
use crate::plan_read::{Spend, TURNS_OF_AGENT};
use crate::ticket_work::TicketWork;

/// The window the spend covers: a day.
pub const WINDOW_MS: i64 = 86_400_000;

/// A subagent is running only if it also moved this recently: a record left
/// open by a session that died says only that nobody closed it. Fifteen
/// minutes covers a long tool call.
pub const RECENT_MS: i64 = 15 * 60_000;

/// One agent on the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentView {
    pub agent: Agent,
    /// What it calls itself on a row: its title, else for a subagent the
    /// description its parent gave it (`subagent_descriptions`).
    pub label: String,
    /// The tickets it works now, oldest first.
    pub tickets: Vec<TicketWork>,
    /// Mid-turn: a pane by the runner's live state, else by a turn in the last
    /// `RECENT_MS`.
    pub working: bool,
    /// Unix milliseconds of its last turn; 0 with none.
    pub last_at: i64,
    /// Its turns in the window.
    pub spend: Spend,
}

/// The agents at work, and what every agent of the window spent.
#[derive(Debug, Default)]
pub(crate) struct Fleet {
    /// Working ones first, the orchestrator before the rest, then by tokens,
    /// most first.
    pub agents: Vec<AgentView>,
    /// Every turn of the board's project in the last `WINDOW_MS`, by any agent,
    /// listed or not. Dollars only when every turn was priced.
    pub spend: Spend,
}

/// Whether `terminal` still runs, and the agent session it runs.
fn terminal_runs(conn: &Connection, terminal: Uuid) -> Result<Option<(bool, Option<String>)>> {
    conn.query_row(
        "SELECT exit_code IS NULL AND exit_signal IS NULL, agent_session_id FROM terminals WHERE id = ?1",
        params![uuid_blob(terminal)],
        |r| Ok((r.get::<_, bool>(0)?, r.get::<_, Option<String>>(1)?)),
    )
    .optional()
    .map_err(crate::error::map_err)
}

/// The running terminal of `workspace` whose agent session is `session`: the
/// parent of a subagent whose record names no pane.
fn session_parent(conn: &Connection, workspace: Uuid, session: Option<&str>) -> Result<Option<Uuid>> {
    let Some(session) = session.filter(|s| !s.is_empty()) else { return Ok(None) };
    let found: Option<Vec<u8>> = conn
        .query_row(
            "SELECT id FROM terminals WHERE workspace_id = ?1 AND agent_session_id = ?2
                AND exit_code IS NULL AND exit_signal IS NULL
              ORDER BY rowid DESC LIMIT 1",
            params![uuid_blob(workspace), session],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_err)?;
    Ok(found.and_then(|b| Uuid::from_slice(&b).ok()))
}

/// One agent's turns in the window: spend and the last one's end.
fn window_spend(conn: &Connection, agent: Uuid, since: i64) -> Result<(Spend, i64)> {
    let (input, output, read, write, priced, unpriced, runs, last): (i64, i64, i64, i64, i64, i64, i64, Option<i64>) = conn
        .query_row(
            &format!(
                "SELECT CAST(coalesce(sum(m.input_tokens), 0) AS INTEGER),
                        CAST(coalesce(sum(m.output_tokens), 0) AS INTEGER),
                        CAST(coalesce(sum(m.cache_read_tokens), 0) AS INTEGER),
                        CAST(coalesce(sum(m.cache_write_tokens), 0) AS INTEGER),
                        CAST(coalesce(sum(m.cost_micros), 0) AS INTEGER),
                        coalesce(sum(m.cost_micros IS NULL), 0),
                        count(DISTINCT t.id),
                        max(t.ended_at)
                   FROM agents a JOIN agent_turns t ON {TURNS_OF_AGENT}
                   JOIN agent_turn_models m ON m.turn_id = t.id
                  WHERE a.id = ?1 AND t.ended_at >= ?2"
            ),
            params![uuid_blob(agent), since],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        )
        .map_err(map_err)?;
    let n = |v: i64| v.max(0) as u64;
    let spend = Spend {
        input_tokens: n(input),
        output_tokens: n(output),
        cache_read_tokens: n(read),
        cache_write_tokens: n(write),
        cost_micros: (runs > 0 && unpriced == 0).then_some(priced),
        runs: runs.max(0) as u32,
        unmeasured_agents: u32::from(runs == 0),
        shared_agents: 0,
    };
    Ok((spend, last.unwrap_or(0)))
}

/// What the board's project spent in the window, every turn counted: those in
/// the board's repository, or filed to no repository and to one of its cards.
fn project_spend(conn: &Connection, workspace: Uuid, since: i64) -> Result<Spend> {
    let (input, output, read, write, priced, unpriced, runs): (i64, i64, i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT CAST(coalesce(sum(m.input_tokens), 0) AS INTEGER),
                    CAST(coalesce(sum(m.output_tokens), 0) AS INTEGER),
                    CAST(coalesce(sum(m.cache_read_tokens), 0) AS INTEGER),
                    CAST(coalesce(sum(m.cache_write_tokens), 0) AS INTEGER),
                    CAST(coalesce(sum(m.cost_micros), 0) AS INTEGER),
                    coalesce(sum(m.cost_micros IS NULL), 0),
                    count(DISTINCT t.id)
               FROM agent_turns t JOIN agent_turn_models m ON m.turn_id = t.id
              WHERE t.ended_at >= ?1
                AND (t.repository_id = (SELECT repository_id FROM workspaces WHERE id = ?2)
                     OR (t.repository_id IS NULL
                         AND t.task_id IN (SELECT id FROM tasks WHERE workspace_id = ?2)))",
            params![since, uuid_blob(workspace)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .map_err(map_err)?;
    let n = |v: i64| v.max(0) as u64;
    Ok(Spend {
        input_tokens: n(input),
        output_tokens: n(output),
        cache_read_tokens: n(read),
        cache_write_tokens: n(write),
        cost_micros: (runs > 0 && unpriced == 0).then_some(priced),
        runs: runs.max(0) as u32,
        unmeasured_agents: 0,
        shared_agents: 0,
    })
}

/// The open work of `agent`, oldest first.
fn open_work(conn: &Connection, agent: Uuid) -> Result<Vec<TicketWork>> {
    let mut stmt = conn
        .prepare(
            "SELECT agent_id, task_id, role, started_at, ended_at, verdict, linked_by FROM ticket_work
              WHERE agent_id = ?1 AND ended_at IS NULL ORDER BY started_at, task_id, role",
        )
        .map_err(map_err)?;
    let rows = stmt.query_map(params![uuid_blob(agent)], crate::ticket_work::row_to_work).map_err(map_err)?;
    rows.collect::<rusqlite::Result<_>>().map_err(map_err)
}

/// The agents at work on `workspace`'s board, and the window's spend.
pub(crate) fn agents_of(conn: &Connection, workspace: Uuid, now_ms: i64, live: &dyn Fn(Uuid) -> Option<bool>) -> Result<Fleet> {
    let since = now_ms - WINDOW_MS;
    let open: Vec<Agent> = {
        let mut stmt = conn
            .prepare(&format!("SELECT {AGENT_COLS} FROM agents WHERE workspace_id = ?1 AND ended_at IS NULL ORDER BY started_at, id"))
            .map_err(map_err)?;
        stmt.query_map(params![uuid_blob(workspace)], row_to_agent)
            .map_err(map_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(map_err)?
    };
    let mut agents = Vec::new();
    for agent in open {
        let (spend, last_at) = window_spend(conn, agent.id, since)?;
        let recent = now_ms - last_at.max(agent.started_at) <= RECENT_MS;
        let working = match &agent.place {
            AgentPlace::Pane { terminal } => {
                if !terminal_runs(conn, *terminal)?.is_some_and(|(runs, _)| runs) {
                    continue;
                }
                live(*terminal).unwrap_or(recent && last_at > 0)
            }
            AgentPlace::Subagent { parent, .. } => {
                let parent = match parent {
                    Some(t) => Some(*t),
                    None => session_parent(conn, workspace, agent.session_id.as_deref())?,
                };
                // "Runs" means the parent's terminal runs the very session the
                // subagent was recorded under: a /clear or a relaunch in the
                // same pane starts another session, and the old one's
                // subagents are gone.
                let parent_runs = match parent {
                    None => false,
                    Some(t) => terminal_runs(conn, t)?.is_some_and(|(runs, now)| {
                        runs && match (&agent.session_id, &now) {
                            (Some(then), Some(now)) => then == now,
                            _ => true,
                        }
                    }),
                };
                let held = parent_runs && now_ms - last_at.max(agent.started_at) <= STALLED_AFTER_MS;
                if !(recent || held) {
                    continue;
                }
                recent
            }
        };
        let label = match (&agent.place, agent.title.is_empty()) {
            (AgentPlace::Subagent { id, .. }, true) => conn
                .query_row("SELECT description FROM subagent_descriptions WHERE agent_id = ?1", params![id], |r| r.get(0))
                .optional()
                .map_err(map_err)?
                .unwrap_or_default(),
            _ => agent.title.clone(),
        };
        let tickets = open_work(conn, agent.id)?;
        agents.push(AgentView { label, tickets, working, last_at, spend, agent });
    }
    agents.sort_by(|a, b| {
        b.working
            .cmp(&a.working)
            .then_with(|| (b.agent.role == AgentRole::Orchestrator).cmp(&(a.agent.role == AgentRole::Orchestrator)))
            .then_with(|| b.spend.tokens().cmp(&a.spend.tokens()))
            .then_with(|| a.agent.started_at.cmp(&b.agent.started_at))
            .then_with(|| a.agent.id.cmp(&b.agent.id))
    });
    Ok(Fleet { agents, spend: project_spend(conn, workspace, since)? })
}

#[cfg(test)]
#[path = "plan_agents_tests.rs"]
mod tests;
