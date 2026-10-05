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
use farcooler_store::plan_read::TrainView;
use farcooler_store::trains::{NewTrain, Train, TrainState, TrainUpdate};

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
            let new = NewTrain { name: p.name, base: p.base };
            let train = svc.store.start_train(workspace, &new, &ids(&p.lane_ids)?, actor)?;
            watcher.announce_plan_changed(workspace, actor);
            Ok(result::Value::BoardTrain(pb_train(&train, &lanes_of(svc, &train)?)))
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
                sha: p.sha,
                add_lanes: ids(&p.add_lane_ids)?,
                remove_lanes: ids(&p.remove_lane_ids)?,
            };
            let train = svc.store.set_train(id, &update, actor)?;
            watcher.announce_plan_changed(train.workspace_id, actor);
            if pushed {
                crate::ci_watch::kick();
            }
            Ok(result::Value::BoardTrain(pb_train(&train, &lanes_of(svc, &train)?)))
        }
        ("train.start" | "train.set", _) => Err(missing()),
        (other, _) => {
            tracing::error!(method = %other, "a train route with no handler");
            Err(DomainError::NotFound)
        }
    }
}

/// A train's lanes, from the plan read, so a write answers with what a read
/// would say.
fn lanes_of(svc: &Service, train: &Train) -> Result<Vec<uuid::Uuid>> {
    let plan = svc.store.plan(train.workspace_id, i64::MIN)?;
    Ok(plan.trains.into_iter().find(|t| t.train.id == train.id).map(|t| t.lanes).unwrap_or_default())
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
    }
}

/// A train in the plan read.
pub(crate) fn pb_train_view(v: &TrainView) -> pb::BoardTrain {
    pb_train(&v.train, &v.lanes)
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
