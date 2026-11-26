//! Read-only Engine presentation for a single authorized workspace.
use crate::session::SessionError;
use crate::workflow_presentation::{self as words, GateKind, Stop, Style};
use farcooler_protocol::v1::{BoardCiStatus, Plan};
use farcooler_workflow::{Definition, Item, Phase, Progression, Reviewer, Stage, StageKind, State};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Engine copy is available locally, including before the first runner read.
pub(crate) fn interface_words(style: crate::plan_view::Style) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("engine", style.label("engine")),
        ("unavailableTitle", style.label("engine unavailable")),
        ("tryAgain", style.label("try again")),
        ("openEngine", style.label("open engine")),
        ("showInEngine", style.label("show in engine")),
        ("showRunInEngine", style.label("show run in engine")),
        ("unavailable", style.label("unavailable")),
        ("stations", style.label("stations")),
        ("runDetails", style.label("run details")),
        ("step", style.label("step")),
        ("selectedAgentModel", style.label("selected agent model")),
        ("agent", style.label("agent")),
        ("reviewState", style.label("review state")),
        ("currentReason", style.label("current reason")),
        ("failureDetails", style.label("failure details")),
        ("sendBacks", style.label("Send-Backs")),
        ("openTicket", style.label("open ticket")),
        ("closeDetails", style.label("close details")),
        ("landingGroups", style.label("landing groups")),
        ("agents", style.label("agents")),
        ("selectedModel", style.label("selected model")),
        ("githubCI", style.label("GitHub CI")),
        ("jobs", style.label("jobs")),
        ("reading", "Reading Engine…".into()),
        ("invalidResponse", "Engine couldn’t load the current workflow.".into()),
        ("openRunHint", "Opens the run in Engine".into()),
        ("stale", "Last updated {time}. Engine couldn’t refresh.".into()),
        ("updated", "Updated at {time}".into()),
        ("used", "{used} of {limit} used".into()),
        ("queued", "{count} queued here".into()),
        ("noSteps", "No workflow steps are available.".into()),
        ("noTickets", "No tickets".into()),
        ("openDetailsHint", "Opens run details".into()),
        ("missingTicket", "This ticket is no longer in the current workflow.".into()),
        ("noLandings", "No landing groups".into()),
        ("missingLanding", "This landing group is no longer available.".into()),
        ("noAgents", "No agent activity reported".into()),
        ("observed", "Observed {time}".into()),
        ("sendBackCount", "{remaining} of {limit} send-backs left".into()),
    ])
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineView {
    pub workspace: String,
    pub epoch: u64,
    pub revision: u64,
    pub capacity: Vec<Capacity>,
    pub stations: Vec<Station>,
    pub landings: Vec<Landing>,
    pub agents: Vec<Agent>,
}

#[derive(Serialize)]
pub struct Capacity {
    id: &'static str,
    label: String,
    used: Option<usize>,
    limit: Option<usize>,
    queued: Option<usize>,
}

#[derive(Serialize)]
pub struct Station {
    id: String,
    label: String,
    tickets: Vec<Ticket>,
    #[serde(skip)]
    pinned_index: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Tone { Normal, Waiting, Failed }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    id: String,
    task_id: Option<String>,
    title: String,
    step: String,
    run: String,
    effect: Option<String>,
    status: String,
    tone: Tone,
    summary: Option<String>,
    reason: Option<String>,
    failure: Option<String>,
    agent: Option<String>,
    review: String,
    send_backs: Option<SendBacks>,
    #[serde(skip)]
    position: Option<usize>,
}

#[derive(Serialize)]
struct SendBacks {
    remaining: u8,
    limit: u8,
    history: Vec<SendBack>,
}

#[derive(Serialize)]
struct SendBack {
    id: String,
    from: String,
    to: String,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Landing {
    id: String,
    label: String,
    status: String,
    tone: Tone,
    since_ms: i64,
    historical: bool,
    members: Vec<Subject>,
    tickets: Vec<Ticket>,
    failure: Option<String>,
    ci: Option<Ci>,
}

#[derive(Serialize)]
struct Subject { id: String, title: String }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Ci {
    status: String,
    sha: String,
    fetched_ms: i64,
    jobs: Vec<CiJob>,
}

#[derive(Serialize)]
struct CiJob { name: String, status: String }

#[derive(Serialize)]
pub struct Agent {
    id: String,
    ticket: String,
    role: String,
    model: Option<String>,
    status: String,
    tone: Tone,
}

#[derive(Deserialize)]
struct Pool {
    cap: usize,
    used: usize,
    #[serde(default)]
    holding: Vec<Placed>,
}

#[derive(Deserialize)]
struct Placed {
    item: String,
    effect: String,
}

#[derive(Deserialize)]
struct Queued {
    #[serde(flatten)]
    placed: Placed,
    pool: String,
    position: usize,
}

#[derive(Default, Deserialize)]
struct Places {
    agent: Option<Pool>,
    landing: Option<Pool>,
    waiting: Option<Vec<Queued>>,
}

/// Decode the existing authorized workflow read and discard its raw definitions and receipts
/// at the presentation boundary. Plan enrichment can be absent on an older or degraded runner.
pub fn from_workflow_reply(workspace: Uuid, raw: &serde_json::Value, plan: Option<&Plan>, style: Style)
    -> Result<EngineView, SessionError>
{
    let invalid = |message: &str| SessionError::Protocol(message.into());
    let context: crate::session::workflow::WorkflowContext = serde_json::from_value(raw["context"].clone())
        .map_err(|_| invalid("Invalid workflow context"))?;
    let workspace = workspace.to_string();
    let auth = context.capabilities;
    if context.binding.is_empty() || !auth.inspect || auth.workspace != workspace || auth.epoch == 0 {
        return Err(invalid("Engine inspection is unavailable"));
    }
    let state = if raw["state"].is_null() {
        State::empty(workspace.clone(), auth.epoch)
    } else {
        serde_json::from_value(raw["state"].clone()).map_err(|_| invalid("Invalid workflow state"))?
    };
    if state.workspace != workspace || state.epoch != auth.epoch || state.schema != 1 {
        return Err(invalid("The workflow read does not match this workspace"));
    }
    let definition: Option<Definition> = serde_json::from_value(raw["configuration"]["definition"].clone())
        .map_err(|_| invalid("Invalid published workflow"))?;
    let places = serde_json::from_value(raw["places"].clone()).unwrap_or_default();
    let stops = serde_json::from_value(raw["stops"].clone()).unwrap_or_default();
    Ok(project(&state, definition.as_ref(), &places, &stops, plan, style))
}

struct Projection<'a> {
    places: &'a Places,
    stops: &'a BTreeMap<String, Stop>,
    cards: BTreeMap<&'a str, &'a farcooler_protocol::v1::PlanCard>,
    style: Style,
}

fn project(state: &State, definition: Option<&Definition>, places: &Places,
    stops: &BTreeMap<String, Stop>, plan: Option<&Plan>, style: Style) -> EngineView
{
    let cards = plan.iter().flat_map(|plan| &plan.cards).map(|c| (c.key.as_str(), c)).collect();
    let projection = Projection { places, stops, cards, style };
    let mut stations = Vec::new();
    if let Some(definition) = definition {
        stations.extend(definition.stages.iter().enumerate().map(|(index, stage)| Station::new(stage, index, style)));
    }
    let mut joined = BTreeMap::<&str, Vec<Ticket>>::new();
    let mut agents = Vec::new();
    for (id, item) in &state.items {
        if item.group_of.is_some() || matches!(item.phase, Phase::Done | Phase::Cancelled) { continue; }
        let lead = item.follows.as_ref().and_then(|id| state.items.get_key_value(id));
        let (run_owner, item) = lead.unwrap_or((id, item));
        let Some(stage) = item.definition.stages.get(item.stage) else { continue; };
        let ticket = projection.ticket(id, run_owner, item, stage, state);
        if run_owner == id {
            if let Some(agent) = projection.agent(id, item, stage, &ticket) { agents.push(agent); }
        }
        if let Some(group) = item.grouped_in.as_deref().filter(|g| state.items.get(*g).is_some_and(|i| i.group_of.is_some())) {
            joined.entry(group).or_default().push(ticket);
            continue;
        }
        let station = match stations.iter().position(|s| s.id == stage.id) {
            Some(index) => &mut stations[index],
            None => {
                stations.push(Station::new(stage, item.stage, style));
                stations.last_mut().expect("station just inserted")
            }
        };
        station.tickets.push(ticket);
    }
    // Keep unmapped stations in workflow order. Tracker pins rotate their
    // step list to start at the ticket's current column.
    let published = definition.map_or(0, |d| d.stages.len());
    stations[published..].sort_by_key(|s| {
        let column = farcooler_workflow::TICKET_COLUMNS.iter().position(|id| *id == s.id);
        (column.is_none(), column.unwrap_or(s.pinned_index))
    });
    for station in &mut stations {
        station.tickets.sort_by_key(|ticket| ticket.position);
    }

    let mut landings = Vec::new();
    for (id, item) in &state.items {
        if item.group_of.is_none() { continue; }
        let phase = observed_phase(item);
        let stage = item.definition.stages.get(item.stage);
        let status = status(&phase, stage, style);
        let status = stage.map_or_else(|| status.clone(), |s| format!("{} · {status}", stage_name(s, style)));
        let failure = item.failure.clone().or_else(|| item.group_failure.as_ref().map(|f| f.first_line.clone()));
        let members: Vec<_> = if item.members.is_empty() {
            item.handed.iter().map(String::as_str).collect()
        } else {
            item.members.iter().map(|m| m.item.as_str()).collect()
        };
        let mut tickets = joined.remove(id.as_str()).unwrap_or_default();
        tickets.sort_by_key(|ticket| members.iter().position(|member| *member == ticket.id));
        landings.push(projection.landing(id, phase, status, item.since_ms, members.into_iter(), tickets,
            failure, item.pushed.as_deref(), plan));
    }
    for (id, record) in &state.archive {
        let Some(group) = &record.group else { continue; };
        if state.items.contains_key(id) { continue; }
        landings.push(projection.landing(id, record.phase.clone(), status(&record.phase, None, style),
            group.since_ms, group.tickets.iter().map(String::as_str), Vec::new(), None, group.pushed.as_deref(), plan));
    }
    landings.sort_by_key(|group| (group.historical, std::cmp::Reverse(group.since_ms)));
    let mut history = 0;
    landings.retain(|group| {
        history += usize::from(group.historical);
        history <= 12
    });
    let capacity = [
        ("agent", "Agent slots", places.agent.as_ref()), ("landing", "Landing slots", places.landing.as_ref()),
        ("load", "1-Minute Load", None), ("disk", "Disk free", None),
    ].into_iter().map(|(id, label, pool)| Capacity {
        id, label: style.label(label), used: pool.map(|p| p.used), limit: pool.map(|p| p.cap),
        queued: pool.and(places.waiting.as_ref()).map(|waiting| waiting.iter().filter(|entry| {
            entry.pool == id && state.items.get(&entry.placed.item).is_some_and(|item| {
                item.pending.as_deref() == Some(&entry.placed.effect)
            })
        }).count()),
    }).collect();
    EngineView { workspace: state.workspace.clone(), epoch: state.epoch, revision: state.revision, capacity, stations, landings, agents }
}

impl Station {
    fn new(stage: &Stage, pinned_index: usize, style: Style) -> Self {
        Self { id: stage.id.clone(), label: stage_name(stage, style), tickets: Vec::new(), pinned_index }
    }
}

impl Projection<'_> {
    fn title(&self, id: &str) -> String {
        self.cards.get(id).map(|c| c.title.as_str()).filter(|title| !title.is_empty()).unwrap_or(id).into()
    }

    fn ticket(&self, id: &str, owner: &str, item: &Item, step: &Stage, state: &State) -> Ticket {
        let style = self.style;
        let phase = observed_phase(item);
        let queue = self.places.waiting.iter().flatten().find(|q| q.placed.item == owner
            && item.pending.as_deref() == Some(&q.placed.effect));
        let position = if phase == Phase::WaitingSlot {
            queue.map(|q| q.position).or_else(|| item.execution.as_ref().and_then(|e| e.position))
        } else { None };
        let failure = item.failure.clone().or_else(|| {
            self.stops.get(owner).filter(|stop| stop.run == item.run).map(|stop| stop.text.clone())
        }).or_else(|| item.reason.as_ref().filter(|_| phase == Phase::Failed).cloned());
        let reason = item.queue_hold.as_ref().map(|hold| hold.reason.clone())
            .or_else(|| reason(item, step, &phase, state, position, style))
            .filter(|reason| Some(reason) != failure.as_ref() && !(phase == Phase::Failed && failure.is_some()));
        let current_failure = matches!(phase, Phase::Failed | Phase::Uncertain | Phase::Blocked | Phase::WaitingSlot);
        let summary = failure.as_deref().filter(|_| current_failure).or(reason.as_deref())
            .and_then(|text| text.lines().find(|line| !line.trim().is_empty())).map(str::to_owned);
        let status = match (queue.map(|q| q.pool.as_str()), &phase) {
            (Some("agent"), Phase::WaitingSlot) => style.label("Waiting for an agent slot"),
            _ => status(&phase, Some(step), style),
        };
        let review = match (&step.kind, &phase) {
            (StageKind::Review(_), _) => status.clone(),
            _ if item.sent_back.last().is_some_and(|back|
                item.definition.stages.iter().any(|s| s.id == back.from && matches!(s.kind, StageKind::Review(_))))
                => style.label("Sent back from review"),
            _ if !item.rework_history.is_empty() => style.label("Changes requested"),
            _ if !item.definition.stages.iter().any(|s| matches!(s.kind, StageKind::Review(_))) => style.label("No review step"),
            _ => style.label("Review state unavailable"),
        };
        let send_backs = (!item.definition.is_ticket()).then(|| {
            let limit = item.definition.send_back_limit();
            let used = u8::try_from(item.sent_back.len()).unwrap_or(u8::MAX);
            let label = |id: &str| item.definition.stages.iter().find(|s| s.id == id)
                .map_or_else(|| id.to_owned(), |stage| stage_name(stage, style));
            SendBacks { limit, remaining: limit.saturating_sub(used), history: item.sent_back.iter().map(|b| SendBack {
                id: b.id.clone(), from: label(&b.from), to: label(&b.to), reason: b.reason.clone(),
            }).collect() }
        });
        Ticket {
            id: id.into(), title: self.title(id), task_id: self.cards.get(id).and_then(|c| Uuid::from_slice(&c.task_id).ok())
                .map(|id| id.to_string()), step: stage_name(step, style), run: item.run.clone(), effect: item.pending.clone(),
            tone: tone(&phase), status, summary, reason, failure, review, send_backs, position,
            agent: item.models.get(&step.id).cloned(),
        }
    }

    fn agent(&self, id: &str, item: &Item, step: &Stage, ticket: &Ticket) -> Option<Agent> {
        if !matches!(step.kind, StageKind::Build | StageKind::Review(Reviewer::Agent)) { return None; }
        let effect = item.pending.as_ref()?;
        let held = self.places.agent.iter().flat_map(|p| &p.holding).any(|p| p.item == id && &p.effect == effect);
        if !held && !matches!(observed_phase(item), Phase::Running | Phase::Uncertain | Phase::Starting | Phase::Stalled) { return None; }
        Some(Agent { id: effect.clone(), ticket: id.into(), role: ticket.step.clone(), model: ticket.agent.clone(),
            status: ticket.status.clone(), tone: ticket.tone })
    }

    #[allow(clippy::too_many_arguments)]
    fn landing<'a>(&self, id: &str, phase: Phase, status: String, since_ms: i64, members: impl Iterator<Item = &'a str>,
        tickets: Vec<Ticket>, failure: Option<String>, pushed: Option<&str>, plan: Option<&Plan>) -> Landing
    {
        let ci = pushed.and_then(|sha| github_ci(plan?, sha, self.style));
        let label = id.strip_prefix("group-").and_then(|id| id.split('-').next())
            .filter(|id| id.parse::<u64>().is_ok()).unwrap_or(id);
        Landing { id: id.into(), label: format!("{} {label}", self.style.label("Landing group")),
            historical: matches!(phase, Phase::Failed | Phase::Done | Phase::Cancelled) && tickets.is_empty(),
            status, tone: tone(&phase), since_ms,
            members: members.map(|id| Subject { id: id.into(), title: self.title(id) }).collect(),
            tickets, failure, ci }
    }
}

fn stage_name(stage: &Stage, style: Style) -> String {
    let label = match (&stage.kind, stage.id.as_str()) {
        (StageKind::Tracker, "backlog") => "Backlog",
        (StageKind::Tracker, "todo" | "to-do") => "To Do",
        (StageKind::Tracker, "doing") => "In progress",
        (StageKind::Tracker, "review") => "In review",
        (StageKind::Tracker, "decision") => "Needs decision",
        (StageKind::Tracker | StageKind::Check | StageKind::Evaluate(_) | StageKind::Action(_), id)
            if !id.starts_with("stage-") => return style.label(&id.replace(['-', '_'], " ")),
        (StageKind::Join { .. }, _) => "Join",
        (StageKind::Run, _) => "Local gate",
        (StageKind::AwaitCi | StageKind::Ci | StageKind::RerunCi, _) => "GitHub CI",
        (kind, _) => words::stage_label(kind),
    };
    style.label(label)
}

fn observed_phase(item: &Item) -> Phase {
    if matches!(item.phase, Phase::Running | Phase::Starting | Phase::WaitingSlot | Phase::Stalled) {
        item.execution.as_ref().map_or(Phase::WaitingSlot, |e| e.phase.clone())
    } else {
        item.phase.clone()
    }
}

fn status(phase: &Phase, stage: Option<&Stage>, style: Style) -> String {
    match (phase, stage.map(|s| &s.kind)) {
        (Phase::Running, Some(StageKind::Build)) => style.label("Building"),
        (Phase::Running, Some(StageKind::Review(_))) => style.label("Reviewing"),
        (Phase::WaitingJoin, _) => style.label("Waiting to join a landing group"),
        _ => words::phase_label(phase, style),
    }
}

fn tone(phase: &Phase) -> Tone {
    match phase {
        Phase::Failed | Phase::Uncertain | Phase::Stalled => Tone::Failed,
        Phase::Blocked | Phase::WaitingSlot | Phase::WaitingApproval | Phase::WaitingReview => Tone::Waiting,
        _ => Tone::Normal,
    }
}

fn reason(item: &Item, step: &Stage, phase: &Phase, state: &State, position: Option<usize>, style: Style) -> Option<String> {
    if let Some(position) = position {
        return Some(format!("Position {position} in the runner’s queue."));
    }
    match phase {
        Phase::WaitingApproval => if let Progression::AwaitApproval { principal } = &step.progression {
            Some(words::gate_card(GateKind::Approval, principal, style).text)
        } else { None },
        Phase::WaitingReview => if let StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) = &step.kind {
            Some(words::gate_card(GateKind::Review, p, style).text)
        } else { None },
        Phase::Blocked => {
            let waiting: Vec<_> = item.dependencies.iter().filter(|id| {
                !state.items.get(*id).is_some_and(|i| i.phase == Phase::Done)
                    && !state.archive.get(*id).is_some_and(|i| i.phase == Phase::Done)
            }).map(String::as_str).collect();
            if waiting.is_empty() { Some("The runner hasn’t reported what’s blocking this step.".into()) }
            else { Some(format!("Waiting for {}.", waiting.join(", "))) }
        }
        Phase::Failed => (item.failure.is_none()).then(|| "The runner hasn’t reported why this step failed.".into()),
        Phase::WaitingSlot | Phase::Uncertain | Phase::Starting | Phase::Stalled => item.execution.as_ref().map(|e| e.text.clone())
            .or_else(|| Some("The runner hasn’t reported an execution reason.".into())),
        _ => None,
    }
}

fn github_ci(plan: &Plan, sha: &str, style: Style) -> Option<Ci> {
    if sha.is_empty() { return None; }
    let subject = format!("sha:{sha}");
    let read = plan.ci.iter().filter(|read| read.sha == sha && read.subject == subject && read.fetched_at > 0)
        .max_by_key(|read| read.fetched_at)?;
    let status = match BoardCiStatus::try_from(read.status) {
        Ok(BoardCiStatus::Passed) => "Passed",
        Ok(BoardCiStatus::Failed) => "Failed",
        Ok(BoardCiStatus::Running) => "Running",
        Ok(BoardCiStatus::Queued) => "Queued",
        Ok(BoardCiStatus::None) => "No run reported",
        Ok(BoardCiStatus::Superseded) => "Superseded",
        _ => "Unavailable",
    };
    Some(Ci { status: style.label(status), sha: sha.into(), fetched_ms: read.fetched_at,
        jobs: read.jobs.iter().map(|job| CiJob { name: job.name.clone(), status: style.label(&job.state) }).collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::{BoardCiRead, PlanCard};
    use farcooler_workflow::{Command, Dispatch, Grant, Input, SendBack as Decision, execution_view};

    fn enqueue(state: &mut State, id: &str, definition: &Definition) {
        let input = Input {
            schema: 1, id: format!("enqueue-{id}"), workspace: state.workspace.clone(),
            expected_revision: state.revision, epoch: state.epoch, source: "fixture".into(),
            source_sequence: state.revision + 1, now_ms: 100, grant: Grant::Controller,
            command: Command::Enqueue { item: id.into(), definition: definition.clone(),
                policy_pin: "policy".into(), resource: id.into(), dependencies: Default::default(),
                initial_candidate: matches!(definition.stages[0].kind, StageKind::Tracker).then(|| "candidate".into()) },
            busy_resources: Default::default(),
        };
        *state = farcooler_workflow::step(state, &input).unwrap().state;
    }

    #[test]
    fn stations_follow_publication_and_keep_legacy_steps_and_runner_queue_order() {
        let mut state = State::empty("w".into(), 1);
        let published = Definition::coding();
        let tracker = Definition::ticket_at("todo").unwrap();
        enqueue(&mut state, "ov-legacy", &tracker);
        for key in ["ov-1", "ov-2"] { enqueue(&mut state, key, &published); }
        for (key, position) in [("ov-1", 3), ("ov-2", 1)] {
            let item = state.items.get_mut(key).unwrap();
            item.stage = 1;
            item.phase = Phase::Running;
            item.execution = Some(execution_view(Dispatch::Pending, Some(position), Vec::new()));
        }
        let view = project(&state, Some(&published), &Places::default(), &BTreeMap::new(), None, Style::Apple);
        assert_eq!(view.stations.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["stage-0", "stage-1", "stage-2", "stage-3", "stage-4", "stage-5", "todo"]);
        let build = &view.stations[1];
        assert_eq!(build.tickets.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["ov-2", "ov-1"]);
        assert_eq!(build.tickets[0].status, "Waiting for a Slot");
        assert!(view.stations[6].tickets[0].send_backs.is_none());
        assert!(view.agents.is_empty());
    }

    #[test]
    fn without_a_publication_active_stations_follow_the_pinned_step_order() {
        let mut state = State::empty("w".into(), 1);
        let definition = Definition::coding();
        enqueue(&mut state, "ov-1", &definition);
        enqueue(&mut state, "ov-2", &definition);
        state.items.get_mut("ov-1").unwrap().stage = 3;
        state.items.get_mut("ov-2").unwrap().stage = 1;
        let view = project(&state, None, &Places::default(), &BTreeMap::new(), None, Style::Apple);
        assert_eq!(view.stations.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["stage-1", "stage-3"]);
    }

    #[test]
    fn landing_hold_is_visible_on_the_plan_ticket() {
        let mut state = State::empty("w".into(), 1);
        let definition = Definition::coding();
        enqueue(&mut state, "ov-1", &definition);
        let item = state.items.get_mut("ov-1").unwrap();
        item.phase = Phase::WaitingJoin;
        let why = "The landing target has no valid head commit.";
        item.queue_hold = Some(farcooler_workflow::QueueHold { reason: why.into(), since_ms: 1, notified: false });
        let view = project(&state, Some(&definition), &Places::default(), &BTreeMap::new(), None, Style::Apple);
        let ticket = view.stations.iter().flat_map(|s| &s.tickets).find(|t| t.id == "ov-1").unwrap();
        assert_eq!(ticket.summary.as_deref(), Some(why));
        assert_eq!(ticket.reason.as_deref(), Some(why));
    }

    #[test]
    fn inspector_keeps_exact_run_failure_separate_from_send_backs_and_review() {
        let mut state = State::empty("w".into(), 1);
        let mut definition = Definition::coding();
        definition.send_backs = Some(0);
        enqueue(&mut state, "ov-1", &definition);
        let item = state.items.get_mut("ov-1").unwrap();
        item.stage = 3;
        item.phase = Phase::Failed;
        item.reason = Some("stage-3: The review could not finish (receipt review-1)".into());
        item.sent_back.push(Decision { id: "back-1".into(), revision: 8, run: "earlier".into(),
            from: "stage-3".into(), to: "stage-1".into(), reason: "Earlier failure".into() });
        let plan = Plan { cards: vec![PlanCard { key: "ov-1".into(), title: "A long ticket title".repeat(8),
            task_id: vec![1; 16].into(), ..Default::default() }], ..Default::default() };
        let mut stops = BTreeMap::from([("ov-1".into(), Stop { run: "stale".into(), text: "Wrong run".into() })]);
        let places = Places::default();
        let view = project(&state, Some(&definition), &places, &stops, Some(&plan), Style::Apple);
        let ticket = &view.stations[3].tickets[0];
        assert_eq!(ticket.failure.as_deref(), state.items["ov-1"].reason.as_deref());
        assert_eq!(ticket.summary, ticket.failure);
        assert!(ticket.reason.is_none());
        stops.insert("ov-1".into(), Stop { run: state.items["ov-1"].run.clone(), text: "Failure\nFull detail".into() });
        let view = project(&state, Some(&definition), &places, &stops, Some(&plan), Style::Apple);
        let ticket = &view.stations[3].tickets[0];
        assert_eq!(ticket.title, plan.cards[0].title);
        assert_eq!(ticket.review, "Failed");
        assert_eq!(ticket.failure.as_deref(), Some("Failure\nFull detail"));
        assert_eq!(ticket.summary.as_deref(), Some("Failure"));
        let back = ticket.send_backs.as_ref().unwrap();
        assert_eq!((back.remaining, back.limit), (0, 0));
        assert_eq!(back.history[0].reason, "Earlier failure");
    }

    #[test]
    fn joined_ticket_only_appears_in_its_group_and_ci_matches_the_pushed_commit() {
        let mut state = State::empty("w".into(), 1);
        let definition = Definition::coding();
        enqueue(&mut state, "ov-1", &definition);
        state.items.insert("group-42-a".into(), state.items["ov-1"].clone());
        let ticket = state.items.get_mut("ov-1").unwrap();
        ticket.phase = Phase::Joined;
        ticket.grouped_in = Some("group-42-a".into());
        let group = state.items.get_mut("group-42-a").unwrap();
        group.group_of = Some(farcooler_workflow::GroupOf { name: "main".into(), pin: definition.pin() });
        group.members.push(farcooler_workflow::Member { item: "ov-1".into(), candidate: "candidate".into() });
        group.pushed = Some("pushed".into());
        let plan = Plan { ci: vec![
            BoardCiRead { sha: "base".into(), subject: "sha:pushed".into(), status: BoardCiStatus::Failed as i32,
                fetched_at: 100, ..Default::default() },
            BoardCiRead { sha: "pushed".into(), subject: "sha:pushed".into(), status: BoardCiStatus::Passed as i32,
                fetched_at: 80, ..Default::default() },
        ], ..Default::default() };
        let view = project(&state, None, &Places::default(), &BTreeMap::new(), Some(&plan), Style::Apple);
        assert!(view.stations.iter().all(|s| s.tickets.is_empty()));
        assert_eq!(view.landings[0].tickets[0].id, "ov-1");
        assert_eq!(view.landings[0].ci.as_ref().unwrap().status, "Passed");
        assert!(!view.landings[0].historical);
    }

    #[test]
    fn authorized_empty_read_keeps_unavailable_capacity_distinct_from_zero() {
        let workspace = Uuid::from_u128(42);
        let mut raw = serde_json::json!({"context": {"binding": "owner", "capabilities": {
            "workspace": workspace, "epoch": 1, "inspect": true, "controller": true
        }}, "places": {"agent": {"cap": 4, "used": 0}, "waiting": []}});
        let view = from_workflow_reply(workspace, &raw, None, Style::Apple).unwrap();
        assert_eq!((view.capacity[0].used, view.capacity[1].used), (Some(0), None));
        raw["context"]["capabilities"]["epoch"] = 0.into();
        assert!(from_workflow_reply(workspace, &raw, None, Style::Apple).is_err());
    }
}
