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
