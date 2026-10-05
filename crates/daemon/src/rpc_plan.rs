//! The plan layer's routes (ov-268): `plan.*`, `board_theme.*` and `lane.*`,
//! behind `board_plan`.
//!
//! Experimental, and beside the board: nothing here touches `task_ops`. Each
//! write announces `plan_changed` once, and none emits `task_changed` itself.
//! The one write that reaches a task is a lane's agent (`lane.agent`, or an
//! `agent` on `lane.create` or `lane.update`), which writes a `worker` note,
//! moves a backlog card to in progress and announces `task_changed` once, all
//! through ov-213's own path. It records the agent
//! as a worker on the lane's first open card through the existing
//! `task.worker` (ov-213), so the runner reads the agent's spend. That is valid
//! without the layer, and stays valid after the layer is removed.
//!
//! The arms live here; `Rpc::dispatch` names every route in one arm, so its
//! coverage test still reads each name out of that file.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, Scope, request, result};
use farcooler_store::models::TaskStatus;
use farcooler_store::plan::{
    AgentRecord, AgentRole, BoardTheme, Lane, LaneCard, LaneState, LaneUpdate, NewLane, NewTheme, PlanEvent, Subject,
    ThemeState, ThemeUpdate,
};
use farcooler_store::plan_read::{LaneSpend, LaneView, Plan, StatusCounts, ThemeView};
use uuid::Uuid;

use crate::service::Service;
use crate::task_ops::{actor_from_wire, required_id};
use crate::watch::Watcher;
use crate::wire::id_bytes;

/// Finished lanes the plan read keeps showing, unless the request asks for all.
const CLOSED_LANES_FOR_MS: i64 = 7 * 24 * 60 * 60 * 1000;

fn payload_missing() -> DomainError {
    DomainError::InvalidArgument { what: "payload" }
}

/// One plan route, as `Rpc::dispatch` hands it over. `scope` is the
/// connection's, which decides whether a lane's worktree path is shown.
pub(crate) async fn dispatch(svc: &Service, watcher: &Watcher, scope: Scope, req: Request) -> Result<result::Value> {
    let admin = scope == Scope::HostAdmin;
    match req.method.as_str() {
        "plan.get" => {
            let Some(request::Payload::PlanGet(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let closed_since = if p.include_closed { i64::MIN } else { now() - CLOSED_LANES_FOR_MS };
            Ok(result::Value::Plan(pb_plan(&svc.store.plan(workspace, closed_since)?, admin)))
        }
        "plan.events" => {
            let Some(request::Payload::PlanEvents(p)) = req.payload else { return Err(payload_missing()) };
            let subject = match p.subject {
                Some(pb::plan_events_request::Subject::ThemeId(id)) => {
                    let id = required_id(&id)?;
                    svc.store.board_theme(id)?;
                    Subject::Theme(id)
                }
                Some(pb::plan_events_request::Subject::LaneId(id)) => {
                    let id = required_id(&id)?;
                    svc.store.lane(id)?;
                    Subject::Lane(id)
                }
                None => return Err(DomainError::InvalidArgument { what: "subject" }),
            };
            let events = svc.store.plan_events(subject, p.since_ms)?;
            Ok(result::Value::PlanEventList(pb::PlanEventList { events: events.iter().map(pb_event).collect() }))
        }
        "plan.set" => {
            let Some(request::Payload::PlanSet(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let lanes = p.lane_ids.iter().map(|id| required_id(id)).collect::<Result<Vec<_>>>()?;
            svc.store.set_plan(workspace, &lanes, actor)?;
            watcher.announce_plan_changed(workspace, actor);
            Ok(result::Value::Plan(pb_plan(&svc.store.plan(workspace, now() - CLOSED_LANES_FOR_MS)?, admin)))
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
        "lane.create" => {
            let Some(request::Payload::LaneCreate(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let cards = lane_cards(&p.cards)?;
            let agent = p.agent.as_ref().map(agent_of).transpose()?;
            let new = NewLane {
                name: p.name,
                reason: p.reason,
                worktree_id: p.worktree_id.as_deref().map(required_id).transpose()?,
                worktree_path: p.worktree_path,
                branch: p.branch,
                harness: p.harness,
                model: p.model,
            };
            let lane = svc.store.create_lane(workspace, &new, &cards, agent.as_ref(), actor)?;
            watcher.announce_plan_changed(workspace, actor);
            if let Some(agent) = &agent {
                record_worker(svc, watcher, &lane, agent, &p.actor);
            }
            Ok(result::Value::Lane(lane_view(svc, &lane, admin)?))
        }
        "lane.update" => {
            let Some(request::Payload::LaneUpdate(p)) = req.payload else { return Err(payload_missing()) };
            let lane = required_id(&p.lane_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let agent = p.agent.as_ref().map(agent_of).transpose()?;
            let update = LaneUpdate {
                state: p.state.map(lane_state_of).transpose()?,
                reason: p.reason,
                train: p.train,
                landed_sha: p.landed_sha,
                worktree_id: p.worktree_id.as_deref().map(required_id).transpose()?,
                worktree_path: p.worktree_path,
                branch: p.branch,
                agent: agent.clone(),
            };
            let lane = svc.store.update_lane(lane, &update, actor)?;
            watcher.announce_plan_changed(lane.workspace_id, actor);
            if let Some(agent) = &agent {
                record_worker(svc, watcher, &lane, agent, &p.actor);
            }
            Ok(result::Value::Lane(lane_view(svc, &lane, admin)?))
        }
        "lane.cards" => {
            let Some(request::Payload::LaneCards(p)) = req.payload else { return Err(payload_missing()) };
            let lane = required_id(&p.lane_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let lane = svc.store.lane_cards(lane, &lane_cards(&p.add)?, &lane_cards(&p.remove)?, actor)?;
            watcher.announce_plan_changed(lane.workspace_id, actor);
            Ok(result::Value::Lane(lane_view(svc, &lane, admin)?))
        }
        "lane.agent" => {
            let Some(request::Payload::LaneAgentSet(p)) = req.payload else { return Err(payload_missing()) };
            let lane = required_id(&p.lane_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let agent = agent_of(p.agent.as_ref().ok_or(DomainError::InvalidArgument { what: "agent" })?)?;
            let lane = svc.store.record_lane_agent(lane, &agent, actor)?;
            watcher.announce_plan_changed(lane.workspace_id, actor);
            record_worker(svc, watcher, &lane, &agent, &p.actor);
            Ok(result::Value::Lane(lane_view(svc, &lane, admin)?))
        }
        "ruling.add" | "ruling.set" => crate::rpc_rulings::dispatch(svc, watcher, req),
        "train.start" | "train.set" => crate::rpc_trains::dispatch(svc, watcher, req),
        other => {
            tracing::error!(method = %other, "a plan route with no handler");
            Err(DomainError::NotFound)
        }
    }
}

fn now() -> i64 {
    crate::review::now_millis()
}

fn ids(raw: &[bytes::Bytes]) -> Result<Vec<Uuid>> {
    raw.iter().map(|id| required_id(id)).collect()
}

fn lane_cards(raw: &[pb::LaneCard]) -> Result<Vec<LaneCard>> {
    raw.iter().map(|c| Ok(LaneCard { task_id: required_id(&c.task_id)?, slice: c.slice.clone() })).collect()
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

fn lane_state_of(raw: i32) -> Result<LaneState> {
    match pb::LaneState::try_from(raw) {
        Ok(pb::LaneState::Queued) => Ok(LaneState::Queued),
        Ok(pb::LaneState::Building) => Ok(LaneState::Building),
        Ok(pb::LaneState::Review) => Ok(LaneState::Review),
        Ok(pb::LaneState::Fixing) => Ok(LaneState::Fixing),
        Ok(pb::LaneState::Landing) => Ok(LaneState::Landing),
        Ok(pb::LaneState::Landed) => Ok(LaneState::Landed),
        Ok(pb::LaneState::Dropped) => Ok(LaneState::Dropped),
        _ => Err(DomainError::InvalidArgument { what: "state" }),
    }
}

fn agent_of(raw: &pb::LaneAgentRecord) -> Result<AgentRecord> {
    let role = match pb::LaneAgentRole::try_from(raw.role) {
        Ok(pb::LaneAgentRole::Build) => AgentRole::Build,
        Ok(pb::LaneAgentRole::Review) => AgentRole::Review,
        Ok(pb::LaneAgentRole::Fix) => AgentRole::Fix,
        _ => return Err(DomainError::InvalidArgument { what: "role" }),
    };
    Ok(AgentRecord {
        harness: raw.harness.clone(),
        agent_id: raw.agent_id.clone(),
        role,
        model: raw.model.clone(),
        ended: raw.ended,
    })
}

/// Record `agent` as a worker on the lane's first card that is still open, so
/// the runner reads its turns (`task.worker`, ov-213).
///
/// The lane's own write has already committed, so a refusal here (every card
/// closed, a harness the board doesn't take) is logged and not returned: the
/// lane is right, and only the spend would go unread.
fn record_worker(svc: &Service, watcher: &Watcher, lane: &Lane, agent: &AgentRecord, actor: &str) {
    let open = |id: Uuid| svc.store.get_task(id).is_ok_and(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled));
    let cards = match svc.store.plan(lane.workspace_id, i64::MIN) {
        Ok(plan) => plan.lanes.into_iter().find(|l| l.lane.id == lane.id).map(|l| l.cards).unwrap_or_default(),
        Err(_) => return,
    };
    let Some(task) = cards.iter().map(|c| c.task_id).find(|id| open(*id)) else { return };
    let req = pb::TaskWorkerSet {
        task_id: id_bytes(task),
        harness: agent.harness.clone(),
        agent_id: agent.agent_id.clone(),
        label: Some(lane.name.clone()),
        model: agent.model.clone(),
        end: agent.ended,
        actor: actor.to_string(),
        ..Default::default()
    };
    if let Err(err) = crate::task_starts::worker(svc, watcher, &req, None) {
        tracing::warn!(lane = %lane.name, error = %err, "a lane's agent was not recorded as a task worker");
    }
}

// ---------------------------------------------------------------------------
// store to wire
// ---------------------------------------------------------------------------

fn theme_view(svc: &Service, theme: BoardTheme) -> Result<pb::BoardThemeView> {
    let plan = svc.store.plan(theme.workspace_id, i64::MIN)?;
    let view = plan.themes.into_iter().find(|v| v.theme.id == theme.id);
    // A dropped theme leaves the read; answer with it, empty.
    Ok(pb_theme_view(&view.unwrap_or(ThemeView { theme, tasks: Vec::new(), counts: StatusCounts::default(), spend: LaneSpend::default() })))
}

fn lane_view(svc: &Service, lane: &Lane, admin: bool) -> Result<pb::Lane> {
    let plan = svc.store.plan(lane.workspace_id, i64::MIN)?;
    let view = plan.lanes.into_iter().find(|v| v.lane.id == lane.id).ok_or(DomainError::NotFound)?;
    Ok(pb_lane(&view, admin))
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
    }
}

fn pb_spend(s: &LaneSpend) -> pb::LaneSpend {
    pb::LaneSpend {
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

/// A lane as the wire carries it. The worktree path is a runner path, so it
/// is shown only to `host_admin`, as every other path on the wire is.
fn pb_lane(v: &LaneView, admin: bool) -> pb::Lane {
    let l = &v.lane;
    pb::Lane {
        id: id_bytes(l.id),
        workspace_id: id_bytes(l.workspace_id),
        name: l.name.clone(),
        state: (match l.state {
            LaneState::Queued => pb::LaneState::Queued,
            LaneState::Building => pb::LaneState::Building,
            LaneState::Review => pb::LaneState::Review,
            LaneState::Fixing => pb::LaneState::Fixing,
            LaneState::Landing => pb::LaneState::Landing,
            LaneState::Landed => pb::LaneState::Landed,
            LaneState::Dropped => pb::LaneState::Dropped,
        }) as i32,
        reason: l.reason.clone(),
        plan_rank: l.plan_rank,
        worktree_id: l.worktree_id.map(id_bytes),
        worktree_path: if admin { l.worktree_path.clone() } else { String::new() },
        branch: l.branch.clone(),
        harness: l.harness.clone(),
        model: l.model.clone(),
        train: l.train.clone(),
        landed_sha: l.landed_sha.clone(),
        state_since: l.state_since,
        created_at: l.created_at,
        resource_version: l.resource_version,
        cards: v
            .cards
            .iter()
            .map(|c| pb::LaneCard { task_id: id_bytes(c.task_id), slice: c.slice.clone() })
            .collect(),
        agents: v
            .agents
            .iter()
            .map(|a| pb::LaneAgent {
                harness: a.harness.clone(),
                agent_id: a.agent_id.clone(),
                role: (match a.role {
                    AgentRole::Build => pb::LaneAgentRole::Build,
                    AgentRole::Review => pb::LaneAgentRole::Review,
                    AgentRole::Fix => pb::LaneAgentRole::Fix,
                }) as i32,
                model: a.model.clone(),
                started_at: a.started_at,
                ended_at: a.ended_at,
            })
            .collect(),
        fix_rounds: v.fix_rounds,
        spend: Some(pb_spend(&v.spend)),
        stale: v.stale,
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

fn pb_plan(p: &Plan, admin: bool) -> pb::Plan {
    pb::Plan {
        now_ms: p.now_ms,
        themes: p.themes.iter().map(pb_theme_view).collect(),
        lanes: p.lanes.iter().map(|l| pb_lane(l, admin)).collect(),
        order: p.order.iter().map(|id| id_bytes(*id)).collect(),
        cards: p
            .cards
            .iter()
            .map(|c| pb::PlanCard {
                task_id: id_bytes(c.task_id),
                key: c.key.clone(),
                title: c.title.clone(),
                status: pb_status(c.status),
            })
            .collect(),
        coverage: p
            .coverage
            .iter()
            .map(|c| pb::PlanCoverage { task_id: id_bytes(c.task_id), live: c.live, landed: c.landed })
            .collect(),
        rulings: p.rulings.iter().map(crate::rpc_rulings::pb_ruling).collect(),
        trains: p.trains.iter().map(crate::rpc_trains::pb_train_view).collect(),
        ci: p.ci.iter().map(crate::rpc_trains::pb_ci).collect(),
        board_counts: Some(pb_counts(&p.board_counts)),
    }
}

fn pb_event(e: &PlanEvent) -> pb::PlanEvent {
    pb::PlanEvent {
        id: id_bytes(e.id),
        at: e.at,
        actor: e.actor.clone(),
        kind: e.kind.clone(),
        body: e.body.clone(),
        extra_json: e.extra.to_string(),
    }
}
