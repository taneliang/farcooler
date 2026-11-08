//! An old landing's phase, row by row (ov-960): the table in
//! `landing_phase.rs`.

use uuid::Uuid;

use super::*;
use crate::landings::Route;

fn left(state: LandingState, sha: Option<&str>) -> Landing {
    Landing {
        id: Uuid::now_v7(),
        workspace_id: Uuid::now_v7(),
        number: 373,
        route: Route::Push,
        base: "origin/main".into(),
        pushed_sha: sha.map(str::to_string),
        state,
        state_since: 42,
        lander_id: None,
        title: String::new(),
        collecting: false,
        actor: "manager".into(),
        created_at: 1,
        landed_at: None,
        resource_version: 1,
        tickets: Vec::new(),
    }
}

#[test]
fn every_row_of_the_history_table() {
    let rows = [
        (LandingState::Landed, Some("abcdef1"), LandingPhase::Landed),
        (LandingState::Dropped, None, LandingPhase::Dropped),
        (LandingState::Pushed, Some("abcdef1"), LandingPhase::Landed),
        (LandingState::Green, Some("abcdef1"), LandingPhase::Landed),
        (LandingState::Red, Some("abcdef1"), LandingPhase::CiFailed),
        (LandingState::Open, None, LandingPhase::Dropped),
        (LandingState::Gating, None, LandingPhase::Dropped),
    ];
    for (state, sha, phase) in rows {
        assert_eq!(history(&left(state, sha)), Derived { phase, since: 42 }, "{state:?}");
    }
}

/// Landing 373 was pushed and waiting on CI when the board moved to the
/// engine. Nothing follows its CI now, so it never reads as waiting for it.
#[test]
fn a_landing_left_waiting_for_ci_is_never_waiting_for_ci() {
    for state in [LandingState::Open, LandingState::Gating, LandingState::Pushed, LandingState::Green, LandingState::Red] {
        let phase = history(&left(state, Some("abcdef1"))).phase;
        assert!(
            !matches!(
                phase,
                LandingPhase::WaitingForCi
                    | LandingPhase::WaitingForTickets
                    | LandingPhase::WaitingForMain
                    | LandingPhase::WaitingForPreviousLanding
                    | LandingPhase::Integrating
                    | LandingPhase::GatingLocally
                    | LandingPhase::ReadyToLand
                    | LandingPhase::Stalled
            ),
            "{state:?} reads as {phase:?}"
        );
    }
}
