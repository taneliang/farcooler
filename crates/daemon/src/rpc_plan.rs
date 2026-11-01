//! The plan layer's routes (ov-268, ov-872): `plan.*` and `board_theme.*`
//! behind `board_plan`, and the line's (`line.*`, `agent.*`) behind
//! `board_line`.
//!
//! Beside the board: nothing here touches `task_ops`. Each write announces
//! `plan_changed` once, and none emits `task_changed` itself.
//!
//! The arms live here; `Rpc::dispatch` names every route in one arm, so its
//! coverage test still reads each name out of that file.

use std::collections::HashSet;
use std::sync::Arc;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, Scope, request, result};
use farcooler_store::agents::{AgentPlace, AgentRole, NewAgent};
use farcooler_store::models::TaskStatus;
use farcooler_store::plan::{BoardTheme, NewTheme, ThemeState, ThemeUpdate};
use farcooler_store::plan_agents::AgentView;
use farcooler_store::plan_cost::PlanCost;
use farcooler_store::plan_read::{CardRef, LineView, Plan, Spend, StatusCounts, ThemeView};
use farcooler_store::ticket_lines::{LineEvent, LineSubject, LineUpdate, Stage};
use farcooler_store::ticket_work::{LinkedBy, Verdict, WorkRole};
use uuid::Uuid;

use crate::service::Service;
use crate::task_ops::{actor_from_wire, required_id};
use crate::watch::Watcher;
use crate::wire::id_bytes;

/// Finished tickets and landings the plan read keeps showing, unless the
/// request asks for all.
const CLOSED_FOR_MS: i64 = 7 * 24 * 60 * 60 * 1000;

fn payload_missing() -> DomainError {
    DomainError::InvalidArgument { what: "payload" }
}

/// One plan route, as `Rpc::dispatch` hands it over. `scope` is the
/// connection's, which decides whether a ticket's worktree path is shown.
pub(crate) async fn dispatch(svc: &Arc<Service>, watcher: &Arc<Watcher>, scope: Scope, req: Request) -> Result<result::Value> {
    let admin = scope == Scope::HostAdmin;
    let control = matches!(scope, Scope::Control | Scope::HostAdmin);
    match req.method.as_str() {
        "plan.get" => {
            let Some(request::Payload::PlanGet(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let closed_since = if p.include_closed { i64::MIN } else { now() - CLOSED_FOR_MS };
            Ok(result::Value::Plan(read_plan(svc, watcher, workspace, closed_since, admin, control).await?))
        }
        "plan.events" => {
            let Some(request::Payload::PlanEvents(p)) = req.payload else { return Err(payload_missing()) };
            let subject = match p.subject {
                Some(pb::plan_events_request::Subject::ThemeId(id)) => {
                    let id = required_id(&id)?;
                    svc.store.board_theme(id)?;
                    LineSubject::Theme(id)
                }
                Some(pb::plan_events_request::Subject::TaskId(id)) => {
                    let id = required_id(&id)?;
                    svc.store.get_task(id)?;
                    LineSubject::Ticket(id)
                }
                Some(pb::plan_events_request::Subject::LandingId(id)) => {
                    let id = required_id(&id)?;
                    svc.store.landing(id)?;
                    LineSubject::Landing(id)
                }
                None => return Err(DomainError::InvalidArgument { what: "subject" }),
            };
            let events = svc.store.line_events(subject, p.since_ms)?;
            Ok(result::Value::PlanEventList(pb::PlanEventList { events: events.iter().map(pb_event).collect() }))
        }
        "line.order" => {
            let Some(request::Payload::LineOrder(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            svc.store.set_line_order(workspace, &ids(&p.task_ids)?, actor)?;
            watcher.announce_plan_changed(workspace, actor);
            Ok(result::Value::Plan(read_plan(svc, watcher, workspace, now() - CLOSED_FOR_MS, admin, control).await?))
        }
        "line.set" => {
            let Some(request::Payload::LineSet(p)) = req.payload else { return Err(payload_missing()) };
            let task = required_id(&p.task_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let stage = p.stage.map(stage_of).transpose()?;
            let was = svc.store.ensure_on_line(task, actor)?.stage;
            let update = LineUpdate {
                stage,
                reason: p.reason,
                branch: p.branch,
                worktree_id: p.worktree_id.as_deref().map(|id| required_id(id).map(Some)).transpose()?,
                worktree_path: p.worktree_path,
                stack_parent: p
                    .stack_parent
                    .as_deref()
                    .map(|id| if id.is_empty() { Ok(None) } else { required_id(id).map(Some) })
                    .transpose()?,
                pr_number: p.pr_number.map(|n| (n > 0).then_some(n)),
                landed_sha: p.landed_sha,
                held_off: None,
            };
            let line = svc.store.update_line(task, &update, actor)?;
            if let Some(tokens) = p.budget_tokens {
                svc.store.set_budget(LineSubject::Ticket(task), (tokens > 0).then_some(tokens), actor)?;
            }
            let workspace = svc.store.get_task(task)?.workspace_id;
            watcher.announce_plan_changed(workspace, actor);
            // A ticket passing review may have sent the collecting landing off.
            watcher.tell_departures();
            // A ticket that moved into review, a fix or landing wants its pull
            // request read now; any other update does not.
            if crate::plan_stage::enters_active(was, stage) {
                if let Ok(ws) = svc.store.get_workspace(workspace) {
                    crate::pr_watch::kick_ticket(line.task_id, ws.repository_id);
                }
            }
            Ok(result::Value::TicketLine(line_view(svc, workspace, task, admin)?))
        }
        "agent.record" => {
            let Some(request::Payload::AgentRecord(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let role = role_of(p.role)?;
            let place = match (p.terminal_id.as_deref(), p.subagent_id.trim()) {
                (Some(terminal), "") => AgentPlace::Pane { terminal: required_id(terminal)? },
                (None, id) if !id.is_empty() => {
                    AgentPlace::Subagent { id: id.to_string(), parent: p.parent_terminal_id.as_deref().map(required_id).transpose()? }
                }
                _ => return Err(DomainError::InvalidArgument { what: "place" }),
            };
            let new = NewAgent {
                role,
                place,
                harness: p.harness,
                session_id: p.session_id,
                session_cwd: None,
                model: p.model,
                title: p.title,
            };
            let agent = svc.store.record_agent(workspace, &new)?;
            let work = farcooler_store::agent_panes::dispatch_role(role);
            for task in ids(&p.task_ids)? {
                svc.store.ensure_on_line(task, actor)?;
                svc.store.start_work(agent.id, task, work, LinkedBy::Orchestrator, actor)?;
                start_card(svc, watcher, task, work, actor)?;
            }
            watcher.announce_plan_changed(workspace, actor);
            Ok(result::Value::Agent(agent_view(svc, watcher, agent.id).await?))
        }
        "agent.work" => {
            let Some(request::Payload::AgentWork(p)) = req.payload else { return Err(payload_missing()) };
            let agent = required_id(&p.agent_id)?;
            let task = required_id(&p.task_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let role = work_role_of(p.role)?;
            match p.verdict.as_deref() {
                Some(word) => {
                    let verdict = Verdict::parse(word.trim()).ok_or(DomainError::InvalidArgument { what: "verdict" })?;
                    svc.store.end_work(agent, task, role, verdict, actor)?;
                }
                None => {
                    svc.store.ensure_on_line(task, actor)?;
                    svc.store.start_work(agent, task, role, LinkedBy::Orchestrator, actor)?;
                    start_card(svc, watcher, task, role, actor)?;
                }
            }
            let workspace = svc.store.agent(agent)?.workspace_id;
            watcher.announce_plan_changed(workspace, actor);
            Ok(result::Value::Agent(agent_view(svc, watcher, agent).await?))
        }
        "agent.end" => {
            let Some(request::Payload::AgentEnd(p)) = req.payload else { return Err(payload_missing()) };
            let agent = required_id(&p.agent_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let verdict = Verdict::parse(p.verdict.trim()).ok_or(DomainError::InvalidArgument { what: "verdict" })?;
            let ended = svc.store.end_agent(agent, verdict)?;
            watcher.announce_plan_changed(ended.workspace_id, actor);
            Ok(result::Value::Agent(pb_agent_row(&ended)))
        }
        "board_theme.create" => {
            let Some(request::Payload::BoardThemeCreate(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let tasks = ids(&p.task_ids)?;
            let new = NewTheme { name: p.name, outcome: p.outcome };
            let theme = svc.store.create_theme(workspace, &new, &tasks, actor)?;
            watcher.announce_plan_changed(workspace, actor);
            Ok(result::Value::BoardThemeView(theme_view(svc, theme)?))
        }
        "board_theme.update" => {
            let Some(request::Payload::BoardThemeUpdate(p)) = req.payload else { return Err(payload_missing()) };
            let theme = required_id(&p.theme_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let state = p.state.map(theme_state_of).transpose()?;
            let update = ThemeUpdate {
                name: p.name,
                outcome: p.outcome,
                story: p.story,
                next: p.next,
                owner_ask: p.owner_ask,
                state,
                ordinal: p.ordinal,
            };
            let theme = svc.store.update_theme(theme, &update, actor)?;
            if let Some(tokens) = p.budget_tokens {
                svc.store.set_budget(LineSubject::Theme(theme.id), (tokens > 0).then_some(tokens), actor)?;
            }
            watcher.announce_plan_changed(theme.workspace_id, actor);
            Ok(result::Value::BoardThemeView(theme_view(svc, theme)?))
        }
        "board_theme.move" => {
            let Some(request::Payload::BoardThemeMove(p)) = req.payload else { return Err(payload_missing()) };
            let theme = required_id(&p.theme_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let theme = svc.store.move_theme(theme, p.to, actor)?;
            watcher.announce_plan_changed(theme.workspace_id, actor);
            Ok(result::Value::BoardThemeView(theme_view(svc, theme)?))
        }
        "board_theme.cards" => {
            let Some(request::Payload::BoardThemeCards(p)) = req.payload else { return Err(payload_missing()) };
            let theme = required_id(&p.theme_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let theme = svc.store.theme_cards(theme, &ids(&p.add)?, &ids(&p.remove)?, actor)?;
            watcher.announce_plan_changed(theme.workspace_id, actor);
            Ok(result::Value::BoardThemeView(theme_view(svc, theme)?))
        }
        "ruling.add" | "ruling.set" | "ruling.keep_all" => crate::rpc_rulings::dispatch(svc, watcher, req),
        "landing.start" | "landing.set" => crate::rpc_landings::dispatch(svc, watcher, req).await,
        other => {
            tracing::error!(method = %other, "a plan route with no handler");
            Err(DomainError::NotFound)
        }
    }
}

fn now() -> i64 {
    crate::review::now_millis()
}

/// An agent set to build or fix a card that hasn't started starts it: a
/// backlog or todo card moves to in progress, with its own note and
/// `task_changed`, as a worker recorded on it always has (ov-213).
fn start_card(svc: &Service, watcher: &Watcher, task: Uuid, role: WorkRole, actor: farcooler_store::models::Actor) -> Result<()> {
    if !matches!(role, WorkRole::Build | WorkRole::Fix) {
        return Ok(());
    }
    if matches!(svc.store.get_task(task)?.status, TaskStatus::Backlog | TaskStatus::Todo) {
        let moved = svc.store.set_task_status(task, TaskStatus::InProgress, actor)?;
        watcher.announce_task_changed(&moved, None, actor);
    }
    Ok(())
}

fn ids(raw: &[bytes::Bytes]) -> Result<Vec<Uuid>> {
    raw.iter().map(|id| required_id(id)).collect()
}

fn theme_state_of(raw: i32) -> Result<ThemeState> {
    match pb::BoardThemeState::try_from(raw) {
        Ok(pb::BoardThemeState::Active) => Ok(ThemeState::Active),
        Ok(pb::BoardThemeState::Paused) => Ok(ThemeState::Paused),
        Ok(pb::BoardThemeState::Done) => Ok(ThemeState::Done),
        Ok(pb::BoardThemeState::Dropped) => Ok(ThemeState::Dropped),
        _ => Err(DomainError::InvalidArgument { what: "state" }),
    }
}

fn stage_of(raw: i32) -> Result<Stage> {
    Ok(match pb::TicketStage::try_from(raw) {
        Ok(pb::TicketStage::Waiting) => Stage::Waiting,
        Ok(pb::TicketStage::Building) => Stage::Building,
        Ok(pb::TicketStage::Reviewing) => Stage::Reviewing,
        Ok(pb::TicketStage::NeedsYou) => Stage::NeedsYou,
        Ok(pb::TicketStage::Landing) => Stage::Landing,
        Ok(pb::TicketStage::Landed) => Stage::Landed,
        Ok(pb::TicketStage::Dropped) => Stage::Dropped,
        _ => return Err(DomainError::InvalidArgument { what: "stage" }),
    })
}

pub(crate) fn pb_stage_of(s: Stage) -> pb::TicketStage {
    match s {
        Stage::Waiting => pb::TicketStage::Waiting,
        Stage::Building => pb::TicketStage::Building,
        Stage::Reviewing => pb::TicketStage::Reviewing,
        Stage::NeedsYou => pb::TicketStage::NeedsYou,
        Stage::Landing => pb::TicketStage::Landing,
        Stage::Landed => pb::TicketStage::Landed,
        Stage::Dropped => pb::TicketStage::Dropped,
    }
}

fn role_of(raw: i32) -> Result<AgentRole> {
    Ok(match pb::AgentRole::try_from(raw) {
        Ok(pb::AgentRole::Orchestrator) => AgentRole::Orchestrator,
        Ok(pb::AgentRole::Builder) => AgentRole::Builder,
        Ok(pb::AgentRole::Reviewer) => AgentRole::Reviewer,
        Ok(pb::AgentRole::Lander) => AgentRole::Lander,
        _ => return Err(DomainError::InvalidArgument { what: "role" }),
    })
}

pub(crate) fn pb_role(r: AgentRole) -> pb::AgentRole {
    match r {
        AgentRole::Orchestrator => pb::AgentRole::Orchestrator,
        AgentRole::Builder => pb::AgentRole::Builder,
        AgentRole::Reviewer => pb::AgentRole::Reviewer,
        AgentRole::Lander => pb::AgentRole::Lander,
    }
}

fn work_role_of(raw: i32) -> Result<WorkRole> {
    Ok(match pb::WorkRole::try_from(raw) {
        Ok(pb::WorkRole::Build) => WorkRole::Build,
        Ok(pb::WorkRole::Fix) => WorkRole::Fix,
        Ok(pb::WorkRole::Review) => WorkRole::Review,
        Ok(pb::WorkRole::Land) => WorkRole::Land,
        _ => return Err(DomainError::InvalidArgument { what: "role" }),
    })
}

fn pb_work_role(r: WorkRole) -> pb::WorkRole {
    match r {
        WorkRole::Build => pb::WorkRole::Build,
        WorkRole::Fix => pb::WorkRole::Fix,
        WorkRole::Review => pb::WorkRole::Review,
        WorkRole::Land => pb::WorkRole::Land,
    }
}

// ---------------------------------------------------------------------------
// store to wire
// ---------------------------------------------------------------------------

/// The plan as the wire carries it, read with its agents, each agent's
/// doing-now line, and each ticket's pull request stage from what the runner
/// last read of GitHub (ov-312). Cache only: nothing here waits on `gh`.
///
/// `review_waits` and `suggested_reviewers` name GitHub logins, and go only
/// to a peer at Control scope (ov-315).
pub(crate) async fn read_plan(
    svc: &Service,
    watcher: &Watcher,
    workspace: Uuid,
    closed_since: i64,
    admin: bool,
    control: bool,
) -> Result<pb::Plan> {
    let plan = svc.store.plan_with_agents(workspace, closed_since, &|t| live_working(svc, t))?;
    let mut wire = pb_plan(svc, &plan, admin)?;
    for (view, agent) in plan.agents.iter().zip(wire.agents.iter_mut()) {
        agent.doing_now = doing_now(svc, watcher, view).await;
    }
    crate::plan_stage::annotate(svc, &plan, &mut wire);
    if control {
        crate::plan_stage::annotate_waits(svc, workspace, &plan, &mut wire);
        crate::pr_suggest::annotate(svc, &plan, &mut wire);
    }
    Ok(wire)
}

/// What an agent is doing now, in a line: a pane's signal line, a subagent's
/// last tool call. Empty when the runner can't see it.
async fn doing_now(svc: &Service, watcher: &Watcher, view: &AgentView) -> String {
    match &view.agent.place {
        AgentPlace::Pane { terminal } => watcher.signal(*terminal).await.unwrap_or_default(),
        AgentPlace::Subagent { id, .. } => svc.worker_seen.get(id).map(|s| s.doing).unwrap_or_default(),
    }
}

fn theme_view(svc: &Service, theme: BoardTheme) -> Result<pb::BoardThemeView> {
    let plan = svc.store.plan(theme.workspace_id, i64::MIN)?;
    let view = plan.themes.into_iter().find(|v| v.theme.id == theme.id);
    // A dropped theme leaves the read; answer with it, empty.
    Ok(pb_theme_view(&view.unwrap_or(ThemeView {
        theme,
        tasks: Vec::new(),
        counts: StatusCounts::default(),
        spend: Spend::default(),
        budget_tokens: None,
        trend: [0; 7],
        last_moved_at: 0,
    })))
}

/// One ticket as the plan read says it, with its pull request stage.
fn line_view(svc: &Service, workspace: Uuid, task: Uuid, admin: bool) -> Result<pb::TicketLine> {
    let plan = svc.store.plan(workspace, i64::MIN)?;
    let view = plan.lines.iter().find(|v| v.line.task_id == task).ok_or(DomainError::NotFound)?;
    let mut wire = pb_line(view, admin);
    if let Ok(ws) = svc.store.get_workspace(workspace) {
        let reads = crate::plan_stage::Reads::of(svc, ws.repository_id);
        wire.pr_stage = crate::plan_stage::line_stage(view, &plan.cards, &reads).as_ref().map(crate::plan_stage::pb_stage);
    }
    Ok(wire)
}

/// One agent as the plan read says it, or as it was recorded when the read
/// leaves it out (one whose pane isn't running yet).
async fn agent_view(svc: &Service, watcher: &Watcher, agent: Uuid) -> Result<pb::Agent> {
    let row = svc.store.agent(agent)?;
    let plan = svc.store.plan_with_agents(row.workspace_id, i64::MIN, &|t| live_working(svc, t))?;
    match plan.agents.iter().find(|a| a.agent.id == agent) {
        Some(view) => {
            let mut wire = pb_agent(view);
            wire.doing_now = doing_now(svc, watcher, view).await;
            Ok(wire)
        }
        None => {
            let mut wire = pb_agent_row(&row);
            wire.tickets = svc
                .store
                .work_of(agent)?
                .into_iter()
                .filter(|w| w.ended_at.is_none())
                .map(|w| pb::AgentTicket { task_id: id_bytes(w.task_id), role: pb_work_role(w.role) as i32, started_at: w.started_at })
                .collect();
            Ok(wire)
        }
    }
}

fn pb_theme(t: &BoardTheme) -> pb::BoardTheme {
    pb::BoardTheme {
        id: id_bytes(t.id),
        workspace_id: id_bytes(t.workspace_id),
        name: t.name.clone(),
        outcome: t.outcome.clone(),
        story: t.story.clone(),
        next: t.next.clone(),
        owner_ask: t.owner_ask.clone(),
        state: (match t.state {
            ThemeState::Active => pb::BoardThemeState::Active,
            ThemeState::Paused => pb::BoardThemeState::Paused,
            ThemeState::Done => pb::BoardThemeState::Done,
            ThemeState::Dropped => pb::BoardThemeState::Dropped,
        }) as i32,
        ordinal: t.ordinal,
        story_at: t.story_at,
        created_at: t.created_at,
        resource_version: t.resource_version,
    }
}

fn pb_counts(c: &StatusCounts) -> pb::PlanStatusCounts {
    pb::PlanStatusCounts {
        backlog: c.backlog,
        todo: c.todo,
        needs_decision: c.needs_decision,
        in_progress: c.in_progress,
        in_review: c.in_review,
        done: c.done,
        cancelled: c.cancelled,
    }
}

fn pb_theme_view(v: &ThemeView) -> pb::BoardThemeView {
    pb::BoardThemeView {
        theme: Some(pb_theme(&v.theme)),
        task_ids: v.tasks.iter().map(|id| id_bytes(*id)).collect(),
        counts: Some(pb_counts(&v.counts)),
        spend: Some(pb_spend(&v.spend)),
        budget_tokens: v.budget_tokens,
        trend_tokens: v.trend.to_vec(),
        last_moved_at: (v.last_moved_at > 0).then_some(v.last_moved_at),
    }
}

/// The week's tokens and the harness and model comparison (ov-307).
fn pb_cost(c: &PlanCost) -> pb::PlanCost {
    pb::PlanCost {
        week_tokens: c.week_tokens,
        compare: c
            .compare
            .iter()
            .map(|p| pb::HarnessModelCost {
                harness: p.harness.clone(),
                model: p.model.clone(),
                card_share_milli: p.card_share_milli,
                tokens: p.tokens,
                cost_micros: p.cost_micros,
            })
            .collect(),
        compare_held_back: c.compare_held_back,
        in_flight_tokens: c.in_flight_tokens,
        in_flight_cost_micros: c.in_flight_cost_micros,
        week: c
            .week
            .iter()
            .map(|w| pb::WeekSpend {
                harness: w.harness.clone(),
                model: w.model.clone(),
                tokens: w.tokens,
                cost_micros: w.cost_micros,
            })
            .collect(),
        week_cost_micros: c.week_cost_micros,
        week_unsplit_cache_write_tokens: c.week_unsplit_cache_write_tokens,
    }
}

pub(crate) fn pb_spend(s: &Spend) -> pb::Spend {
    pb::Spend {
        input_tokens: s.input_tokens,
        output_tokens: s.output_tokens,
        cache_read_tokens: s.cache_read_tokens,
        cache_write_tokens: s.cache_write_tokens,
        cost_micros: s.cost_micros,
        runs: s.runs,
        unmeasured_agents: s.unmeasured_agents,
        shared_agents: s.shared_agents,
    }
}

/// A ticket on the line as the wire carries it. The worktree path is a runner
/// path, so it is shown only to `host_admin`, as every other path on the wire
/// is.
pub(crate) fn pb_line(v: &LineView, admin: bool) -> pb::TicketLine {
    let l = &v.line;
    pb::TicketLine {
        task_id: id_bytes(l.task_id),
        stage: pb_stage_of(l.stage) as i32,
        reason: l.reason.clone(),
        branch: l.branch.clone(),
        worktree_id: l.worktree_id.map(id_bytes),
        worktree_path: if admin { l.worktree_path.clone() } else { String::new() },
        stack_parent: l.stack_parent.map(id_bytes),
        same_branch: l.same_branch.map(id_bytes),
        pr_number: l.pr_number,
        landed_sha: l.landed_sha.clone(),
        plan_rank: l.plan_rank,
        stage_since: l.stage_since,
        held_off: l.held_off,
        created_at: l.created_at,
        resource_version: l.resource_version,
        work: v
            .work
            .iter()
            .map(|w| pb::TicketWork {
                agent_id: id_bytes(w.agent_id),
                task_id: id_bytes(w.task_id),
                role: pb_work_role(w.role) as i32,
                started_at: w.started_at,
                ended_at: w.ended_at,
                verdict: w.verdict.map(|v| v.as_str().to_string()).unwrap_or_default(),
                linked_by_description: w.linked_by == LinkedBy::Description,
            })
            .collect(),
        fix_rounds: v.fix_rounds,
        spend: Some(pb_spend(&v.spend)),
        stale: v.stale,
        budget_tokens: v.budget_tokens,
        pr_stage: None,
        landing: v.landing,
    }
}

fn pb_status(status: TaskStatus) -> i32 {
    (match status {
        TaskStatus::Backlog => pb::TaskStatus::Backlog,
        TaskStatus::Todo => pb::TaskStatus::Todo,
        TaskStatus::NeedsDecision => pb::TaskStatus::NeedsDecision,
        TaskStatus::InProgress => pb::TaskStatus::InProgress,
        TaskStatus::InReview => pb::TaskStatus::InReview,
        TaskStatus::Done => pb::TaskStatus::Done,
        TaskStatus::Cancelled => pb::TaskStatus::Cancelled,
    }) as i32
}

fn pb_card(c: &CardRef) -> pb::PlanCard {
    pb::PlanCard { task_id: id_bytes(c.task_id), key: c.key.clone(), title: c.title.clone(), status: pb_status(c.status) }
}

/// The plan as the wire carries it. The cards are every card a theme, ticket,
/// landing or agent names, so a client draws an agent's tickets by key and
/// title whether or not they are on the line.
fn pb_plan(svc: &Service, p: &Plan, admin: bool) -> Result<pb::Plan> {
    let mut cards: Vec<pb::PlanCard> = p.cards.iter().map(pb_card).collect();
    let mut named: HashSet<Uuid> = p.cards.iter().map(|c| c.task_id).collect();
    for task in p.agents.iter().flat_map(|a| a.tickets.iter().map(|w| w.task_id)) {
        if named.insert(task)
            && let Ok(t) = svc.store.get_task(task)
        {
            cards.push(pb_card(&CardRef { task_id: t.id, key: t.key, title: t.title, status: t.status }));
        }
    }
    Ok(pb::Plan {
        now_ms: p.now_ms,
        themes: p.themes.iter().map(pb_theme_view).collect(),
        cards,
        rulings: p.rulings.iter().map(crate::rpc_rulings::pb_ruling).collect(),
        ci: p.ci.iter().map(crate::rpc_landings::pb_ci).collect(),
        board_counts: Some(pb_counts(&p.board_counts)),
        cost: Some(pb_cost(&p.cost)),
        review_waits: Vec::new(),
        lines: p.lines.iter().map(|l| pb_line(l, admin)).collect(),
        landings: p.landings.iter().map(crate::rpc_landings::pb_landing_view).collect(),
        agents: p.agents.iter().map(pb_agent).collect(),
        agents_spend: Some(pb_spend(&p.agents_spend)),
        suggested_reviewers: Vec::new(),
    })
}

/// Whether the daemon's live session for `terminal` is mid-turn: the state the
/// agents show. `None` when it has no state worth trusting (a shell, a pane it
/// can't read), so the store falls back to how recently the pane spoke.
fn live_working(svc: &Service, terminal: Uuid) -> Option<bool> {
    use pb::AgentActivity as A;
    match svc.live_activity(terminal) {
        A::Working => Some(true),
        A::Idle | A::Blocked | A::Done => Some(false),
        A::Unspecified | A::None | A::Unknown => None,
    }
}

/// An agent as it was recorded, with nothing observed.
pub(crate) fn pb_agent_row(a: &farcooler_store::agents::Agent) -> pb::Agent {
    let (terminal, subagent, parent) = match &a.place {
        AgentPlace::Pane { terminal } => (Some(id_bytes(*terminal)), String::new(), None),
        AgentPlace::Subagent { id, parent } => (None, id.clone(), parent.map(id_bytes)),
    };
    pb::Agent {
        id: id_bytes(a.id),
        role: pb_role(a.role) as i32,
        harness: a.harness.clone(),
        model: a.model.clone(),
        title: a.title.clone(),
        terminal_id: terminal,
        subagent_id: subagent,
        parent_terminal_id: parent,
        started_at: a.started_at,
        stop_asked_at: a.stop_asked_at,
        tickets: Vec::new(),
        working: false,
        doing_now: String::new(),
        last_at: 0,
        spend: None,
    }
}

/// An agent on the plan, as the wire names it.
fn pb_agent(v: &AgentView) -> pb::Agent {
    let mut wire = pb_agent_row(&v.agent);
    wire.title = v.label.clone();
    wire.tickets = v
        .tickets
        .iter()
        .map(|w| pb::AgentTicket { task_id: id_bytes(w.task_id), role: pb_work_role(w.role) as i32, started_at: w.started_at })
        .collect();
    wire.working = v.working;
    wire.last_at = v.last_at;
    wire.spend = Some(pb_spend(&v.spend));
    wire
}

fn pb_event(e: &LineEvent) -> pb::PlanEvent {
    pb::PlanEvent {
        id: id_bytes(e.id),
        at: e.at,
        actor: e.actor.clone(),
        kind: e.kind.clone(),
        body: e.body.clone(),
        extra_json: e.extra.to_string(),
    }
}
