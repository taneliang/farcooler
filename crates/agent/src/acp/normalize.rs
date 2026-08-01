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
/// Never an empty vec for an update that carried meaning: an update this
/// version cannot interpret produces a `Gap`, because a silently dropped
/// update is a transcript that is wrong without saying so.
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
