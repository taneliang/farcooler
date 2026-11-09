//! The plan's sections and ticket cards as one view model (ov-1016, ov-1015
//! phase 1; ruling R-112). Mac, iPhone, Android and the web each drew the
//! Plan from their own copy of the same rules (`PlanSections.swift`,
//! `PlanSections.kt`, `plan.ts`); this is the one copy. It decides which
//! ticket is in which section, the order of the landings, every word a
//! section header, a ticket card, a landing card, an agent row and a theme
//! tile says, and which run of the workspace's workflow engine a card shows.
//! The apps lay it out.
//!
//! Pure: no I/O, no clock (the plan carries the runner's `now_ms`) and no
//! authority. A request is `plan.get`'s answer, what the app knows beside it
//! (`Context`) and, when the workspace has a published workflow, the
//! presentation `workflow.snapshot` answered (`workflow_presentation`).
//!
//! Glance, cost, rulings and the sidebar trees are still drawn from the plan
//! natively; they move here in the next phases (ov-1015 §2).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::workflow_presentation::{Presentation, RunCard};

mod build;
pub mod column;
pub mod input;
pub mod words;

pub use build::{LandingPhase, DetailStage, TicketDetails, TicketDetail, TrackKind};
pub use words::Style;

pub const SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    NeedsYou,
    Landing,
    InReview,
    Building,
    Waiting,
    OnLandedLanding,
    Agents,
    Themes,
    /// The tickets that landed on the viewer's day; drawn last.
    LandedToday,
}

/// A section to draw, in the order drawn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub kind: SectionKind,
    pub title: String,
    /// Its rows; 0 draws no count.
    pub count: u64,
    /// What it says with nothing in it; nil when it has rows or says nothing.
    pub empty: Option<String>,
    /// Drawn folded until opened.
    pub folded: bool,
    /// Said on hover over (or a long press of) the title, never its rows.
    pub help: Option<String>,
}

/// What a ticket's mark shows: its stage, a fix round, or its place in Next Up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Rank,
    Waiting,
    Building,
    Fixing,
    Reviewing,
    NeedsYou,
    Landing,
    Landed,
    Dropped,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    /// The pull request's stage.
    Stage,
    /// The engine run's step and gate.
    Run,
    /// Waits on the owner, or hasn't moved.
    Warning,
    OverBudget,
}

/// A line under a ticket's second line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub kind: NoteKind,
    pub text: String,
    /// Drawn in amber.
    pub attention: bool,
    /// Every word of it, for a hover or a long press.
    pub help: Option<String>,
}

/// The agent working a ticket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRef {
    pub id: String,
    /// "Builder · Sonnet".
    pub who: String,
    /// "380K"; nil when no turn stated any.
    pub tokens: Option<String>,
    /// Who it is and what it has spent, as its own line under a card's
    /// second line on Android ("Builder · sonnet · 380K").
    pub line: String,
    /// The pane, to open it.
    pub terminal: Option<String>,
    /// The model as recorded ("claude-opus-4-1"), which a screen reader says
    /// when `who` shortens it; "" when none is known.
    #[serde(default)]
    pub model: String,
}

/// The engine's run of a ticket, from the workspace's workflow presentation:
/// the step it is on, the gate it waits at, and the phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStep {
    pub run: String,
    pub phase: farcooler_workflow::Phase,
    pub phase_label: String,
    /// The step it is on; nil past the last.
    pub step: Option<String>,
    pub gate: Option<Gate>,
    /// It waits on a person, failed, or its outcome is uncertain.
    pub attention: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    pub label: String,
    pub text: String,
    pub principal: String,
}

/// A ticket's row: in In Review, Building, Waiting or Waiting for the Next
/// Landing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketCard {
    /// The card's task id.
    pub id: String,
    pub key: String,
    /// Its title, else its key.
    pub heading: String,
    /// The theme it serves.
    pub theme: Option<String>,
    /// The stage, as the plan's wire spells it.
    pub stage: String,
    /// "Fixing · round 1", "Waiting · 2nd", "In Review · in Landing 8".
    pub status: String,
    /// The line under the title: Next Up's reason, else the stage, key, agent,
    /// tokens and how long it has been in this stage.
    pub second: String,
    /// Its place in Next Up, from 1.
    pub rank: Option<u64>,
    pub mark: Mark,
    /// The mark is drawn in amber.
    pub mark_attention: bool,
    pub notes: Vec<Note>,
    pub waits_on_owner: bool,
    /// The landing that landed which it still names.
    pub landed_landing: Option<String>,
    pub agent: Option<AgentRef>,
    pub run: Option<RunStep>,
    pub branch: String,
    /// On a landing, the word drawn beside the title: "Needs you" while it
    /// waits on the owner (which `notes` then leads with), "Landed · ac840108"
    /// once it has; nil when it is simply aboard.
    #[serde(default)]
    pub note: Option<String>,
    /// What a screen reader says for the row.
    pub accessibility: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitingKind {
    Next,
    NotRanked,
    OnYou,
}

/// One of Waiting's groups; only groups with cards are sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingGroup {
    pub kind: WaitingKind,
    /// Drawn only when another group shares the section.
    pub title: String,
    /// The title is drawn in amber.
    pub attention: bool,
    pub cards: Vec<TicketCard>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LadderStep {
    pub label: String,
    pub explanation: String,
}

/// The ladder a landing climbs: three steps, and the one it stands on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ladder {
    pub steps: Vec<LadderStep>,
    /// 0 to 2, or 3 once everything is done.
    pub at: u64,
    /// "Checking on CI · step 2 of 3".
    pub compact: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandingCard {
    pub id: String,
    pub number: u64,
    /// "Landing 86".
    pub label: String,
    /// The title without the label in front, or nil.
    pub subtitle: Option<String>,
    /// Its tickets' titles on one line.
    pub carries: Option<String>,
    /// The phase, spelled as the plan's wire spells it.
    pub phase: LandingPhase,
    /// "Checking on CI".
    pub phase_word: String,
    /// "checking on CI", inside a sentence.
    pub phase_sentence: String,
    pub explanation: String,
    /// CI failed, or nothing has moved.
    pub attention: bool,
    /// When it entered this phase, in ms.
    pub since: i64,
    pub ladder: Ladder,
    pub lander: Option<AgentRef>,
    /// "Lander · Opus · 1.4M".
    pub lander_words: Option<String>,
    pub pushed_sha: Option<String>,
    /// What stands in for CI's line when a commit is pushed and the app has no CI read of it yet.
    pub ci_unread: Option<String>,
    pub tickets: Vec<TicketCard>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRow {
    pub id: String,
    pub who: String,
    pub working: bool,
    /// The plan, its tickets by key, else "Not on a ticket".
    pub works_for: String,
    /// "Working · 22 min", "Idle · active 1 h ago".
    pub state: String,
    /// "“Design ov-503” · ov-503 · Working · 22 min".
    pub second: String,
    pub doing_now: String,
    pub tokens: Option<String>,
    /// "≈$1.10".
    pub cost: Option<String>,
    pub terminal: Option<String>,
    pub accessibility: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agents {
    /// "5 agents, including the orchestrator · 4 working · 3.9M tokens in the
    /// last 24 hours · ≈$26", or "No agents running."
    pub header: String,
    /// Working first, then by spend.
    pub rows: Vec<AgentRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Glyph {
    Done,
    InProgress,
    NotStarted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileCard {
    pub id: String,
    pub key: String,
    pub title: String,
    pub glyph: Glyph,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeTile {
    pub id: String,
    pub name: String,
    pub done: u64,
    pub total: u64,
    /// "6 of 10 done".
    pub progress: String,
    /// How full the meter is, 0 to 1.
    pub fraction: f64,
    /// Up to three: in progress first, then the most recent done, then the
    /// next not started.
    pub cards: Vec<TileCard>,
    /// What waits on the owner, the one amber line.
    pub owner_ask: Option<String>,
    /// A ticket of it is landing, in review or building.
    pub moving: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Themes {
    /// Themes in motion first, then the rest by last move.
    pub tiles: Vec<ThemeTile>,
    /// Done themes, folded into one tile.
    pub finished: Vec<ThemeTile>,
    pub finished_title: Option<String>,
    /// "7 themes".
    pub finished_words: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanView {
    pub schema: u32,
    /// The runner's clock.
    pub now: i64,
    /// The sections to draw, in order.
    pub sections: Vec<Section>,
    pub needs_you: u64,
    /// Live landings, furthest along first.
    pub landings: Vec<LandingCard>,
    pub in_review: Vec<TicketCard>,
    pub building: Vec<TicketCard>,
    /// Next Up ranked, Not Ranked, then Waiting on You; empty groups left out.
    pub waiting: Vec<WaitingGroup>,
    pub on_landed_landing: Vec<TicketCard>,
    /// Landed on the viewer's day, newest first.
    pub landed_today: Vec<TicketCard>,
    pub agents: Agents,
    pub themes: Themes,
    /// The plan column beside the conversation: facts, initiatives, the
    /// pipeline of agents and the footer counts (ov-867).
    pub column: column::Column,
    /// What ticket details and theme tracks say (ov-1038).
    pub details: TicketDetails,
}

impl PlanView {
    /// The cards in a Waiting group, or none.
    pub fn waiting(&self, kind: WaitingKind) -> &[TicketCard] {
        self.waiting.iter().find(|g| g.kind == kind).map_or(&[], |g| &g.cards)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    plan: input::Plan,
    #[serde(default)]
    context: input::Context,
    /// `workflow.snapshot`'s presentation, when the workspace has a published
    /// workflow.
    #[serde(default)]
    workflow: Option<Presentation>,
}

#[derive(Serialize)]
struct Response {
    schema: u32,
    view: Option<PlanView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    words: Option<words::Words>,
    error: Option<&'static str>,
}

/// `{"schema":1,"words":{"style":"apple"}}`: the phrases other screens share with the plan.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WordsRequest {
    schema: u32,
    words: WordsStyle,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WordsStyle {
    #[serde(default)]
    style: words::Style,
}

/// Build the view from a plan, what the app knows beside it, and the
/// workspace's workflow presentation.
pub fn view(plan: &input::Plan, context: &input::Context, workflow: Option<&Presentation>) -> PlanView {
    build::build(plan, context, workflow)
}

/// The JSON bridge every platform calls: a request in, `{schema, view}` or
/// `{schema, error: "invalid"}` out.
pub fn json(request: &str) -> String {
    if let Ok(asked) = serde_json::from_str::<WordsRequest>(request) {
        let words = (asked.schema == SCHEMA).then(|| words::Words::new(asked.words.style));
        let error = words.is_none().then_some("invalid");
        return serde_json::to_string(&Response { schema: SCHEMA, view: None, words, error }).expect("plan view types serialize");
    }
    let parsed = (request.len() <= 8 * 1024 * 1024)
        .then(|| serde_json::from_str::<serde_json::Value>(request).ok())
        .flatten()
        .and_then(|mut value| {
            // A null reads as the field left out, as every additive field on
            // the plan's wire is: an older runner's null title is "".
            if let Some(map) = value.as_object_mut() {
                map.retain(|_, v| !v.is_null());
            }
            for key in ["plan", "context"] {
                if let Some(part) = value.get_mut(key) {
                    drop_nulls(part);
                }
            }
            serde_json::from_value::<Request>(value).ok()
        })
        .filter(|r| r.schema == SCHEMA);
    let response = match parsed {
        Some(r) => Response { schema: SCHEMA, view: Some(view(&r.plan, &r.context, r.workflow.as_ref())), words: None, error: None },
        None => Response { schema: SCHEMA, view: None, words: None, error: Some("invalid") },
    };
    serde_json::to_string(&response).expect("plan view types serialize")
}

fn drop_nulls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            map.values_mut().for_each(drop_nulls);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(drop_nulls),
        _ => {}
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "plan_view/column_tests.rs"]
mod column_tests;

#[cfg(test)]
#[path = "plan_view/ticket_detail_tests.rs"]
mod ticket_detail_tests;
