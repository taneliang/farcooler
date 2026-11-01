//! A landing's phase, row by row (ov-503, ov-872): the table in
//! `landing_phase.rs`.

use super::*;

const MIN: i64 = 60_000;
const NOW: i64 = 10_000 * MIN;

fn on(state: Stage, since_min_ago: i64) -> TicketOn {
    TicketOn { task: Uuid::now_v7(), stage: state, stage_since: NOW - since_min_ago * MIN, left_ready_at: None }
}

fn at(state: LandingState, since_min_ago: i64, tickets: &[TicketOn]) -> Derived {
    derive(&PhaseInput {
        state,
        state_since: NOW - since_min_ago * MIN,
        lander_open: false,
        lander_started_at: None,
        last_ticket_on: None,
        tickets,
        collecting: false,
        main_red: false,
        ahead: false,
        now: NOW,
    })
}

#[test]
fn every_row_of_the_phase_table() {
    let ready = on(Stage::Landing, 5);
    let building = on(Stage::Building, 5);
    let fresh = |state, tickets: &[TicketOn]| at(state, 5, tickets).phase;
    // Waiting for tickets: a ticket on it isn't ready.
    assert_eq!(fresh(LandingState::Open, &[ready, building]), LandingPhase::WaitingForTickets);
    // The open landing, with a landing ahead of it: waiting for it, however long.
    let ahead = |state, tickets: &[TicketOn]| {
        derive(&PhaseInput {
            state,
            state_since: NOW - 600 * MIN,
            lander_open: false,
            lander_started_at: None,
            last_ticket_on: None,
            tickets,
            collecting: true,
            main_red: false,
            ahead: true,
            now: NOW,
        })
        .phase
    };
    assert_eq!(ahead(LandingState::Open, &[ready]), LandingPhase::WaitingForPreviousLanding);
    assert_eq!(ahead(LandingState::Open, &[]), LandingPhase::WaitingForPreviousLanding);
    // No landing ahead and nothing ready (or main red): waiting for tickets, never stalled.
    let alone = |tickets: &[TicketOn]| {
        derive(&PhaseInput {
            state: LandingState::Open,
            state_since: NOW - 600 * MIN,
            lander_open: false,
            lander_started_at: None,
            last_ticket_on: None,
            tickets,
            collecting: true,
            main_red: false,
            ahead: false,
            now: NOW,
        })
        .phase
    };
    assert_eq!(alone(&[]), LandingPhase::WaitingForTickets);
    assert_eq!(alone(&[ready]), LandingPhase::WaitingForTickets);
    // A ticket ready, nothing ahead and main red: waiting for main, not for tickets.
    let red = derive(&PhaseInput {
        state: LandingState::Open,
        state_since: NOW - 600 * MIN,
        lander_open: false,
        lander_started_at: None,
        last_ticket_on: None,
        tickets: &[ready],
        collecting: true,
        main_red: true,
        ahead: false,
        now: NOW,
    });
    assert_eq!(red.phase, LandingPhase::WaitingForMain);
    // A landing started by hand is not the collecting one: it stalls like any other.
    assert_eq!(at(LandingState::Open, 600, &[]).phase, LandingPhase::Stalled);
    // Integrating: every ticket ready, no SHA.
    assert_eq!(fresh(LandingState::Open, &[ready]), LandingPhase::Integrating);
    assert_eq!(fresh(LandingState::Gating, &[ready]), LandingPhase::GatingLocally);
    assert_eq!(fresh(LandingState::Pushed, &[ready]), LandingPhase::WaitingForCi);
    assert_eq!(fresh(LandingState::Red, &[ready]), LandingPhase::CiFailed);
    assert_eq!(fresh(LandingState::Green, &[ready]), LandingPhase::ReadyToLand);
    assert_eq!(fresh(LandingState::Landed, &[]), LandingPhase::Landed);
    assert_eq!(fresh(LandingState::Dropped, &[]), LandingPhase::Dropped);
    // A closed ticket never blocks: a landing that carried a landed ticket waits on no one.
    let landed = on(Stage::Landed, 5);
    assert_eq!(fresh(LandingState::Open, &[ready, landed]), LandingPhase::Integrating);
}

#[test]
fn stalled_after_two_hours_without_an_open_agent_before_ci() {
    let ready = on(Stage::Landing, 600);
    let building = on(Stage::Building, 600);
    for state in [LandingState::Open, LandingState::Gating] {
        assert_eq!(at(state, 121, &[ready]).phase, LandingPhase::Stalled, "{state:?}");
        assert_eq!(at(state, 119, &[ready]).phase, if state == LandingState::Gating { LandingPhase::GatingLocally } else { LandingPhase::Integrating });
    }
    assert_eq!(at(LandingState::Open, 300, &[ready, building]).phase, LandingPhase::Stalled);
    // From Waiting for CI on, a long wait is CI's, not the landing's.
    for state in [LandingState::Pushed, LandingState::Red, LandingState::Green] {
        assert!(!matches!(at(state, 600, &[ready]).phase, LandingPhase::Stalled), "{state:?}");
    }
    // An open lander keeps it from stalling; a finished one doesn't.
    let with_agent = |open| {
        derive(&PhaseInput {
            state: LandingState::Open,
            state_since: NOW - 300 * MIN,
            lander_open: open,
            lander_started_at: Some(NOW - 10 * MIN),
            last_ticket_on: None,
            tickets: &[ready],
            collecting: false,
            main_red: false,
            ahead: false,
            now: NOW,
        })
    };
    assert_eq!(with_agent(true).phase, LandingPhase::Integrating);
    assert_eq!(with_agent(true).since, NOW - 10 * MIN, "integrating since its agent started");
    assert_eq!(with_agent(false).phase, LandingPhase::Stalled);
}
