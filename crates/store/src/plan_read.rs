//! What the plan says about a board, derived on read (ov-268, ov-872).
//!
//! One read, `Store::plan`, draws every Plan surface: themes with their cards'
//! status counts, the tickets on the line with who works them, their fix
//! rounds and spend, the landings, and (from `Store::plan_with_agents`) the
//! agents. Nothing here is stored that could go stale: progress, a ticket's
//! cost and whether it has sat too long are computed against the board as it
//! is.
//!
//! A card now on another board is left out of every list (`prune_moved` in
//! `plan.rs` removes its theme row on the next write).

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{Connection, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::board_ci::{CiRead, ci_of};
use crate::error::map_err;
use crate::landing_phase::{LandingPhase, history};
use crate::landings::{Landing, landing_title, landings_for_plan};
use crate::models::{TaskStatus, get_uuid, uuid_blob};
use crate::plan::{BoardTheme, THEME_COLS, row_to_theme};
use crate::plan_cost::{PlanCost, TREND_DAYS, budgets_of, cost_of, ticket_days, trend_start};
use crate::rulings::{Ruling, rulings_of};
use crate::store::Store;
use crate::tasks::now_millis;
use crate::ticket_lines::{Stage, TicketLine};
use crate::ticket_work::TicketWork;

/// A ticket sitting in one stage longer than this reads as stale. Derived from
/// `stage_since`, so nothing ever has to clear it.
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

/// A card a theme or a ticket names: enough to draw its row.
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
    /// What was spent on its cards (ov-306): each of its tickets' spend
    /// summed, finished tickets included.
    pub spend: Spend,
    /// Its token budget, when it has one (ov-307).
    pub budget_tokens: Option<u64>,
    /// Its tokens on each of the last seven UTC days, oldest first, today
    /// last.
    pub trend: [u64; TREND_DAYS],
    /// When it last moved (ov-331): its story, its tickets on the line, its
    /// rulings and its cards' status moves, whichever is newest. 0 for a theme
    /// nothing has touched.
    pub last_moved_at: i64,
}

/// What agents spent, from the runner's own record of their turns.
///
/// Tokens first and dollars second: dollars are notional on a subscription.
/// `unmeasured_agents` counts the agents (a codex one, or a Claude one the
/// runner has read no turn of) whose spend isn't in the totals, and a client
/// says "Not reported" for them rather than zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spend {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Millionths of a US dollar; `None` when no model's price is known.
    pub cost_micros: Option<i64>,
    /// How many runs the totals cover.
    pub runs: u32,
    pub unmeasured_agents: u32,
    /// How many of a ticket's agents also worked other tickets. Each such
    /// agent's spend is split evenly across its tickets, so summing tickets
    /// counts it once, and a client says the ticket's figure holds a split.
    pub shared_agents: u32,
}

impl Spend {
    /// All four token counts.
    pub fn tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

/// A ticket on the line, with everything about it that is derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineView {
    pub line: TicketLine,
    /// Who worked it, oldest first, finished work included.
    pub work: Vec<TicketWork>,
    /// How many fix rounds it has had.
    pub fix_rounds: u32,
    pub spend: Spend,
    /// Its token budget, when it has one (ov-307).
    pub budget_tokens: Option<u64>,
    /// Has sat in a stage that isn't waiting or finished for over an hour.
    pub stale: bool,
    /// The number of the live landing it is on. Always none since the old
    /// landings became history (ov-960); kept for the wire.
    pub landing: Option<u32>,
}

/// A landing, with its derived words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandingView {
    pub landing: Landing,
    /// "Landing 72", or the title the orchestrator wrote.
    pub title: String,
    /// One line of what it carries: its tickets' titles, in the order they
    /// joined.
    pub summary: String,
    /// What its lander has spent; zero without one.
    pub spend: Spend,
    /// What it is doing (`landing_phase.rs`), and since when.
    pub phase: LandingPhase,
    pub phase_since: i64,
}

/// The plan's whole read for one board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The runner's clock when this was read, so `stale` and ages agree
    /// across devices.
    pub now_ms: i64,
    pub themes: Vec<ThemeView>,
    /// Open tickets on the line, and finished ones since `closed_since_ms`:
    /// the plan's order first, then by how long each has been where it is.
    pub lines: Vec<LineView>,
    /// Every card any theme, ticket or landing names, once.
    pub cards: Vec<CardRef>,
    /// Decided for you (ov-304): every standing ruling, newest first, then the
    /// settled ones, most recently settled first (`rulings_of` says how many).
    pub rulings: Vec<Ruling>,
    /// The old landings that moved since `closed_since_ms`, most recent first,
    /// each read as history (ov-960); then the engine's landing groups
    /// (ov-958).
    pub landings: Vec<LandingView>,
    /// What the runner last read of CI for the subjects the board names: its
    /// landings' pushed SHAs, and its pages' CI references (ov-306).
    pub ci: Vec<CiRead>,
    /// How many of the board's cards are in each status, every card counted.
    pub board_counts: StatusCounts,
    /// The week's tokens, and finished cards' cost by harness and model.
    pub cost: PlanCost,
    /// The agents still at work (`plan_agents.rs`). Empty from `Store::plan`:
    /// only `Store::plan_with_agents` reads it.
    pub agents: Vec<crate::plan_agents::AgentView>,
    /// What every agent of the last 24 hours spent, the ones no longer in
    /// `agents` included. Zero from `Store::plan`.
    pub agents_spend: Spend,
}

impl Store {
    /// `plan`, with the agents read too: what `plan.get` answers.
    ///
    /// `live` is the runner's live state for a pane's terminal, when it has
    /// one: `Some(true)` mid-turn, `Some(false)` idle or waiting. A turn is
    /// recorded only when it ends, so without it a long turn reads as idle.
    pub fn plan_with_agents(
        &self,
        workspace: Uuid,
        closed_since_ms: i64,
        live: &dyn Fn(Uuid) -> Option<bool>,
    ) -> Result<Plan> {
        let mut plan = self.plan(workspace, closed_since_ms)?;
        let fleet = crate::plan_agents::agents_of(&self.conn(), workspace, plan.now_ms, live)?;
        plan.agents = fleet.agents;
        plan.agents_spend = fleet.spend;
        Ok(plan)
    }

    /// Per board, its themes asking the owner something: not dropped, with an
    /// `owner_ask`. What the Needs You count adds to a board's needs-you
    /// items; one cheap query, since the runner's count is taken on every
    /// notice.
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

    /// A board's plan. Finished tickets (landed or dropped) appear only if
    /// their last move was at or after `closed_since_ms`. Settled rulings are
    /// history and don't age out: the last `SETTLED_READ_CAP` of them, or all
    /// when `closed_since_ms` is 0 or less.
    pub fn plan(&self, workspace: Uuid, closed_since_ms: i64) -> Result<Plan> {
        let conn = self.conn();
        crate::agents::board_exists(&conn, workspace)?;
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
                    spend: Spend::default(),
                    budget_tokens: None,
                    trend: [0; TREND_DAYS],
                    last_moved_at: 0,
                });
            }
        }

        let theme_of: HashMap<Uuid, usize> =
            themes.iter().enumerate().flat_map(|(i, v)| v.tasks.iter().map(move |t| (*t, i))).collect();
        let mut sums = vec![Sum::default(); themes.len()];
        let budgets = budgets_of(&conn, workspace)?;
        let trend_from = trend_start(now_ms);
        let mut trends = vec![[0.0f64; TREND_DAYS]; themes.len()];
        let mut lines = Vec::new();
        for line in crate::ticket_lines::lines_of(&conn, workspace)? {
            if !statuses.contains_key(&line.task_id) {
                continue;
            }
            let theme = theme_of.get(&line.task_id).copied();
            let in_window = !(line.stage.is_closed() && line.stage_since < closed_since_ms);
            if !in_window && theme.is_none() {
                continue;
            }
            let spend = spend_of_ticket(&conn, line.task_id)?;
            if let Some(theme) = theme {
                sums[theme].add(&spend);
                let days = ticket_days(&conn, line.task_id, trend_from)?;
                for (sum, n) in trends[theme].iter_mut().zip(days) {
                    *sum += n;
                }
            }
            if !in_window {
                continue;
            }
            named.insert(line.task_id, ());
            let work = crate::ticket_work::works(&conn, "task_id", line.task_id)?;
            let fix_rounds = fix_rounds_of(&conn, line.task_id)?;
            let stale = !line.stage.is_closed() && line.stage != Stage::Waiting && now_ms - line.stage_since > STALE_AFTER_MS;
            let budget_tokens = budgets.get(&line.task_id).copied();
            // No ticket is on an old landing any more: they are history (ov-960).
            lines.push(LineView { line, work, fix_rounds, spend, budget_tokens, stale, landing: None });
        }
        for ((view, sum), days) in themes.iter_mut().zip(&sums).zip(&trends) {
            view.spend = sum.total();
            view.budget_tokens = budgets.get(&view.theme.id).copied();
            view.trend = days.map(|n| n.round().max(0.0) as u64);
        }

        // A ruling's cards carry their own keys and stay out of `cards`: a card
        // a ruling names is on no line by design (review 1005a F2).
        let rulings = rulings_of(&conn, workspace, closed_since_ms, &statuses)?;
        let moved = card_moves(&conn, &ws)?;
        for view in &mut themes {
            view.last_moved_at = theme_last_moved(view, &lines, &rulings, &moved);
        }

        // The old landings are history: each reads as where it ended (ov-960).
        let mut landings = Vec::new();
        for landing in landings_for_plan(&conn, workspace, closed_since_ms)? {
            for task in &landing.tickets {
                if statuses.contains_key(task) {
                    named.insert(*task, ());
                }
            }
            let summary = landing
                .tickets
                .iter()
                .filter_map(|t| statuses.get(t))
                .map(|c| sentence(&c.title))
                .collect::<Vec<_>>()
                .join("; ");
            let spend = match landing.lander_id {
                Some(id) => spend_of_agent(&conn, &crate::agents::agent_in(&conn, id)?)?,
                None => Spend::default(),
            };
            let ended = history(&landing);
            landings.push(LandingView {
                title: landing_title(&landing),
                landing,
                summary,
                spend,
                phase: ended.phase,
                phase_since: ended.since,
            });
        }
        let ci = ci_of(&conn, workspace)?;
        // The engine's landing groups draw as landings too (ov-958); their tickets are named here.
        drop(conn);
        for mut group in self.workflow_groups(workspace)? {
            // A ticket that left this board is not drawn on it; a finished group left empty goes too.
            let before = group.tickets.len();
            group.tickets.retain(|t| statuses.contains_key(t));
            if before > 0 && group.tickets.is_empty() && group.is_finished() {
                continue;
            }
            let read = group.pushed_sha.as_deref().map(crate::board_ci::sha_subject).and_then(|s| ci.iter().find(|c| c.subject == s));
            for task in &group.tickets {
                if statuses.contains_key(task) {
                    named.insert(*task, ());
                }
            }
            let summary = group.tickets.iter().filter_map(|t| statuses.get(t)).map(|c| sentence(&c.title)).collect::<Vec<_>>().join("; ");
            let landing = group.landing(workspace, read);
            landings.push(LandingView {
                title: landing_title(&landing), landing, summary, spend: Spend::default(),
                phase: group.phase(read), phase_since: group.since,
            });
        }
        let conn = self.conn();
        let mut board_counts = StatusCounts::default();
        for card in statuses.values() {
            board_counts.add(card.status);
        }
        let cards = named.keys().filter_map(|id| statuses.get(id).cloned()).collect();
        let cost = cost_of(&conn, workspace, &statuses, now_ms)?;
        Ok(Plan {
            now_ms,
            themes,
            lines,
            cards,
            rulings,
            landings,
            ci,
            board_counts,
            cost,
            agents: Vec::new(),
            agents_spend: Spend::default(),
        })
    }
}

/// When each card last moved, by task id: `Task::updated_at`, the card's own
/// clock (a status move, a note, an edit), so the runner, the Mac and the
/// phones agree on "quiet".
fn card_moves(conn: &Connection, workspace: &[u8]) -> Result<HashMap<Uuid, i64>> {
    let mut stmt = conn
        .prepare(&format!("SELECT id, {} FROM tasks WHERE workspace_id = ?1", crate::tasks::UPDATED_AT))
        .map_err(map_err)?;
    let rows = stmt.query_map(params![workspace], |r| Ok((get_uuid(r, 0)?, r.get::<_, i64>(1)?))).map_err(map_err)?;
    rows.collect::<rusqlite::Result<HashMap<_, _>>>().map_err(map_err)
}

/// The newest of a theme's story, its tickets' moves on the line (a dropped
/// ticket is gone, and doesn't count), its rulings made or settled, and its
/// cards' own last moves (ov-331). The same rule the apps use.
fn theme_last_moved(view: &ThemeView, lines: &[LineView], rulings: &[Ruling], moved: &HashMap<Uuid, i64>) -> i64 {
    let tasks: HashSet<Uuid> = view.tasks.iter().copied().collect();
    let mut at = view.theme.story_at;
    for task in &view.tasks {
        at = at.max(moved.get(task).copied().unwrap_or(0));
    }
    for line in lines {
        if line.line.stage != Stage::Dropped && tasks.contains(&line.line.task_id) {
            at = at.max(line.line.stage_since);
        }
    }
    for ruling in rulings {
        if ruling.theme_id == Some(view.theme.id) {
            at = at.max(ruling.created_at).max(ruling.settled_at.unwrap_or(0));
        }
    }
    at
}

/// A theme's tickets' spend, summed.
#[derive(Debug, Clone, Default)]
struct Sum {
    tokens: [u64; 4],
    cost: Option<i64>,
    runs: u32,
    unmeasured: u32,
    shared: u32,
}

impl Sum {
    fn add(&mut self, spend: &Spend) {
        let tokens = [spend.input_tokens, spend.output_tokens, spend.cache_read_tokens, spend.cache_write_tokens];
        for (sum, n) in self.tokens.iter_mut().zip(tokens) {
            *sum += n;
        }
        if let Some(micros) = spend.cost_micros {
            *self.cost.get_or_insert(0) += micros;
        }
        self.runs += spend.runs;
        self.unmeasured += spend.unmeasured_agents;
        self.shared += spend.shared_agents;
    }

    fn total(&self) -> Spend {
        Spend {
            input_tokens: self.tokens[0],
            output_tokens: self.tokens[1],
            cache_read_tokens: self.tokens[2],
            cache_write_tokens: self.tokens[3],
            cost_micros: self.cost,
            runs: self.runs,
            unmeasured_agents: self.unmeasured,
            shared_agents: self.shared,
        }
    }
}

/// A card title as a line in a summary: the area lead ("Plan: ") dropped, and
/// the first letter capital.
pub(crate) fn sentence(card_title: &str) -> String {
    let t = card_title.trim();
    let body = match t.split_once(": ") {
        Some((lead, rest)) if !rest.trim().is_empty() && lead.chars().count() <= 24 && !lead.contains(char::is_whitespace) => rest.trim(),
        _ => t,
    };
    let mut chars = body.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Fix rounds are derived: each fix the ticket's timeline records starting,
/// and each move into fixing that the lane it came from recorded.
fn fix_rounds_of(conn: &Connection, task: Uuid) -> Result<u32> {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM line_events WHERE task_id = ?1 AND landing_id IS NULL
                AND ((kind = 'agent' AND json_extract(extra, '$.role') = 'fix' AND json_extract(extra, '$.verdict') IS NULL)
                     OR (kind = 'state' AND json_extract(extra, '$.to') = 'fixing'))",
            params![uuid_blob(task)],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    Ok(n.max(0) as u32)
}

/// The turns an agent `a` ran, joined as `t`: a Claude subagent's are keyed
/// `claude-log:agent:<agentId>` (`daemon/src/usage.rs`), and a pane's are
/// every turn filed under its terminal, whatever its harness, but for the runs
/// of subagents recorded as agents of their own, which those count.
pub(crate) const TURNS_OF_AGENT: &str = "((a.kind = 'subagent' AND a.harness = 'claude'
          AND t.turn_key = 'claude-log:agent:' || a.subagent_id)
     OR (a.kind = 'pane' AND t.terminal_id = a.terminal_id
         AND NOT (t.kind = 'subagent' AND EXISTS (SELECT 1 FROM agents o WHERE o.kind = 'subagent'
                  AND o.harness = 'claude' AND t.turn_key = 'claude-log:agent:' || o.subagent_id))))";

/// Totals as a `Spend`: input, output, cache read, cache write, priced micros,
/// how many turns were unpriced, and runs.
fn spend_from(row: (i64, i64, i64, i64, i64, i64, i64), unmeasured: u32, shared: u32) -> Spend {
    let (input, output, read, write, priced, unpriced, runs) = row;
    let n = |v: i64| v.max(0) as u64;
    Spend {
        input_tokens: n(input),
        output_tokens: n(output),
        cache_read_tokens: n(read),
        cache_write_tokens: n(write),
        cost_micros: (runs > 0 && unpriced == 0).then_some(priced),
        runs: runs.max(0) as u32,
        unmeasured_agents: unmeasured,
        shared_agents: shared,
    }
}

/// What a ticket's agents spent on it (`TURNS_OF_AGENT`).
///
/// An agent that worked several tickets (a reviewer given two, a builder with
/// a stack) did one run's work for all of them, and its turns can't say which
/// part was whose. So its spend is split evenly across its tickets: each gets
/// `1 / tickets` of every figure, and the tickets' totals sum to what it spent.
/// `shared_agents` says how many such agents a ticket holds, so the figure is
/// labeled a split. Runs still count whole.
pub(crate) fn spend_of_ticket(conn: &Connection, task: Uuid) -> Result<Spend> {
    let row = conn
        .query_row(
            &format!(
                "WITH tickets_of AS (SELECT agent_id, count(DISTINCT task_id) AS n FROM ticket_work GROUP BY agent_id),
                      worked AS (SELECT DISTINCT agent_id FROM ticket_work WHERE task_id = ?1)
                 SELECT CAST(coalesce(round(sum(m.input_tokens * 1.0 / s.n)), 0) AS INTEGER),
                        CAST(coalesce(round(sum(m.output_tokens * 1.0 / s.n)), 0) AS INTEGER),
                        CAST(coalesce(round(sum(m.cache_read_tokens * 1.0 / s.n)), 0) AS INTEGER),
                        CAST(coalesce(round(sum(m.cache_write_tokens * 1.0 / s.n)), 0) AS INTEGER),
                        CAST(coalesce(round(sum(m.cost_micros * 1.0 / s.n)), 0) AS INTEGER),
                        coalesce(sum(m.cost_micros IS NULL), 0),
                        count(DISTINCT t.id)
                   FROM worked w
                   JOIN agents a ON a.id = w.agent_id
                   JOIN tickets_of s ON s.agent_id = a.id
                   JOIN agent_turns t ON {TURNS_OF_AGENT}
                   JOIN agent_turn_models m ON m.turn_id = t.id"
            ),
            params![uuid_blob(task)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .map_err(map_err)?;
    let (agents, shared, measured): (i64, i64, i64) = conn
        .query_row(
            &format!(
                "SELECT count(DISTINCT w.agent_id),
                        count(DISTINCT CASE WHEN EXISTS (SELECT 1 FROM ticket_work o WHERE o.agent_id = w.agent_id
                                                          AND o.task_id <> w.task_id) THEN w.agent_id END),
                        count(DISTINCT CASE WHEN EXISTS (SELECT 1 FROM agents a JOIN agent_turns t ON {TURNS_OF_AGENT}
                                                          WHERE a.id = w.agent_id) THEN w.agent_id END)
                   FROM ticket_work w WHERE w.task_id = ?1"
            ),
            params![uuid_blob(task)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(map_err)?;
    Ok(spend_from(row, (agents - measured).max(0) as u32, shared.max(0) as u32))
}

/// What one agent spent, all its turns (`TURNS_OF_AGENT`). An agent no turn
/// of is read is `unmeasured_agents`, never zero.
pub(crate) fn spend_of_agent(conn: &Connection, agent: &crate::agents::Agent) -> Result<Spend> {
    let row: (i64, i64, i64, i64, i64, i64, i64) = conn
        .query_row(
            &format!(
                "SELECT CAST(coalesce(sum(m.input_tokens), 0) AS INTEGER),
                        CAST(coalesce(sum(m.output_tokens), 0) AS INTEGER),
                        CAST(coalesce(sum(m.cache_read_tokens), 0) AS INTEGER),
                        CAST(coalesce(sum(m.cache_write_tokens), 0) AS INTEGER),
                        CAST(coalesce(sum(m.cost_micros), 0) AS INTEGER),
                        coalesce(sum(m.cost_micros IS NULL), 0),
                        count(DISTINCT t.id)
                   FROM agents a JOIN agent_turns t ON {TURNS_OF_AGENT}
                   JOIN agent_turn_models m ON m.turn_id = t.id
                  WHERE a.id = ?1"
            ),
            params![uuid_blob(agent.id)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .map_err(map_err)?;
    let runs = row.6;
    Ok(spend_from(row, u32::from(runs == 0), 0))
}

#[cfg(test)]
#[path = "plan_read_tests.rs"]
mod tests;
