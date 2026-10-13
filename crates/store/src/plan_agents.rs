//! The agents on the plan (ov-503, phase 3): every agent that ran a turn for
//! this board's project in the last 24 hours, joined to what it worked for.
//!
//! # Derived, never typed
//!
//! Nothing here is stored. An agent is a group of `agent_turns` rows, and the
//! join to its lane, train or card is read from the typed records that happen
//! to exist (`lane_agents`, a train's `agent_id`, `task_workers`, a terminal's
//! role). A record that was never written hides nothing: the agent is still
//! counted, as "not on a lane" (`PlanAgentRole::Subagent` or `Agent` with no
//! lane, train or card), so the board's totals and this list agree.
//!
//! # What an agent is
//!
//! - A Claude **subagent**: the rows of kind `subagent`, keyed
//!   `claude-log:agent:<agentId>` (`daemon/src/usage.rs`). One row per run,
//!   growing as the agent works. Its rows are filed to its orchestrator's
//!   terminal too, so rows of this kind are never counted as the pane's own.
//! - A **pane**: the other rows of one terminal, whatever its harness. The
//!   workspace's orchestrator terminal is the orchestrator; any other is a
//!   lane's pane or an unjoined agent.
//! - Turns filed to no terminal (a log read with no pane) are grouped by
//!   harness, so their spend is counted rather than dropped.
//!
//! # What a transcript carries
//!
//! A subagent's own transcript carries no description. The one-line summary
//! the orchestrator gave it is on the *parent's* `Agent` tool call and in the
//! sibling `agent-<id>.meta.json`. The runner keeps it two ways: as the
//! `label` of a `task_workers` record (which wins), and, for every subagent
//! whose meta file it read, in `subagent_descriptions` (ov-509), so `label` is
//! empty only for an agent whose meta file was never read.
//!
//! # Time
//!
//! A turn counts when it ended in the last `WINDOW_MS`. A subagent's run is
//! one row that grows, so it counts whole when it last moved in the window,
//! even if it began before it.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::error::map_err;
use crate::models::{TerminalRole, get_uuid, uuid_blob};
use crate::plan_read::LaneSpend;
use crate::trains::train_number;
use farcooler_core::Result;

/// Agents that ran a turn this recently are on the list: a day.
pub const WINDOW_MS: i64 = 86_400_000;

/// An agent is working only if it also moved this recently: a record left open
/// (nothing tells the runner an agent ended) or a pane left running says only
/// that it hasn't been closed. Fifteen minutes covers a long tool call.
const RECENT_MS: i64 = 15 * 60_000;

/// What an agent is, to the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanAgentRole {
    /// The workspace's orchestrator terminal.
    Orchestrator,
    Build,
    Review,
    Fix,
    /// The agent integrating a train.
    Integrator,
    /// A subagent no lane, train or worker record names.
    Subagent,
    /// A pane no lane or card names.
    Agent,
}

/// One agent on the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanAgent {
    /// `agent:<agentId>` for a subagent, `pane:<terminal id>` for a pane
    /// (the lane agent id `plan_panes::pane_agent_id` records), `turns:<harness>`
    /// for turns filed to no terminal.
    pub id: String,
    pub role: PlanAgentRole,
    pub harness: String,
    /// The model that spent the most; empty when none was named.
    pub model: String,
    /// What the orchestrator called a subagent: its worker record's label, else
    /// the description its parent gave it (`subagent_descriptions`).
    pub label: String,
    pub lane: Option<(Uuid, String)>,
    /// The train it integrates, with the name a person calls it.
    pub train: Option<(Uuid, String)>,
    pub card: Option<(Uuid, String)>,
    pub terminal: Option<Uuid>,
    pub working: bool,
    /// Unix milliseconds of its first turn in the window.
    pub started_at: i64,
    /// Unix milliseconds of its last turn.
    pub last_at: i64,
    /// Its turns in the window, in the lane spend's words.
    pub spend: LaneSpend,
}

/// What the turns of one agent add up to, before it is joined.
#[derive(Default)]
struct Group {
    harness: String,
    terminal: Option<Uuid>,
    task: Option<Uuid>,
    started: Option<i64>,
    last: i64,
    spend: LaneSpend,
    priced: i64,
    unpriced: bool,
    models: HashMap<String, u64>,
    runs: std::collections::HashSet<Uuid>,
}

/// Which agent a turn belongs to.
fn agent_key(kind: &str, turn_key: &str, terminal: Option<Uuid>, harness: &str) -> String {
    if kind == "subagent" {
        return format!("agent:{}", turn_key.strip_prefix("claude-log:agent:").unwrap_or(turn_key));
    }
    match terminal {
        Some(t) => crate::plan_panes::pane_agent_id(t),
        None => format!("turns:{harness}"),
    }
}

/// `train-72` is "Train 72"; a train the orchestrator titled keeps its title.
fn train_words(name: &str, title: &str) -> String {
    if !title.is_empty() {
        return title.to_string();
    }
    train_number(name).map_or_else(|| name.to_string(), |n| format!("Train {n}"))
}

/// Every agent with turns in `workspace`'s project in the last `WINDOW_MS`:
/// working ones first, then by tokens, most first.
pub(crate) fn agents_of(
    conn: &Connection,
    workspace: Uuid,
    now_ms: i64,
    live: &dyn Fn(Uuid) -> Option<bool>,
) -> Result<Vec<PlanAgent>> {
    let ws = uuid_blob(workspace);
    // The project is the board's repository, as the week's cost counts it (a
    // turn filed to no repository belongs when one of this board's cards does).
    let mut stmt = conn
        .prepare(
            "SELECT t.id, t.turn_key, t.kind, t.terminal_id, t.task_id, t.harness, t.started_at, t.ended_at,
                    m.model, m.input_tokens, m.output_tokens, m.cache_read_tokens, m.cache_write_tokens,
                    m.cost_micros
               FROM agent_turns t JOIN agent_turn_models m ON m.turn_id = t.id
              WHERE t.ended_at >= ?1
                AND (t.repository_id = (SELECT repository_id FROM workspaces WHERE id = ?2)
                     OR (t.repository_id IS NULL
                         AND t.task_id IN (SELECT id FROM tasks WHERE workspace_id = ?2)))
              ORDER BY t.ended_at, t.id",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![now_ms - WINDOW_MS, ws], |r| {
            let terminal: Option<Vec<u8>> = r.get(3)?;
            let task: Option<Vec<u8>> = r.get(4)?;
            Ok((
                get_uuid(r, 0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                terminal.and_then(|b| Uuid::from_slice(&b).ok()),
                task.and_then(|b| Uuid::from_slice(&b).ok()),
                r.get::<_, String>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, String>(8)?,
                [r.get::<_, i64>(9)?, r.get::<_, i64>(10)?, r.get::<_, i64>(11)?, r.get::<_, i64>(12)?],
                r.get::<_, Option<i64>>(13)?,
            ))
        })
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;

    let mut groups: HashMap<String, Group> = HashMap::new();
    for (turn, key, kind, terminal, task, harness, started, ended, model, counts, cost) in rows {
        let g = groups.entry(agent_key(&kind, &key, terminal, &harness)).or_default();
        g.harness = harness;
        g.terminal = g.terminal.or(terminal);
        g.task = task.or(g.task);
        g.started = Some(g.started.map_or(started.unwrap_or(ended), |s| s.min(started.unwrap_or(ended))));
        g.last = g.last.max(ended);
        let n = |v: i64| v.max(0) as u64;
        g.spend.input_tokens += n(counts[0]);
        g.spend.output_tokens += n(counts[1]);
        g.spend.cache_read_tokens += n(counts[2]);
        g.spend.cache_write_tokens += n(counts[3]);
        *g.models.entry(model).or_default() += counts.iter().map(|c| n(*c)).sum::<u64>();
        match cost {
            Some(micros) => g.priced += micros,
            None => g.unpriced = true,
        }
        if g.runs.insert(turn) {
            g.spend.runs += 1;
        }
    }

    let mut out = Vec::new();
    for (id, g) in groups {
        out.push(join(conn, workspace, &id, g, now_ms, live)?);
    }
    out.sort_by(|a, b| {
        b.working
            .cmp(&a.working)
            .then_with(|| total(&b.spend).cmp(&total(&a.spend)))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(out)
}

fn total(s: &LaneSpend) -> u64 {
    s.input_tokens + s.output_tokens + s.cache_read_tokens + s.cache_write_tokens
}

/// One lane agent record: the lane, the role and whether it's still open.
struct LaneJoin {
    lane: Uuid,
    name: String,
    role: String,
    model: String,
    open: bool,
}

/// The lane that records `agent_id`, the newest live one first. A subagent's
/// id is Claude's; a pane's is `pane:<terminal>` in any harness.
fn lane_of(conn: &Connection, ws: &[u8], agent_id: &str, is_subagent: bool) -> Result<Option<LaneJoin>> {
    conn.query_row(
        "SELECT l.id, l.name, a.role, a.model, a.ended_at IS NULL AND l.state NOT IN ('landed', 'dropped')
                AND (a.role <> 'build' OR l.state IN ('queued', 'building'))
           FROM lane_agents a JOIN lanes l ON l.id = a.lane_id
          WHERE l.workspace_id = ?1 AND a.agent_id = ?2 AND (?3 = 0 OR a.harness = 'claude')
          ORDER BY (l.state NOT IN ('landed', 'dropped')) DESC, a.started_at DESC LIMIT 1",
        params![ws, agent_id, is_subagent],
        |r| Ok(LaneJoin { lane: get_uuid(r, 0)?, name: r.get(1)?, role: r.get(2)?, model: r.get(3)?, open: r.get(4)? }),
    )
    .optional()
    .map_err(map_err)
}

fn join(
    conn: &Connection,
    workspace: Uuid,
    id: &str,
    g: Group,
    now_ms: i64,
    live: &dyn Fn(Uuid) -> Option<bool>,
) -> Result<PlanAgent> {
    let ws = uuid_blob(workspace);
    let is_subagent = id.starts_with("agent:");
    let agent_id = id.strip_prefix("agent:").unwrap_or(id);

    // The lane: an agent recorded on a lane, the newest live one first.
    let mut lane = lane_of(conn, &ws, agent_id, is_subagent)?;
    let recorded_on_lane = lane.is_some();

    // The train it integrates.
    let train: Option<(Uuid, String, bool)> = conn
        .query_row(
            "SELECT id, name, title, agent_ended_at IS NULL FROM board_trains
              WHERE workspace_id = ?1 AND agent_id = ?2 AND state NOT IN ('landed', 'dropped')
                AND (?3 = 0 OR agent_harness = 'claude')
              ORDER BY created_at DESC LIMIT 1",
            params![ws, agent_id, is_subagent],
            |r| {
                let (name, title): (String, String) = (r.get(1)?, r.get(2)?);
                Ok((get_uuid(r, 0)?, train_words(&name, &title), r.get(3)?))
            },
        )
        .optional()
        .map_err(map_err)?;

    // A subagent's worker record: the description it was given, and the card.
    let worker: Option<(Uuid, String, String, bool)> = if is_subagent {
        conn.query_row(
            "SELECT w.task_id, w.label, w.model, w.ended_at IS NULL FROM task_workers w
               JOIN tasks t ON t.id = w.task_id
              WHERE w.harness = 'claude' AND w.agent_id = ?1 AND t.workspace_id = ?2
              ORDER BY w.started_at DESC, w.rowid DESC LIMIT 1",
            params![agent_id, ws],
            |r| Ok((get_uuid(r, 0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(map_err)?
    } else {
        None
    };

    // A pane: its terminal's role, whether it still runs, and the card it opened for.
    let terminal = g.terminal.filter(|_| !is_subagent);
    let pane: Option<(i64, bool, Option<Uuid>, bool)> = match terminal {
        None => None,
        Some(t) => conn
            .query_row(
                "SELECT role, exit_code IS NULL AND exit_signal IS NULL, task_id, workspace_id IS ?2
                   FROM terminals WHERE id = ?1",
                params![uuid_blob(t), ws],
                |r| {
                    let task: Option<Vec<u8>> = r.get(2)?;
                    Ok((r.get(0)?, r.get(1)?, task.and_then(|b| Uuid::from_slice(&b).ok()), r.get(3)?))
                },
            )
            .optional()
            .map_err(map_err)?,
    };
    // A subagent no record puts on a lane works for the lane of the pane that
    // spawned it, as the lane's spend counts it (`spend_of`).
    if is_subagent && lane.is_none() && train.is_none() {
        if let Some(t) = g.terminal {
            lane = lane_of(conn, &ws, &crate::plan_panes::pane_agent_id(t), false)?;
        }
    }
    let is_orchestrator =
        pane.is_some_and(|(role, _, _, ours)| ours && TerminalRole::from_i64(role) == TerminalRole::Orchestrator);

    let role = if is_orchestrator {
        PlanAgentRole::Orchestrator
    } else if let Some(l) = &lane {
        match l.role.as_str() {
            "review" => PlanAgentRole::Review,
            "fix" => PlanAgentRole::Fix,
            _ => PlanAgentRole::Build,
        }
    } else if train.is_some() {
        PlanAgentRole::Integrator
    } else if is_subagent {
        PlanAgentRole::Subagent
    } else {
        PlanAgentRole::Agent
    };

    // The card: a worker's task, else the terminal's, else the one the turns were filed to.
    let card_id = worker.as_ref().map(|w| w.0).or(pane.and_then(|p| p.2)).or(g.task);
    let card: Option<(Uuid, String)> = match card_id {
        None => None,
        Some(task) => conn
            .query_row("SELECT key FROM tasks WHERE id = ?1 AND workspace_id = ?2", params![uuid_blob(task), ws], |r| r.get(0))
            .optional()
            .map_err(map_err)?
            .map(|key| (task, key)),
    };

    // Working: moved in the last `RECENT_MS`, and not recorded as finished. A
    // pane also needs its terminal running, and the runner's live state for it
    // (`live`) decides when it has one; a subagent with a lane, train or
    // worker record of its own needs one of them open.
    let recent = now_ms - g.last <= RECENT_MS;
    let working = if is_subagent {
        let recorded = recorded_on_lane || train.is_some() || worker.is_some();
        recent
            && (!recorded
                || (recorded_on_lane && lane.as_ref().is_some_and(|l| l.open))
                || train.as_ref().is_some_and(|t| t.2)
                || worker.as_ref().is_some_and(|w| w.3))
    } else {
            // A pane mid-turn writes no turn row until the turn ends, so the
            // runner's live state for the pane, when it has one, wins over recency.
            let active = terminal.and_then(live).unwrap_or(recent);
            active && pane.is_some_and(|p| p.1) && lane.as_ref().is_none_or(|l| l.open || is_orchestrator)
        };

    let model = g
        .models
        .iter()
        .filter(|(m, _)| !m.is_empty())
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(m, _)| m.clone())
        .or_else(|| lane.as_ref().map(|l| l.model.clone()).filter(|m| !m.is_empty()))
        .or_else(|| worker.as_ref().map(|w| w.2.clone()).filter(|m| !m.is_empty()))
        .unwrap_or_default();

    // Its name: the worker record's label, else what the parent called it.
    let label = match worker.map(|w| w.1).filter(|l| !l.is_empty()) {
        Some(label) => label,
        None if is_subagent => {
            conn.query_row("SELECT description FROM subagent_descriptions WHERE agent_id = ?1", params![agent_id], |r| r.get(0))
                .optional()
                .map_err(map_err)?
                .unwrap_or_default()
        }
        None => String::new(),
    };

    let mut spend = g.spend;
    spend.cost_micros = (spend.runs > 0 && !g.unpriced).then_some(g.priced);
    Ok(PlanAgent {
        id: id.to_string(),
        role,
        harness: g.harness,
        model,
        label,
        lane: lane.map(|l| (l.lane, l.name)),
        train: train.map(|t| (t.0, t.1)),
        card,
        terminal,
        working,
        started_at: g.started.unwrap_or(g.last),
        last_at: g.last,
        spend,
    })
}

#[cfg(test)]
#[path = "plan_agents_tests.rs"]
mod tests;
