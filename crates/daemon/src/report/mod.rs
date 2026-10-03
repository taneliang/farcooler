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
//! **The shape is JSON, and this file is its one definition.** The wire
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
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use compute::{area_of, compute, median, p90};
pub use gather::{Narrowing, gather};
pub use serve::serve;

/// The schema's version. See the module doc for when it moves.
pub const SCHEMA: u32 = 1;

/// How many tasks each `notable` list names at most.
pub const NOTABLE_LIMIT: usize = 5;

/// The whole report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    /// Unix milliseconds: the period, `[since, until)`.
    pub since: i64,
    pub until: i64,
    /// Unix milliseconds: when the runner computed this.
    pub generated_at: i64,
    /// What the report covers: the whole runner, one repository, or one
    /// workspace.
    pub scope: Scope,
    /// Everything in scope, added up.
    pub totals: Tally,
    /// The same tally per repository, per workspace, per area (the title's
    /// `Area:` prefix) and per label. A group with nothing in the period is
    /// left out. Ordered by tasks completed, most first, then by name.
    pub by_repository: Vec<Group>,
    pub by_workspace: Vec<Group>,
    pub by_area: Vec<Group>,
    pub by_label: Vec<Group>,
    pub notable: Notable,
    /// What agents spent in the period. `None` when no turn in scope ended
    /// in it, and from a runner older than ov-195.
    #[serde(default)]
    pub spend: Option<spend::SpendReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// `runner`, `repository` or `workspace`.
    pub kind: String,
    /// The repository's or workspace's name; `None` for the runner.
    pub name: Option<String>,
    /// The repository a workspace is in.
    pub repository: Option<String>,
}

impl Default for Scope {
    /// The whole runner.
    fn default() -> Scope {
        Scope { kind: "runner".into(), name: None, repository: None }
    }
}

/// One slice of the report: a repository, a workspace, an area or a label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    /// The repository a workspace is in; `None` for every other kind.
    pub repository: Option<String>,
    #[serde(flatten)]
    pub tally: Tally,
}

/// The numbers, for any set of tasks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tally {
    /// Tasks filed in the period.
    pub created: u32,
    /// Tasks worked on (in In Progress at some point) that moved to Done in
    /// the period. A task done twice counts once; one done and then moved on
    /// again inside the period is not done.
    pub completed: u32,
    /// Tasks that reached Done in the period without ever being in
    /// progress: filed already done, a record of work done elsewhere. Kept
    /// out of `completed` and every duration, where a time to done of zero
    /// would drag the medians toward nothing.
    pub filed_done: u32,
    /// Tasks that moved to Canceled in the period, read as `completed` is:
    /// one canceled and restored inside the period is not canceled.
    pub canceled: u32,
    /// Moves from Done or Canceled back to an open status in the period:
    /// work that came back. Done to Canceled is not one.
    pub reopened: u32,
    /// Moves from In Review back to In Progress in the period.
    pub fix_rounds: u32,
    /// Filed to done, for the tasks completed in the period.
    pub time_to_done: Option<Spread>,
    /// First In Progress to done, for the same tasks: the work time,
    /// without the wait in the backlog.
    pub work_time: Option<Spread>,
    /// Time spent in each open status inside the period, across every task,
    /// in board order. Done and Canceled are endings, not places to wait,
    /// so they are not here. A status no task sat in is left out.
    pub time_in_status: Vec<StatusTime>,
    pub decisions: Decisions,
    pub needs_you: NeedsYou,
    /// The acceptance lines of the tasks completed in the period.
    pub acceptance: Acceptance,
    /// Tokens, cost and agent time, from ov-194's per-turn records. `None`
    /// for a set of tasks none of which has any.
    pub usage: Option<Usage>,
}

/// The middle and the slow end of a set of durations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spread {
    /// How many durations this describes.
    pub count: u32,
    /// Milliseconds. The middle value, or the mean of the two middle values
    /// when there is an even number.
    pub median_ms: i64,
    /// Milliseconds. Nearest rank: the smallest value at least 90% of the
    /// set is no greater than.
    pub p90_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusTime {
    /// The stored word: `backlog`, `todo`, `needs_decision`, `in_progress`,
    /// `in_review`.
    pub status: String,
    /// Milliseconds, added up across tasks.
    pub total_ms: i64,
    /// How many tasks spent any of the period in it.
    pub tasks: u32,
    /// Milliseconds: the median task's share of `total_ms`.
    pub median_ms: i64,
}

/// Questions put to a person, and how they were answered.
///
/// An answer answers every question on its task asked before it and not yet
/// answered, which is how `needs_you` reads a task's questions. A question
/// another question supersedes was withdrawn and is not counted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decisions {
    /// Questions asked in the period.
    pub asked: u32,
    /// Questions whose answer came in the period, whenever they were asked.
    pub answered: u32,
    /// Of those, answered by a person (`user`).
    pub answered_by_you: u32,
    /// Of those, answered by the orchestrator (`manager`), which is also how
    /// a ruling it relays from a person is recorded today.
    pub answered_by_orchestrator: u32,
    /// Questions still without an answer when the period ended. A question
    /// on a task that was done or canceled without one is not waiting.
    pub unanswered: u32,
    /// Questions whose task was done or canceled in the period with no
    /// answer note: settled some other way, or dropped.
    pub closed_unanswered: u32,
    /// Question to answer, for every question answered in the period.
    pub latency: Option<Spread>,
    /// The same, for the ones a person answered.
    pub latency_you: Option<Spread>,
    /// `Decision` notes written in the period: choices recorded, by anyone.
    pub recorded: u32,
}

/// Spells a task waited on a person: in Needs Decision, with a question
/// only a person can answer.
///
/// In Review is not one. On a board the orchestrator owns, the orchestrator
/// does the reviewing, so a spell in review waits on it, not on you. The
/// board has no marker yet for a review a person must do; when it gets one,
/// those spells belong here too (future work, ov-188 phase 2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeedsYou {
    /// Spells that began in the period.
    pub times: u32,
    /// Spells that ended in the period, whenever they began.
    pub cleared: u32,
    /// Spells still open when the period ended.
    pub waiting: u32,
    /// Start to end, for the spells that ended in the period.
    pub time_to_clear: Option<Spread>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acceptance {
    /// Lines ticked, on the tasks completed in the period.
    pub met: u32,
    /// Lines in all, on those tasks.
    pub total: u32,
    /// Tasks completed with every line ticked. A task with no lines is not
    /// one of these.
    pub tasks_fully_met: u32,
    /// Tasks completed with no acceptance lines at all.
    pub tasks_without_lines: u32,
}

/// Tokens and agent time, from ov-194's per-turn records.
///
/// Every field is optional because harnesses report different things:
/// Codex has no cache-write count, and a turn the runner didn't watch has
/// tokens but no agent time. A sum carries a field only when some turn did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// Milliseconds an agent spent working: turn start to turn end.
    pub agent_ms: Option<i64>,
    /// Finished turns.
    #[serde(default)]
    pub turns: Option<u64>,
    /// Millionths of a dollar, kept apart by provenance as `usage_words`
    /// says them: what the agents reported, what the price table estimated.
    #[serde(default)]
    pub cost_reported_micros: Option<i64>,
    #[serde(default)]
    pub cost_estimated_micros: Option<i64>,
    /// Tokens with no known price.
    #[serde(default)]
    pub unpriced_tokens: Option<u64>,
}

impl Usage {
    /// Field by field: absent plus absent stays absent.
    pub fn plus(self, other: Usage) -> Usage {
        fn sum<T: std::ops::Add<Output = T>>(a: Option<T>, b: Option<T>) -> Option<T> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a + b),
                (a, b) => a.or(b),
            }
        }
        Usage {
            input_tokens: sum(self.input_tokens, other.input_tokens),
            output_tokens: sum(self.output_tokens, other.output_tokens),
            cache_read_tokens: sum(self.cache_read_tokens, other.cache_read_tokens),
            cache_write_tokens: sum(self.cache_write_tokens, other.cache_write_tokens),
            agent_ms: sum(self.agent_ms, other.agent_ms),
            turns: sum(self.turns, other.turns),
            cost_reported_micros: sum(self.cost_reported_micros, other.cost_reported_micros),
            cost_estimated_micros: sum(self.cost_estimated_micros, other.cost_estimated_micros),
            unpriced_tokens: sum(self.unpriced_tokens, other.unpriced_tokens),
        }
    }
}

/// What stood out, by task key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notable {
    /// Completed in the period, longest from filed to done first.
    pub slowest: Vec<NotableTask>,
    /// The longest a person was waited on: a question until its answer. One
    /// still open counts up to the end of the period.
    pub longest_waits: Vec<NotableWait>,
    /// Moved out of Done in the period: the closest thing to a failure the
    /// board records today.
    pub reopened: Vec<NotableTask>,
    /// The most fix rounds in the period, at least one.
    pub most_fix_rounds: Vec<NotableTask>,
    /// Canceled in the period.
    pub canceled: Vec<NotableTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotableTask {
    pub key: String,
    pub title: String,
    pub workspace: String,
    /// Milliseconds, where the list is about a duration: filed to done.
    pub ms: Option<i64>,
    /// Where the list is about a count: times reopened, fix rounds.
    pub count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotableWait {
    pub key: String,
    pub title: String,
    pub workspace: String,
    /// `question`, today the only kind. A person's review would be another,
    /// once the board can mark one.
    pub kind: String,
    pub ms: i64,
    /// Whether it was still open when the period ended.
    pub open: bool,
}

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
