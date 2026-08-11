//! What an agent is doing, read off the protocol instead of the screen.
//!
//! `core::activity` answers the same question for a TUI by matching substrings
//! on a captured pane, and has to be generous because a missed `blocked` is a
//! notification that never arrives. Here the agent says so itself.
//!
//! Only the OBSERVATION lives here. `advance`, `seen` and `wants_attention`
//! stay in `core` and are reused unchanged, so `Done` means the same thing and
//! a phone, a Mac badge and a notification cannot disagree.

use farcooler_protocol::v1::AgentActivity;

use crate::event::AgentEvent;

/// The activity this event implies, or `None` if it implies nothing.
pub fn observe(event: &AgentEvent) -> Option<AgentActivity> {
    match event {
        AgentEvent::Permission { .. } => Some(AgentActivity::Blocked),
        AgentEvent::TurnEnded { .. } => Some(AgentActivity::Idle),
        // The AGENT's words. What the user typed says nothing about what the
        // agent is doing — a pane that went busy because a person typed was
        // reporting the wrong actor, and `send_next_queued` emits exactly that
        // event every time a queued prompt goes out.
        AgentEvent::Message { role: crate::event::Role::Agent | crate::event::Role::Thought, .. }
        | AgentEvent::ToolCall { .. }
        | AgentEvent::ToolUpdate { .. }
        | AgentEvent::Plan { .. }
        | AgentEvent::Resolved { .. } => Some(AgentActivity::Working),
        AgentEvent::Message { role: crate::event::Role::User, .. } => None,
        // Bookkeeping. Says nothing about whether the agent needs you.
        AgentEvent::SessionStarted { .. }
        | AgentEvent::ModeSet { .. }
        | AgentEvent::ConfigSet { .. }
        | AgentEvent::Usage { .. }
        | AgentEvent::SessionInfo { .. }
        | AgentEvent::CommandsAvailable { .. }
        // What is WAITING to be sent says nothing about what the agent is
        // doing. A queue that made a pane look busy would be reporting the
        // user's typing as the agent's work.
        | AgentEvent::PromptQueue { .. }
        | AgentEvent::Gap { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, AgentGapReason, EndReason, PermissionOption, Role, ToolStatus};
    use farcooler_protocol::v1::AgentActivity;

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
            observe(&AgentEvent::Message { role: Role::Agent, text: "hi".into(), parent: None }),
            Some(AgentActivity::Working)
        );
        assert_eq!(
            observe(&AgentEvent::ToolUpdate {
                id: "t".into(),
                status: ToolStatus::InProgress,
                title: None,
                content: None,
                diff: None,
                locations: Vec::new(), parent: None, subagent: None, }),
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

    #[test]
    fn your_own_words_are_not_the_agent_working() {
        // A pane went busy because a PERSON typed. `send_next_queued` and
        // `steer_queued` both emit the user's message, and each one moved the
        // badge before the agent had done anything at all.
        assert_eq!(
            observe(&AgentEvent::Message { role: Role::User, text: "hi".into(), parent: None }),
            None
        );
        // The agent's own words still are.
        assert_eq!(
            observe(&AgentEvent::Message { role: Role::Agent, text: "hi".into(), parent: None }),
            Some(AgentActivity::Working)
        );
    }
}
