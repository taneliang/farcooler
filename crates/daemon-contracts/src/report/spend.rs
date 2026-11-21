//! Spend breakdowns included in a report.

use farcooler_core::usage_words::Spend;
use serde::{Deserialize, Serialize};

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
