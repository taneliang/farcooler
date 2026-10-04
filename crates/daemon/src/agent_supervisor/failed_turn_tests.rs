//! A turn that ended `Failed`, as the supervisor folds and holds it (ov-140).
//! Its own file to keep `agent_supervisor.rs` inside its size budget.

use super::*;
use farcooler_agent::event::{AgentEvent, EndReason, FailureKind};

#[test]
fn a_failed_turn_with_nothing_before_it_still_reaches_somebody() {
    // A refused key now ends the turn as `Failed` with no words in front
    // of it. From Idle, an ordinary end folds to Idle: nobody is told.
    let failed = AgentEvent::TurnEnded {
        reason: EndReason::Failed {
            kind: FailureKind::Auth,
            detail: "401".into(),
        },
    };
    assert_eq!(fold_activity(AgentActivity::Idle, &failed), AgentActivity::Done);
    assert_eq!(fold_activity(AgentActivity::Idle, &AgentEvent::TurnEnded { reason: EndReason::EndTurn }), AgentActivity::Idle);
}

#[test]
fn the_last_turns_failure_is_held_until_the_next_turn_ends() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let failed = AgentEvent::TurnEnded {
        reason: EndReason::Failed {
            kind: FailureKind::Quota,
            detail: String::new(),
        },
    };
    supervisor.record(terminal, vec![failed], &|_, _| {});
    assert!(supervisor.turn_failed(terminal), "a failed turn has to reach the row");
    assert_eq!(supervisor.failed_turns(terminal), 1);
    assert_eq!(supervisor.activity(terminal), AgentActivity::Done);
    supervisor.record(terminal, vec![AgentEvent::TurnEnded { reason: EndReason::EndTurn }], &|_, _| {});
    assert!(!supervisor.turn_failed(terminal), "and stop once a turn works");
}

#[test]
fn a_session_that_establishes_stops_reporting_the_failure_before_it() {
    // A pane that failed, was switched back to a terminal and switched in
    // again would otherwise keep drawing a failure row over a chat that is
    // working perfectly.
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    supervisor.apply(
        terminal,
        ShimMessage::Failed { failure: AgentFailure::NoAdapter },
        &|_, _| {},
    );
    assert_eq!(supervisor.failure(terminal), Some(AgentFailure::NoAdapter));

    supervisor.apply(
        terminal,
        ShimMessage::Established { session_id: "s".into(), available_modes: Vec::new() },
        &|_, _| {},
    );
    assert_eq!(supervisor.failure(terminal), None);
}

#[test]
fn a_failure_word_after_a_failed_turn_leaves_the_row_done() {
    // An adapter that dies mid-turn ends that turn `Failed` (which folds to
    // `Done`) and then announces `adapter-failed`. The second must not wipe
    // the first, or the unseen result a notice is made from is gone.
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let events = vec![
        AgentEvent::Message { role: farcooler_agent::event::Role::Agent, text: "half a turn".into(), parent: None },
        AgentEvent::TurnEnded { reason: EndReason::Failed { kind: FailureKind::Other, detail: "closed".into() } },
    ];
    let batch = events.into_iter().enumerate().map(|(seq, event)| Sequenced { seq: seq as u64, event }).collect();
    supervisor.apply(terminal, ShimMessage::Events { events: batch }, &|_, _| {});
    assert_eq!(supervisor.activity(terminal), AgentActivity::Done);

    supervisor.apply(terminal, ShimMessage::Failed { failure: AgentFailure::AdapterFailed }, &|_, _| {});
    assert_eq!(supervisor.activity(terminal), AgentActivity::Done);
    assert_eq!(supervisor.failed_turns(terminal), 1);
}
