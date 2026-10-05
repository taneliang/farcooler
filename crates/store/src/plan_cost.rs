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
//! last seven days and nothing more; a client shows it with no percentage. A
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
    /// Finished cards' cost by harness and model, those with at least
    /// `MIN_CARDS_TO_COMPARE` cards behind them, most cards first.
    pub compare: Vec<HarnessModelCost>,
    /// How many harness-and-model pairs had finished cards but too few to
    /// compare, so a client can say some are held back.
    pub compare_held_back: u32,
}

/// What the cards a harness and model worked, once finished, cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessModelCost {
    pub harness: String,
    /// Empty when the harness named no model.
    pub model: String,
    /// How many finished cards it worked.
    pub cards: u32,
    /// Tokens on those cards by this pair, all four counts.
    pub tokens: u64,
    /// Millionths of a US dollar on those cards by this pair; `None` unless
    /// every one of its turns was priced.
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

/// A lane's tokens on each of the last seven UTC days, oldest first. A Claude
/// agent shared with another lane counts here by its even split, as the
/// lane's spend does, so summing lanes counts it once.
pub(crate) fn lane_days(conn: &Connection, lane: Uuid, start_ms: i64) -> Result<[f64; TREND_DAYS]> {
    let mut stmt = conn
        .prepare(
            "WITH lanes_of AS (
                 SELECT harness, agent_id, count(*) AS lanes FROM lane_agents GROUP BY harness, agent_id
             )
             SELECT min(CAST((t.ended_at - ?2) / ?3 AS INTEGER), ?4),
                    sum((m.input_tokens + m.output_tokens + m.cache_read_tokens + m.cache_write_tokens) * 1.0 / s.lanes)
               FROM lane_agents a
               JOIN lanes_of s ON s.harness = a.harness AND s.agent_id = a.agent_id
               JOIN agent_turns t ON t.turn_key = 'claude-log:agent:' || a.agent_id
               JOIN agent_turn_models m ON m.turn_id = t.id
              WHERE a.lane_id = ?1 AND a.harness = 'claude' AND t.ended_at >= ?2
              GROUP BY 1",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(lane), start_ms, DAY_MS, TREND_DAYS as i64 - 1], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    let mut days = [0.0; TREND_DAYS];
    for (day, tokens) in rows {
        if let Some(slot) = days.get_mut(day.max(0) as usize) {
            *slot += tokens;
        }
    }
    Ok(days)
}

/// The cost half of a board's plan read.
pub(crate) fn cost_of(conn: &Connection, cards: &HashMap<Uuid, CardRef>, now_ms: i64) -> Result<PlanCost> {
    let week_tokens: i64 = conn
        .query_row(
            "SELECT coalesce(sum(m.input_tokens + m.output_tokens + m.cache_read_tokens + m.cache_write_tokens), 0)
               FROM agent_turns t JOIN agent_turn_models m ON m.turn_id = t.id
              WHERE t.ended_at >= ?1",
            params![now_ms - TREND_DAYS as i64 * DAY_MS],
            |r| r.get(0),
        )
        .map_err(map_err)?;

    // Per card first, so a finished card counts once for each pair that
    // worked it, and the filter to finished cards is the board's own status.
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
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    let mut pairs: HashMap<(String, String), (u32, u64, i64, bool)> = HashMap::new();
    for (task, harness, model, tokens, micros, unpriced) in rows {
        if cards.get(&task).is_none_or(|c| c.status != TaskStatus::Done) {
            continue;
        }
        let pair = pairs.entry((harness, model)).or_insert((0, 0, 0, true));
        pair.0 += 1;
        pair.1 += tokens.max(0) as u64;
        pair.2 += micros;
        pair.3 &= unpriced == 0;
    }
    let mut compare: Vec<HarnessModelCost> = Vec::new();
    let mut compare_held_back = 0;
    for ((harness, model), (n, tokens, micros, priced)) in pairs {
        if n < MIN_CARDS_TO_COMPARE {
            compare_held_back += 1;
            continue;
        }
        compare.push(HarnessModelCost { harness, model, cards: n, tokens, cost_micros: priced.then_some(micros) });
    }
    compare.sort_by(|a, b| b.cards.cmp(&a.cards).then_with(|| (&a.harness, &a.model).cmp(&(&b.harness, &b.model))));
    Ok(PlanCost { week_tokens: week_tokens.max(0) as u64, compare, compare_held_back })
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
