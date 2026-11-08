//! Landings (ov-309, ov-872) as the plan read carries them: the old landing
//! rows, kept as history, and the engine's landing groups (ov-958). This file
//! converts `Plan.landings` and `Plan.ci` for `plan.get`.
//!
//! The old writes, `landing.start` and `landing.set`, are gone (ov-960): the
//! workflow engine lands, so both are refused with `engine_lands`, which an
//! old client turns into a sentence that says so.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, result};
use farcooler_store::board_ci::{CiRead, CiStatus};
use farcooler_store::landing_phase::LandingPhase;
use farcooler_store::landings::{Landing, LandingState, Route, landing_title};
use farcooler_store::plan_read::LandingView;

use crate::wire::id_bytes;

/// `landing.start` and `landing.set`: the engine lands now (ov-960).
pub(crate) fn refuse() -> Result<result::Value> {
    Err(DomainError::InvalidArgument { what: "engine_lands" })
}

fn pb_route(r: Route) -> pb::LandingRoute {
    match r {
        Route::Push => pb::LandingRoute::Push,
        Route::PullRequest => pb::LandingRoute::PullRequest,
        Route::Ask => pb::LandingRoute::Ask,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// ov-960: both old landing writes are refused, whatever they carry.
    #[test]
    fn the_old_landing_writes_say_the_engine_lands() {
        assert!(matches!(refuse(), Err(DomainError::InvalidArgument { what: "engine_lands" })));
    }
}
