//! A turn that ended `Failed`, as the supervisor folds it (ov-140).
//! Its own file to keep `agent_supervisor.rs` inside its size budget.

use super::*;
use farcooler_agent_core::event::{AgentEvent, EndReason, FailureKind};

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
