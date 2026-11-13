//! The plan column (ov-867, C10f), as AgentKit's `PlanColumn.swift` and the
//! web's `plan-column.ts` each had it (ov-1016): what the plan side beside the
//! conversation says. The mental model is `.claude/agent/reports/ov-867/intent.md`:
//!
//! 1. the facts line: what needs you, what's stuck, the next landing, and how
//!    much of the board is done;
//! 2. Initiatives (today's themes): the larger goals, each with its goal,
//!    progress and next step;
//! 3. a unique agent roster with runner-reported activity, then tickets grouped
//!    by their own stage, with assignment and stage age as ticket context;
//! 4. the footer counts, which open the ticket lists.
//!
//! Needs You sits between the facts and Initiatives, but its rows are the
//! runner's own list, which the plan doesn't carry, so the apps add them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use super::build::{LandingPhase, Reader, Stage, Status};
use super::input::Line;
use super::words;

/// One run of a fact's text: the number is strong.
#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub text: String,
    pub strong: bool,
}

#[typeshare]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    NeedsYou,
    Stuck,
    NextLanding,
    Done,
}

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub kind: FactKind,
    pub runs: Vec<Run>,
    /// Drawn in amber: it needs the owner, or something is stuck.
    pub attention: bool,
    /// The runs as one string, for a label.
    pub text: String,
}

#[typeshare]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Initiative {
    /// The theme's id: an initiative is today's theme.
    pub id: String,
    pub name: String,
    /// The outcome the theme was written for; empty when it has none.
    pub goal: String,
    #[typeshare(serialized_as = "I54")]
    pub done: u64,
    #[typeshare(serialized_as = "I54")]
    pub total: u64,
    /// "20 of 29".
    pub progress: String,
    /// How full its meter is, 0 to 1.
    pub fraction: f64,
    /// "Next: …", or "Waiting on you: …" (`asks_owner`).
    pub next: Option<String>,
    /// `next` is the owner's to act on: drawn in amber.
    pub asks_owner: bool,
}

/// A ticket's canonical stage; `Stuck` is the legacy wire name for Needs You.
#[typeshare]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipeStage {
    Stuck,
    Building,
    Reviewing,
    Landing,
}

impl PipeStage {
    const ALL: [PipeStage; 4] = [PipeStage::Stuck, PipeStage::Building, PipeStage::Reviewing, PipeStage::Landing];

    fn title(self) -> &'static str {
        match self {
            PipeStage::Stuck => "Needs You",
            PipeStage::Building => "Building",
            PipeStage::Reviewing => "Reviewing",
            PipeStage::Landing => "Landing",
        }
    }
}

/// What an agent does; the apps draw its glyph.
#[typeshare]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Builder,
    Reviewer,
    Lander,
    #[serde(other)]
    Agent,
}

impl Role {
    fn of(wire: &str) -> Role {
        super::build::wire(wire).unwrap_or(Role::Agent)
    }

    /// "Builder": the variant's own name.
    fn word(self) -> String {
        format!("{self:?}")
    }
}

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnTicket {
    pub task: String,
    pub key: String,
    /// Its title, else the board's, else its key.
    pub title: String,
    /// The initiative it is for: the ticket's own.
    pub initiative: Option<String>,
    pub initiative_id: Option<String>,
    pub stage: PipeStage,
    /// When it entered its stage, in ms; 0 when nothing says.
    #[typeshare(serialized_as = "I54")]
    pub since: i64,
    /// Assignment and stage age are ticket context, never execution or liveness.
    pub detail: Option<String>,
}

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnAgent {
    pub id: String,
    pub role: Role,
    /// The orchestrator's title, kept separate from current activity.
    pub title: String,
    /// "Builder 2": its role, numbered when there are several.
    pub name: String,
    /// Runner-reported activity, with a qualified fallback when no words were sent.
    pub doing: String,
    /// Reserved for agent attention with provenance; ticket age cannot set it.
    pub doing_is_trouble: bool,
    /// Time since the agent started, not time executing an assigned ticket.
    #[typeshare(serialized_as = "Option<I54>")]
    pub elapsed: Option<i64>,
    /// "47 min".
    pub time: Option<String>,
    /// Its pane, to open its chat.
    pub terminal: Option<String>,
    /// The first ticket it works, for its page when there's no pane to open.
    pub ticket: Option<String>,
    pub tickets: Vec<ColumnTicket>,
    pub assignment_words: String,
    pub working: bool,
    /// Legacy decoder slot, always zero: section membership never belongs to an agent.
    #[typeshare(serialized_as = "I54")]
    pub more: u64,
}

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub stage: PipeStage,
    pub title: String,
    /// Legacy decoder slot, empty; the column's roster owns agent rows.
    pub agents: Vec<ColumnAgent>,
    /// Assigned tickets in their own stage, each once, outside the agent roster.
    pub assigned: Vec<ColumnTicket>,
    /// Tickets in this stage with no known open assignment in either source list.
    pub unattended: Vec<ColumnTicket>,
    /// The tickets in this stage, each once.
    #[typeshare(serialized_as = "I54")]
    pub count: u64,
    pub count_words: String,
    /// The first few unattended tickets, independent of the agent roster.
    pub preview: Vec<ColumnTicket>,
    /// The unattended tickets only the list shows.
    #[typeshare(serialized_as = "I54")]
    pub hidden: u64,
    /// "12 more waiting for review".
    pub unattended_words: Option<String>,
}

#[typeshare]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountKind {
    Waiting,
    NotStarted,
    Done,
}

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Count {
    pub kind: CountKind,
    #[typeshare(serialized_as = "I54")]
    pub count: u64,
    /// The board's list it opens, as the store spells a status.
    pub status: String,
    /// "10 tickets waiting for a builder", "22 not started", "796 done".
    pub words: String,
}

/// A ticket waiting for a builder, as the footer's list draws it.
#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingTicket {
    pub task: String,
    pub key: String,
    pub title: String,
    #[typeshare(serialized_as = "I54")]
    pub since: i64,
}

#[typeshare]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub facts: Vec<Fact>,
    /// In motion or waiting on the owner, in the board's order.
    pub initiatives: Vec<Initiative>,
    /// The rest that are open, said on one quiet line.
    pub quiet_initiatives: Vec<Initiative>,
    /// "Nothing moving: Mac look and feel, 33 of 34; Marketing, 6 of 8".
    pub quiet_line: Option<String>,
    #[typeshare(serialized_as = "I54")]
    pub initiative_count: u64,
    /// The stages with anything in them, in order.
    pub pipeline: Vec<Group>,
    /// Each listed non-orchestrator agent once, including agents with no eligible tickets.
    pub roster: Vec<ColumnAgent>,
    /// "10 agents".
    pub agent_words: String,
    pub footer: Vec<Count>,
    /// The tickets waiting for a builder, each once, in the plan's order.
    pub waiting: Vec<WaitingTicket>,
}

const PREVIEW: usize = 3;

pub(super) fn column(r: &Reader) -> Column {
    let plan = r.plan;
    let (pipeline, roster) = pipeline(r);
    let moving: BTreeSet<&str> = pipeline
        .iter()
        .flat_map(|g| g.assigned.iter().chain(&g.unattended))
        .map(|t| t.task.as_str())
        .collect();
    let (initiatives, quiet) = initiatives(r, &moving);
    let waiting = waiting(r);
    let footer = footer(r, &waiting);
    let facts = facts(r, &pipeline);
    let agents = roster.len();
    let _ = plan;
    Column {
        quiet_line: (!quiet.is_empty()).then(|| {
            format!(
                "Nothing moving: {}",
                quiet.iter().map(|i| format!("{}, {}", i.name, i.progress)).collect::<Vec<_>>().join("; ")
            )
        }),
        initiative_count: (initiatives.len() + quiet.len()) as u64,
        initiatives,
        quiet_initiatives: quiet,
        agent_words: if agents == 1 { "1 agent".into() } else { format!("{agents} agents") },
        pipeline,
        roster,
        footer,
        waiting,
        facts,
    }
}

fn initiatives(r: &Reader, moving: &BTreeSet<&str>) -> (Vec<Initiative>, Vec<Initiative>) {
    let rank = |state: &str| match state {
        "active" => 0,
        "paused" => 1,
        "done" => 2,
        _ => 3,
    };
    let mut themes: Vec<_> = r.plan.themes.iter().filter(|t| t.state != "dropped").collect();
    themes.sort_by(|a, b| rank(&a.state).cmp(&rank(&b.state)).then(a.ordinal.cmp(&b.ordinal)));
    let (mut shown, mut quiet) = (Vec::new(), Vec::new());
    for theme in themes.into_iter().filter(|t| t.state != "done") {
        let total = theme.counts.total();
        if total > 0 && theme.counts.done >= total {
            continue;
        }
        let ask = theme.owner_ask.trim();
        let next = theme.next.trim();
        let initiative = Initiative {
            id: theme.id.clone(),
            name: theme.name.clone(),
            goal: theme.outcome.trim().to_string(),
            done: theme.counts.done,
            total,
            progress: format!("{} of {total}", theme.counts.done),
            fraction: if total > 0 { (theme.counts.done as f64 / total as f64).min(1.0) } else { 0.0 },
            next: if !ask.is_empty() {
                Some(format!("Waiting on you: {ask}"))
            } else {
                (!next.is_empty()).then(|| format!("Next: {next}"))
            },
            asks_owner: !ask.is_empty(),
        };
        let in_motion = theme.cards.iter().any(|c| moving.contains(c.task.as_str()));
        // Paused themes are open but not in motion: they go to the quiet line.
        if (in_motion && theme.state == "active") || !ask.is_empty() {
            shown.push(initiative);
        } else {
            quiet.push(initiative);
        }
    }
    (shown, quiet)
}

/// Tickets waiting for a builder, each once, a cancelled one left out.
fn waiting(r: &Reader) -> Vec<WaitingTicket> {
    let mut seen = BTreeSet::new();
    r.plan
        .lines
        .iter()
        .filter(|l| r.stage(l) == Stage::Waiting)
        .filter(|l| r.status_of(&l.task) != Some(Status::Cancelled) && seen.insert(l.task.as_str()))
        .map(|l| WaitingTicket { task: l.task.clone(), key: l.key.clone(), title: r.heading(l), since: l.stage_since })
        .collect()
}

fn footer(r: &Reader, waiting: &[WaitingTicket]) -> Vec<Count> {
    let mut counts = Vec::new();
    let n = waiting.len() as u64;
    if n > 0 {
        let lead = if n == 1 { "1 ticket".to_string() } else { format!("{n} tickets") };
        counts.push(Count {
            kind: CountKind::Waiting,
            count: n,
            status: "todo".into(),
            words: format!("{lead} waiting for a builder"),
        });
    }
    if let Some(c) = &r.plan.board_counts {
        // Not started, less the ones already counted as waiting.
        let unstarted = waiting
            .iter()
            .filter(|w| matches!(r.status_of(&w.task).unwrap_or(Status::Todo), Status::Backlog | Status::Todo))
            .count() as u64;
        let not_started = (c.backlog + c.todo).saturating_sub(unstarted);
        if not_started > 0 {
            counts.push(Count {
                kind: CountKind::NotStarted,
                count: not_started,
                status: "backlog".into(),
                words: format!("{not_started} not started"),
            });
        }
        if c.done > 0 {
            counts.push(Count { kind: CountKind::Done, count: c.done, status: "done".into(), words: format!("{} done", c.done) });
        }
    }
    counts
}

fn facts(r: &Reader, pipeline: &[Group]) -> Vec<Fact> {
    let run = |text: String, strong: bool| Run { text, strong };
    let fact = |kind: FactKind, runs: Vec<Run>, attention: bool| Fact {
        kind,
        text: runs.iter().map(|r| r.text.as_str()).collect(),
        runs,
        attention,
    };
    let mut facts = Vec::new();
    let needs_you = r.context.needs_you;
    if needs_you > 0 {
        facts.push(fact(FactKind::NeedsYou, vec![run(needs_you.to_string(), true), run(" need you".into(), false)], true));
    }
    if let Some(stuck) = pipeline.iter().find(|g| g.stage == PipeStage::Stuck) {
        let words = if stuck.count == 1 { "1 ticket needs you".into() } else { format!("{} tickets need you", stuck.count) };
        facts.push(fact(FactKind::Stuck, vec![run(words, false)], true));
    }
    // A landing carrying no ticket yet is not a landing to name.
    if let Some((tickets, since)) = next_landing(r)
        && tickets > 0
    {
        let mut runs = vec![
            run("Next landing: ".into(), false),
            run(tickets.to_string(), true),
            run(if tickets == 1 { " ticket".into() } else { " tickets".into() }, false),
        ];
        if let Some(since) = since {
            runs.push(run(format!(", {}", words::span(r.plan.now_ms - since)), false));
        }
        facts.push(fact(FactKind::NextLanding, runs, false));
    }
    if let Some(c) = &r.plan.board_counts
        && c.total() > 0
    {
        facts.push(fact(FactKind::Done, vec![run(c.done.to_string(), true), run(format!(" of {} done", c.total()), false)], false));
    }
    facts
}

/// The live landing furthest along: how many tickets it carries, and since
/// when it has been in its phase.
fn next_landing(r: &Reader) -> Option<(usize, Option<i64>)> {
    let mut best: Option<(&super::input::Landing, u8)> = None;
    for landing in r.plan.landings.iter().filter(|l| l.state != "landed" && l.state != "dropped") {
        let phase = r.phase(landing);
        if matches!(phase, LandingPhase::CiFailed | LandingPhase::Stalled) {
            continue;
        }
        // The last of the furthest along, as Swift's `max(by:)` keeps it.
        if best.is_none_or(|(_, p)| phase.progress() >= p) {
            best = Some((landing, phase.progress()));
        }
    }
    let (landing, _) = best?;
    let tickets: BTreeSet<&str> = r.lines_on(landing).into_iter()
        .filter(|l| r.stage(l) == Stage::Landing
            && !matches!(r.status_of(&l.task), Some(Status::Done | Status::Cancelled)))
        .map(|l| l.task.as_str()).collect();
    let since = r.phase_since(landing);
    Some((tickets.len(), (since > 0).then_some(since)))
}

/// Ticket stage, assignment, and runner-reported activity are independent facts.
/// A stage's age cannot diagnose an agent, and an open assignment does not say
/// which ticket is executing. Keep a canonical ticket partition and a unique roster.
fn pipeline(r: &Reader) -> (Vec<Group>, Vec<ColumnAgent>) {
    let plan = r.plan;
    let mut seen = BTreeSet::new();
    let live: Vec<&Line> = plan.lines.iter().filter(|l| {
        matches!(r.stage(l), Stage::Building | Stage::Reviewing | Stage::Landing | Stage::NeedsYou)
            && !matches!(r.status_of(&l.task), Some(Status::Cancelled | Status::Done))
            && seen.insert(l.task.as_str())
    }).collect();
    let stage_of = |line: &Line| match r.stage(line) {
        Stage::NeedsYou => PipeStage::Stuck,
        Stage::Reviewing => PipeStage::Reviewing,
        Stage::Landing => PipeStage::Landing,
        _ => PipeStage::Building,
    };
    let ticket = |line: &Line, detail: Option<String>| {
        let theme = r.theme_of(&line.task);
        ColumnTicket {
            task: line.task.clone(), key: line.key.clone(), title: r.heading(line),
            initiative: theme.map(|t| t.name.clone()), initiative_id: theme.map(|t| t.id.clone()),
            stage: stage_of(line), since: line.stage_since, detail,
        }
    };

    let mut ids = BTreeSet::new();
    let mut agents: Vec<_> = plan.agents.iter()
        .filter(|a| a.role != "orchestrator" && ids.insert(a.id.as_str())).collect();
    agents.sort_by_key(|a| a.started_at);
    let mut roles: BTreeMap<u8, usize> = BTreeMap::new();
    for a in &agents { *roles.entry(Role::of(&a.role) as u8).or_default() += 1; }
    let mut numbers: BTreeMap<u8, usize> = BTreeMap::new();
    let roster: Vec<ColumnAgent> = agents.iter().map(|agent| {
        let role = Role::of(&agent.role);
        let n = numbers.entry(role as u8).or_default();
        *n += 1;
        let name = if roles[&(role as u8)] > 1 { format!("{} {n}", role.word()) } else { role.word() };
        let mut seen = BTreeSet::new();
        let mine: Vec<_> = live.iter().filter(|l| {
            (agent.tickets.iter().any(|t| t.task == l.task)
                || l.work.iter().any(|w| w.agent == agent.id && w.ended_at.is_none()))
                && seen.insert(l.task.as_str())
        }).map(|l| ticket(l, None)).collect();
        let elapsed = (agent.started_at > 0).then(|| (plan.now_ms - agent.started_at).max(0));
        ColumnAgent {
            id: agent.id.clone(), role, title: agent.title.clone(), name,
            // The Boolean has no idle/blocked provenance. Do not invent it.
            doing: if agent.doing_now.trim().is_empty() {
                if agent.working { "Runner reports working".into() } else { "No activity reported".into() }
            } else { agent.doing_now.clone() },
            doing_is_trouble: false, elapsed, time: elapsed.map(|e| format!("Started {} ago", words::span(e))),
            terminal: agent.terminal.clone(), ticket: mine.first().map(|t| t.task.clone()),
            assignment_words: format!("{} pipeline {} assigned", mine.len(), if mine.len() == 1 { "ticket" } else { "tickets" }),
            tickets: mine, working: agent.working, more: 0,
        }
    }).collect();

    let mut assigned: BTreeMap<PipeStage, Vec<ColumnTicket>> = BTreeMap::new();
    let mut unattended: BTreeMap<PipeStage, Vec<ColumnTicket>> = BTreeMap::new();
    for line in live {
        // Both wire paths are assignment evidence. Retain an open assignment
        // even if that agent is absent from the current roster.
        let mut owners: BTreeSet<String> = line.work.iter().filter(|w| w.ended_at.is_none() && !w.agent.trim().is_empty())
            .map(|w| w.agent.clone()).collect();
        owners.extend(agents.iter().filter(|a| a.tickets.iter().any(|t| t.task == line.task)).map(|a| a.id.clone()));
        let labels: Vec<_> = owners.iter().map(|id| {
            roster.iter().find(|a| &a.id == id).map(|a| a.name.clone())
                .unwrap_or_else(|| format!("{id} (not listed)"))
        }).collect();
        let mut detail = if labels.is_empty() { "No listed assignee".into() } else { format!("Assigned: {}", labels.join(", ")) };
        if line.stage_since > 0 {
            detail.push_str(&format!(" · In stage for {}", words::span((plan.now_ms - line.stage_since).max(0))));
        }
        if r.stage(line) == Stage::NeedsYou && !line.reason.trim().is_empty() {
            detail.push_str(&format!(" · {}", line.reason));
        }
        // An explicit landing phase is ticket context. The producer may derive
        // Stalled from age, so that phase must not diagnose agent activity.
        if let Some(landing) = line.landing.and_then(|n| plan.landings.iter().find(|l| l.number == n))
            && let Some(phase) = landing.phase.as_deref().and_then(super::build::wire::<LandingPhase>)
            && !matches!(phase, LandingPhase::Stalled | LandingPhase::Unknown)
        {
            detail.push_str(&format!(" · Landing: {}", phase.sentence()));
        }
        let stage = stage_of(line);
        let t = ticket(line, Some(detail));
        if owners.is_empty() { unattended.entry(stage).or_default().push(t); }
        else { assigned.entry(stage).or_default().push(t); }
    }
    let pipeline = PipeStage::ALL.into_iter().filter_map(|stage| {
        let assigned = assigned.remove(&stage).unwrap_or_default();
        let loose = unattended.remove(&stage).unwrap_or_default();
        if assigned.is_empty() && loose.is_empty() { return None; }
        let n = loose.len();
        let preview: Vec<_> = loose.iter().take(PREVIEW).cloned().collect();
        Some(Group {
            stage, title: stage.title().into(), count: (assigned.len() + n) as u64,
            count_words: format!("{} {}", assigned.len() + n, if assigned.len() + n == 1 { "ticket" } else { "tickets" }),
            agents: Vec::new(), assigned, hidden: (n - preview.len()) as u64, preview,
            unattended: loose,
            unattended_words: (n > 0).then(|| format!("{n} {} with no listed assignee", if n == 1 { "ticket" } else { "tickets" })),
        })
    }).collect();
    (pipeline, roster)
}
