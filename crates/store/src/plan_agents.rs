//! The agents on the plan (ov-503, phase 3): every agent that still exists
//! for this board's project, joined to what it worked for (ov-518).
//!
//! # Only agents that still exist
//!
//! The list holds the orchestrator and every pane whose terminal is still
//! running, plus the subagents still running. A subagent that finished, a
//! pane whose terminal exited and turns filed to no terminal are not on it:
//! they were agents, not agents now. Their spend is still the board's, so
//! `Fleet::spend` totals every agent that ran in the window, listed or not,
//! and the header can say what the day cost.
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
//! A turn counts toward the spend when it ended in the last `WINDOW_MS`. A
//! subagent's run is one row that grows, so it counts whole when it last
//! moved in the window, even if it began before it.
//!
//! # When a subagent has ended
//!
//! The runner reads the parent's `Agent` tool_result (a foreground spawn) and
//! the `<task-notification>` a background one sends (`TurnEvent::SubagentEnded`,
//! applied by `watch::workers`), and ends the agent's `task_workers` record
//! with it; a `SendMessage` that resumes it reopens the record. That is the
//! most reliable signal there is, but only for a subagent the runner follows,
//! which is one with a worker record. So, in order:
//! - a worker record decides alone: open, or ended before the agent's last
//!   turn (it was resumed, however it was woken, as `watch::workers` reads it);
//! - else a lane or train record the manager keeps open;
//! - else nothing was recorded, and a turn in the last `RECENT_MS` is all
//!   there is to go on.
//!
//! An open record keeps a subagent listed while the session that spawned it
//! (its worker's orchestrator terminal, else the pane its turns were filed to,
//! else the running terminal bound to the worker's session, for a subagent the
//! runner linked from its description) still runs, however long its last tool call takes: a train's full test run
//! writes nothing for many minutes. A subagent with no record, or whose parent
//! has exited, has ended once it has not moved in `RECENT_MS`.
//!
//! An orchestrator's terminal is listed while it runs, however long idle, and a
//! pane mid-turn is listed whatever its lane says.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::error::map_err;
use crate::models::{TerminalRole, get_uuid, uuid_blob};
use crate::plan_read::LaneSpend;
use crate::trains::train_number;
use farcooler_core::Result;

/// The window the spend covers: a day.
pub const WINDOW_MS: i64 = 86_400_000;

/// A subagent is running only if it also moved this recently: a record left
/// open by a session that died says only that nobody closed it. Fifteen
/// minutes covers a long tool call.
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
    /// Mid-turn. A subagent on the list is always working; an idle pane that
    /// still runs is on the list and not working.
    pub working: bool,
    /// Unix milliseconds of its first turn in the window.
    pub started_at: i64,
    /// Unix milliseconds of its last turn; 0 for an orchestrator with no turn in the window.
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

/// The agents that exist, and what every agent of the window spent.
#[derive(Debug, Default)]
pub(crate) struct Fleet {
    /// Working ones first, then by tokens, most first.
    pub agents: Vec<PlanAgent>,
    /// Every agent with turns in the last `WINDOW_MS`, the ones gone included.
    /// Dollars only when every one of them is priced.
    pub spend: LaneSpend,
}

/// The agents that still exist in `workspace`'s project, and the window's spend.
pub(crate) fn agents_of(
    conn: &Connection,
    workspace: Uuid,
    now_ms: i64,
    live: &dyn Fn(Uuid) -> Option<bool>,
) -> Result<Fleet> {
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

    // The orchestrator's terminal, while it runs, is listed however long it
    // has been idle, even with no turn in the window.
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, command_preset FROM terminals WHERE workspace_id = ?1 AND role = ?2 AND exit_code IS NULL AND exit_signal IS NULL",
            )
            .map_err(map_err)?;
        let found = stmt
            .query_map(params![ws, TerminalRole::Orchestrator.as_i64()], |r| Ok((get_uuid(r, 0)?, r.get::<_, String>(1)?)))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        for (t, preset) in found {
            let g = groups.entry(crate::plan_panes::pane_agent_id(t)).or_default();
            if g.terminal.is_none() {
                // The harness is the terminal's preset's ("claude:opus", "codex"). No turn in
                // the window: `last_at` is 0, and a client says Idle with no age, not a made-up one.
                g.harness = preset.split(':').next().unwrap_or_default().to_string();
                g.terminal = Some(t);
            }
        }
    }
    let mut out = Vec::new();
    let mut spend = LaneSpend::default();
    let mut priced = Some(0i64);
    for (id, g) in groups {
        spend.input_tokens += g.spend.input_tokens;
        spend.output_tokens += g.spend.output_tokens;
        spend.cache_read_tokens += g.spend.cache_read_tokens;
        spend.cache_write_tokens += g.spend.cache_write_tokens;
        spend.runs += g.spend.runs;
        // A group with no turn (an idle orchestrator) spent nothing and leaves the dollars as they were.
        if g.spend.runs > 0 {
            priced = priced.zip((!g.unpriced).then_some(g.priced)).map(|(sum, micros)| sum + micros);
        }
        out.extend(join(conn, workspace, &id, g, now_ms, live)?);
    }
    spend.cost_micros = priced.filter(|_| spend.runs > 0);
    out.sort_by(|a, b| {
        b.working
            .cmp(&a.working)
            .then_with(|| total(&b.spend).cmp(&total(&a.spend)))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(Fleet { agents: out, spend })
}

fn total(s: &LaneSpend) -> u64 {
    s.input_tokens + s.output_tokens + s.cache_read_tokens + s.cache_write_tokens
}

/// A subagent's worker record: its task, label, model, whether it is open (or resumed), the terminal and the
/// session it was recorded under.
type WorkerRead = (Uuid, String, String, bool, Option<Uuid>, Option<String>);

/// One lane agent record: the lane, the role and whether it's still open. A
/// build record is open while the lane builds, and again when the lane is
/// fixing: the builder is resumed for the fix round.
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
                AND (a.role <> 'build' OR l.state IN ('queued', 'building', 'fixing'))
           FROM lane_agents a JOIN lanes l ON l.id = a.lane_id
          WHERE l.workspace_id = ?1 AND a.agent_id = ?2 AND (?3 = 0 OR a.harness = 'claude')
          ORDER BY (l.state NOT IN ('landed', 'dropped')) DESC, a.started_at DESC LIMIT 1",
        params![ws, agent_id, is_subagent],
        |r| Ok(LaneJoin { lane: get_uuid(r, 0)?, name: r.get(1)?, role: r.get(2)?, model: r.get(3)?, open: r.get(4)? }),
    )
    .optional()
    .map_err(map_err)
}

/// The running terminal of this workspace whose agent session is `session`: the
/// parent of a subagent whose record names no terminal. `None` when the worker
/// recorded no session, or no running terminal is bound to it yet (the binding
/// can lag the record; the subagent then ends by the quiet quarter hour until
/// it catches up, and nothing is held on a guess). A split pane copies its
/// session, so more than one terminal can match; the newest running one stands
/// for them, and any of them runs the session.
fn session_parent(conn: &Connection, ws: &[u8], session: Option<&str>) -> Result<Option<Uuid>> {
    let Some(session) = session.filter(|s| !s.is_empty()) else { return Ok(None) };
    let found: Option<Vec<u8>> = conn
        .query_row(
            "SELECT id FROM terminals WHERE workspace_id = ?1 AND agent_session_id = ?2
                AND exit_code IS NULL AND exit_signal IS NULL
              ORDER BY rowid DESC LIMIT 1",
            params![ws, session],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_err)?;
    Ok(found.and_then(|b| Uuid::from_slice(&b).ok()))
}

fn join(
    conn: &Connection,
    workspace: Uuid,
    id: &str,
    g: Group,
    now_ms: i64,
    live: &dyn Fn(Uuid) -> Option<bool>,
) -> Result<Option<PlanAgent>> {
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
    let worker: Option<WorkerRead> = if is_subagent {
        conn.query_row(
            // Open, or ended before the agent's last turn: it was resumed.
            "SELECT w.task_id, w.label, w.model, w.ended_at IS NULL OR w.ended_at < ?3, w.orchestrator_terminal, w.session_id FROM task_workers w
               JOIN tasks t ON t.id = w.task_id
              WHERE w.harness = 'claude' AND w.agent_id = ?1 AND t.workspace_id = ?2
              ORDER BY w.started_at DESC, w.rowid DESC LIMIT 1",
            params![agent_id, ws, g.last],
            |r| {
                let parent: Option<Vec<u8>> = r.get(4)?;
                Ok((get_uuid(r, 0)?, r.get(1)?, r.get(2)?, r.get(3)?, parent.and_then(|b| Uuid::from_slice(&b).ok()), r.get(5)?))
            },
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

    // Does it still exist, and is it working? A subagent exists while it runs
    // (see the module doc): its worker record alone decides when it has one,
    // else a lane or train record it is open on, else it moved lately. A pane
    // exists while its terminal runs, unless its lane has finished with it;
    // the runner's live state for it, when it has one, says whether it is
    // mid-turn (a pane mid-turn writes no turn row until the turn ends), else
    // recency does.
    let recent = now_ms - g.last <= RECENT_MS;
    let working = if is_subagent {
        let recorded = worker.is_some() || recorded_on_lane || train.is_some();
        let open = match &worker {
            Some(w) => w.3,
            None => {
                !recorded || lane.as_ref().is_some_and(|l| l.open && recorded_on_lane) || train.as_ref().is_some_and(|t| t.2)
            }
        };
        // An open record keeps it listed while the session that spawned it
        // still runs, however long its last tool call takes (a train's full
        // test run is many minutes with nothing written), but only for
        // `STALLED_AFTER_MS`. "Runs" means the parent's terminal runs the very
        // session the worker was recorded under: a /clear or a relaunch in the
        // same pane starts another session, and the old one's subagents are
        // gone. Past the quiet quarter hour it reads Idle, not Working.
        let spawned_in = worker.as_ref().and_then(|w| w.5.clone());
        // A worker the runner linked from the subagent's description names no
        // terminal and its turns may be filed to none: the parent is then the
        // running terminal that runs the worker's session (`session_parent`).
        let parent = match worker.as_ref().and_then(|w| w.4).or(g.terminal) {
            Some(t) => Some(t),
            None => session_parent(conn, &ws, spawned_in.as_deref())?,
        };
        let parent_runs = match parent {
            None => false,
            Some(t) => conn
                .query_row(
                    "SELECT exit_code IS NULL AND exit_signal IS NULL, agent_session_id FROM terminals WHERE id = ?1",
                    params![uuid_blob(t)],
                    |r| Ok((r.get::<_, bool>(0)?, r.get::<_, Option<String>>(1)?)),
                )
                .optional()
                .map_err(map_err)?
                .is_some_and(|(runs, now_session)| {
                    runs && match (&spawned_in, &now_session) {
                        (Some(then), Some(now)) => then == now,
                        _ => true,
                    }
                }),
        };
        let held = recorded && parent_runs && now_ms - g.last <= crate::train_phase::STALLED_AFTER_MS;
        if !(open && (recent || held)) {
            return Ok(None);
        }
        recent
    } else {
        let runs = pane.is_some_and(|p| p.1);
        let mid_turn = terminal.and_then(live) == Some(true);
        // A pane mid-turn is listed whatever its lane says: it exists and is
        // working. The lane's record hides only an idle pane it has finished with.
        if !(runs && (mid_turn || lane.as_ref().is_none_or(|l| l.open || is_orchestrator))) {
            return Ok(None);
        }
        terminal.and_then(live).unwrap_or(recent)
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
    Ok(Some(PlanAgent {
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
    }))
}

#[cfg(test)]
#[path = "plan_agents_tests.rs"]
mod tests;
