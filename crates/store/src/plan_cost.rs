//! Cost on the plan (ov-307): budgets, a theme's seven-day trend, the week's
//! tokens, and cost per finished card by harness and model (folding in ov-261).
//!
//! # Additive and removable, as the plan layer is
//!
//! One new table (migration 0028, `Older::Welcome`): `plan_budgets`, a token
//! budget a theme or a lane may carry. Its rows go with their workspace,
//! theme or lane by cascade. Nothing old gains a column or a trigger, and
//! nothing the board runs on names this table (`scripts/plan-layer-lint.py`
//! checks). Everything else here is derived on read from `agent_turns`, the
//! runner's own record of turns, and is never stored.
//!
//! # What a "token" is
//!
//! The four counts a lane's spend carries, added: input, output, cache read
//! and cache write. A budget is a number of those, the same figure the plan
//! prints as "17M tokens", so what the plan flags is what a person can see.
//!
//! # The weekly limit
//!
//! The runner can't know a plan's weekly limit, so it never shows a share.
//! Neither Claude Code nor Codex reports the limit or how much of it is used
//! to the runner: a Claude turn's usage carries token counts and, at best, its
//! own cost, and the one rate-limit event in the stream is ignored because it
//! carries no figure. `week_tokens` is therefore the runner's tokens over the
//! last seven UTC days (today so far) and nothing more; a client shows it with no percentage. A
//! budget is the way to give the plan a number to measure against.

use std::collections::HashMap;

use rusqlite::{Connection, Transaction, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, TaskStatus, get_uuid, uuid_blob};
use crate::plan::{Subject, event};
use crate::plan_read::CardRef;
use crate::store::Store;
use crate::tasks::now_millis;

/// The days a trend covers, today included.
pub const TREND_DAYS: usize = 7;
const DAY_MS: i64 = 86_400_000;

/// A comparison of a harness and model needs at least this many finished
/// cards behind it: a mean of one or two cards is an anecdote, and a client
/// that drew it would invite a conclusion the data can't hold.
pub const MIN_CARDS_TO_COMPARE: u32 = 3;

/// A budget is a number of tokens; a hundred trillion is a script that lost
/// its mind, not a budget.
const BUDGET_MAX: u64 = 100_000_000_000_000;

/// One new table. `Older::Welcome`: a build from before it reads every table
/// it knows as it did, and its rows go with their workspace, theme or lane by
/// cascade whichever build deletes it. No trigger, no column on an old table.
pub(crate) fn migration_0028_plan_budgets(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE plan_budgets (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            theme_id BLOB REFERENCES board_themes(id) ON DELETE CASCADE,
            lane_id BLOB REFERENCES lanes(id) ON DELETE CASCADE,
            tokens INTEGER NOT NULL CHECK (tokens > 0),
            set_at INTEGER NOT NULL,
            CHECK ((theme_id IS NULL) <> (lane_id IS NULL))
        );
        CREATE UNIQUE INDEX plan_budgets_by_theme ON plan_budgets (theme_id) WHERE theme_id IS NOT NULL;
        CREATE UNIQUE INDEX plan_budgets_by_lane ON plan_budgets (lane_id) WHERE lane_id IS NOT NULL;
        "#,
    )
}

/// The cost half of a plan read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanCost {
    /// The runner's tokens over the last seven days, every harness and every
    /// board. No limit and no share: see the module doc.
    pub week_tokens: u64,
    /// The same week split by harness and model, most tokens first; the
    /// tokens add up to `week_tokens` (ov-434).
    pub week: Vec<WeekSpend>,
    /// Millionths of a US dollar the week cost, API-equivalent; `None` unless
    /// every pair in `week` was priced.
    pub week_cost_micros: Option<i64>,
    /// Cost per landed card by harness and model, those with at least
    /// `MIN_CARDS_TO_COMPARE` landed cards' worth of share behind them, most
    /// share first.
    pub compare: Vec<HarnessModelCost>,
    /// Spend on the board's cards that haven't landed (open or cancelled), by
    /// every pair, shown apart from the comparison so it is neither dropped
    /// nor charged to a landed card.
    pub in_flight_tokens: u64,
    /// Millionths of a US dollar of it; `None` unless every turn was priced.
    pub in_flight_cost_micros: Option<i64>,
    /// How many harness-and-model pairs had finished cards but too few to
    /// compare, so a client can say some are held back.
    pub compare_held_back: u32,
}

/// What one harness and model spent over the last seven days (ov-434).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekSpend {
    pub harness: String,
    /// Empty when the harness named no model.
    pub model: String,
    pub tokens: u64,
    /// Millionths of a US dollar; `None` unless every turn was priced.
    pub cost_micros: Option<i64>,
}

/// What the cards a harness and model worked, once finished, cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessModelCost {
    pub harness: String,
    /// Empty when the harness named no model.
    pub model: String,
    /// Its share of the landed cards, in thousandths of a card: the sum, over
    /// the landed cards it worked, of its fraction of each card's tokens.
    pub card_share_milli: u32,
    /// Tokens it spent on landed cards, all four counts.
    pub tokens: u64,
    /// Millionths of a US dollar it spent on landed cards; `None` unless every
    /// one of its turns was priced.
    pub cost_micros: Option<i64>,
}

/// A board's budgets, by the theme's or lane's id.
pub(crate) fn budgets_of(conn: &Connection, workspace: Uuid) -> Result<HashMap<Uuid, u64>> {
    let mut stmt = conn
        .prepare("SELECT coalesce(theme_id, lane_id), tokens FROM plan_budgets WHERE workspace_id = ?1")
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(workspace)], |r| Ok((get_uuid(r, 0)?, r.get::<_, i64>(1)?.max(0) as u64)))
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    Ok(rows.into_iter().collect())
}

/// The start of the UTC day `now_ms` falls in, six days back: where a trend's
/// first day begins.
pub(crate) fn trend_start(now_ms: i64) -> i64 {
    now_ms.div_euclid(DAY_MS) * DAY_MS - (TREND_DAYS as i64 - 1) * DAY_MS
}

/// `tokens` spent over `[started, ended]`, shared out over the seven UTC days
/// from `window_start`, in proportion to the time on each day. A Claude
/// subagent's run is one row that grows as the agent works, so a run that began
/// Monday and was resumed Thursday must not land whole on Thursday. A row with
/// no start, or one that began where it ended, lands on its end day. What fell
/// before the window is not in it.
pub(crate) fn spread(tokens: f64, started: Option<i64>, ended: i64, window_start: i64) -> [f64; TREND_DAYS] {
    let mut days = [0.0; TREND_DAYS];
    let start = started.filter(|s| *s < ended);
    let Some(start) = start else {
        let day = (ended - window_start).div_euclid(DAY_MS);
        if (0..TREND_DAYS as i64).contains(&day) {
            days[day as usize] = tokens;
        }
        return days;
    };
    let span = (ended - start) as f64;
    for (i, slot) in days.iter_mut().enumerate() {
        let from = window_start + i as i64 * DAY_MS;
        let overlap = (ended.min(from + DAY_MS) - start.max(from)).max(0);
        *slot = tokens * overlap as f64 / span;
    }
    days
}

/// A lane's tokens on each of the last seven UTC days, oldest first, each run
/// spread over the days it ran (see `spread`). A Claude agent shared with
/// another lane counts here by its even split, as the lane's spend does, so
/// summing lanes counts it once.
pub(crate) fn lane_days(conn: &Connection, lane: Uuid, start_ms: i64) -> Result<[f64; TREND_DAYS]> {
    let mut stmt = conn
        .prepare(
            "WITH lanes_of AS (
                 SELECT harness, agent_id, count(*) AS lanes FROM lane_agents GROUP BY harness, agent_id
             )
             SELECT t.started_at, t.ended_at,
                    sum((m.input_tokens + m.output_tokens + m.cache_read_tokens + m.cache_write_tokens) * 1.0 / s.lanes)
               FROM lane_agents a
               JOIN lanes_of s ON s.harness = a.harness AND s.agent_id = a.agent_id
               JOIN agent_turns t ON t.turn_key = 'claude-log:agent:' || a.agent_id
               JOIN agent_turn_models m ON m.turn_id = t.id
              WHERE a.lane_id = ?1 AND a.harness = 'claude' AND t.ended_at >= ?2
              GROUP BY t.id",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(lane), start_ms], |r| {
            Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?))
        })
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    let mut days = [0.0; TREND_DAYS];
    for (started, ended, tokens) in rows {
        for (sum, n) in days.iter_mut().zip(spread(tokens, started, ended, start_ms)) {
            *sum += n;
        }
    }
    Ok(days)
}

/// The cost half of a board's plan read.
pub(crate) fn cost_of(conn: &Connection, cards: &HashMap<Uuid, CardRef>, now_ms: i64) -> Result<PlanCost> {
    // One window for the week and the trend: the same seven UTC days, today
    // so far, each run spread over the days it ran.
    let window = trend_start(now_ms);
    // Per turn, harness and model, so the total can be split the way the
    // comparison is. `week_tokens` is these rows' sum, never a second query.
    let mut stmt = conn
        .prepare(
            "SELECT t.started_at, t.ended_at, t.harness, m.model,
                    sum(m.input_tokens + m.output_tokens + m.cache_read_tokens + m.cache_write_tokens),
                    coalesce(sum(m.cost_micros), 0), coalesce(sum(m.cost_micros IS NULL), 0)
               FROM agent_turns t JOIN agent_turn_models m ON m.turn_id = t.id
              WHERE t.ended_at >= ?1 GROUP BY t.id, t.harness, m.model",
        )
        .map_err(map_err)?;
    let week_rows = stmt
        .query_map(params![window], |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, f64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    // A run that began before the window counts only the part inside it, for
    // its dollars as for its tokens.
    let mut week_pairs: HashMap<(String, String), (f64, f64, bool)> = HashMap::new();
    for (started, ended, harness, model, tokens, micros, unpriced) in week_rows {
        let inside: f64 = spread(1.0, started, ended, window).iter().sum();
        let slot = week_pairs.entry((harness, model)).or_default();
        slot.0 += tokens * inside;
        slot.1 += micros as f64 * inside;
        slot.2 |= unpriced > 0 && inside > 0.0;
    }
    let mut week: Vec<WeekSpend> = week_pairs
        .into_iter()
        .map(|((harness, model), (tokens, micros, unpriced))| WeekSpend {
            harness,
            model,
            tokens: tokens.round().max(0.0) as u64,
            cost_micros: (!unpriced).then(|| micros.round() as i64),
        })
        .filter(|w| w.tokens > 0)
        .collect();
    week.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| (&a.harness, &a.model).cmp(&(&b.harness, &b.model))));
    let week_tokens: u64 = week.iter().map(|w| w.tokens).sum();
    let week_cost_micros = week.iter().try_fold(0i64, |sum, w| w.cost_micros.map(|m| sum + m));

    // Every pair's spend on every card on the board, so spend on cards that
    // didn't land is counted (as in flight) rather than left out, and a card's
    // tokens can be shared out between the pairs that worked it.
    let mut stmt = conn
        .prepare(
            "SELECT t.task_id, t.harness, m.model,
                    sum(m.input_tokens + m.output_tokens + m.cache_read_tokens + m.cache_write_tokens),
                    coalesce(sum(m.cost_micros), 0), coalesce(sum(m.cost_micros IS NULL), 0)
               FROM agent_turns t JOIN agent_turn_models m ON m.turn_id = t.id
              WHERE t.task_id IS NOT NULL
              GROUP BY t.task_id, t.harness, m.model",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                get_uuid(r, 0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?.max(0) as u64,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    let rows: Vec<_> = rows.into_iter().filter(|(task, ..)| cards.contains_key(task)).collect();
    let mut card_tokens: HashMap<Uuid, u64> = HashMap::new();
    for (task, _, _, tokens, ..) in &rows {
        *card_tokens.entry(*task).or_default() += tokens;
    }
    #[derive(Default)]
    struct Pair {
        share: f64,
        tokens: u64,
        micros: i64,
        unpriced: bool,
    }
    let mut pairs: HashMap<(String, String), Pair> = HashMap::new();
    let (mut flying, mut flying_micros, mut flying_unpriced) = (0u64, 0i64, false);
    for (task, harness, model, tokens, micros, unpriced) in rows {
        if cards[&task].status != TaskStatus::Done {
            flying += tokens;
            flying_micros += micros;
            flying_unpriced |= unpriced > 0;
            continue;
        }
        let pair = pairs.entry((harness, model)).or_default();
        let whole = card_tokens[&task].max(1) as f64;
        pair.share += tokens as f64 / whole;
        pair.tokens += tokens;
        pair.micros += micros;
        pair.unpriced |= unpriced > 0;
    }
    let mut compare: Vec<HarnessModelCost> = Vec::new();
    let mut compare_held_back = 0;
    for ((harness, model), pair) in pairs {
        if pair.share < MIN_CARDS_TO_COMPARE as f64 - 1e-9 {
            compare_held_back += 1;
            continue;
        }
        compare.push(HarnessModelCost {
            harness,
            model,
            card_share_milli: (pair.share * 1000.0).round() as u32,
            tokens: pair.tokens,
            cost_micros: (!pair.unpriced).then_some(pair.micros),
        });
    }
    compare.sort_by(|a, b| {
        b.card_share_milli.cmp(&a.card_share_milli).then_with(|| (&a.harness, &a.model).cmp(&(&b.harness, &b.model)))
    });
    Ok(PlanCost {
        week_tokens,
        week,
        week_cost_micros,
        compare,
        compare_held_back,
        in_flight_tokens: flying,
        in_flight_cost_micros: (flying > 0 && !flying_unpriced).then_some(flying_micros),
    })
}

impl Store {
    /// Give a theme or a lane a budget of `tokens`, or take it away with
    /// `None`. The event records the old and the new, so a raised budget is
    /// not silent.
    pub fn set_budget(&self, subject: Subject, tokens: Option<u64>, actor: Actor) -> Result<()> {
        if tokens.is_some_and(|t| t == 0 || t > BUDGET_MAX) {
            return Err(DomainError::InvalidArgument { what: "budget" });
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let (column, id, workspace) = match subject {
            Subject::Theme(id) => ("theme_id", id, workspace_of(&tx, "board_themes", id)?),
            Subject::Lane(id) => ("lane_id", id, workspace_of(&tx, "lanes", id)?),
        };
        let before: Option<i64> = tx
            .query_row(
                &format!("SELECT tokens FROM plan_budgets WHERE {column} = ?1"),
                params![uuid_blob(id)],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })
            .map_err(map_err)?;
        tx.execute(&format!("DELETE FROM plan_budgets WHERE {column} = ?1"), params![uuid_blob(id)])
            .map_err(map_err)?;
        if let Some(tokens) = tokens {
            tx.execute(
                &format!(
                    "INSERT INTO plan_budgets (id, workspace_id, {column}, tokens, set_at) VALUES (?1, ?2, ?3, ?4, ?5)"
                ),
                params![uuid_blob(Uuid::now_v7()), uuid_blob(workspace), uuid_blob(id), tokens as i64, now_millis()],
            )
            .map_err(map_err)?;
        }
        let body = match tokens {
            Some(t) => format!("budget {t} tokens"),
            None => "budget removed".to_string(),
        };
        event(&tx, subject, "budget", actor, &body, serde_json::json!({ "from": before, "to": tokens }))?;
        tx.commit().map_err(map_err)
    }
}

fn workspace_of(conn: &Connection, table: &str, id: Uuid) -> Result<Uuid> {
    conn.query_row(&format!("SELECT workspace_id FROM {table} WHERE id = ?1"), params![uuid_blob(id)], |r| {
        get_uuid(r, 0)
    })
    .map_err(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { DomainError::NotFound } else { map_err(e) })
}

#[cfg(test)]
#[path = "plan_cost_tests.rs"]
mod tests;
