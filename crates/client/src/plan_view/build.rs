//! The rules (ov-503, ov-522, ov-872), as AgentKit's `PlanSections` had them.
//!
//! A ticket belongs to exactly one place:
//! - on a live landing, it's in that landing's card (Landing), whatever its
//!   stage;
//! - naming a landing that landed while it's still waiting or landing, it's in
//!   Waiting for the Next Landing (folded, and each row says which landing);
//! - in review, needing the owner, or landing with no landing, it's In Review;
//! - building, it's Building; in a fix round after a review, it's In Review;
//! - a ticket building or in review is never hidden by a landed landing: it
//!   routes by its stage and the row says it names that landing;
//! - waiting, it's Waiting on You when its card needs a decision, else Next Up
//!   when the plan ranks it, else Not Ranked.
//!
//! Order (owner decision 7; ov-522): Needs You, Landing, In Review, Building,
//! Waiting, Agents, Themes. The word is landing, never shipping.

use farcooler_core::usage_words::{dollars, tokens};
use farcooler_workflow::Phase;

use super::input::{Agent, Landing, Line, Plan, PrStage, Theme};
use super::words::{self, DOT, Style};
use super::*;

mod pages;
pub use pages::{PageStage, Pages, ThemeTrack, TicketPage, TrackKind};

/// A ticket's stage, from the wire's word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    Waiting,
    Building,
    Reviewing,
    NeedsYou,
    Landing,
    Landed,
    Dropped,
    #[serde(other)]
    Unknown,
}

/// A wire word read as one of the enums above; a word it doesn't know is `None`.
pub(super) fn wire<T: serde::de::DeserializeOwned>(word: &str) -> Option<T> {
    serde_json::from_value(word.into()).ok()
}

impl Stage {
    fn of(word: &str) -> Stage {
        wire(word).unwrap_or(Stage::Unknown)
    }

    /// Neither landed nor dropped.
    pub(super) fn is_live(self) -> bool {
        self != Stage::Landed && self != Stage::Dropped
    }

    fn word(self, style: Style) -> String {
        style.label(match self {
            Stage::Waiting => "waiting",
            Stage::Building => "building",
            Stage::Reviewing => "in review",
            Stage::NeedsYou => "needs you",
            Stage::Landing => "landing",
            Stage::Landed => "landed",
            Stage::Dropped => "dropped",
            Stage::Unknown => "unknown",
        })
    }

    fn mark(self) -> Mark {
        match self {
            Stage::Waiting => Mark::Waiting,
            Stage::Building => Mark::Building,
            Stage::Reviewing => Mark::Reviewing,
            Stage::NeedsYou => Mark::NeedsYou,
            Stage::Landing => Mark::Landing,
            Stage::Landed => Mark::Landed,
            Stage::Dropped => Mark::Dropped,
            Stage::Unknown => Mark::Unknown,
        }
    }
}

/// A card's status on the board, from the store's word or the board's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Status {
    Backlog,
    Todo,
    NeedsDecision,
    InProgress,
    InReview,
    Done,
    #[serde(alias = "canceled")]
    Cancelled,
}

impl Status {
    /// The store's word (`needs_decision`) or the board's ("Needs Decision").
    fn of(word: &str) -> Option<Status> {
        wire(&word.chars().filter(|c| *c != ' ' && *c != '_').collect::<String>().to_lowercase())
    }

    /// In review counts as in progress; waiting on a decision hasn't started.
    fn glyph(self) -> Glyph {
        match self {
            Status::Done => Glyph::Done,
            Status::InProgress | Status::InReview => Glyph::InProgress,
            _ => Glyph::NotStarted,
        }
    }
}

/// Where a landing stands, from the wire's word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LandingPhase {
    WaitingForTickets,
    WaitingForPreviousLanding,
    WaitingForMain,
    Integrating,
    GatingLocally,
    WaitingForCi,
    CiFailed,
    ReadyToLand,
    Landed,
    Stalled,
    Dropped,
    #[serde(other)]
    Unknown,
}

impl LandingPhase {
    fn of(word: &str) -> LandingPhase {
        wire(word).unwrap_or(LandingPhase::Unknown)
    }

    /// The phase inside a sentence: "Landing 86 checking on CI". Every word
    /// maps to a state the runner derives; combining the tickets, the local
    /// gates and the full test run are one step to a person (ov-522).
    pub(super) fn sentence(self) -> &'static str {
        match self {
            LandingPhase::WaitingForTickets => "waiting for tickets",
            LandingPhase::WaitingForPreviousLanding => "waiting for the earlier landing",
            LandingPhase::WaitingForMain => "waiting for main's checks to pass",
            LandingPhase::Integrating | LandingPhase::GatingLocally => "checking here",
            LandingPhase::WaitingForCi => "checking on CI",
            LandingPhase::CiFailed => "CI failed",
            LandingPhase::ReadyToLand => "ready to land",
            LandingPhase::Landed => "landed",
            LandingPhase::Stalled => "stalled",
            LandingPhase::Dropped => "dropped",
            LandingPhase::Unknown => "unknown",
        }
    }

    /// The phase alone, as a label.
    fn word(self, style: Style) -> String {
        style.label(self.sentence())
    }

    fn explanation(self) -> &'static str {
        match self {
            LandingPhase::WaitingForTickets => "Some of its tickets aren't ready to land yet.",
            LandingPhase::WaitingForPreviousLanding => "Ready tickets are collected here while the earlier landing finishes.",
            LandingPhase::WaitingForMain => "Ready tickets are collected here until main's checks pass, then they land.",
            LandingPhase::Integrating | LandingPhase::GatingLocally => "The full test run, on the runner.",
            LandingPhase::WaitingForCi => "Pushed, with CI running its checks.",
            LandingPhase::CiFailed => "A CI check failed, so it needs a fix.",
            LandingPhase::ReadyToLand => "Every check passed, so it's ready to land.",
            LandingPhase::Landed => "Everything passed and it has landed on main.",
            LandingPhase::Stalled => "Nothing here has moved for a while.",
            LandingPhase::Dropped => "It was dropped without landing.",
            LandingPhase::Unknown => "This runner didn't say where it stands.",
        }
    }

    /// How far along, for ordering landings furthest first. A stalled landing
    /// has lost its place, and an unknown one never leads.
    pub(super) fn progress(self) -> u8 {
        match self {
            LandingPhase::Landed | LandingPhase::Dropped => 8,
            LandingPhase::ReadyToLand => 7,
            LandingPhase::WaitingForCi => 6,
            LandingPhase::CiFailed => 5,
            LandingPhase::GatingLocally => 4,
            LandingPhase::Integrating => 3,
            LandingPhase::WaitingForTickets | LandingPhase::WaitingForPreviousLanding | LandingPhase::WaitingForMain => 2,
            LandingPhase::Stalled => 1,
            LandingPhase::Unknown => 0,
        }
    }

    /// The ladder step it stands on, 0 to 2; 3 once everything is done.
    fn step(self) -> u64 {
        match self {
            LandingPhase::WaitingForCi | LandingPhase::CiFailed => 1,
            LandingPhase::ReadyToLand => 2,
            LandingPhase::Landed | LandingPhase::Dropped => 3,
            _ => 0,
        }
    }
}

const LADDER: [(&str, &str); 3] = [
    ("Checking here", "The full test run, on the runner."),
    ("Checking on CI", "Pushed, with CI running its checks."),
    ("Landed", "Everything passed and it has landed on main."),
];

/// What a landing is, where one first appears.
const LANDING_EXPLANATION: &str = "Reviewed tickets land on main together, after one round of checks.";

pub(super) struct Reader<'a> {
    pub(super) plan: &'a Plan,
    pub(super) context: &'a input::Context,
    pub(super) style: Style,
    statuses: BTreeMap<&'a str, Status>,
    runs: Vec<&'a RunCard>,
}

impl<'a> Reader<'a> {
    pub(super) fn stage(&self, line: &Line) -> Stage {
        Stage::of(&line.stage)
    }

    /// The board's status for a card, when the app read one.
    pub(super) fn status_of(&self, task: &str) -> Option<Status> {
        self.statuses.get(task).copied()
    }

    /// What every surface shows first: the ticket's title, else its card's in
    /// the plan, else the board's, else its key.
    pub(super) fn heading(&self, line: &Line) -> String {
        if !line.title.is_empty() {
            return line.title.clone();
        }
        let card = self.plan.cards.iter().find(|c| c.task == line.task).map(|c| c.title.as_str());
        match card.filter(|t| !t.is_empty()).or_else(|| self.context.titles.get(&line.task).map(String::as_str)) {
            Some(title) if !title.is_empty() => title.to_string(),
            _ => line.key.clone(),
        }
    }

    /// A builder has it again, for a fix round: the work open on it is a fix.
    pub(super) fn is_fixing(&self, line: &Line) -> bool {
        self.stage(line) == Stage::Building
            && line.work.iter().rev().find(|w| w.ended_at.is_none()).is_some_and(|w| w.role == "fix")
    }

    /// A live ticket waits on the owner: the runner says so, or its card needs
    /// a decision by the board's own statuses.
    fn waits_on_owner(&self, line: &Line) -> bool {
        let stage = self.stage(line);
        stage.is_live()
            && (stage == Stage::NeedsYou || self.statuses.get(line.task.as_str()) == Some(&Status::NeedsDecision))
    }

    /// The agent that works it: not the orchestrator, working ones first, then
    /// the one that spent the most.
    fn agent_of(&self, line: &Line) -> Option<&'a Agent> {
        let mut mine: Vec<&Agent> = self
            .plan
            .agents
            .iter()
            .filter(|a| a.role != "orchestrator" && a.tickets.iter().any(|t| t.task == line.task))
            .collect();
        mine.sort_by(|a, b| b.working.cmp(&a.working).then(b.spend.total().cmp(&a.spend.total())));
        mine.first().copied()
    }

    fn landed_landing(&self, line: &Line) -> Option<&'a Landing> {
        let number = line.landing?;
        self.plan.landings.iter().find(|l| l.state == "landed" && l.number == number)
    }

    /// The tickets on a landing: those it lists, plus any that name its number.
    pub(super) fn lines_on(&self, landing: &Landing) -> Vec<&'a Line> {
        self.plan
            .lines
            .iter()
            .filter(|l| landing.tickets.iter().any(|t| t.task == l.task) || l.landing == Some(landing.number))
            .collect()
    }

    /// The phase to show: the runner's, else what `state` and the tickets say.
    pub(super) fn phase(&self, landing: &Landing) -> LandingPhase {
        if let Some(phase) = landing.phase.as_deref().map(LandingPhase::of).filter(|p| *p != LandingPhase::Unknown) {
            return phase;
        }
        match landing.state.as_str() {
            "pushed" => LandingPhase::WaitingForCi,
            "red" => LandingPhase::CiFailed,
            "green" => LandingPhase::ReadyToLand,
            "landed" => LandingPhase::Landed,
            "dropped" => LandingPhase::Dropped,
            "gating" => LandingPhase::GatingLocally,
            _ => {
                let blocked = self.lines_on(landing).iter().any(|l| {
                    let s = self.stage(l);
                    s.is_live() && s != Stage::Landing
                });
                if blocked { LandingPhase::WaitingForTickets } else { LandingPhase::Integrating }
            }
        }
    }

    pub(super) fn phase_since(&self, landing: &Landing) -> i64 {
        let known = landing.phase.as_deref().map(LandingPhase::of).is_some_and(|p| p != LandingPhase::Unknown);
        match landing.phase_since {
            Some(since) if known => since,
            _ => landing.state_since,
        }
    }

    pub(super) fn theme_of(&self, task: &str) -> Option<&'a Theme> {
        self.plan.themes.iter().find(|t| t.state != "dropped" && t.cards.iter().any(|c| c.task == task))
    }

    fn run_of(&self, key: &str) -> Option<RunStep> {
        let card = self.runs.iter().find(|r| r.item.eq_ignore_ascii_case(key))?;
        let style = self.style;
        let gate = card.gate.as_ref().map(|g| Gate {
            label: style.from_title(&g.label),
            text: g.text.clone(),
            principal: g.principal.clone(),
        });
        let attention = gate.is_some() || matches!(card.phase, Phase::Failed | Phase::Uncertain);
        Some(RunStep {
            run: card.run.clone(),
            phase: card.phase.clone(),
            phase_label: style.from_title(&card.phase_label),
            step: card.stage.as_ref().map(|s| style.from_title(&s.label)),
            gate,
            attention,
        })
    }

    fn agent_ref(&self, agent: &Agent) -> AgentRef {
        let who = who(agent, self.style);
        let tokens = (agent.spend.total() > 0).then(|| tokens(agent.spend.total()));
        AgentRef {
            id: agent.id.clone(),
            line: [Some(who.clone()), tokens.clone()].into_iter().flatten().collect::<Vec<_>>().join(DOT),
            who,
            tokens,
            terminal: agent.terminal.clone(),
            model: agent.model.clone(),
        }
    }
}

// MARK: Words

fn role_word(role: &str) -> String {
    words::title_case(if matches!(role, "orchestrator" | "builder" | "reviewer" | "lander") { role } else { "agent" })
}

/// "Builder · Sonnet": its role and its model, else its harness.
fn who(agent: &Agent, _style: Style) -> String {
    let model = words::model(&agent.model);
    format!("{}{DOT}{}", role_word(&agent.role), if model.is_empty() { &agent.harness } else { model })
}

/// A pull request stage in the Plan's words, or nil for a kind this build
/// does not know and the runner gave no words for.
fn pr_words(s: &PrStage, style: Style) -> Option<String> {
    Some(match s.kind.as_str() {
        "building" => style.label("building"),
        "agent_review" => style.label("agent review"),
        "fixing" => style.label("fixing"),
        // A login is spelled as GitHub spells it, so the phrase isn't cased.
        "waiting_on_reviewer" => match s.reviewers.as_slice() {
            [] => "Waiting on a reviewer".to_string(),
            [one] => format!("Waiting on {one}"),
            [one, two] => format!("Waiting on {one} and {two}"),
            [one, rest @ ..] => format!("Waiting on {one} and {} others", rest.len()),
        },
        "changes_requested" => style.label("changes requested"),
        "approved_checks_running" => style.label("approved, checks running"),
        "approved_checks_failing" => style.label("checks failing"),
        "approved" => style.label("approved"),
        "approved_conflicts" => style.label("merge conflicts"),
        "queued" if s.queue_position > 0 => format!("{} {}", style.label("queued, position"), s.queue_position),
        "queued" => style.label("queued"),
        "merged" => style.label("merged"),
        "closed" => style.label("closed"),
        "unknown" => style.label("PR state unknown"),
        _ if !s.label.is_empty() => s.label.clone(),
        _ => return None,
    })
}

/// What stays in the gray state line when the words lead with the problem.
fn pr_state_words(s: &PrStage, style: Style) -> Option<String> {
    matches!(s.kind.as_str(), "approved_checks_failing" | "approved_conflicts").then(|| style.label("approved"))
}

/// The stage in one phrase, state first: "Approved, Checks Failing".
fn pr_full_words(s: &PrStage, style: Style) -> Option<String> {
    let words = pr_words(s, style)?;
    Some(match pr_state_words(s, style) {
        // Sentence case reads "Approved, checks failing": the problem follows the state mid-phrase.
        Some(state) if style == Style::Material => {
            let mut chars = words.chars();
            let lowered = chars.next().map(|c| c.to_lowercase().chain(chars).collect::<String>()).unwrap_or_default();
            format!("{state}, {lowered}")
        }
        Some(state) => format!("{state}, {words}"),
        None => words,
    })
}

fn pr_needs_attention(s: &PrStage) -> bool {
    matches!(s.kind.as_str(), "changes_requested" | "approved_checks_failing" | "approved_conflicts")
}

/// "PR 31 · 2 threads open".
fn pr_detail(s: &PrStage) -> Option<String> {
    if s.pr_number == 0 {
        return None;
    }
    let mut parts = vec![format!("PR {}", s.pr_number)];
    match s.unresolved_threads {
        Some(1) => parts.push("1 thread open".into()),
        Some(n) if n > 1 => parts.push(format!("{n} threads open")),
        _ => {}
    }
    Some(parts.join(DOT))
}

impl Reader<'_> {
    /// The pull request stage a row shows: only while the ticket is live and
    /// the stage has something to add to the stage word.
    fn shown_stage<'l>(&self, line: &'l Line) -> Option<&'l PrStage> {
        let s = line.pr_stage.as_ref()?;
        (self.stage(line).is_live() && s.kind != "building" && pr_words(s, self.style).is_some()).then_some(s)
    }

    /// "Fixing · round 1", "Landing · in Landing 8", "Waiting · 2nd",
    /// "Landed · ac840108". With `with_stage` false, the pull request's stage
    /// is left to a line of its own and only its state word is kept.
    fn status(&self, line: &Line, with_stage: bool) -> String {
        let style = self.style;
        let stage = self.stage(line);
        let fixing = self.is_fixing(line);
        let mut parts = vec![if fixing { style.label("fixing") } else { stage.word(style) }];
        if let Some(shown) = self.shown_stage(line)
            && let Some(words) = pr_full_words(shown, style)
        {
            if !with_stage {
                if let Some(kept) = pr_state_words(shown, style) {
                    parts[0] = kept;
                }
            } else if shown.kind == "unknown" {
                parts.push(words);
            } else {
                parts[0] = words;
            }
        }
        if fixing && line.fix_rounds > 0 {
            parts.push(format!("round {}", line.fix_rounds));
        }
        if stage == Stage::Waiting
            && let Some(rank) = line.plan_rank
        {
            parts.push(words::ordinal(rank));
        }
        if stage == Stage::Landed
            && let Some(sha) = line.landed_sha.as_deref().filter(|s| !s.is_empty())
        {
            parts.push(sha.chars().take(8).collect());
        }
        if stage.is_live()
            && let Some(landing) = line.landing
        {
            parts.push(format!("in Landing {landing}"));
        }
        parts.join(DOT)
    }

    /// A stale ticket's warning: "No move in 2 h".
    pub(super) fn stale(&self, line: &Line) -> Option<String> {
        if !line.stale {
            return None;
        }
        let minutes = (self.plan.now_ms - line.stage_since).max(0) / 60_000;
        Some(if minutes < 120 { "No move in an hour".into() } else { format!("No move in {} h", minutes / 60) })
    }

    fn over_budget(&self, line: &Line) -> Option<(String, String)> {
        let budget = line.budget_tokens?;
        let used = line.spend.total();
        (used > budget).then(|| {
            (
                format!("Over budget: {} of {} tokens", tokens(used), tokens(budget)),
                format!("Over budget. {} of {} tokens used.", words::spoken_tokens(used), words::spoken_tokens(budget)),
            )
        })
    }

    fn card(&self, line: &Line, rank: Option<u64>, agent: Option<&Agent>, landed: Option<String>) -> TicketCard {
        let style = self.style;
        let now = self.plan.now_ms;
        let waits = self.waits_on_owner(line);
        let warning = if waits { Some("Needs you".to_string()) } else { self.stale(line) };
        let over = self.over_budget(line);
        let shown = self.shown_stage(line);
        let run = self.run_of(&line.key);
        let heading = self.heading(line);

        let second = if rank.is_some() {
            if line.reason.is_empty() { line.key.clone() } else { line.reason.clone() }
        } else {
            let mut parts = vec![if shown.is_some() { self.status(line, false) } else { self.status(line, true) }];
            if let Some(landed) = &landed
                && !parts[0].contains(landed.as_str())
            {
                parts.push(format!("{landed} landed"));
            }
            parts.push(line.key.clone());
            // Android draws the agent as a line of its own and no time in
            // stage; the Apple apps and the web say both in this one line.
            let material = style == Style::Material;
            if let Some(agent) = agent.filter(|_| !material) {
                parts.push(self.agent_ref(agent).line);
            }
            if !material && warning.is_none() && line.stage_since > 0 && now > line.stage_since {
                parts.push(words::span(now - line.stage_since));
            }
            parts.retain(|p| !p.is_empty());
            parts.join(DOT)
        };

        let mut notes = Vec::new();
        if let Some(stage) = shown
            && let Some(words) = pr_words(stage, style)
        {
            let attention = pr_needs_attention(stage);
            let full = pr_full_words(stage, style);
            notes.push(Note {
                kind: NoteKind::Stage,
                text: if attention { words } else { full.clone().unwrap_or(words) },
                attention,
                help: Some([full, pr_detail(stage)].into_iter().flatten().collect::<Vec<_>>().join(DOT)),
            });
        }
        if let Some(run) = &run {
            notes.push(run_note(run));
        }
        if let Some(warning) = &warning {
            notes.push(Note { kind: NoteKind::Warning, text: warning.clone(), attention: true, help: None });
        }
        if let Some((text, _)) = &over {
            notes.push(Note { kind: NoteKind::OverBudget, text: text.clone(), attention: true, help: None });
        }

        let theme = self.theme_of(&line.task).map(|t| t.name.clone());
        let mut spoken: Vec<String> = Vec::new();
        if let Some(rank) = rank {
            spoken.push(format!("{} up", words::ordinal(rank)));
        }
        spoken.push(heading.clone());
        spoken.extend(theme.clone());
        spoken.push(second.clone());
        spoken.extend(shown.and_then(|s| pr_full_words(s, style)));
        spoken.extend(run.as_ref().map(|r| run_note(r).text));
        spoken.extend(warning.clone());
        spoken.extend(over.map(|(_, spoken)| spoken));
        let accessibility =
            words::accessible_model(&spoken.join(", "), agent.map_or("", |a| a.model.as_str()));

        let stage = self.stage(line);
        TicketCard {
            id: line.task.clone(),
            key: line.key.clone(),
            heading,
            theme,
            stage: line.stage.clone(),
            status: self.status(line, true),
            second,
            rank,
            mark: if rank.is_some() {
                Mark::Rank
            } else if self.is_fixing(line) {
                Mark::Fixing
            } else {
                stage.mark()
            },
            mark_attention: notes.iter().any(|n| n.attention),
            notes,
            waits_on_owner: waits,
            landed_landing: landed,
            agent: agent.map(|a| self.agent_ref(a)),
            run,
            branch: line.branch.clone(),
            note: None,
            accessibility,
        }
    }
}

impl Reader<'_> {
    /// A ticket on a live landing: its card, with the lines a landing says
    /// differently. Waiting on the owner is its one warning, so it leads its
    /// notes and is not also "stale"; "Ready" says nothing, so only a ticket
    /// that waits and one that has landed carry a `note`; a ticket on a
    /// landing shows no stage, run or time in stage (ov-522).
    fn landing_ticket(&self, line: &Line) -> TicketCard {
        let mut card = self.card(line, None, self.agent_of(line), None);
        let waits = card.waits_on_owner;
        let landed = self.stage(line) == Stage::Landed;
        card.note = if waits {
            Some("Needs you".to_string())
        } else if landed {
            Some(card.status.clone())
        } else {
            None
        };
        let over = self.over_budget(line);
        let stale = if waits { None } else { self.stale(line) };
        let note = |kind, text: &String| Note { kind, text: text.clone(), attention: true, help: None };
        card.notes = card.note.iter().filter(|_| waits).map(|text| note(NoteKind::Warning, text))
            .chain(stale.iter().map(|text| note(NoteKind::Warning, text)))
            .chain(over.iter().map(|(text, _)| note(NoteKind::OverBudget, text)))
            .collect();
        card.mark_attention = !card.notes.is_empty();
        let mut second: Vec<String> = card.note.iter().filter(|_| !waits).cloned().collect();
        second.push(line.key.clone());
        if !waits && stale.is_none() && line.stage_since > 0 && self.plan.now_ms > line.stage_since {
            second.push(words::span(self.plan.now_ms - line.stage_since));
        }
        card.second = second.join(DOT);
        card.accessibility = [Some(card.heading.clone()), card.note.clone(), stale, over.map(|(_, spoken)| spoken)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        card
    }
}

/// "Build · Running", "Review · Review Required": the step and what it waits on.
fn run_note(run: &RunStep) -> Note {
    let waits = run.gate.as_ref().map_or(run.phase_label.as_str(), |g| g.label.as_str());
    let text = match &run.step {
        Some(step) => format!("{step}{DOT}{waits}"),
        None => waits.to_string(),
    };
    Note { kind: NoteKind::Run, text, attention: run.attention, help: run.gate.as_ref().map(|g| g.text.clone()) }
}

// MARK: Agents

fn agent_row(agent: &Agent, now: i64, style: Style) -> AgentRow {
    let works_for = if agent.role == "orchestrator" {
        "Plan".to_string()
    } else if !agent.tickets.is_empty() {
        agent.tickets.iter().map(|t| t.key.as_str()).collect::<Vec<_>>().join(", ")
    } else {
        "Not on a ticket".to_string()
    };
    let state = if agent.working {
        format!("Working{DOT}{}", words::span(now - agent.started_at))
    } else if agent.last_at > 0 {
        format!("Idle{DOT}active {}", words::ago(agent.last_at, now))
    } else {
        "Idle".to_string()
    };
    let mut second = Vec::new();
    if !agent.title.is_empty() {
        second.push(format!("\u{201C}{}\u{201D}", agent.title));
    }
    second.push(works_for.clone());
    second.push(state.clone());
    let who = who(agent, style);
    let tokens = (agent.spend.total() > 0).then(|| tokens(agent.spend.total()));
    let cost = agent.spend.cost_micros.map(|m| format!("\u{2248}{}", dollars(m)));
    let accessibility = words::accessible_model(
        &[Some(who.clone()), Some(second.join(", ")), tokens.as_ref().map(|t| format!("{t} tokens"))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", "),
        &agent.model,
    );
    AgentRow {
        id: agent.id.clone(),
        who,
        working: agent.working,
        works_for,
        state,
        second: second.join(DOT),
        doing_now: agent.doing_now.clone(),
        tokens,
        cost,
        terminal: agent.terminal.clone(),
        accessibility,
    }
}

fn agents(plan: &Plan, style: Style) -> Agents {
    let mut list: Vec<&Agent> = plan.agents.iter().collect();
    list.sort_by(|a, b| b.working.cmp(&a.working).then(b.spend.total().cmp(&a.spend.total())));
    let total = plan.agents_spend.as_ref().map_or_else(|| list.iter().map(|a| a.spend.total()).sum(), |s| s.total());
    // Dollars only when every agent is priced: a sum with a hole in it would read low.
    let cost = match &plan.agents_spend {
        Some(spend) => spend.cost_micros,
        None => list.iter().try_fold(0i64, |sum, a| a.spend.cost_micros.map(|m| sum + m)),
    };
    let header = if list.is_empty() && total == 0 {
        "No agents running.".to_string()
    } else {
        let mut parts = if list.is_empty() {
            vec!["No agents running".to_string()]
        } else {
            let noun = if list.len() == 1 { "1 agent".to_string() } else { format!("{} agents", list.len()) };
            // The Agents card beside the conversation counts without the
            // orchestrator, so a list that includes it says so (ov-660).
            let noun = if list.iter().any(|a| a.role == "orchestrator") {
                format!("{noun}, including the orchestrator")
            } else {
                noun
            };
            vec![noun, format!("{} working", list.iter().filter(|a| a.working).count())]
        };
        parts.push(format!("{} tokens in the last 24 hours", tokens(total)));
        if let Some(micros) = cost {
            parts.push(format!("\u{2248}{}", dollars(micros)));
        }
        parts.join(DOT)
    };
    Agents { header, rows: list.into_iter().map(|a| agent_row(a, plan.now_ms, style)).collect() }
}

// MARK: Themes

fn themes(r: &Reader, moving: &BTreeSet<&str>) -> Themes {
    let plan = r.plan;
    struct Tile {
        tile: ThemeTile,
        finished: bool,
        last_moved: i64,
        ordinal: i64,
    }
    let all: Vec<Tile> = plan
        .themes
        .iter()
        .filter(|t| t.state != "dropped")
        .map(|theme| {
            let total = theme.counts.total();
            let done = theme.counts.done;
            let tasks: BTreeSet<&str> = theme.cards.iter().map(|c| c.task.as_str()).collect();
            let mut last_moved = theme.story_at.max(theme.last_moved_at.unwrap_or(0));
            for task in &tasks {
                last_moved = last_moved.max(r.context.activity.get(*task).copied().unwrap_or(0));
            }
            for line in plan.lines.iter().filter(|l| l.stage != "dropped" && tasks.contains(l.task.as_str())) {
                last_moved = last_moved.max(line.stage_since);
            }
            for ruling in plan.rulings.iter().filter(|x| x.theme_id.as_deref() == Some(theme.id.as_str())) {
                last_moved = last_moved.max(ruling.created_at).max(ruling.settled_at.unwrap_or(0));
            }
            let is_moving = plan.lines.iter().any(|l| tasks.contains(l.task.as_str()) && moving.contains(l.task.as_str()));

            // The board's status wins; else the plan's own word for it. A card
            // the plan can't place, or one that was canceled, isn't shown.
            let listed: Vec<(TileCard, i64)> = theme
                .cards
                .iter()
                .filter_map(|c| {
                    let known = plan.cards.iter().find(|k| k.task == c.task);
                    let status = r.statuses.get(c.task.as_str()).copied().or_else(|| known.and_then(|k| Status::of(&k.status)))?;
                    (status != Status::Cancelled).then(|| {
                        let card = TileCard {
                            id: c.task.clone(),
                            key: c.key.clone(),
                            title: known.map(|k| k.title.clone()).unwrap_or_default(),
                            glyph: status.glyph(),
                        };
                        (card, r.context.activity.get(c.task.as_str()).copied().unwrap_or(0))
                    })
                })
                .collect();
            const SHOWN: usize = 3;
            let not_started: Vec<TileCard> =
                listed.iter().find(|(c, _)| c.glyph == Glyph::NotStarted).map(|(c, _)| c.clone()).into_iter().collect();
            let in_progress: Vec<TileCard> = listed
                .iter()
                .filter(|(c, _)| c.glyph == Glyph::InProgress)
                .take(SHOWN - not_started.len())
                .map(|(c, _)| c.clone())
                .collect();
            let room = SHOWN - not_started.len() - in_progress.len();
            // Later in a theme's list is later added; the board's word on
            // when it moved breaks that.
            let mut finished: Vec<(usize, &(TileCard, i64))> =
                listed.iter().enumerate().filter(|(_, (c, _))| c.glyph == Glyph::Done).collect();
            finished.sort_by(|(ia, (_, a)), (ib, (_, b))| (b, ib).cmp(&(a, ia)));
            let cards = in_progress
                .into_iter()
                .chain(finished.into_iter().take(room).map(|(_, (c, _))| c.clone()))
                .chain(not_started)
                .collect();
            Tile {
                tile: ThemeTile {
                    id: theme.id.clone(),
                    name: theme.name.clone(),
                    done,
                    total,
                    progress: format!("{done} of {total} done"),
                    fraction: if total > 0 { done as f64 / total as f64 } else { 0.0 },
                    cards,
                    owner_ask: (!theme.owner_ask.is_empty()).then(|| theme.owner_ask.clone()),
                    moving: is_moving,
                },
                finished: theme.state == "done" || (total > 0 && done == total),
                last_moved,
                ordinal: theme.ordinal,
            }
        })
        .collect();
    let (mut finished, mut tiles): (Vec<Tile>, Vec<Tile>) = all.into_iter().partition(|t| t.finished);
    tiles.sort_by(|a, b| {
        b.tile.moving.cmp(&a.tile.moving).then(b.last_moved.cmp(&a.last_moved)).then(a.ordinal.cmp(&b.ordinal))
    });
    finished.sort_by_key(|t| std::cmp::Reverse(t.last_moved));
    let count = finished.len();
    Themes {
        tiles: tiles.into_iter().map(|t| t.tile).collect(),
        finished: finished.into_iter().map(|t| t.tile).collect(),
        finished_title: (count > 0).then(|| r.style.label("finished")),
        finished_words: (count > 0).then(|| if count == 1 { "1 theme".to_string() } else { format!("{count} themes") }),
    }
}

// MARK: The view

pub(super) fn build(plan: &Plan, context: &input::Context, workflow: Option<&Presentation>) -> PlanView {
    let style = context.style;
    let r = Reader {
        plan,
        context,
        style,
        statuses: context.statuses.iter().filter_map(|(task, word)| Some((task.as_str(), Status::of(word)?))).collect(),
        runs: workflow.map(|w| w.runs.iter().collect()).unwrap_or_default(),
    };
    let row = |line: &Line, rank: Option<u64>| {
        r.card(line, rank, r.agent_of(line), r.landed_landing(line).map(|l| format!("Landing {}", l.number)))
    };

    // Landings, furthest along first; the plan's order breaks a tie.
    let mut live: Vec<&Landing> = plan.landings.iter().filter(|l| l.state != "landed" && l.state != "dropped").collect();
    live.sort_by_key(|l| std::cmp::Reverse(r.phase(l).progress()));
    let mut on_live: BTreeSet<&str> = BTreeSet::new();
    let landings: Vec<LandingCard> = live
        .into_iter()
        .map(|landing| {
            let phase = r.phase(landing);
            let mine = r.lines_on(landing);
            on_live.extend(mine.iter().map(|l| l.task.as_str()));
            let label = format!("Landing {}", landing.number);
            let mut subtitle = landing.title.as_str();
            if let Some(rest) = subtitle.strip_prefix(&format!("{label}{DOT}")) {
                subtitle = rest;
            }
            let lander = landing.lander.as_ref().and_then(|id| plan.agents.iter().find(|a| &a.id == id));
            let at = phase.step();
            let word = phase.word(style);
            LandingCard {
                id: landing.id.clone(),
                number: landing.number,
                subtitle: (!subtitle.is_empty() && subtitle != label).then(|| subtitle.to_string()),
                label,
                carries: (!landing.summary.is_empty()).then(|| landing.summary.clone()),
                phase,
                phase_word: word.clone(),
                phase_sentence: phase.sentence().to_string(),
                explanation: phase.explanation().to_string(),
                attention: matches!(phase, LandingPhase::CiFailed | LandingPhase::Stalled),
                since: r.phase_since(landing),
                ladder: Ladder {
                    steps: LADDER
                        .iter()
                        .enumerate()
                        .map(|(i, (label, explanation))| LadderStep {
                            label: if i as u64 == at { word.clone() } else { style.label(label) },
                            explanation: if i as u64 == at { phase.explanation() } else { explanation }.to_string(),
                        })
                        .collect(),
                    at,
                    compact: if at as usize >= LADDER.len() {
                        word.clone()
                    } else {
                        format!("{word}{DOT}step {} of {}", at + 1, LADDER.len())
                    },
                },
                lander: lander.map(|a| r.agent_ref(a)),
                lander_words: lander.map(|a| {
                    let mut parts = vec!["Lander".to_string()];
                    let model = words::model(&a.model);
                    if !model.is_empty() {
                        parts.push(model.to_string());
                    }
                    if a.spend.total() > 0 {
                        parts.push(tokens(a.spend.total()));
                    } else if !a.working {
                        parts.push("done".into());
                    }
                    parts.join(DOT)
                }),
                pushed_sha: landing.pushed_sha.clone(),
                ci_unread: landing.pushed_sha.as_ref().map(|_| "CI not read yet".to_string()),
                tickets: mine.into_iter().map(|line| r.landing_ticket(line)).collect(),
            }
        })
        .collect();

    let free: Vec<&Line> =
        plan.lines.iter().filter(|l| r.stage(l).is_live() && !on_live.contains(l.task.as_str())).collect();
    // Only waiting and landing tickets go to the folded section; the rest route
    // by their stage and say which landing they name.
    let (parked, loose): (Vec<&Line>, Vec<&Line>) = free.into_iter().partition(|l| {
        matches!(r.stage(l), Stage::Waiting | Stage::Landing) && r.landed_landing(l).is_some()
    });
    // A fix round follows a review, so a ticket in one is In Review still.
    let in_review: Vec<TicketCard> = loose
        .iter()
        .filter(|l| matches!(r.stage(l), Stage::Reviewing | Stage::NeedsYou | Stage::Landing) || r.is_fixing(l))
        .map(|l| row(l, None))
        .collect();
    let building: Vec<TicketCard> =
        loose.iter().filter(|l| r.stage(l) == Stage::Building && !r.is_fixing(l)).map(|l| row(l, None)).collect();
    let waiting_lines: Vec<&&Line> = loose.iter().filter(|l| r.stage(l) == Stage::Waiting).collect();
    let mut ranked: Vec<&&Line> =
        waiting_lines.iter().copied().filter(|l| l.plan_rank.is_some() && !r.waits_on_owner(l)).collect();
    ranked.sort_by_key(|l| l.plan_rank.unwrap_or(u64::MAX));
    let not_ranked: Vec<TicketCard> = waiting_lines
        .iter()
        .filter(|l| l.plan_rank.is_none() && !r.waits_on_owner(l))
        .map(|l| row(l, None))
        .collect();
    let on_you: Vec<TicketCard> = waiting_lines.iter().filter(|l| r.waits_on_owner(l)).map(|l| row(l, None)).collect();
    let next: Vec<TicketCard> = ranked.iter().enumerate().map(|(i, l)| row(l, Some(i as u64 + 1))).collect();
    let next_count = next.len();
    let waiting: Vec<WaitingGroup> = [
        (WaitingKind::Next, "next up", false, next),
        (WaitingKind::NotRanked, "not ranked", false, not_ranked),
        (WaitingKind::OnYou, "waiting on you", true, on_you),
    ]
    .into_iter()
    .filter(|(_, _, _, cards)| !cards.is_empty())
    .map(|(kind, title, attention, cards)| WaitingGroup { kind, title: style.label(title), attention, cards })
    .collect();
    let on_landed_landing: Vec<TicketCard> = parked.iter().map(|l| row(l, None)).collect();

    // Landed on the viewer's day, newest first.
    let day = |ms: i64| (ms / 1000 + context.utc_offset).div_euclid(86_400);
    let mut landed: Vec<&Line> =
        plan.lines.iter().filter(|l| r.stage(l) == Stage::Landed && day(l.stage_since) == day(plan.now_ms)).collect();
    landed.sort_by_key(|l| std::cmp::Reverse(l.stage_since));
    let landed_today: Vec<TicketCard> = landed.into_iter().map(|l| row(l, None)).collect();

    let agents = agents(plan, style);
    let moving: BTreeSet<&str> = landings
        .iter()
        .flat_map(|l| l.tickets.iter().map(|t| t.id.as_str()))
        .chain(in_review.iter().chain(&building).map(|c| c.id.as_str()))
        .collect();
    let themes = themes(&r, &moving);

    // Needs You, In Review and Themes are drawn only with rows; Landing,
    // Waiting and Agents always are, and say so when empty; Building also when
    // something is queued, since the gap between Next and Landing is then
    // worth saying. Agents is left out for a runner that lists none.
    let waiting_count: usize = waiting.iter().map(|g| g.cards.len()).sum();
    let kinds: [(SectionKind, &str, bool, usize, Option<String>); 9] = [
        (SectionKind::NeedsYou, "needs you", context.needs_you > 0, context.needs_you as usize, None),
        (SectionKind::Landing, "landing", true, landings.len(), Some("Nothing is landing right now.".into())),
        (SectionKind::InReview, "in review", !in_review.is_empty(), in_review.len(), None),
        (
            SectionKind::Building,
            "building",
            !building.is_empty() || next_count > 0,
            building.len(),
            Some("Nothing is building.".into()),
        ),
        (SectionKind::Waiting, "waiting", true, waiting_count, Some("Nothing is waiting.".into())),
        (SectionKind::OnLandedLanding, "waiting for the next landing", !on_landed_landing.is_empty(), on_landed_landing.len(), None),
        // Agents says what the day spent when it spent something.
        (SectionKind::Agents, "agents", context.agents_supported, agents.rows.len(), Some(agents.header.clone())),
        (SectionKind::Themes, "themes", !themes.tiles.is_empty() || !themes.finished.is_empty(), themes.tiles.len() + themes.finished.len(), None),
        (SectionKind::LandedToday, "landed today", !landed_today.is_empty(), landed_today.len(), None),
    ];
    let sections = kinds
        .into_iter()
        .filter(|(_, _, shown, _, _)| *shown)
        .map(|(kind, title, _, count, empty)| Section {
            kind,
            title: style.label(title),
            count: count as u64,
            empty: if count == 0 { empty } else { None },
            folded: matches!(kind, SectionKind::OnLandedLanding | SectionKind::LandedToday),
            help: (kind == SectionKind::Landing).then(|| LANDING_EXPLANATION.to_string()),
        })
        .collect();

    let column = super::column::column(&r);
    let pages = pages::pages(&r);
    PlanView {
        schema: SCHEMA,
        now: plan.now_ms,
        sections,
        needs_you: context.needs_you,
        landings,
        in_review,
        building,
        waiting,
        on_landed_landing,
        landed_today,
        agents,
        themes,
        column,
        pages,
    }
}
