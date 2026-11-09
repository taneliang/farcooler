//! The plan column (ov-867, C10f), as AgentKit's `PlanColumn.swift` and the
//! web's `plan-column.ts` each had it (ov-1016): what the plan side beside the
//! conversation says. The mental model is `.claude/agent/reports/ov-867/intent.md`:
//!
//! 1. the facts line: what needs you, what's stuck, the next landing, and how
//!    much of the board is done;
//! 2. Initiatives (today's themes): the larger goals, each with its goal,
//!    progress and next step;
//! 3. the pipeline: status, then agent, then tickets. Each agent says what it
//!    does now, and its tickets are listed under it. A ticket's initiative is
//!    the ticket's, never the agent's;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// `next` is the owner's to act on: drawn in amber.
    pub asks_owner: bool,
}

/// Where an agent's work stands, in the order the pipeline lists them.
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
            PipeStage::Stuck => "Stuck",
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiative: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiative_id: Option<String>,
    pub stage: PipeStage,
    /// When it entered its stage, in ms; 0 when nothing says.
    #[typeshare(serialized_as = "I54")]
    pub since: i64,
}

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnAgent {
    pub id: String,
    pub role: Role,
    /// "Builder 2": its role, numbered when there are several.
    pub name: String,
    /// What it does now, in its own words; the problem when it is stuck.
    pub doing: String,
    /// `doing` says why it's stuck: drawn in amber.
    pub doing_is_trouble: bool,
    /// How long it has been at it, in ms.
    #[typeshare(serialized_as = "Option<I54>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed: Option<i64>,
    /// "47 min".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// Its pane, to open its chat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    /// The first ticket it works, for its page when there's no pane to open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    pub tickets: Vec<ColumnTicket>,
    pub working: bool,
    /// Tickets in this stage nobody is on, said under the last card.
    #[typeshare(serialized_as = "I54")]
    pub more: u64,
}

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub stage: PipeStage,
    pub title: String,
    pub agents: Vec<ColumnAgent>,
    /// Tickets in this stage with no agent on them.
    pub unattended: Vec<ColumnTicket>,
    /// The tickets in this stage, each once.
    #[typeshare(serialized_as = "I54")]
    pub count: u64,
    /// The unattended tickets drawn in place: the first few in a stage with no
    /// agent; none in a stage with agents, whose "more" line opens them all.
    pub preview: Vec<ColumnTicket>,
    /// The unattended tickets only the list shows.
    #[typeshare(serialized_as = "I54")]
    pub hidden: u64,
    /// "12 more waiting for review".
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_line: Option<String>,
    #[typeshare(serialized_as = "I54")]
    pub initiative_count: u64,
    /// The stages with anything in them, in order.
    pub pipeline: Vec<Group>,
    /// "10 agents".
    pub agent_words: String,
    pub footer: Vec<Count>,
    /// The tickets waiting for a builder, each once, in the plan's order.
    pub waiting: Vec<WaitingTicket>,
}

const PREVIEW: usize = 3;

pub(super) fn column(r: &Reader) -> Column {
    let plan = r.plan;
    let pipeline = pipeline(r);
    let moving: BTreeSet<&str> = pipeline
        .iter()
        .flat_map(|g| g.agents.iter().flat_map(|a| &a.tickets).chain(&g.unattended))
        .map(|t| t.task.as_str())
        .collect();
    let (initiatives, quiet) = initiatives(r, &moving);
    let waiting = waiting(r);
    let footer = footer(r, &waiting);
    let facts = facts(r, &pipeline);
    let agents: usize = pipeline.iter().map(|g| g.agents.len()).sum();
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
        let longest = stuck.agents.iter().filter_map(|a| a.elapsed).max();
        let words = format!("{} stuck{}", stuck.count, longest.map(|l| format!(", {}", words::span(l))).unwrap_or_default());
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
    let tickets: BTreeSet<&str> =
        r.lines_on(landing).into_iter().filter(|l| r.stage(l) == Stage::Landing).map(|l| l.task.as_str()).collect();
    let since = r.phase_since(landing);
    Some((tickets.len(), (since > 0).then_some(since)))
}

struct Card {
    stage: PipeStage,
    agent: ColumnAgent,
    started: i64,
}

fn pipeline(r: &Reader) -> Vec<Group> {
    let plan = r.plan;
    let live: Vec<&Line> = plan
        .lines
        .iter()
        .filter(|l| {
            let s = r.stage(l);
            s.is_live() && s != Stage::Waiting && r.status_of(&l.task) != Some(Status::Cancelled)
        })
        .collect();
    let mut by_task: BTreeMap<&str, &Line> = BTreeMap::new();
    for l in &live {
        by_task.entry(l.task.as_str()).or_insert(l);
    }
    let ticket = |line: &Line, stage: PipeStage| {
        let theme = r.theme_of(&line.task);
        ColumnTicket {
            task: line.task.clone(),
            key: line.key.clone(),
            title: r.heading(line),
            initiative: theme.map(|t| t.name.clone()),
            initiative_id: theme.map(|t| t.id.clone()),
            stage,
            since: line.stage_since,
        }
    };
    // Where a ticket's own state puts it, whoever works it.
    let stage_of = |line: &Line| {
        if line.stale || r.stage(line) == Stage::NeedsYou {
            return PipeStage::Stuck;
        }
        match r.stage(line) {
            Stage::Reviewing => PipeStage::Reviewing,
            Stage::Landing => PipeStage::Landing,
            _ => PipeStage::Building,
        }
    };

    let mut cards: Vec<Card> = Vec::new();
    let mut attended: BTreeSet<String> = BTreeSet::new();
    for agent in plan.agents.iter().filter(|a| a.role != "orchestrator") {
        let role = Role::of(&agent.role);
        let lines: Vec<&Line> = agent.tickets.iter().filter_map(|t| by_task.get(t.task.as_str()).copied()).collect();
        // The landing a lander works: its phase says if it is stuck.
        let landing = (role == Role::Lander)
            .then(|| lines.iter().find_map(|l| l.landing.and_then(|n| plan.landings.iter().find(|x| x.number == n))))
            .flatten();
        let mut doing = agent.doing_now.clone();
        let mut trouble = false;
        let mut stage = match role {
            Role::Lander => {
                let phase = landing.map(|l| r.phase(l));
                match phase {
                    Some(p) if matches!(p, LandingPhase::CiFailed | LandingPhase::Stalled) => {
                        doing = words::capitalize(p.sentence());
                        trouble = true;
                        PipeStage::Stuck
                    }
                    _ => PipeStage::Landing,
                }
            }
            Role::Reviewer => PipeStage::Reviewing,
            Role::Builder | Role::Agent => PipeStage::Building,
        };
        if stage != PipeStage::Stuck
            && let Some(stale) = lines.iter().find(|l| l.stale)
        {
            stage = PipeStage::Stuck;
            if let Some(words) = r.stale(stale) {
                doing = words;
                trouble = true;
            }
        } else if stage != PipeStage::Stuck && lines.iter().any(|l| r.stage(l) == Stage::NeedsYou) {
            stage = PipeStage::Stuck;
        }
        if doing.is_empty() {
            doing = if !agent.title.is_empty() {
                agent.title.clone()
            } else if agent.working {
                "Working".into()
            } else {
                "Idle".into()
            };
        }
        let mine: Vec<ColumnTicket> = lines.iter().map(|l| ticket(l, stage)).collect();
        attended.extend(mine.iter().map(|t| t.task.clone()));
        let elapsed = (agent.started_at > 0).then(|| (plan.now_ms - agent.started_at).max(0));
        cards.push(Card {
            stage,
            agent: ColumnAgent {
                id: agent.id.clone(),
                role,
                name: role.word(),
                doing,
                doing_is_trouble: trouble,
                time: elapsed.map(words::span),
                elapsed,
                terminal: agent.terminal.clone(),
                ticket: lines.first().map(|l| l.task.clone()),
                tickets: mine,
                working: agent.working,
                more: 0,
            },
            started: agent.started_at,
        });
    }

    // Each ticket is on one agent: the lander of its landing first, then the
    // agent whose job matches its stage, then the first to claim it.
    let rank = |agent: &ColumnAgent, task: &str| {
        if agent.role == Role::Lander {
            return 0;
        }
        let wanted = match by_task.get(task).map(|l| r.stage(l)) {
            Some(Stage::Building) => Some(Role::Builder),
            Some(Stage::Reviewing) => Some(Role::Reviewer),
            _ => None,
        };
        if wanted == Some(agent.role) { 1 } else { 2 }
    };
    let mut owner: BTreeMap<String, (u8, usize)> = BTreeMap::new();
    for (index, card) in cards.iter().enumerate() {
        for t in &card.agent.tickets {
            let mine = (rank(&card.agent, &t.task), index);
            if owner.get(&t.task).is_some_and(|have| *have <= mine) {
                continue;
            }
            owner.insert(t.task.clone(), mine);
        }
    }
    for (index, card) in cards.iter_mut().enumerate() {
        card.agent.tickets.retain(|t| owner.get(&t.task).map(|o| o.1) == Some(index));
    }

    // Number each role in the order its agents started: "Builder 1".
    let mut by_role: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    for (index, card) in cards.iter().enumerate() {
        by_role.entry(card.agent.role as u8).or_default().push(index);
    }
    for indices in by_role.values().filter(|i| i.len() > 1) {
        let mut ordered = indices.clone();
        ordered.sort_by_key(|i| (cards[*i].started, *i));
        for (n, index) in ordered.into_iter().enumerate() {
            cards[index].agent.name = format!("{} {}", cards[index].agent.role.word(), n + 1);
        }
    }

    // Tickets nobody is on, by the stage their own state gives them.
    let mut unattended: BTreeMap<PipeStage, Vec<ColumnTicket>> = BTreeMap::new();
    let mut seen = attended;
    for line in &live {
        if seen.insert(line.task.clone()) {
            let s = stage_of(line);
            unattended.entry(s).or_default().push(ticket(line, s));
        }
    }

    PipeStage::ALL
        .into_iter()
        .filter_map(|stage| {
            let mut agents: Vec<&Card> = cards.iter().filter(|c| c.stage == stage).collect();
            // Agents with tickets first, then in start order.
            agents.sort_by_key(|c| (c.agent.tickets.is_empty(), c.started));
            let mut agents: Vec<ColumnAgent> = agents.into_iter().map(|c| c.agent.clone()).collect();
            let loose = unattended.remove(&stage).unwrap_or_default();
            // Said under the last card that has tickets, never under an empty one.
            if let Some(last) = agents.iter().rposition(|a| !a.tickets.is_empty()) {
                agents[last].more = loose.len() as u64;
            }
            if agents.is_empty() && loose.is_empty() {
                return None;
            }
            let count = agents
                .iter()
                .flat_map(|a| a.tickets.iter().map(|t| t.task.as_str()))
                .chain(loose.iter().map(|t| t.task.as_str()))
                .collect::<BTreeSet<_>>()
                .len() as u64;
            let preview: Vec<ColumnTicket> =
                if agents.is_empty() { loose.iter().take(PREVIEW).cloned().collect() } else { Vec::new() };
            let n = loose.len();
            let unattended_words = (n > 0).then(|| {
                let lead = if !agents.is_empty() {
                    format!("{n} more")
                } else if n == 1 {
                    "1 ticket".into()
                } else {
                    format!("{n} tickets")
                };
                match stage {
                    PipeStage::Stuck => format!("{lead} with no agent on {}", if n == 1 { "it" } else { "them" }),
                    PipeStage::Building => format!("{lead} with no builder running"),
                    PipeStage::Reviewing => format!("{lead} waiting for review"),
                    PipeStage::Landing => format!("{lead} waiting to land"),
                }
            });
            Some(Group {
                stage,
                title: r.style.label(&stage.title().to_lowercase()),
                hidden: (n - preview.len()) as u64,
                preview,
                count,
                agents,
                unattended: loose,
                unattended_words,
            })
        })
        .collect()
}
