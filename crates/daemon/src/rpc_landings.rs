//! Landings (ov-309, ov-872): `landing.start` and `landing.set`, behind
//! `board_line`. Read through `plan.get`, whose `Plan.landings` and `Plan.ci`
//! this file also converts.
//!
//! Part of the plan layer, and beside the board as it is: nothing here touches
//! `task_ops` or emits `task_changed`. Each write announces `plan_changed`
//! once, and a write that gives a landing a SHA asks the CI watch
//! (`ci_watch.rs`) to read it now rather than at its next turn.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, request, result};
use farcooler_store::board_ci::{CiRead, CiStatus};
use farcooler_store::landing_phase::LandingPhase;
use farcooler_store::landings::{Landing, LandingState, LandingUpdate, NewLanding, Route, landing_title};
use farcooler_store::plan_read::LandingView;

use crate::service::Service;
use crate::task_ops::{actor_from_wire, required_id};
use crate::watch::Watcher;
use crate::wire::id_bytes;

/// One landing route, as `rpc_plan::dispatch` hands it over.
pub(crate) fn dispatch(svc: &Service, watcher: &Watcher, req: Request) -> Result<result::Value> {
    let missing = || DomainError::InvalidArgument { what: "payload" };
    let ids = |raw: &[bytes::Bytes]| raw.iter().map(|id| required_id(id)).collect::<Result<Vec<_>>>();
    match (req.method.as_str(), req.payload) {
        ("landing.start", Some(request::Payload::LandingStart(p))) => {
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let new = NewLanding {
                route: route_of(p.route)?,
                base: p.base,
                title: p.title,
                lander: p.lander_id.as_deref().map(required_id).transpose()?,
                collecting: false,
            };
            let landing = svc.store.start_landing(workspace, &new, &ids(&p.task_ids)?, actor)?;
            watcher.announce_plan_changed(workspace, actor);
            watcher.tell_departures();
            Ok(result::Value::Landing(view_of(svc, &landing)?))
        }
        ("landing.set", Some(request::Payload::LandingSet(p))) => {
            let id = required_id(&p.landing_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let state = match pb::LandingState::try_from(p.state) {
                Ok(pb::LandingState::Unspecified) => None,
                Ok(s) => Some(landing_state(s).ok_or(DomainError::InvalidArgument { what: "state" })?),
                Err(_) => return Err(DomainError::InvalidArgument { what: "state" }),
            };
            let pushed = p.sha.is_some();
            let update = LandingUpdate {
                state,
                base: p.base,
                title: p.title,
                lander: p.lander_id.as_deref().map(required_id).transpose()?,
                sha: p.sha,
                add_tickets: ids(&p.add_task_ids)?,
                remove_tickets: ids(&p.remove_task_ids)?,
                expect: None,
            };
            let landing = svc.store.update_landing(id, &update, actor)?;
            watcher.announce_plan_changed(landing.workspace_id, actor);
            // A landing that settled, or was dropped, may have sent the collecting one off.
            watcher.tell_departures();
            if pushed {
                crate::ci_watch::kick();
            }
            Ok(result::Value::Landing(view_of(svc, &landing)?))
        }
        ("landing.start" | "landing.set", _) => Err(missing()),
        (other, _) => {
            tracing::error!(method = %other, "a landing route with no handler");
            Err(DomainError::NotFound)
        }
    }
}

/// A landing as the plan read says it, so a write answers with what a read
/// would: its title, summary, phase and lander's spend.
fn view_of(svc: &Service, landing: &Landing) -> Result<pb::Landing> {
    let plan = svc.store.plan(landing.workspace_id, i64::MIN)?;
    // The collecting landing with nothing on it is left out of the plan read,
    // so a write to it answers with the landing as it is.
    Ok(plan.landings.iter().find(|l| l.landing.id == landing.id).map_or_else(
        || {
            let mut wire = pb_landing(landing);
            wire.phase = pb::LandingPhase::WaitingForTickets as i32;
            wire.phase_since = landing.state_since;
            wire
        },
        pb_landing_view,
    ))
}

fn route_of(raw: i32) -> Result<Route> {
    match pb::LandingRoute::try_from(raw) {
        Ok(pb::LandingRoute::Push | pb::LandingRoute::Unspecified) => Ok(Route::Push),
        Ok(pb::LandingRoute::PullRequest) => Ok(Route::PullRequest),
        Ok(pb::LandingRoute::Ask) => Ok(Route::Ask),
        Err(_) => Err(DomainError::InvalidArgument { what: "route" }),
    }
}

fn pb_route(r: Route) -> pb::LandingRoute {
    match r {
        Route::Push => pb::LandingRoute::Push,
        Route::PullRequest => pb::LandingRoute::PullRequest,
        Route::Ask => pb::LandingRoute::Ask,
    }
}

fn landing_state(s: pb::LandingState) -> Option<LandingState> {
    Some(match s {
        pb::LandingState::Open => LandingState::Open,
        pb::LandingState::Gating => LandingState::Gating,
        pb::LandingState::Pushed => LandingState::Pushed,
        pb::LandingState::Green => LandingState::Green,
        pb::LandingState::Red => LandingState::Red,
        pb::LandingState::Landed => LandingState::Landed,
        pb::LandingState::Dropped => LandingState::Dropped,
        pb::LandingState::Unspecified => return None,
    })
}

fn pb_landing_state(s: LandingState) -> pb::LandingState {
    match s {
        LandingState::Open => pb::LandingState::Open,
        LandingState::Gating => pb::LandingState::Gating,
        LandingState::Pushed => pb::LandingState::Pushed,
        LandingState::Green => pb::LandingState::Green,
        LandingState::Red => pb::LandingState::Red,
        LandingState::Landed => pb::LandingState::Landed,
        LandingState::Dropped => pb::LandingState::Dropped,
    }
}

/// A landing as the wire carries it, with nothing derived.
pub(crate) fn pb_landing(l: &Landing) -> pb::Landing {
    pb::Landing {
        id: id_bytes(l.id),
        workspace_id: id_bytes(l.workspace_id),
        number: l.number,
        route: pb_route(l.route) as i32,
        base: l.base.clone(),
        pushed_sha: l.pushed_sha.clone(),
        state: pb_landing_state(l.state) as i32,
        state_since: l.state_since,
        lander_id: l.lander_id.map(id_bytes),
        collecting: l.collecting,
        actor: l.actor.clone(),
        created_at: l.created_at,
        landed_at: l.landed_at,
        resource_version: l.resource_version,
        task_ids: l.tickets.iter().map(|id| id_bytes(*id)).collect(),
        ci_subject: l.ci_subject().unwrap_or_default(),
        title: landing_title(l),
        summary: String::new(),
        phase: pb::LandingPhase::Unspecified as i32,
        phase_since: 0,
        spend: None,
    }
}

fn pb_phase(p: LandingPhase) -> pb::LandingPhase {
    match p {
        LandingPhase::WaitingForPreviousLanding => pb::LandingPhase::WaitingForPreviousLanding,
        LandingPhase::WaitingForMain => pb::LandingPhase::WaitingForMain,
        LandingPhase::WaitingForTickets => pb::LandingPhase::WaitingForTickets,
        LandingPhase::Integrating => pb::LandingPhase::Integrating,
        LandingPhase::GatingLocally => pb::LandingPhase::GatingLocally,
        LandingPhase::WaitingForCi => pb::LandingPhase::WaitingForCi,
        LandingPhase::CiFailed => pb::LandingPhase::CiFailed,
        LandingPhase::ReadyToLand => pb::LandingPhase::ReadyToLand,
        LandingPhase::Landed => pb::LandingPhase::Landed,
        LandingPhase::Stalled => pb::LandingPhase::Stalled,
        LandingPhase::Dropped => pb::LandingPhase::Dropped,
    }
}

/// A landing in the plan read: the landing, with the title, summary, phase
/// and lander's spend the read derived.
pub(crate) fn pb_landing_view(v: &LandingView) -> pb::Landing {
    let mut wire = pb_landing(&v.landing);
    wire.title = v.title.clone();
    wire.summary = v.summary.clone();
    wire.phase = pb_phase(v.phase) as i32;
    wire.phase_since = v.phase_since;
    wire.spend = v.landing.lander_id.map(|_| crate::rpc_plan::pb_spend(&v.spend));
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
