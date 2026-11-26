//! The plan layer as JSON (ov-274, ov-872): `plan.get` and `plan.events` in
//! the shape the apps decode, which is the shape `farcooler plan --json`
//! prints, so the Mac and the phones read one fixture, `test/fixtures/plan.json`.
//!
//! EXPERIMENTAL, behind `board_plan`, and removable with the rest of the layer
//! (design 8). The phones read these two answers and never write the plan: the
//! orchestrator is its one writer.
//!
//! The CLI builds the same objects from the same wire messages
//! (`crates/cli/src/plan.rs`). Both are held to the one fixture by a test, so
//! a key renamed on either side fails there instead of on a phone.

use farcooler_protocol::{v1 as pb, workflow_commands::Action};
use serde_json::{Value, json};

use crate::session::{SessionError, short, uuid_of};

/// The original single-response request, also understood by older runners.
pub fn workflow_request(workspace: uuid::Uuid) -> pb::Request {
    workflow_request_for(workspace, &Action::Get)
}

fn workflow_request_for(workspace: uuid::Uuid, action: &Action) -> pb::Request {
    use farcooler_protocol::capability;
    let mut request = farcooler_transport::request(action.method());
    request.target_resource_id = Some(bytes::Bytes::copy_from_slice(workspace.as_bytes()));
    request.required_capabilities = vec![capability::WORKFLOW_COMMANDS.into()];
    request.payload = Some(pb::request::Payload::WorkflowCommand(pb::WorkflowCommand {
        schema: 1,
        json: serde_json::to_string(action).expect("closed action"),
    }));
    request
}

/// Assemble Plan's workflow items and stops before projecting the run cards.
pub struct WorkflowPlanPages {
    workspace: uuid::Uuid,
    next: Option<String>,
    state: Option<Value>,
}

impl WorkflowPlanPages {
    pub fn new(workspace: uuid::Uuid) -> Self {
        Self { workspace, next: None, state: None }
    }

    pub fn request(&self) -> pb::Request {
        let mut request = workflow_request(self.workspace);
        request.accepts.push(farcooler_protocol::accepts::WORKFLOW_PLAN.into());
        request.accepts.extend(self.next.iter().cloned());
        request
    }

    pub fn push(&mut self, reply: pb::result::Value) -> Result<Option<Value>, SessionError> {
        let mut page = workflow_reply(reply)?;
        let next = page.as_object_mut().and_then(|fields| fields.remove("plan_next"));
        self.next = next.map(|next| {
            next.as_str()
                .filter(|s| s.starts_with(farcooler_protocol::accepts::WORKFLOW_PLAN_PAGE))
                .map(str::to_owned)
                .ok_or_else(|| SessionError::Protocol("Invalid workflow Plan continuation.".into()))
        }).transpose()?;
        if let Some(raw) = &mut self.state {
            if raw["context"] != page["context"] || raw["state"]["revision"] != page["state"]["revision"]
                || raw["state"]["epoch"] != page["state"]["epoch"]
            {
                return Err(SessionError::Protocol("The workflow changed while Plan was loading. Try again.".into()));
            }
            merge_workflow_rows(raw, &mut page, "/state/items")?;
            merge_workflow_rows(raw, &mut page, "/stops")?;
        } else {
            self.state = Some(page);
        }
        Ok(if self.next.is_none() { self.state.take() } else { None })
    }
}

fn merge_workflow_rows(raw: &mut Value, page: &mut Value, path: &str) -> Result<(), SessionError> {
    let invalid = || SessionError::Protocol("Invalid workflow Plan page.".into());
    let rows = raw.pointer_mut(path).and_then(Value::as_object_mut).ok_or_else(invalid)?;
    let incoming = page.pointer_mut(path).and_then(Value::as_object_mut).ok_or_else(invalid)?;
    if (path == "/state/items" && incoming.is_empty()) || incoming.keys().any(|id| rows.contains_key(id)) {
        return Err(invalid());
    }
    rows.extend(std::mem::take(incoming));
    Ok(())
}

pub(crate) fn workflow_reply(reply: pb::result::Value) -> Result<Value, SessionError> {
    match reply {
        pb::result::Value::WorkflowReply(reply) if reply.schema == 1 =>
            serde_json::from_str(&reply.json).map_err(|e| SessionError::Protocol(e.to_string())),
        _ => Err(SessionError::Protocol("invalid workflow reply".into())),
    }
}

#[derive(serde::Deserialize)]
struct WorkflowPage {
    offset: usize,
    next_offset: Option<usize>,
    digest: String,
    chunk: String,
}

/// Assemble one authorized workflow snapshot before projecting any cards or actions.
pub async fn read_workflow(
    workspace: uuid::Uuid,
    call: impl AsyncFnMut(pb::Request) -> Result<pb::result::Value, SessionError>,
) -> Result<Value, SessionError> {
    read_workflow_view(workspace, None, call).await
}

/// Assemble a workflow snapshot using the requested projection on every page.
pub async fn read_workflow_view(
    workspace: uuid::Uuid,
    view: Option<&str>,
    mut call: impl AsyncFnMut(pb::Request) -> Result<pb::result::Value, SessionError>,
) -> Result<Value, SessionError> {
    use std::ops::ControlFlow;
    let (mut pages, mut request) = WorkflowPages::start_with_view(workspace, view);
    loop {
        match pages.receive(call(request).await?)? {
            ControlFlow::Break(snapshot) => return Ok(snapshot),
            ControlFlow::Continue(next) => request = next,
        }
    }
}

pub(crate) struct WorkflowPages {
    workspace: uuid::Uuid,
    view: Option<String>,
    snapshot: String,
    digest: String,
    restarted: bool,
}

impl WorkflowPages {
    pub(crate) fn start(workspace: uuid::Uuid) -> (Self, pb::Request) {
        Self::start_with_view(workspace, None)
    }

    fn start_with_view(workspace: uuid::Uuid, view: Option<&str>) -> (Self, pb::Request) {
        let pages = Self { workspace, view: view.map(String::from), snapshot: String::new(), digest: String::new(), restarted: false };
        let request = pages.request(&Action::Get);
        (pages, request)
    }

    fn request(&self, action: &Action) -> pb::Request {
        let mut request = workflow_request_for(self.workspace, action);
        request.accepts.push(farcooler_protocol::accepts::WORKFLOW_PAGES.into());
        request.accepts.extend(self.view.iter().cloned());
        request
    }

    pub(crate) fn receive(&mut self, reply: pb::result::Value)
        -> Result<std::ops::ControlFlow<Value, pb::Request>, SessionError>
    {
        use std::ops::ControlFlow;
        let invalid = || SessionError::Protocol("Invalid workflow snapshot".into());
        let pb::result::Value::WorkflowReply(reply) = reply else { return Err(invalid()); };
        if reply.schema != 1 { return Err(invalid()); }
        let mut raw: Value = serde_json::from_str(&reply.json).map_err(|_| invalid())?;
        let Some(page) = raw.as_object_mut().and_then(|fields| fields.remove("workflow_page")) else {
            return complete_workflow(raw).map(ControlFlow::Break);
        };
        let page: WorkflowPage = serde_json::from_value(page).map_err(|_| invalid())?;
        if page.offset == 0 {
            if !self.snapshot.is_empty() {
                if self.restarted || page.digest == self.digest { return Err(invalid()); }
                self.restarted = true;
                self.snapshot.clear();
            }
            self.digest = page.digest.clone();
        }
        if page.offset != self.snapshot.len() || page.digest != self.digest || page.chunk.is_empty() {
            return Err(invalid());
        }
        self.snapshot.push_str(&page.chunk);
        let Some(offset) = page.next_offset else {
            if farcooler_workflow::digest(&self.snapshot) != self.digest { return Err(invalid()); }
            let raw = serde_json::from_str(&self.snapshot).map_err(|_| invalid())?;
            return complete_workflow(raw).map(ControlFlow::Break);
        };
        if offset != self.snapshot.len() { return Err(invalid()); }
        let action = Action::GetPage { offset: offset as u64, digest: self.digest.clone() };
        Ok(ControlFlow::Continue(self.request(&action)))
    }
}

fn complete_workflow(raw: Value) -> Result<Value, SessionError> {
    if raw["items_omitted"].as_u64().unwrap_or_default() > 0 {
        return Err(SessionError::Protocol("The workflow snapshot is incomplete".into()));
    }
    Ok(raw)
}

/// The single enrichment and compatibility policy for CLI, native and web Plan reads.
/// A missing future means the runner did not advertise workflow commands.
pub async fn with_workflow(
    plan: &pb::Plan,
    workspace: uuid::Uuid,
    read: Option<impl std::future::Future<Output = Result<pb::result::Value, crate::session::SessionError>>>,
) -> Result<Value, crate::session::SessionError> {
    with_workflow_state(plan, workspace, read.map(|read| async { workflow_reply(read.await?) })).await
}

pub async fn with_workflow_state(
    plan: &pb::Plan,
    workspace: uuid::Uuid,
    read: Option<impl std::future::Future<Output = Result<Value, SessionError>>>,
) -> Result<Value, SessionError> {
    use crate::session::workflow::decode_presentation;
    let mut value = plan_json(plan);
    let Some(read) = read else { return Ok(value); };
    match read.await {
        Ok(raw) => {
            let (_, _, presentation) = decode_presentation(workspace, &raw, crate::workflow_presentation::Style::Apple)?;
            value["workflow"] = serde_json::to_value(presentation).expect("presentation serializes");
        }
        // Older and read-only runners can still supply the un-enriched Plan.
        Err(SessionError::Refused { code, .. }) if matches!(pb::ErrorCode::try_from(code),
            Ok(pb::ErrorCode::ScopeDenied | pb::ErrorCode::CapabilityUnsupported)) => {}
        Err(error) => return Err(error),
    }
    Ok(value)
}

fn id_text(id: &[u8]) -> String {
    uuid_of(id).to_string()
}

/// A card's status in the board's words, as the CLI says it.
fn status_word(status: i32) -> &'static str {
    match pb::TaskStatus::try_from(status) {
        Ok(pb::TaskStatus::Backlog) => "Backlog",
        Ok(pb::TaskStatus::Todo) => "To Do",
        Ok(pb::TaskStatus::NeedsDecision) => "Needs Decision",
        Ok(pb::TaskStatus::InProgress) => "In Progress",
        Ok(pb::TaskStatus::InReview) => "In Review",
        Ok(pb::TaskStatus::Done) => "Done",
        Ok(pb::TaskStatus::Cancelled) => "Cancelled",
        _ => "Unknown",
    }
}

fn theme_state_word(state: i32) -> &'static str {
    match pb::BoardThemeState::try_from(state) {
        Ok(pb::BoardThemeState::Active) => "active",
        Ok(pb::BoardThemeState::Paused) => "paused",
        Ok(pb::BoardThemeState::Done) => "done",
        Ok(pb::BoardThemeState::Dropped) => "dropped",
        _ => "unknown",
    }
}

/// A ticket's stage as the JSON spells it: the store's word.
pub fn stage_word(stage: i32) -> &'static str {
    match pb::TicketStage::try_from(stage) {
        Ok(pb::TicketStage::Waiting) => "waiting",
        Ok(pb::TicketStage::Building) => "building",
        Ok(pb::TicketStage::Reviewing) => "reviewing",
        Ok(pb::TicketStage::NeedsYou) => "needs_you",
        Ok(pb::TicketStage::Landing) => "landing",
        Ok(pb::TicketStage::Landed) => "landed",
        Ok(pb::TicketStage::Dropped) => "dropped",
        _ => "unknown",
    }
}

/// An agent's role as the JSON spells it.
pub fn role_word(role: i32) -> &'static str {
    match pb::AgentRole::try_from(role) {
        Ok(pb::AgentRole::Orchestrator) => "orchestrator",
        Ok(pb::AgentRole::Builder) => "builder",
        Ok(pb::AgentRole::Reviewer) => "reviewer",
        Ok(pb::AgentRole::Lander) => "lander",
        _ => "unknown",
    }
}

/// What an agent does for a ticket, as the JSON spells it.
pub fn work_word(role: i32) -> &'static str {
    match pb::WorkRole::try_from(role) {
        Ok(pb::WorkRole::Build) => "build",
        Ok(pb::WorkRole::Fix) => "fix",
        Ok(pb::WorkRole::Review) => "review",
        Ok(pb::WorkRole::Land) => "land",
        _ => "unknown",
    }
}

/// A card's key from the plan's own cards, or its short id for one the plan
/// doesn't list.
fn key_of(plan: &pb::Plan, task: &[u8]) -> String {
    plan.cards.iter().find(|c| c.task_id == task).map_or_else(|| short(task), |c| c.key.clone())
}

/// A spend as the JSON carries it: a ticket's, an agent's, a landing's
/// lander's, or a theme's sum (ov-306).
fn spend_json(s: &pb::Spend) -> Value {
    json!({
        "input_tokens": s.input_tokens, "output_tokens": s.output_tokens,
        "cache_read_tokens": s.cache_read_tokens, "cache_write_tokens": s.cache_write_tokens,
        "cost_micros": s.cost_micros, "runs": s.runs, "unmeasured_agents": s.unmeasured_agents,
        "shared_agents": s.shared_agents,
    })
}

/// The week's tokens and the harness and model comparison as the JSON carries
/// them (ov-307). `null` from a runner without `board_cost`.
fn cost_json(c: &pb::PlanCost) -> Value {
    json!({
        "week_tokens": c.week_tokens,
        "compare": c.compare.iter().map(|p| json!({
            "harness": p.harness, "model": p.model, "card_share_milli": p.card_share_milli, "tokens": p.tokens,
            "cost_micros": p.cost_micros,
        })).collect::<Vec<_>>(),
        "compare_held_back": c.compare_held_back,
        "in_flight_tokens": c.in_flight_tokens,
        "in_flight_cost_micros": c.in_flight_cost_micros,
        "week": c.week.iter().map(|w| json!({
            "harness": w.harness, "model": w.model, "tokens": w.tokens, "cost_micros": w.cost_micros,
        })).collect::<Vec<_>>(),
        "week_cost_micros": c.week_cost_micros,
        "week_unsplit_cache_write_tokens": c.week_unsplit_cache_write_tokens,
    })
}

/// Status counts as the JSON carries them: a theme's, or the board's (ov-306).
fn counts_json(c: &pb::PlanStatusCounts) -> Value {
    json!({
        "backlog": c.backlog, "todo": c.todo, "needs_decision": c.needs_decision,
        "in_progress": c.in_progress, "in_review": c.in_review, "done": c.done, "cancelled": c.cancelled,
    })
}

fn theme_json(plan: &pb::Plan, view: &pb::BoardThemeView) -> Value {
    let t = view.theme.clone().unwrap_or_default();
    let c = view.counts.unwrap_or_default();
    json!({
        "id": id_text(&t.id),
        "short": short(&t.id),
        "name": t.name,
        "outcome": t.outcome,
        "story": t.story,
        "story_at": t.story_at,
        "next": t.next,
        "owner_ask": t.owner_ask,
        "state": theme_state_word(t.state),
        "ordinal": t.ordinal,
        "cards": view.task_ids.iter().map(|id| json!({ "task": id_text(id), "key": key_of(plan, id) })).collect::<Vec<_>>(),
        "counts": counts_json(&c),
        "spend": spend_json(&view.spend.unwrap_or_default()),
        "budget_tokens": view.budget_tokens,
        "trend_tokens": view.trend_tokens,
        "last_moved_at": view.last_moved_at,
    })
}

/// A pull request stage as the JSON carries it (ov-312): the words the runner
/// chose, and the facts behind them. Left out of the record when the runner
/// said nothing (`with_stage`), as every additive field on this wire is.
pub fn stage_json(s: &pb::PrStage) -> Value {
    let kind = match pb::PrStageKind::try_from(s.kind) {
        Ok(pb::PrStageKind::Building) => "building",
        Ok(pb::PrStageKind::AgentReview) => "agent_review",
        Ok(pb::PrStageKind::Fixing) => "fixing",
        Ok(pb::PrStageKind::WaitingOnReviewer) => "waiting_on_reviewer",
        Ok(pb::PrStageKind::ChangesRequested) => "changes_requested",
        Ok(pb::PrStageKind::ApprovedChecksRunning) => "approved_checks_running",
        Ok(pb::PrStageKind::ApprovedChecksFailing) => "approved_checks_failing",
        Ok(pb::PrStageKind::Approved) => "approved",
        Ok(pb::PrStageKind::ApprovedConflicts) => "approved_conflicts",
        Ok(pb::PrStageKind::Queued) => "queued",
        Ok(pb::PrStageKind::Merged) => "merged",
        Ok(pb::PrStageKind::Closed) => "closed",
        Ok(pb::PrStageKind::Unknown) => "unknown",
        _ => "unspecified",
    };
    json!({
        "kind": kind, "label": s.label, "reviewers": s.reviewers, "queue_position": s.queue_position,
        "checks": match pb::CheckState::try_from(s.checks) {
            Ok(pb::CheckState::Passing) => "passing",
            Ok(pb::CheckState::Failing) => "failing",
            Ok(pb::CheckState::Pending) => "pending",
            _ => "unknown",
        },
        "pr_number": s.pr_number, "pr_url": s.pr_url, "unresolved_threads": s.unresolved_threads,
        "read_at": s.read_at,
    })
}

pub fn with_stage(mut record: Value, stage: &Option<pb::PrStage>) -> Value {
    if let (Some(stage), Some(map)) = (stage, record.as_object_mut()) {
        map.insert("pr_stage".into(), stage_json(stage));
    }
    record
}

/// A ticket's engine run (ov-1040): the facts the core words the note from.
/// Left out of the record when the runner said nothing, as `pr_stage` is.
pub fn with_run(mut record: Value, run: &Option<pb::TicketRun>) -> Value {
    if let (Some(run), Some(map)) = (run, record.as_object_mut()) {
        let mut facts = json!({ "run": run.run, "phase": run.phase, "stage_kind": run.stage_kind, "gate": run.gate, "gate_principal": run.gate_principal });
        if let Some(reason) = &run.reason { facts["reason"] = json!(reason); }
        if let Some(reason) = &run.send_back_refused { facts["send_back_refused"] = json!(reason); }
        map.insert("run".into(), facts);
    }
    record
}

/// A card named by a ticket, a landing or an agent: its id and its key.
fn card_ref(plan: &pb::Plan, task: &[u8]) -> Value {
    json!({ "task": id_text(task), "key": key_of(plan, task) })
}

/// A ticket on the line (ov-872), with its card's key, title and status, who
/// worked it, and its pull request stage when the runner had one to say.
fn line_json(plan: &pb::Plan, l: &pb::TicketLine) -> Value {
    let card = plan.cards.iter().find(|c| c.task_id == l.task_id);
    with_run(with_stage(json!({
        "task": id_text(&l.task_id),
        "key": key_of(plan, &l.task_id),
        "title": card.map(|c| c.title.clone()).unwrap_or_default(),
        "status": card.map_or("Unknown", |c| status_word(c.status)),
        "stage": stage_word(l.stage),
        "reason": l.reason,
        "plan_rank": l.plan_rank,
        "branch": l.branch,
        "worktree_path": l.worktree_path,
        "stack_parent": l.stack_parent.as_deref().map(|p| card_ref(plan, p)),
        "same_branch": l.same_branch.as_deref().map(id_text),
        "pr_number": l.pr_number,
        "landed_sha": l.landed_sha,
        "stage_since": l.stage_since,
        "held_off": l.held_off,
        "stale": l.stale,
        "budget_tokens": l.budget_tokens,
        "fix_rounds": l.fix_rounds,
        "landing": l.landing,
        "work": l.work.iter().map(|w| json!({
            "agent": id_text(&w.agent_id), "role": work_word(w.role), "started_at": w.started_at,
            "ended_at": w.ended_at, "verdict": w.verdict, "linked_by_description": w.linked_by_description,
        })).collect::<Vec<_>>(),
        "spend": spend_json(&l.spend.unwrap_or_default()),
    }), &l.pr_stage), &l.run)
}

fn ruling_state_word(state: i32) -> &'static str {
    match pb::BoardRulingState::try_from(state) {
        Ok(pb::BoardRulingState::Standing) => "standing",
        Ok(pb::BoardRulingState::Confirmed) => "confirmed",
        Ok(pb::BoardRulingState::Reversed) => "reversed",
        _ => "unknown",
    }
}

/// A ruling (ov-304), with its short id and its theme's name ("" when it
/// names none, or one the plan no longer lists).
fn ruling_json(plan: &pb::Plan, r: &pb::BoardRuling) -> Value {
    let theme = r.theme_id.as_ref().and_then(|id| {
        plan.themes.iter().filter_map(|v| v.theme.as_ref()).find(|t| t.id == *id).map(|t| t.name.clone())
    });
    json!({
        "id": id_text(&r.id),
        "short": format!("R-{}", r.number),
        "number": r.number,
        "decision": r.decision,
        "why": r.why,
        "reversal": r.reversal,
        "cards": r.task_ids.iter().enumerate().map(|(i, id)| json!({
            "task": id_text(id), "key": r.task_keys.get(i).cloned().unwrap_or_else(|| key_of(plan, id)),
        })).collect::<Vec<_>>(),
        "theme_id": r.theme_id.as_deref().map(id_text),
        "theme": theme.unwrap_or_default(),
        "state": ruling_state_word(r.state),
        "reversed_sha": r.reversed_sha,
        "note": r.note,
        "actor": r.actor,
        "created_at": r.created_at,
        "settled_by": r.settled_by,
        "settled_at": r.settled_at,
    })
}

fn landing_state_word(state: i32) -> &'static str {
    match pb::LandingState::try_from(state) {
        Ok(pb::LandingState::Open) => "open",
        Ok(pb::LandingState::Gating) => "gating",
        Ok(pb::LandingState::Pushed) => "pushed",
        Ok(pb::LandingState::Green) => "green",
        Ok(pb::LandingState::Red) => "red",
        Ok(pb::LandingState::Landed) => "landed",
        Ok(pb::LandingState::Dropped) => "dropped",
        _ => "unknown",
    }
}

fn route_word(route: i32) -> &'static str {
    match pb::LandingRoute::try_from(route) {
        Ok(pb::LandingRoute::Push) => "push",
        Ok(pb::LandingRoute::PullRequest) => "pull_request",
        Ok(pb::LandingRoute::Ask) => "ask",
        _ => "unknown",
    }
}

fn ci_status_word(status: i32) -> &'static str {
    match pb::BoardCiStatus::try_from(status) {
        Ok(pb::BoardCiStatus::Passed) => "passed",
        Ok(pb::BoardCiStatus::Failed) => "failed",
        Ok(pb::BoardCiStatus::Running) => "running",
        Ok(pb::BoardCiStatus::Queued) => "queued",
        Ok(pb::BoardCiStatus::Superseded) => "superseded",
        Ok(pb::BoardCiStatus::None) => "none",
        _ => "unknown",
    }
}

/// A landing's phase as the JSON spells it (ov-503): `null` when the runner
/// derived none, where `state` is all there is.
fn landing_phase_word(phase: i32) -> Option<&'static str> {
    Some(match pb::LandingPhase::try_from(phase) {
        Ok(pb::LandingPhase::WaitingForPreviousLanding) => "waiting_for_previous_landing",
        Ok(pb::LandingPhase::WaitingForMain) => "waiting_for_main",
        Ok(pb::LandingPhase::WaitingForTickets) => "waiting_for_tickets",
        Ok(pb::LandingPhase::Integrating) => "integrating",
        Ok(pb::LandingPhase::GatingLocally) => "gating_locally",
        Ok(pb::LandingPhase::WaitingForCi) => "waiting_for_ci",
        Ok(pb::LandingPhase::CiFailed) => "ci_failed",
        Ok(pb::LandingPhase::ReadyToLand) => "ready_to_land",
        Ok(pb::LandingPhase::Landed) => "landed",
        Ok(pb::LandingPhase::Stalled) => "stalled",
        Ok(pb::LandingPhase::Dropped) => "dropped",
        _ => return None,
    })
}

/// A landing (ov-309, ov-872), with its tickets named by key.
pub fn landing_json(plan: &pb::Plan, l: &pb::Landing) -> Value {
    let mut record = json!({
        "id": id_text(&l.id),
        "short": short(&l.id),
        "number": l.number,
        "title": l.title,
        "summary": l.summary,
        "route": route_word(l.route),
        "base": l.base,
        "pushed_sha": l.pushed_sha,
        "state": landing_state_word(l.state),
        "state_since": l.state_since,
        "phase": landing_phase_word(l.phase),
        "phase_since": (l.phase != 0).then_some(l.phase_since),
        "collecting": l.collecting,
        "actor": l.actor,
        "created_at": l.created_at,
        "landed_at": l.landed_at,
        "tickets": l.task_ids.iter().map(|id| card_ref(plan, id)).collect::<Vec<_>>(),
        "ci_subject": l.ci_subject,
        "lander": l.lander_id.as_deref().map(id_text),
        "spend": l.spend.as_ref().map(spend_json),
    });
    if let Some(reason) = &l.reason { record["reason"] = json!(reason); }
    record
}

/// An agent at work (ov-872): its role, where it runs, the tickets it works
/// now by key, and what it is doing now.
pub fn agent_json(plan: &pb::Plan, a: &pb::Agent) -> Value {
    json!({
        "id": id_text(&a.id),
        "short": short(&a.id),
        "role": role_word(a.role),
        "harness": a.harness,
        "model": a.model,
        "title": a.title,
        "terminal": a.terminal_id.as_deref().map(id_text),
        "subagent_id": a.subagent_id,
        "parent_terminal": a.parent_terminal_id.as_deref().map(id_text),
        "started_at": a.started_at,
        "stop_asked_at": a.stop_asked_at,
        "tickets": a.tickets.iter().map(|t| json!({
            "task": id_text(&t.task_id), "key": key_of(plan, &t.task_id), "role": work_word(t.role),
            "started_at": t.started_at,
        })).collect::<Vec<_>>(),
        "working": a.working,
        "doing_now": a.doing_now,
        "last_at": a.last_at,
        "spend": spend_json(&a.spend.unwrap_or_default()),
    })
}

/// What the runner last read of one CI subject (ov-309, ov-306).
fn ci_json(r: &pb::BoardCiRead) -> Value {
    json!({
        "subject": r.subject,
        "sha": r.sha,
        "status": ci_status_word(r.status),
        "url": r.url,
        "jobs": r.jobs.iter().map(|j| json!({ "name": j.name, "state": j.state, "url": j.url })).collect::<Vec<_>>(),
        "fetched_at": r.fetched_at,
        "changed_at": r.changed_at,
        "asked_at": r.asked_at,
    })
}

/// Cards that are neither done nor canceled.
fn is_open(status: i32) -> bool {
    status != pb::TaskStatus::Done as i32 && status != pb::TaskStatus::Cancelled as i32
}

/// `plan.get`'s answer: the whole plan, with the two flags the CLI's
/// reconciliation derives, `landed_not_closed` (a card still open whose
/// ticket landed) and `off_the_line` (a card in progress or in review that
/// isn't on the line), and the rulings (ov-304) in the runner's order:
/// standing first, newest first.
pub fn plan_json(plan: &pb::Plan) -> Value {
    let stage_of = |task: &[u8]| plan.lines.iter().find(|l| l.task_id == task).map(|l| l.stage);
    let flagged = |wanted: &dyn Fn(&pb::PlanCard, Option<i32>) -> bool| -> Vec<Value> {
        plan.cards
            .iter()
            .filter(|c| is_open(c.status) && wanted(c, stage_of(&c.task_id)))
            .map(|c| json!({ "task": id_text(&c.task_id), "key": c.key, "status": status_word(c.status) }))
            .collect()
    };
    let mut out = json!({
        "now_ms": plan.now_ms,
        "themes": plan.themes.iter().map(|v| theme_json(plan, v)).collect::<Vec<_>>(),
        "lines": plan.lines.iter().map(|l| line_json(plan, l)).collect::<Vec<_>>(),
        "cards": plan.cards.iter().map(|c| json!({
            "task": id_text(&c.task_id), "key": c.key, "title": c.title, "status": status_word(c.status),
        })).collect::<Vec<_>>(),
        "landed_not_closed": flagged(&|_, stage| stage == Some(pb::TicketStage::Landed as i32)),
        "off_the_line": flagged(&|c, stage| {
            stage.is_none()
                && (c.status == pb::TaskStatus::InProgress as i32 || c.status == pb::TaskStatus::InReview as i32)
        }),
        "rulings": plan.rulings.iter().map(|r| ruling_json(plan, r)).collect::<Vec<_>>(),
        "landings": plan.landings.iter().map(|l| landing_json(plan, l)).collect::<Vec<_>>(),
        "ci": plan.ci.iter().map(ci_json).collect::<Vec<_>>(),
        "board_counts": counts_json(&plan.board_counts.unwrap_or_default()),
        "cost": plan.cost.as_ref().map(cost_json),
        "agents": plan.agents.iter().map(|a| agent_json(plan, a)).collect::<Vec<_>>(),
        "agents_spend": plan.agents_spend.as_ref().map(spend_json),
    });
    if let Some(count) = plan.unthemed_open { out["unthemed_open"] = json!(count); }
    if plan.overview_truncated {
        out["overview_truncated"] = json!(true);
    }
    // Only a runner that sent waits (ov-579): the fixture the Mac decodes keeps its shape.
    if !plan.review_waits.is_empty() {
        out["review_waits"] = plan.review_waits.iter().map(|w| json!({
            "reviewer": w.reviewer, "prs": w.prs, "median_wait_ms": w.median_wait_ms, "samples": w.samples,
        })).collect();
    }
    // Only a runner that sent suggestions (ov-580), for the same reason.
    if !plan.suggested_reviewers.is_empty() {
        out["suggested_reviewers"] = plan.suggested_reviewers.iter().map(|s| json!({
            "task": id_text(&s.task_id), "key": key_of(plan, &s.task_id), "pr": s.pr, "reviewers": s.reviewers,
            "sources": s.sources,
        })).collect();
    }
    out
}

fn event_json(e: &pb::PlanEvent) -> Value {
    json!({
        "at": e.at, "actor": e.actor, "kind": e.kind, "body": e.body,
        "extra": serde_json::from_str::<Value>(&e.extra_json).unwrap_or(Value::Null),
    })
}

/// `plan.events`' answer: `{"events": [...]}`, the record a page reads (the
/// CLI's `show --json` carries the same list beside its subject).
pub fn events_json(list: &pb::PlanEventList) -> Value {
    json!({ "events": list.events.iter().map(event_json).collect::<Vec<_>>() })
}

#[cfg(test)]
#[path = "plan_json_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "plan_json_stage_tests.rs"]
mod stage_tests;
