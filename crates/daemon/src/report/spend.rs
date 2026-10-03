//! What agents spent in a report's period (ov-195): ov-194's per-turn
//! records, read for the report's scope and period.
//!
//! Two things come out. [`SpendReport`] is the whole scope's spend and the
//! same by task, harness, model and period, every turn that ended in the
//! period counted, filed on a task or not. And a `Usage` per task, which is
//! the seam `compute` was built with: it sums into every tally (each
//! repository, workspace, area and label) with no other change.
//!
//! Costs keep their provenance (`usage_words`): a reader is told what was
//! reported, what was estimated, and what nobody knows, and never one sum of
//! unlike things.

use std::collections::HashMap;

use farcooler_core::Result;
use farcooler_core::usage::PRICE_TABLE;
use farcooler_core::usage_words::Spend;
use farcooler_store::Store;
use farcooler_store::usage::{GroupBy, UsageFilter, UsageGroup, UsageTotals};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Narrowing, Period, Usage};

/// How many tasks `by_task` names at most; `other_tasks` counts the rest.
pub const SPEND_TASK_LIMIT: usize = 10;

/// The spend section of a report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendReport {
    /// Every turn in scope that ended in the period, on a task or not.
    pub total: Spend,
    /// The tasks that spent the most: most dollars first, then most tokens.
    /// Turns on no task are one line with no key, named "No task".
    pub by_task: Vec<SpendLine>,
    /// Tasks past `SPEND_TASK_LIMIT`, left out of `by_task`.
    pub other_tasks: u32,
    /// `claude`, `codex`, or an ACP adapter's preset; ordered as `by_task`.
    pub by_harness: Vec<SpendLine>,
    /// The model each harness named; "" when it named none. A turn on two
    /// models is under both, so turns don't add up across these lines.
    pub by_model: Vec<SpendLine>,
    /// `day`, `week` or `month`, from the period's length: what `by_period`
    /// is cut by, in the client's own time zone.
    pub period_unit: String,
    /// Oldest first. A day is `YYYY-MM-DD`, a week its Monday as
    /// `YYYY-MM-DD`, a month `YYYY-MM`.
    pub by_period: Vec<SpendLine>,
    /// The price table this runner estimates from today.
    pub price_table: String,
}

/// One line of a breakdown.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendLine {
    /// The task's key, the harness, the model or the period.
    pub name: String,
    /// The task's title, for a task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(flatten)]
    pub spend: Spend,
}

/// The cut a period is broken down by: days for two weeks or less, weeks
/// for about a quarter, months beyond.
pub fn period_unit(period: Period) -> GroupBy {
    const DAY: i64 = 24 * 60 * 60 * 1000;
    match period.until - period.since {
        span if span <= 14 * DAY => GroupBy::Day,
        span if span <= 92 * DAY => GroupBy::Week,
        _ => GroupBy::Month,
    }
}

fn unit_word(unit: GroupBy) -> &'static str {
    match unit {
        GroupBy::Day => "day",
        GroupBy::Week => "week",
        _ => "month",
    }
}

/// Store totals in the shape every client reads.
pub fn spend_of(t: &UsageTotals) -> Spend {
    Spend {
        turns: t.turns,
        turns_partial: t.turns_partial,
        turns_not_reported: t.turns_not_reported,
        subagent_runs: t.subagent_runs,
        active_ms: t.active_ms,
        input_tokens: t.tokens.input,
        output_tokens: t.tokens.output,
        cache_read_tokens: t.tokens.cache_read,
        cache_write_tokens: t.tokens.cache_write,
        cost_reported_micros: t.cost_reported_micros,
        cost_estimated_micros: t.cost_estimated_micros,
        unpriced_tokens: t.unpriced_tokens,
        price_tables: t.price_tables.clone(),
    }
}

/// A task's spend as the tallies sum it. Agent time is absent when no turn
/// was timed, as `Usage` has it.
fn usage_of(s: &Spend) -> Usage {
    Usage {
        input_tokens: Some(s.input_tokens),
        output_tokens: Some(s.output_tokens),
        cache_read_tokens: Some(s.cache_read_tokens),
        cache_write_tokens: Some(s.cache_write_tokens),
        agent_ms: (s.active_ms > 0).then_some(s.active_ms),
        turns: Some(s.turns),
        turns_partial: Some(s.turns_partial),
        turns_not_reported: Some(s.turns_not_reported),
        cost_reported_micros: Some(s.cost_reported_micros),
        cost_estimated_micros: Some(s.cost_estimated_micros),
        unpriced_tokens: Some(s.unpriced_tokens),
    }
}

/// Most dollars first, then most tokens, then by name.
fn biggest_first(lines: &mut [SpendLine]) {
    lines.sort_by(|a, b| {
        let cost = |s: &Spend| s.cost_reported_micros + s.cost_estimated_micros;
        cost(&b.spend)
            .cmp(&cost(&a.spend))
            .then_with(|| b.spend.total_tokens().cmp(&a.spend.total_tokens()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// The period's spend in `narrowing`, and each task's share of it. Nothing
/// when no turn in scope ended in the period. Read-only.
pub fn read(
    store: &Store,
    narrowing: Narrowing,
    period: Period,
    utc_offset_minutes: i32,
) -> Result<(Option<SpendReport>, HashMap<Uuid, Usage>)> {
    let mut filter = UsageFilter { since: Some(period.since), until: Some(period.until), ..Default::default() };
    match narrowing {
        Narrowing::Runner => {}
        Narrowing::Repository(id) => filter.repository_id = Some(id),
        Narrowing::Workspace(id) => filter.workspace_id = Some(id),
    }
    let summary = |by: &[GroupBy]| store.usage_summary(&filter, by, utc_offset_minutes);
    let Some(total) = summary(&[])?.pop().map(|g| spend_of(&g.totals)).filter(|s| !s.is_empty()) else {
        return Ok((None, HashMap::new()));
    };

    let mut per_task = HashMap::new();
    let mut by_task = Vec::new();
    for UsageGroup { key, totals } in summary(&[GroupBy::Task])? {
        let spend = spend_of(&totals);
        let line = match key.task_id {
            Some(id) => {
                per_task.insert(id, usage_of(&spend));
                match store.get_task(id) {
                    Ok(task) => SpendLine { name: task.key, title: Some(task.title), spend },
                    // A task deleted since: its turns still count.
                    Err(_) => SpendLine { name: id.to_string()[..8].to_string(), title: None, spend },
                }
            }
            None => SpendLine { name: "No task".into(), title: None, spend },
        };
        by_task.push(line);
    }
    biggest_first(&mut by_task);
    let other_tasks = by_task.len().saturating_sub(SPEND_TASK_LIMIT) as u32;
    by_task.truncate(SPEND_TASK_LIMIT);

    let lines = |by: GroupBy, name: &dyn Fn(&UsageGroup) -> String| -> Result<Vec<SpendLine>> {
        Ok(summary(&[by])?
            .iter()
            .map(|g| SpendLine { name: name(g), title: None, spend: spend_of(&g.totals) })
            .collect())
    };
    let mut by_harness = lines(GroupBy::Harness, &|g| g.key.harness.clone().unwrap_or_default())?;
    biggest_first(&mut by_harness);
    let mut by_model = lines(GroupBy::Model, &|g| g.key.model.clone().unwrap_or_default())?;
    biggest_first(&mut by_model);
    let unit = period_unit(period);
    // In key order, which for these is oldest first.
    let by_period = lines(unit, &|g| g.key.period.clone().unwrap_or_default())?;

    let report = SpendReport {
        total,
        by_task,
        other_tasks,
        by_harness,
        by_model,
        period_unit: unit_word(unit).to_string(),
        by_period,
        price_table: PRICE_TABLE.to_string(),
    };
    Ok((Some(report), per_task))
}
