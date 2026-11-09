//! `plan.get`'s answer as the view model reads it: the shape `plan_json`
//! writes and `farcooler plan --json` prints (`test/fixtures/plan.json`).
//! Every field defaults, so a field an older runner leaves out reads as
//! nothing rather than as a plan that can't be read.

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub now_ms: i64,
    pub themes: Vec<Theme>,
    pub lines: Vec<Line>,
    pub cards: Vec<Card>,
    pub rulings: Vec<Ruling>,
    pub landings: Vec<Landing>,
    pub agents: Vec<Agent>,
    pub agents_spend: Option<Spend>,
    pub board_counts: Option<Counts>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct CardRef {
    pub task: String,
    pub key: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Counts {
    pub backlog: u64,
    pub todo: u64,
    pub needs_decision: u64,
    pub in_progress: u64,
    pub in_review: u64,
    pub done: u64,
    pub cancelled: u64,
}

impl Counts {
    /// The cards a theme's progress counts: all but the canceled.
    pub fn total(&self) -> u64 {
        self.backlog + self.todo + self.needs_decision + self.in_progress + self.in_review + self.done
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub outcome: String,
    pub next: String,
    pub story_at: i64,
    pub owner_ask: String,
    pub state: String,
    pub ordinal: i64,
    pub cards: Vec<CardRef>,
    pub counts: Counts,
    pub last_moved_at: Option<i64>,
    pub spend: Spend,
    pub budget_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Spend {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_micros: Option<i64>,
}

impl Spend {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Work {
    pub agent: String,
    pub role: String,
    pub ended_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PrStage {
    pub kind: String,
    pub label: String,
    pub reviewers: Vec<String>,
    pub queue_position: u64,
    pub pr_number: u64,
    pub unresolved_threads: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Line {
    pub task: String,
    pub key: String,
    pub title: String,
    pub stage: String,
    pub reason: String,
    pub plan_rank: Option<u64>,
    pub branch: String,
    pub landed_sha: Option<String>,
    pub stage_since: i64,
    pub stale: bool,
    pub budget_tokens: Option<u64>,
    pub fix_rounds: u64,
    pub landing: Option<u64>,
    pub work: Vec<Work>,
    pub spend: Spend,
    pub pr_stage: Option<PrStage>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Card {
    pub task: String,
    pub key: String,
    pub title: String,
    pub status: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Ruling {
    pub theme_id: Option<String>,
    pub created_at: i64,
    pub settled_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Landing {
    pub id: String,
    pub number: u64,
    pub title: String,
    pub summary: String,
    pub state: String,
    pub state_since: i64,
    pub phase: Option<String>,
    pub phase_since: Option<i64>,
    pub tickets: Vec<CardRef>,
    pub lander: Option<String>,
    pub pushed_sha: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AgentTicket {
    pub task: String,
    pub key: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Agent {
    pub id: String,
    pub role: String,
    pub harness: String,
    pub model: String,
    pub title: String,
    pub terminal: Option<String>,
    pub started_at: i64,
    pub tickets: Vec<AgentTicket>,
    pub working: bool,
    pub doing_now: String,
    pub last_at: i64,
    pub spend: Spend,
}

/// What the app knows beside the plan: the board's own reads.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Context {
    /// The workspace's Needs You count, for the section's row.
    pub needs_you: u64,
    /// The board's card statuses by task id, in either the store's word
    /// (`needs_decision`) or the board's ("Needs Decision").
    pub statuses: BTreeMap<String, String>,
    /// When each card last moved, in ms, by task id.
    pub activity: BTreeMap<String, i64>,
    /// The board's card titles by task id, for a card the plan carries none for.
    pub titles: BTreeMap<String, String>,
    /// Whether the runner lists agents (`board_line`); without it, the Agents
    /// section isn't drawn, since "No agents running" would be false.
    pub agents_supported: bool,
    /// The viewer's offset from UTC in seconds, which says where their day ends (the app sends its own; none is UTC).
    pub utc_offset: i64,
    pub style: super::words::Style,
}

impl Default for Context {
    fn default() -> Self {
        Context {
            needs_you: 0,
            statuses: BTreeMap::new(),
            activity: BTreeMap::new(),
            titles: BTreeMap::new(),
            agents_supported: true,
            utc_offset: 0,
            style: super::words::Style::Apple,
        }
    }
}
