//! What an agent is doing, read off the protocol instead of the screen.
//!
//! `core::activity` answers the same question for a TUI by matching substrings
//! on a captured pane, and has to be generous because a missed `blocked` is a
//! notification that never arrives. Here the agent says so itself.
//!
//! Only the OBSERVATION lives here. `advance`, `seen` and `wants_attention`
//! stay in `core` and are reused unchanged, so `Done` means the same thing and
//! a phone, a Mac badge and a notification cannot disagree.

use overnight_protocol::v1::AgentActivity;

use crate::event::AgentEvent;

/// The activity this event implies, or `None` if it implies nothing.
pub fn observe(event: &AgentEvent) -> Option<AgentActivity> {
    match event {
        AgentEvent::Permission { .. } => Some(AgentActivity::Blocked),
        AgentEvent::TurnEnded { .. } => Some(AgentActivity::Idle),
        AgentEvent::Message { .. }
        | AgentEvent::ToolCall { .. }
        | AgentEvent::ToolUpdate { .. }
        | AgentEvent::Plan { .. }
        | AgentEvent::Resolved { .. } => Some(AgentActivity::Working),
        // Bookkeeping. Says nothing about whether the agent needs you.
        AgentEvent::SessionStarted { .. }
        | AgentEvent::ModeSet { .. }
        | AgentEvent::ConfigSet { .. }
        | AgentEvent::Usage { .. }
        | AgentEvent::SessionInfo { .. }
        | AgentEvent::CommandsAvailable { .. }
        | AgentEvent::Gap { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, AgentGapReason, EndReason, PermissionOption, Role, ToolStatus};
    use overnight_protocol::v1::AgentActivity;

    #[test]
    fn a_permission_request_is_exactly_blocked() {
        // The signal the whole notification story rests on. In agent pane mode
        // it is a protocol fact rather than a substring match.
        let e = AgentEvent::Permission {
            id: "r".into(),
            tool_call: "t".into(),
            options: vec![PermissionOption { id: "a".into(), name: "Yes".into(), kind: "allow_once".into() }],
        };
        assert_eq!(observe(&e), Some(AgentActivity::Blocked));
    }

    #[test]
    fn a_turn_ending_is_idle_so_advance_can_make_it_done() {
        // `advance(Working, Idle) == Done` lives in core and is not duplicated
        // here. This only has to report the observation honestly.
        assert_eq!(
            observe(&AgentEvent::TurnEnded { reason: EndReason::EndTurn }),
            Some(AgentActivity::Idle)
        );
    }

    #[test]
    fn work_in_flight_is_working() {
        assert_eq!(
            observe(&AgentEvent::Message { role: Role::Agent, text: "hi".into() }),
            Some(AgentActivity::Working)
        );
        assert_eq!(
            observe(&AgentEvent::ToolUpdate {
                id: "t".into(),
                status: ToolStatus::InProgress,
                title: None,
                content: None,
                diff: None,
                locations: Vec::new(),
            }),
            Some(AgentActivity::Working)
        );
    }

    #[test]
    fn bookkeeping_events_do_not_move_the_badge() {
        // A gap or a mode change says nothing about whether the agent needs
        // you, and reporting one would make a fleet row flicker.
        assert_eq!(observe(&AgentEvent::Gap { reason: AgentGapReason::RingTrimmed }), None);
        assert_eq!(observe(&AgentEvent::ModeSet { agent_mode: "plan".into() }), None);
    }
}
