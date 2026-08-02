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

/// A title that says what the tool is doing, not merely what kind it is.
///
/// Most tools name themselves usefully once resolved — a Bash call becomes
/// its command. A few do not: `Skill` stays "Skill" however many skills there
/// are, and the one thing a reader wants is which. The argument that carries
/// it is the tool's own, so the lookup is by the names those tools actually
/// use rather than a general rule.
fn detail(title: &str, raw_input: &serde_json::Value) -> String {
    for key in ["skill", "name", "command", "pattern", "description"] {
        if let Some(value) = raw_input.get(key).and_then(|v| v.as_str()) {
            if !value.is_empty() && !title.contains(value) {
                return format!("{title}: {value}");
            }
        }
    }
    title.to_string()
}

/// A tool's output, as one readable string.
///
/// The adapter fences console output as markdown (```console …```), which is
/// right for a renderer that speaks markdown and noise for one that shows a
/// monospace block. The fence is stripped here so every client does not have
/// to.
fn tool_text(content: &[crate::acp::wire::ToolContent]) -> Option<String> {
    let joined: String = content
        .iter()
        .filter_map(|c| c.content.as_ref().map(|b| b.text.clone()))
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if joined.is_empty() {
        return None;
    }
    Some(strip_fence(&joined))
}

fn strip_fence(text: &str) -> String {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else { return text.to_string() };
    // The word after the opening fence is a language tag, not content.
    let body = rest.split_once('\n').map(|(_, b)| b).unwrap_or("");
    body.trim_end().strip_suffix("```").unwrap_or(body).trim_end().to_string()
}

/// A diff carried directly on a tool call, rather than reconstructed.
fn tool_diff(content: &[crate::acp::wire::ToolContent]) -> Option<crate::event::Diff> {
    content.iter().find(|c| c.kind == "diff").map(|c| crate::event::Diff {
        path: c.path.clone(),
        old_text: c.old_text.clone(),
        new_text: c.new_text.clone().unwrap_or_default(),
    })
}

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
        SessionUpdate::ToolCall { tool_call_id, title, kind, status: s, locations, content } => {
            vec![AgentEvent::ToolCall {
                id: tool_call_id.clone(),
                title: title.clone(),
                kind: kind.clone(),
                status: status(s),
                locations: locations.iter().map(|l| l.path.clone()).collect(),
            }]
            .into_iter()
            .chain(tool_text(content).map(|text| AgentEvent::ToolUpdate {
                id: tool_call_id.clone(),
                status: status(s),
                title: None,
                content: Some(text),
                diff: tool_diff(content),
                locations: Vec::new(),
            }))
            .collect()
        }
        SessionUpdate::ToolCallUpdate {
            tool_call_id,
            status: s,
            title,
            content,
            locations,
            raw_input,
        } => {
            vec![AgentEvent::ToolUpdate {
                id: tool_call_id.clone(),
                status: status(s),
                title: title
                    .clone()
                    .filter(|t| !t.is_empty())
                    .map(|t| detail(&t, raw_input)),
                content: tool_text(content),
                diff: tool_diff(content),
                locations: locations.iter().map(|l| l.path.clone()).collect(),
            }]
        }
        SessionUpdate::Plan { entries } => {
            vec![AgentEvent::Plan { entries: entries.iter().map(plan_entry).collect() }]
        }
        SessionUpdate::SessionInfoUpdate { title } => {
            vec![AgentEvent::SessionInfo { title: title.clone() }]
        }
        SessionUpdate::UsageUpdate { used, size } => {
            vec![AgentEvent::Usage { used: *used, size: *size }]
        }
        SessionUpdate::ConfigOptionUpdate { config_id, value } => {
            let value = match value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Bool(b) => b.to_string(),
                other => other.to_string(),
            };
            // Both, deliberately: `ConfigSet` is the general truth, and
            // `ModeSet` keeps the surfaces that ask about mode by name working
            // without every one of them learning the generic form first.
            let mut out = vec![AgentEvent::ConfigSet { id: config_id.clone(), value: value.clone() }];
            if config_id == "mode" {
                out.push(AgentEvent::ModeSet { agent_mode: value });
            }
            out
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

    #[test]
    fn session_metadata_never_becomes_a_gap() {
        // Two updates that carry no conversation: the slash-command menu and
        // the session's own title. Both fire on ordinary turns, and both drew
        // a "history missing" break until they were modelled. A gap means
        // something was LOST; nothing here was.
        for raw in [
            r#"{"sessionUpdate":"session_info_update","title":"Say ok.","updatedAt":"2026-08-01T21:37:40Z"}"#,
            r#"{"sessionUpdate":"usage_update","used":20107,"size":200000}"#,
            r#"{"sessionUpdate":"available_commands_update","availableCommands":[{"name":"init"}]}"#,
        ] {
            let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
            let events = update_to_events(&update);
            assert!(
                !events.iter().any(|e| matches!(e, AgentEvent::Gap { .. })),
                "{raw} produced a gap"
            );
        }
    }

    #[test]
    fn a_tool_update_carries_its_new_name_and_its_output() {
        // Copied from a live adapter. A `Bash` call starts life titled
        // "Terminal" and is renamed to the command it ran; its output arrives
        // fenced as markdown. Dropping both left a row that said a command ran
        // and refused to say which, or what happened.
        let raw = r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed",
            "title":"echo hello && ls",
            "content":[{"type":"content","content":{"type":"text","text":"```console\nhello\nmain.rs\n```"}}]}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let events = update_to_events(&update);
        let AgentEvent::ToolUpdate { title, content, .. } = &events[0] else {
            panic!("expected a tool update, got {events:?}")
        };
        assert_eq!(title.as_deref(), Some("echo hello && ls"));
        // Unfenced: the fence is markdown for a renderer that speaks it, and
        // noise in a monospace block.
        assert_eq!(content.as_deref(), Some("hello\nmain.rs"));
    }

    #[test]
    fn a_skill_call_says_which_skill() {
        // Most tools name themselves once resolved — a Bash call becomes its
        // command. `Skill` stays "Skill" however many skills exist, and the
        // one thing a reader wants from that row is which one ran.
        let raw = r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed",
            "title":"Skill","rawInput":{"skill":"superpowers:brainstorming"}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::ToolUpdate { title, .. } = &update_to_events(&update)[0] else {
            panic!("expected a tool update")
        };
        assert_eq!(title.as_deref(), Some("Skill: superpowers:brainstorming"));
    }

    #[test]
    fn a_title_that_already_says_it_is_left_alone() {
        // A Bash call is renamed to its command, so appending the command
        // again would read "echo hi: echo hi".
        let raw = r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed",
            "title":"echo hi","rawInput":{"command":"echo hi"}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::ToolUpdate { title, .. } = &update_to_events(&update)[0] else {
            panic!("expected a tool update")
        };
        assert_eq!(title.as_deref(), Some("echo hi"));
    }
}

/// Rewrite absolute paths inside the worktree as paths relative to it.
///
/// An adapter reports `/Users/someone/Library/Application Support/…/worktrees/
/// overnight-sample-refactor-api/README.md`, and in a tiled pane that title is
/// eighty characters of prefix and one of information. Everything in a
/// conversation about a worktree is already understood to be in it, so the
/// prefix says nothing the reader does not know.
///
/// Paths OUTSIDE the worktree are left absolute, because there the location is
/// the surprising part and shortening it would hide it.
pub fn relativize(events: &mut [AgentEvent], worktree: &std::path::Path) {
    let prefix = format!("{}/", worktree.display());
    let shorten = |text: &mut String| {
        if text.contains(&prefix) {
            *text = text.replace(&prefix, "");
        }
    };
    for event in events {
        match event {
            AgentEvent::ToolCall { title, locations, .. } => {
                shorten(title);
                for location in locations {
                    shorten(location);
                }
            }
            AgentEvent::ToolUpdate { title, locations, content, .. } => {
                if let Some(title) = title {
                    shorten(title);
                }
                for location in locations {
                    shorten(location);
                }
                if let Some(content) = content {
                    shorten(content);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod relativize_tests {
    use super::*;

    #[test]
    fn a_path_inside_the_worktree_loses_the_part_everyone_already_knows() {
        let worktree = std::path::Path::new("/tmp/wt");
        let mut events = vec![AgentEvent::ToolCall {
            id: "1".into(),
            title: "Read /tmp/wt/src/main.rs".into(),
            kind: "read".into(),
            status: ToolStatus::Pending,
            locations: vec!["/tmp/wt/src/main.rs".into()],
        }];
        relativize(&mut events, worktree);
        let AgentEvent::ToolCall { title, locations, .. } = &events[0] else { panic!() };
        assert_eq!(title, "Read src/main.rs");
        assert_eq!(locations[0], "src/main.rs");
    }

    #[test]
    fn a_path_outside_the_worktree_keeps_saying_so() {
        // Where a file is only matters when it is somewhere unexpected, so
        // that is exactly the case that must not be shortened.
        let worktree = std::path::Path::new("/tmp/wt");
        let mut events = vec![AgentEvent::ToolCall {
            id: "1".into(),
            title: "Read /etc/hosts".into(),
            kind: "read".into(),
            status: ToolStatus::Pending,
            locations: vec!["/etc/hosts".into()],
        }];
        relativize(&mut events, worktree);
        let AgentEvent::ToolCall { title, .. } = &events[0] else { panic!() };
        assert_eq!(title, "Read /etc/hosts");
    }
}
