//! Trains (ov-309): `train.start` and `train.set`, behind `board_trains`. Read
//! through `plan.get`, whose `Plan.trains` and `Plan.ci` this file also
//! converts.
//!
//! Part of the plan layer, and beside the board as it is: nothing here touches
//! `task_ops` or emits `task_changed`. Each write announces `plan_changed`
//! once, and a write that gives a train a SHA asks the CI watch
//! (`ci_watch.rs`) to read it now rather than at its next turn.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, request, result};
use farcooler_store::board_ci::{CiRead, CiStatus};
use farcooler_store::models::TaskStatus;
use farcooler_store::plan_read::TrainView;
use farcooler_store::trains::Train;
use farcooler_store::trains::{NewTrain, TrainAgentRecord, TrainState, TrainUpdate};

use crate::service::Service;
use crate::task_ops::{actor_from_wire, required_id};
use crate::watch::Watcher;
use crate::wire::id_bytes;

/// One train route, as `rpc_plan::dispatch` hands it over.
pub(crate) fn dispatch(svc: &Service, watcher: &Watcher, req: Request) -> Result<result::Value> {
    let missing = || DomainError::InvalidArgument { what: "payload" };
    let ids = |raw: &[bytes::Bytes]| raw.iter().map(|id| required_id(id)).collect::<Result<Vec<_>>>();
    match (req.method.as_str(), req.payload) {
        ("train.start", Some(request::Payload::TrainStart(p))) => {
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let new = NewTrain {
                name: p.name,
                base: p.base,
                title: p.title,
                card: p.card_id.as_deref().map(required_id).transpose()?,
                agent: p.agent.map(agent_of),
            };
            let train = svc.store.start_train(workspace, &new, &ids(&p.lane_ids)?, actor)?;
            watcher.announce_plan_changed(workspace, actor);
            record_worker(svc, watcher, &train, &p.actor);
            Ok(result::Value::BoardTrain(view_of(svc, &train)?))
        }
        ("train.set", Some(request::Payload::TrainSet(p))) => {
            let id = required_id(&p.train_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let state = match pb::BoardTrainState::try_from(p.state) {
                Ok(pb::BoardTrainState::Unspecified) => None,
                Ok(s) => Some(train_state(s).ok_or(DomainError::InvalidArgument { what: "state" })?),
                Err(_) => return Err(DomainError::InvalidArgument { what: "state" }),
            };
            let pushed = p.sha.is_some();
            let update = TrainUpdate {
                state,
                base: p.base,
                title: p.title,
                card: p.card_id.as_deref().map(required_id).transpose()?,
                agent: p.agent.map(agent_of),
                sha: p.sha,
                add_lanes: ids(&p.add_lane_ids)?,
                remove_lanes: ids(&p.remove_lane_ids)?,
            };
            let train = svc.store.set_train(id, &update, actor)?;
            watcher.announce_plan_changed(train.workspace_id, actor);
            if pushed {
                crate::ci_watch::kick();
            }
            record_worker(svc, watcher, &train, &p.actor);
            Ok(result::Value::BoardTrain(view_of(svc, &train)?))
        }
        ("train.start" | "train.set", _) => Err(missing()),
        (other, _) => {
            tracing::error!(method = %other, "a train route with no handler");
            Err(DomainError::NotFound)
        }
    }
}

/// A train as the plan read says it, so a write answers with what a read
/// would: its lanes, title, summary and agent's spend.
fn view_of(svc: &Service, train: &Train) -> Result<pb::BoardTrain> {
    let plan = svc.store.plan(train.workspace_id, i64::MIN)?;
    plan.trains.iter().find(|t| t.train.id == train.id).map(pb_train_view).ok_or(DomainError::NotFound)
}

fn agent_of(raw: pb::TrainAgentRecord) -> TrainAgentRecord {
    TrainAgentRecord { harness: raw.harness, agent_id: raw.agent_id, model: raw.model, ended: raw.ended }
}

/// Record the train's agent as a worker on its card, or else on the first open
/// card of its lanes, so the runner reads its turns (`task.worker`, ov-213).
/// The train's own write has committed, so a refusal here is logged, not
/// returned: only the spend would go unread.
fn record_worker(svc: &Service, watcher: &Watcher, train: &Train, actor: &str) {
    let Some(agent) = &train.agent else { return };
    let open = |id: uuid::Uuid| svc.store.get_task(id).is_ok_and(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled));
    let Ok(plan) = svc.store.plan(train.workspace_id, i64::MIN) else { return };
    let from_lanes = plan.trains.iter().find(|t| t.train.id == train.id).into_iter().flat_map(|t| t.lanes.iter()).flat_map(|id| {
        plan.lanes.iter().filter(move |l| l.lane.id == *id).flat_map(|l| l.cards.iter().map(|c| c.task_id))
    });
    let Some(task) = train.card_id.into_iter().chain(from_lanes).find(|id| open(*id)) else { return };
    let req = pb::TaskWorkerSet {
        task_id: id_bytes(task),
        harness: agent.harness.clone(),
        agent_id: agent.agent_id.clone(),
        label: Some(train.name.clone()),
        model: (!agent.model.is_empty()).then(|| agent.model.clone()),
        end: agent.ended_at.is_some(),
        actor: actor.to_string(),
        ..Default::default()
    };
    if let Err(err) = crate::task_starts::worker(svc, watcher, &req, None) {
        tracing::warn!(train = %train.name, error = %err, "a train's agent was not recorded as a task worker");
    }
}

fn train_state(s: pb::BoardTrainState) -> Option<TrainState> {
    Some(match s {
        pb::BoardTrainState::Integrating => TrainState::Integrating,
        pb::BoardTrainState::Gating => TrainState::Gating,
        pb::BoardTrainState::Pushed => TrainState::Pushed,
        pb::BoardTrainState::Green => TrainState::Green,
        pb::BoardTrainState::Red => TrainState::Red,
        pb::BoardTrainState::Landed => TrainState::Landed,
        pb::BoardTrainState::Dropped => TrainState::Dropped,
        pb::BoardTrainState::Unspecified => return None,
    })
}

fn pb_train_state(s: TrainState) -> pb::BoardTrainState {
    match s {
        TrainState::Integrating => pb::BoardTrainState::Integrating,
        TrainState::Gating => pb::BoardTrainState::Gating,
        TrainState::Pushed => pb::BoardTrainState::Pushed,
        TrainState::Green => pb::BoardTrainState::Green,
        TrainState::Red => pb::BoardTrainState::Red,
        TrainState::Landed => pb::BoardTrainState::Landed,
        TrainState::Dropped => pb::BoardTrainState::Dropped,
    }
}

/// A train as the wire carries it.
pub(crate) fn pb_train(t: &Train, lanes: &[uuid::Uuid]) -> pb::BoardTrain {
    pb::BoardTrain {
        id: id_bytes(t.id),
        workspace_id: id_bytes(t.workspace_id),
        name: t.name.clone(),
        base: t.base.clone(),
        pushed_sha: t.pushed_sha.clone(),
        state: pb_train_state(t.state) as i32,
        state_since: t.state_since,
        actor: t.actor.clone(),
        created_at: t.created_at,
        landed_at: t.landed_at,
        lane_ids: lanes.iter().map(|id| id_bytes(*id)).collect(),
        ci_subject: t.ci_subject().unwrap_or_default(),
        resource_version: t.resource_version,
        title: String::new(),
        summary: String::new(),
        agent: None,
        card_id: t.card_id.map(id_bytes),
    }
}

/// A train in the plan read: the train, with the title, summary and the
/// agent's spend the read derived.
pub(crate) fn pb_train_view(v: &TrainView) -> pb::BoardTrain {
    let mut wire = pb_train(&v.train, &v.lanes);
    wire.title = v.title.clone();
    wire.summary = v.summary.clone();
    wire.agent = v.train.agent.as_ref().map(|a| pb::TrainAgent {
        harness: a.harness.clone(),
        agent_id: a.agent_id.clone(),
        model: a.model.clone(),
        started_at: a.started_at,
        ended_at: a.ended_at,
        spend: Some(crate::rpc_plan::pb_spend(&v.spend)),
    });
    wire
}

/// A CI read as the wire carries it.
pub(crate) fn pb_ci(r: &CiRead) -> pb::BoardCiRead {
    pb::BoardCiRead {
        subject: r.subject.clone(),
        sha: r.sha.clone(),
        status: (match r.status {
            CiStatus::Passed => pb::BoardCiStatus::Passed,
            CiStatus::Failed => pb::BoardCiStatus::Failed,
            CiStatus::Running => pb::BoardCiStatus::Running,
            CiStatus::Queued => pb::BoardCiStatus::Queued,
            CiStatus::Superseded => pb::BoardCiStatus::Superseded,
            CiStatus::None => pb::BoardCiStatus::None,
            CiStatus::Unknown => pb::BoardCiStatus::Unknown,
        }) as i32,
        url: r.url.clone(),
        jobs: r
            .jobs
            .iter()
            .map(|j| pb::BoardCiJob { name: j.name.clone(), state: j.state.clone(), url: j.url.clone() })
            .collect(),
        fetched_at: r.fetched_at,
        changed_at: r.changed_at,
        asked_at: r.asked_at,
    }
}
