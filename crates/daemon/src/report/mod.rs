//! `report.get`: what the agents on this runner achieved in a period.
//!
//! Facts only, computed from what the board already records: the
//! `StatusChange` notes (when a task moved, and where to), the `Question` and
//! `Answer` notes (who was asked, and how long the answer took), `Decision`
//! notes, and each task's acceptance lines. Nothing here writes; `gather`
//! reads the store and `compute` is a pure function of what it read, a period
//! and a clock, which is what the tests pin. The prose a person would write
//! about the same week is the orchestrator's (phase 4 of ov-188's plan), not
//! this module's.
//!
//! **The shape is JSON, defined in `farcooler_daemon_contracts::report`.** The wire
//! carries it as `Report.report_json`, for `AgentEventFrame.payload_json`'s
//! reason: a report is a tree of optional, growing parts, and restating it in
//! protobuf would make a second definition to keep in step. The CLI decodes
//! these same structs to print its summary; an app decodes the JSON.
//! [`SCHEMA`] moves only when a field changes meaning; adding one does not
//! move it, and a reader ignores fields it doesn't know.
//!
//! **What agents spent** comes from ov-194's per-turn records (ov-195).
//! `spend::read` fills [`Report::spend`], the scope's tokens, cost and agent
//! time by task, harness, model and period, and [`Inputs::usage`] per task,
//! which every tally sums. See [`Usage`] and [`spend::SpendReport`].
//!
//! A period is `[since, until)` in Unix milliseconds. An event counts when it
//! happened inside it; a span (time in a status, a wait) counts only the part
//! inside it, and never the part after `now`.

mod compute;
mod gather;
mod serve;
pub mod spend;
#[cfg(test)]
mod spend_tests;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use farcooler_store::models::{Task, TaskNote};
use uuid::Uuid;

pub use compute::{area_of, compute, median, p90};
pub use gather::{Narrowing, gather};
pub use serve::serve;

pub use farcooler_daemon_contracts::report::{Acceptance, Decisions, Group, NOTABLE_LIMIT, NeedsYou, Notable, NotableTask,
    NotableWait, Report, SCHEMA, Scope, Spread, StatusTime, Tally, Usage};

/// One task, as `compute` reads it.
#[derive(Debug, Clone)]
pub struct TaskFacts {
    pub task: Task,
    pub workspace: String,
    pub repository: String,
    /// Its record, oldest first: `Store::notes_for`'s order.
    pub notes: Vec<TaskNote>,
}

/// Everything `compute` reads. Filled by `gather`; built by hand in tests.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub tasks: Vec<TaskFacts>,
    /// Token usage, cost and agent time inside the period, per task.
    pub usage: HashMap<Uuid, Usage>,
    /// The period's spend, as the report carries it.
    pub spend: Option<spend::SpendReport>,
    pub scope: Scope,
}

/// `[since, until)`, in Unix milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    pub since: i64,
    pub until: i64,
}

impl Period {
    pub fn contains(self, at: i64) -> bool {
        self.since <= at && at < self.until
    }
}
