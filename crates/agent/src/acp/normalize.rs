//! ACP updates in, normalized events out.

use crate::acp::wire::{SessionUpdate, WirePlanEntry};
use crate::event::{AgentEvent, AgentGapReason, PlanEntry, Role, ToolStatus};

fn status(raw: &str) -> ToolStatus {
    match raw {
        "in_progress" => ToolStatus::InProgress,
        "completed" => ToolStatus::Completed,
        "failed" => ToolStatus::Failed,
        _ => ToolStatus::Pending,
    }
}

fn plan_entry(e: &WirePlanEntry) -> PlanEntry {
    PlanEntry { content: e.content.clone(), priority: e.priority.clone(), status: e.status.clone() }
}

/// One update becomes zero or more events.
///
/// Almost never an empty vec for an update that carried meaning: an update
/// this version cannot interpret produces a `Gap`, because a silently
/// dropped update is a transcript that is wrong without saying so. The one
/// deliberate exception is `AvailableCommandsUpdate` — see its arm below for
/// why an empty vec there is correct rather than an oversight.
pub fn update_to_events(update: &SessionUpdate) -> Vec<AgentEvent> {
    match update {
        SessionUpdate::AgentMessageChunk { content } => {
            vec![AgentEvent::Message { role: Role::Agent, text: content.text.clone() }]
        }
        SessionUpdate::UserMessageChunk { content } => {
            vec![AgentEvent::Message { role: Role::User, text: content.text.clone() }]
        }
        SessionUpdate::AgentThoughtChunk { content } => {
            vec![AgentEvent::Message { role: Role::Thought, text: content.text.clone() }]
        }
        SessionUpdate::AvailableCommandsUpdate { available_commands } => {
            // Its own event, for two reasons it is worth being explicit about.
            //
            // Not a `Gap`: a gap means "history is missing here" and draws a
            // visible break telling the user their transcript is incomplete.
            // Nothing was lost — an agent resent its slash-command menu, which
            // it does once per turn whether or not anyone asked. The old
            // behaviour routed this through `Unknown` and turned every single
            // turn into a false "history missing" break.
            //
            // Not a second `SessionStarted` either, even though that event has
            // an `available_commands` field meant for exactly this data. A
            // consumer is entitled to assume a session starts once, so a
            // repeat reads as a restart and resets everything derived from the
            // first one.
            vec![AgentEvent::CommandsAvailable {
                commands: available_commands.iter().map(|c| c.name.clone()).collect(),
            }]
        }
        SessionUpdate::ToolCall { tool_call_id, title, kind, status: s, locations } => {
            vec![AgentEvent::ToolCall {
                id: tool_call_id.clone(),
                title: title.clone(),
                kind: kind.clone(),
                status: status(s),
                locations: locations.iter().map(|l| l.path.clone()).collect(),
            }]
        }
        SessionUpdate::ToolCallUpdate { tool_call_id, status: s } => {
            vec![AgentEvent::ToolUpdate {
                id: tool_call_id.clone(),
                status: status(s),
                content: None,
                diff: None,
            }]
        }
        SessionUpdate::Plan { entries } => {
            vec![AgentEvent::Plan { entries: entries.iter().map(plan_entry).collect() }]
        }
        SessionUpdate::CurrentModeUpdate { current_mode_id } => {
            vec![AgentEvent::ModeSet { agent_mode: current_mode_id.clone() }]
        }
        SessionUpdate::Unknown => vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::wire::SessionUpdate;
    use crate::event::{AgentEvent, AgentGapReason, Role, ToolStatus};

    #[test]
    fn an_agent_message_chunk_becomes_an_agent_message() {
        let update: SessionUpdate =
            serde_json::from_str(r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"}}"#)
                .expect("parses");
        let events = update_to_events(&update);
        assert_eq!(events, vec![AgentEvent::Message { role: Role::Agent, text: "hi".into() }]);
    }

    #[test]
    fn a_tool_call_maps_its_status() {
        let update: SessionUpdate = serde_json::from_str(
            r#"{"sessionUpdate":"tool_call","toolCallId":"t1","title":"Run ls","kind":"execute","status":"in_progress"}"#,
        )
        .expect("parses");
        let events = update_to_events(&update);
        assert_eq!(
            events,
            vec![AgentEvent::ToolCall {
                id: "t1".into(),
                title: "Run ls".into(),
                kind: "execute".into(),
                status: ToolStatus::InProgress,
                locations: vec![],
            }]
        );
    }

    #[test]
    fn an_unknown_session_update_becomes_exactly_one_gap() {
        // The rule the whole design rests on: an update we cannot interpret
        // must be VISIBLE, never silently dropped, or a future adapter release
        // quietly shortens every transcript.
        let update: SessionUpdate =
            serde_json::from_str(r#"{"sessionUpdate":"something_from_a_future_version"}"#)
                .expect("parses as unknown");
        let events = update_to_events(&update);
        assert_eq!(events, vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]);
    }
}
