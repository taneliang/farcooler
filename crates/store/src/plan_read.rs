//! What the plan layer says about a board, derived on read (ov-268).
//!
//! One read, `Store::plan`, draws every Plan surface: themes with their
//! cards' status counts, lanes with their cards, agents, fix rounds and spend,
//! and the plan's order. Nothing here is stored that could go stale: progress,
//! a card's coverage, a lane's cost and whether it has sat too long are
//! computed against the board as it is, the way ov-212 derives a task's place
//! in a line.
//!
//! A card now on another board is left out of every list (`prune_moved` in
//! `plan.rs` removes its rows on the next write).

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{Connection, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::models::{TaskStatus, get_uuid, uuid_blob};
use crate::plan::{
    AgentRole, BoardTheme, LANE_COLS, THEME_COLS, Lane, LaneAgent, LaneCard, LaneState, PlanEvent, Subject, row_to_lane, row_to_theme,
};
use crate::rulings::{Ruling, rulings_of};
use crate::board_ci::{CiRead, ci_of};
use crate::trains::{Train, trains_of};
use crate::plan_cost::{PlanCost, TREND_DAYS, budgets_of, cost_of, lane_days, trend_start};
use crate::store::Store;
use crate::tasks::now_millis;

/// A lane sitting in one state longer than this reads as stale. Derived from
/// `state_since`, so nothing ever has to clear it.
pub const STALE_AFTER_MS: i64 = 60 * 60 * 1000;

/// How many of a theme's cards are in each status, using the board's own
/// statuses unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub backlog: u32,
    pub todo: u32,
    pub needs_decision: u32,
    pub in_progress: u32,
    pub in_review: u32,
    pub done: u32,
    pub cancelled: u32,
}

impl StatusCounts {
    fn add(&mut self, status: TaskStatus) {
        let slot = match status {
            TaskStatus::Backlog => &mut self.backlog,
            TaskStatus::Todo => &mut self.todo,
            TaskStatus::NeedsDecision => &mut self.needs_decision,
            TaskStatus::InProgress => &mut self.in_progress,
            TaskStatus::InReview => &mut self.in_review,
            TaskStatus::Done => &mut self.done,
            TaskStatus::Cancelled => &mut self.cancelled,
        };
        *slot += 1;
    }

    /// Every card counted.
    pub fn total(&self) -> u32 {
        self.backlog + self.todo + self.needs_decision + self.in_progress + self.in_review + self.done + self.cancelled
    }
}

/// A card a theme or a lane names: enough to draw its row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardRef {
    pub task_id: Uuid,
    pub key: String,
    pub title: String,
    pub status: TaskStatus,
}

/// A theme, with its cards and how far along they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeView {
    pub theme: BoardTheme,
    /// Its cards, oldest first.
    pub tasks: Vec<Uuid>,
    pub counts: StatusCounts,
    /// What its lanes spent on its cards (ov-306): each lane's spend shared
    /// out over the lane's cards, and the theme's cards' shares summed, so a
    /// lane working two themes' cards is counted once across them. Every lane
    /// that ever worked its cards, finished ones included.
    pub spend: LaneSpend,
    /// Its token budget, when it has one (ov-307).
    pub budget_tokens: Option<u64>,
    /// Its tokens on each of the last seven UTC days, oldest first, today
    /// last: each lane's days shared out over its cards as `spend` is.
    pub trend: [u64; TREND_DAYS],
}

/// What a lane's agents have spent, from the runner's own record of their
/// turns.
///
/// Tokens first and dollars second: dollars are notional on a subscription.
/// Only a Claude agent's turns are keyed by agent, so `unmeasured_agents`
/// counts the lane's agents (a codex one, or a Claude one the runner has read
/// no turn of) whose spend isn't in the totals, and a client says "Not
/// reported" for them rather than zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LaneSpend {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Millionths of a US dollar; `None` when no model's price is known.
    pub cost_micros: Option<i64>,
    /// How many runs the totals cover.
    pub runs: u32,
    pub unmeasured_agents: u32,
    /// How many of the lane's Claude agents are also recorded on another
    /// lane. Each such agent's spend is split evenly across its lanes, so
    /// summing lanes counts it once (review 1004j P1), and a client says the
    /// lane's figure holds a split.
    pub shared_agents: u32,
}

/// A lane, with everything about it that is derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneView {
    pub lane: Lane,
    pub cards: Vec<LaneCard>,
    pub agents: Vec<LaneAgent>,
    /// How many times it moved into `fixing`.
    pub fix_rounds: u32,
    pub spend: LaneSpend,
    /// Its token budget, when it has one (ov-307).
    pub budget_tokens: Option<u64>,
    /// Has sat in a state that isn't `queued` or finished for over an hour.
    pub stale: bool,
}

/// One card's lanes: what NO-LANE and LANDED-NOT-CLOSED are derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub task_id: Uuid,
    /// Lanes working it that aren't landed or dropped.
    pub live: u32,
    pub landed: u32,
}

/// A train (ov-309), with the lanes on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainView {
    pub train: Train,
    /// The lanes whose `train` names it, oldest first, finished ones included:
    /// a train that landed still says what it carried.
    pub lanes: Vec<Uuid>,
}

/// The plan layer's whole read for one board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The runner's clock when this was read, so `stale` and ages agree
    /// across devices.
    pub now_ms: i64,
    pub themes: Vec<ThemeView>,
    /// Live lanes, and finished ones since `closed_since_ms`.
    pub lanes: Vec<LaneView>,
    /// The queued lanes in the plan, first is next up.
    pub order: Vec<Uuid>,
    /// Every card any theme or lane names, once.
    pub cards: Vec<CardRef>,
    pub coverage: Vec<Coverage>,
    /// Decided for you (ov-304): every standing ruling, newest first, then
    /// those settled since `closed_since_ms`, most recently settled first.
    pub rulings: Vec<Ruling>,
    /// Trains (ov-309): every one not landed or dropped, oldest first, then
    /// those settled since `closed_since_ms`, most recent first.
    pub trains: Vec<TrainView>,
    /// What the runner last read of CI for the subjects the board names: its
    /// trains' pushed SHAs, and its pages' CI references (ov-306).
    pub ci: Vec<CiRead>,
    /// How many of the board's cards are in each status, every card counted
    /// (ov-306): what a page's card-count references draw.
    pub board_counts: StatusCounts,
    /// The week's tokens, and finished cards' cost by harness and model
    /// (ov-307).
    pub cost: PlanCost,
}

impl Store {
    /// Per board, its themes asking the owner something: not dropped, with
    /// an `owner_ask`. What the Needs You count adds to a board's needs-you
    /// items (ov-310, ov-321's `WorkspaceNeedsYou.count`); one cheap query,
    /// since the runner's count is taken on every notice.
    pub fn theme_asks(&self) -> Result<HashMap<Uuid, u32>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT workspace_id, COUNT(*) FROM board_themes
                  WHERE state <> 'dropped' AND owner_ask <> '' GROUP BY workspace_id",
            )
            .map_err(map_err)?;
        let rows = stmt.query_map([], |row| Ok((get_uuid(row, 0)?, row.get::<_, u32>(1)?))).map_err(map_err)?;
        rows.collect::<rusqlite::Result<HashMap<_, _>>>().map_err(map_err)
    }

    /// A board's plan layer. Finished lanes (landed or dropped) and settled
    /// rulings appear only if their last move was at or after
    /// `closed_since_ms`.
    pub fn plan(&self, workspace: Uuid, closed_since_ms: i64) -> Result<Plan> {
        let conn = self.conn();
        crate::plan::board_exists(&conn, workspace)?;
        let now_ms = now_millis();
        let ws = uuid_blob(workspace);

        let mut statuses: HashMap<Uuid, CardRef> = HashMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT id, key, title, status FROM tasks WHERE workspace_id = ?1")
                .map_err(map_err)?;
            let rows = stmt
                .query_map(params![ws], |r| {
                    let status: String = r.get(3)?;
                    Ok(CardRef {
                        task_id: get_uuid(r, 0)?,
                        key: r.get(1)?,
                        title: r.get(2)?,
                        status: TaskStatus::parse(&status).unwrap_or(TaskStatus::Backlog),
                    })
                })
                .map_err(map_err)?;
            for row in rows {
                let card = row.map_err(map_err)?;
                statuses.insert(card.task_id, card);
            }
        }
        let mut named: BTreeMap<Uuid, ()> = BTreeMap::new();

        let mut themes = Vec::new();
        {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {THEME_COLS} FROM board_themes WHERE workspace_id = ?1 AND state <> 'dropped'
                      ORDER BY ordinal, created_at, id"
                ))
                .map_err(map_err)?;
            let found = stmt
                .query_map(params![ws], row_to_theme)
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
            for theme in found {
                let mut tasks = Vec::new();
                let mut counts = StatusCounts::default();
                let mut stmt = conn
                    .prepare("SELECT task_id FROM board_theme_tasks WHERE theme_id = ?1 ORDER BY added_at, rowid")
                    .map_err(map_err)?;
                let ids = stmt
                    .query_map(params![uuid_blob(theme.id)], |r| get_uuid(r, 0))
                    .map_err(map_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(map_err)?;
                for id in ids {
                    if let Some(card) = statuses.get(&id) {
                        counts.add(card.status);
                        tasks.push(id);
                        named.insert(id, ());
                    }
                }
                themes.push(ThemeView {
                    theme,
                    tasks,
                    counts,
                    spend: LaneSpend::default(),
                    budget_tokens: None,
                    trend: [0; TREND_DAYS],
                });
            }
        }

        let theme_of: HashMap<Uuid, usize> =
            themes.iter().enumerate().flat_map(|(i, v)| v.tasks.iter().map(move |t| (*t, i))).collect();
        let mut shares = vec![Shares::default(); themes.len()];
        let budgets = budgets_of(&conn, workspace)?;
        let trend_from = trend_start(now_ms);
        let mut trends = vec![[0.0f64; TREND_DAYS]; themes.len()];
        let mut lanes = Vec::new();
        let mut order: Vec<(u32, Uuid)> = Vec::new();
        let mut live: HashMap<Uuid, u32> = HashMap::new();
        let mut landed: HashMap<Uuid, u32> = HashMap::new();
        {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {LANE_COLS} FROM lanes WHERE workspace_id = ?1 ORDER BY created_at, id"
                ))
                .map_err(map_err)?;
            let found = stmt
                .query_map(params![ws], row_to_lane)
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
            for lane in found {
                let cards = lane_cards_of(&conn, lane.id, &statuses)?;
                let linked: HashSet<Uuid> = cards.iter().map(|c| c.task_id).collect();
                for task in &linked {
                    match lane.state {
                        LaneState::Landed => *landed.entry(*task).or_default() += 1,
                        LaneState::Dropped => {}
                        _ => *live.entry(*task).or_default() += 1,
                    }
                }
                let in_themes: Vec<usize> = linked.iter().filter_map(|t| theme_of.get(t).copied()).collect();
                let in_window = !(lane.state.is_closed() && lane.state_since < closed_since_ms);
                let lane_spend = if in_window || !in_themes.is_empty() {
                    Some(spend_of(&conn, lane.id, &agents_of(&conn, lane.id)?)?)
                } else {
                    None
                };
                if let Some(spend) = &lane_spend {
                    // Each theme once per lane, with the share its cards make
                    // up, so a lane's runs and agents count once in it (review
                    // train-1005c L3).
                    let mut per_theme: BTreeMap<usize, usize> = BTreeMap::new();
                    for theme in &in_themes {
                        *per_theme.entry(*theme).or_default() += 1;
                    }
                    for (theme, cards) in per_theme {
                        shares[theme].add(spend, cards as f64 / linked.len() as f64);
                    }
                    if !in_themes.is_empty() {
                        let days = lane_days(&conn, lane.id, trend_from)?;
                        for theme in &in_themes {
                            for (sum, n) in trends[*theme].iter_mut().zip(days) {
                                *sum += n / linked.len() as f64;
                            }
                        }
                    }
                }
                if !in_window {
                    continue;
                }
                for card in &cards {
                    named.insert(card.task_id, ());
                }
                if let (LaneState::Queued, Some(rank)) = (lane.state, lane.plan_rank) {
                    order.push((rank, lane.id));
                }
                let agents = agents_of(&conn, lane.id)?;
                let fix_rounds = fix_rounds_of(&conn, lane.id)?;
                let spend = lane_spend.unwrap_or_default();
                let stale = !lane.state.is_closed()
                    && lane.state != LaneState::Queued
                    && now_ms - lane.state_since > STALE_AFTER_MS;
                let budget_tokens = budgets.get(&lane.id).copied();
                lanes.push(LaneView { lane, cards, agents, fix_rounds, spend, budget_tokens, stale });
            }
        }
        order.sort();
        for ((view, share), days) in themes.iter_mut().zip(&shares).zip(&trends) {
            view.spend = share.total();
            view.budget_tokens = budgets.get(&view.theme.id).copied();
            view.trend = days.map(|n| n.round().max(0.0) as u64);
        }

        // A ruling's cards carry their own keys and stay out of `cards`, which
        // the reconciliation reads: a card a ruling names is in no lane by
        // design (review 1005a F2).
        let rulings = rulings_of(&conn, workspace, closed_since_ms, &statuses)?;

        let mut trains = Vec::new();
        for train in trains_of(&conn, workspace, closed_since_ms)? {
            let mut stmt = conn
                .prepare("SELECT id FROM lanes WHERE workspace_id = ?1 AND train = ?2 COLLATE NOCASE ORDER BY created_at, id")
                .map_err(map_err)?;
            let lanes = stmt
                .query_map(params![ws, train.name], |r| get_uuid(r, 0))
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
            trains.push(TrainView { train, lanes });
        }
        let ci = ci_of(&conn, workspace)?;
        let mut board_counts = StatusCounts::default();
        for card in statuses.values() {
            board_counts.add(card.status);
        }

        let cards = named.keys().filter_map(|id| statuses.get(id).cloned()).collect();
        let mut coverage: Vec<Coverage> = live
            .keys()
            .chain(landed.keys())
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|id| Coverage {
                task_id: *id,
                live: live.get(id).copied().unwrap_or(0),
                landed: landed.get(id).copied().unwrap_or(0),
            })
            .collect();
        coverage.sort_by_key(|c| c.task_id);
        let order = order.into_iter().map(|(_, id)| id).collect();
        let cost = cost_of(&conn, &statuses, now_ms)?;
        Ok(Plan { now_ms, themes, lanes, order, cards, coverage, rulings, trains, ci, board_counts, cost })
    }

    /// A theme's or lane's timeline, oldest first, from `since_ms` on.
    pub fn plan_events(&self, subject: Subject, since_ms: i64) -> Result<Vec<PlanEvent>> {
        let conn = self.conn();
        let (column, id) = match subject {
            Subject::Theme(id) => ("theme_id", id),
            Subject::Lane(id) => ("lane_id", id),
        };
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id, at, actor, theme_id, lane_id, kind, body, extra FROM plan_events
                  WHERE {column} = ?1 AND at >= ?2 ORDER BY at, id"
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![uuid_blob(id), since_ms], |r| {
                let opt = |i: usize| -> rusqlite::Result<Option<Uuid>> {
                    let raw: Option<Vec<u8>> = r.get(i)?;
                    Ok(raw.and_then(|b| Uuid::from_slice(&b).ok()))
                };
                let extra: String = r.get(7)?;
                Ok(PlanEvent {
                    id: get_uuid(r, 0)?,
                    at: r.get(1)?,
                    actor: r.get(2)?,
                    theme_id: opt(3)?,
                    lane_id: opt(4)?,
                    kind: r.get(5)?,
                    body: r.get(6)?,
                    extra: serde_json::from_str(&extra).unwrap_or_default(),
                })
            })
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        Ok(rows)
    }
}

/// A theme's share of its lanes' spend, summed before rounding.
#[derive(Debug, Clone, Default)]
struct Shares {
    tokens: [f64; 4],
    cost: Option<f64>,
    runs: u32,
    unmeasured: u32,
    shared: u32,
}

impl Shares {
    /// `part` of `spend`: the share of a lane's cards that are the theme's.
    fn add(&mut self, spend: &LaneSpend, part: f64) {
        let tokens = [spend.input_tokens, spend.output_tokens, spend.cache_read_tokens, spend.cache_write_tokens];
        for (sum, n) in self.tokens.iter_mut().zip(tokens) {
            *sum += n as f64 * part;
        }
        if let Some(micros) = spend.cost_micros {
            *self.cost.get_or_insert(0.0) += micros as f64 * part;
        }
        self.runs += spend.runs;
        self.unmeasured += spend.unmeasured_agents;
        self.shared += spend.shared_agents;
    }

    fn total(&self) -> LaneSpend {
        let n = |v: f64| v.round().max(0.0) as u64;
        LaneSpend {
            input_tokens: n(self.tokens[0]),
            output_tokens: n(self.tokens[1]),
            cache_read_tokens: n(self.tokens[2]),
            cache_write_tokens: n(self.tokens[3]),
            cost_micros: self.cost.map(|c| c.round() as i64),
            runs: self.runs,
            unmeasured_agents: self.unmeasured,
            shared_agents: self.shared,
        }
    }
}

fn lane_cards_of(conn: &Connection, lane: Uuid, on_board: &HashMap<Uuid, CardRef>) -> Result<Vec<LaneCard>> {
    let mut stmt = conn
        .prepare("SELECT task_id, slice FROM lane_tasks WHERE lane_id = ?1 ORDER BY rowid")
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(lane)], |r| Ok(LaneCard { task_id: get_uuid(r, 0)?, slice: r.get(1)? }))
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    Ok(rows.into_iter().filter(|c| on_board.contains_key(&c.task_id)).collect())
}

fn agents_of(conn: &Connection, lane: Uuid) -> Result<Vec<LaneAgent>> {
    let mut stmt = conn
        .prepare(
            "SELECT harness, agent_id, role, model, started_at, ended_at FROM lane_agents
              WHERE lane_id = ?1 ORDER BY started_at, rowid",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(lane)], |r| {
            let role: String = r.get(2)?;
            Ok(LaneAgent {
                harness: r.get(0)?,
                agent_id: r.get(1)?,
                role: AgentRole::parse(&role).unwrap_or(AgentRole::Build),
                model: r.get(3)?,
                started_at: r.get(4)?,
                ended_at: r.get(5)?,
            })
        })
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    Ok(rows)
}

/// Fix rounds are derived: the number of moves into `fixing`.
fn fix_rounds_of(conn: &Connection, lane: Uuid) -> Result<u32> {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM plan_events WHERE lane_id = ?1 AND kind = 'state'
                AND json_extract(extra, '$.to') = 'fixing'",
            params![uuid_blob(lane)],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    Ok(n.max(0) as u32)
}

/// Totals over the turns recorded for the lane's Claude agents: a subagent's
/// turns are keyed `claude-log:agent:<agentId>` (`daemon/src/usage.rs`).
///
/// An agent recorded on several lanes (a reviewer given two branches) did
/// one run's work for all of them, and its turns can't say which part was
/// whose. So its spend is split evenly across its lanes: each lane gets
/// `1 / lanes` of every figure, and the lanes' totals sum to what it spent,
/// not to that times the lanes. `shared_agents` says how many such agents a
/// lane holds, so the figure is labeled a split. Runs still count whole: a
/// run is in each lane it worked for.
fn spend_of(conn: &Connection, lane: Uuid, agents: &[LaneAgent]) -> Result<LaneSpend> {
    let (input, output, read, write, priced, unpriced, runs): (i64, i64, i64, i64, i64, i64, i64) = conn
        .query_row(
            "WITH lanes_of AS (
                 SELECT harness, agent_id, count(*) AS lanes FROM lane_agents GROUP BY harness, agent_id
             )
             SELECT CAST(coalesce(round(sum(m.input_tokens * 1.0 / s.lanes)), 0) AS INTEGER),
                    CAST(coalesce(round(sum(m.output_tokens * 1.0 / s.lanes)), 0) AS INTEGER),
                    CAST(coalesce(round(sum(m.cache_read_tokens * 1.0 / s.lanes)), 0) AS INTEGER),
                    CAST(coalesce(round(sum(m.cache_write_tokens * 1.0 / s.lanes)), 0) AS INTEGER),
                    CAST(coalesce(round(sum(m.cost_micros * 1.0 / s.lanes)), 0) AS INTEGER),
                    coalesce(sum(m.cost_micros IS NULL), 0),
                    count(DISTINCT t.id)
               FROM lane_agents a
               JOIN lanes_of s ON s.harness = a.harness AND s.agent_id = a.agent_id
               JOIN agent_turns t ON t.turn_key = 'claude-log:agent:' || a.agent_id
               JOIN agent_turn_models m ON m.turn_id = t.id
              WHERE a.lane_id = ?1 AND a.harness = 'claude'",
            params![uuid_blob(lane)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .map_err(map_err)?;
    let shared: i64 = conn
        .query_row(
            "SELECT count(*) FROM lane_agents a
              WHERE a.lane_id = ?1 AND a.harness = 'claude'
                AND EXISTS (SELECT 1 FROM lane_agents o
                             WHERE o.harness = a.harness AND o.agent_id = a.agent_id AND o.lane_id != a.lane_id)",
            params![uuid_blob(lane)],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    let measured: i64 = conn
        .query_row(
            "SELECT count(DISTINCT a.agent_id)
               FROM lane_agents a JOIN agent_turns t ON t.turn_key = 'claude-log:agent:' || a.agent_id
              WHERE a.lane_id = ?1 AND a.harness = 'claude'",
            params![uuid_blob(lane)],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    let n = |v: i64| v.max(0) as u64;
    Ok(LaneSpend {
        input_tokens: n(input),
        output_tokens: n(output),
        cache_read_tokens: n(read),
        cache_write_tokens: n(write),
        cost_micros: (runs > 0 && unpriced == 0).then_some(priced),
        runs: runs.max(0) as u32,
        unmeasured_agents: (agents.len() as i64 - measured).max(0) as u32,
        shared_agents: shared.max(0) as u32,
    })
}
