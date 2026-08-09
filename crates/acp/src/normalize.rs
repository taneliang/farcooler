//! ACP updates in, normalized events out.

use crate::wire::{SessionUpdate, WirePlanEntry};
use farcooler_agent_core::event::{AgentEvent, AgentGapReason, PlanEntry, Role, ToolStatus};

fn status(raw: &str) -> ToolStatus {
    match raw {
        "in_progress" => ToolStatus::InProgress,
        "completed" => ToolStatus::Completed,
        "failed" => ToolStatus::Failed,
        _ => ToolStatus::Pending,
    }
}

/// The adapter's report of a finished subagent, as clients render it.
fn summary(result: &crate::wire::SubagentResult) -> farcooler_agent_core::event::SubagentSummary {
    farcooler_agent_core::event::SubagentSummary {
        agent_type: result.agent_type.clone(),
        model: result.resolved_model.clone(),
        tokens: result.total_tokens,
        tool_uses: result.total_tool_use_count,
        duration_ms: result.total_duration_ms,
        status: result.status.clone(),
    }
}

fn plan_entry(e: &WirePlanEntry) -> PlanEntry {
    PlanEntry { content: e.content.clone(), priority: e.priority.clone(), status: e.status.clone() }
}

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
fn tool_text(content: &[crate::wire::ToolContent]) -> Option<String> {
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
fn tool_diff(content: &[crate::wire::ToolContent]) -> Option<farcooler_agent_core::event::Diff> {
    content.iter().find(|c| c.kind == "diff").map(|c| farcooler_agent_core::event::Diff {
        path: c.path.clone(),
        old_text: c.old_text.clone(),
        new_text: c.new_text.clone().unwrap_or_default(),
    })
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
        SessionUpdate::AgentMessageChunk { content, meta } => {
            vec![AgentEvent::Message {
                role: Role::Agent,
                text: content.text.clone(),
                parent: meta.claude_code.parent_tool_use_id.clone(),
            }]
        }
        SessionUpdate::UserMessageChunk { content, meta } => {
            vec![AgentEvent::Message {
                role: Role::User,
                text: content.text.clone(),
                parent: meta.claude_code.parent_tool_use_id.clone(),
            }]
        }
        SessionUpdate::AgentThoughtChunk { content, meta } => {
            vec![AgentEvent::Message {
                role: Role::Thought,
                text: content.text.clone(),
                parent: meta.claude_code.parent_tool_use_id.clone(),
            }]
        }
        SessionUpdate::AvailableCommandsUpdate { available_commands } => {
            // Its own event, for two reasons it is worth being explicit about.
            //
            // Not a `Gap`: a gap means "history is missing here" and draws a
            // visible break telling the user their transcript is incomplete.
            // Nothing was lost — an agent resent its slash-command menu, which
            // it does once per turn whether or not anyone asked. The old
            // behavior routed this through `Unknown` and turned every single
            // turn into a false "history missing" break.
            //
            // Not a second `SessionStarted` either, even though that event has
            // an `available_commands` field meant for exactly this data. A
            // consumer is entitled to assume a session starts once, so a
            // repeat reads as a restart and resets everything derived from the
            // first one.
            vec![AgentEvent::CommandsAvailable {
                commands: available_commands
                    .iter()
                    .map(|c| farcooler_agent_core::event::AgentChoice {
                        id: c.name.clone(),
                        name: c.name.clone(),
                        description: c.description.clone(),
                    })
                    .collect(),
            }]
        }
        SessionUpdate::ToolCall { tool_call_id, title, kind, status: s, locations, content, meta } => {
            vec![AgentEvent::ToolCall {
                id: tool_call_id.clone(),
                title: title.clone(),
                kind: kind.clone(),
                status: status(s),
                locations: locations.iter().map(|l| l.path.clone()).collect(),
                parent: meta.claude_code.parent_tool_use_id.clone(),
                subagent: meta.claude_code.subagent,
            }]
            .into_iter()
            .chain(tool_text(content).map(|text| AgentEvent::ToolUpdate {
                id: tool_call_id.clone(),
                status: status(s),
                title: None,
                content: Some(text),
                diff: tool_diff(content),
                locations: Vec::new(),
                parent: meta.claude_code.parent_tool_use_id.clone(),
                subagent: None,
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
            meta,
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
                parent: meta.claude_code.parent_tool_use_id.clone(),
                // Gated on `agent_type`, and it has to be. EVERY tool reports
                // a `toolResponse` — a `Read` sends the file it read — and
                // because each field of `SubagentResult` defaults, any of them
                // deserializes into one happily. Without this an ordinary tool
                // row carried an empty subagent summary and rendered as a
                // subagent that had reported nothing. Caught by replaying a
                // real capture, which is the entire reason those exist.
                subagent: meta
                    .claude_code
                    .tool_response
                    .as_ref()
                    .filter(|r| !r.agent_type.is_empty())
                    .map(summary),
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
    use crate::wire::SessionUpdate;
    use farcooler_agent_core::event::{AgentEvent, AgentGapReason, Role, ToolStatus};

    #[test]
    fn an_agent_message_chunk_becomes_an_agent_message() {
        let update: SessionUpdate =
            serde_json::from_str(r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"}}"#)
                .expect("parses");
        let events = update_to_events(&update);
        assert_eq!(events, vec![AgentEvent::Message { role: Role::Agent, text: "hi".into(), parent: None }]);
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
                locations: vec![], parent: None, subagent: false, }]
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
            r#"{"sessionUpdate":"available_commands_update","availableCommands":[{"name":"init","description":"Set up a project"}]}"#,
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
    fn a_subagents_message_says_whose_it_is() {
        // The bug this whole change exists for: without the parent, a
        // subagent's words render as the agent that dispatched it talking.
        let raw = r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"I'll read the file."},
            "_meta":{"claudeCode":{"parentToolUseId":"toolu_01Wnr"}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::Message { parent, .. } = &update_to_events(&update)[0] else {
            panic!("expected a message")
        };
        assert_eq!(parent.as_deref(), Some("toolu_01Wnr"));
    }

    #[test]
    fn a_dispatch_row_is_marked_as_one() {
        let raw = r#"{"sessionUpdate":"tool_call","toolCallId":"t1","title":"Task","kind":"think",
            "status":"pending","_meta":{"claudeCode":{"subagent":true}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::ToolCall { subagent, .. } = &update_to_events(&update)[0] else {
            panic!("expected a tool call")
        };
        assert!(subagent);
    }

    #[test]
    fn a_finished_dispatch_carries_its_summary() {
        let raw = r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed",
            "_meta":{"claudeCode":{"toolResponse":{"status":"completed","agentType":"general-purpose",
            "resolvedModel":"claude-opus-5[1m]","totalDurationMs":4962,"totalTokens":12479,
            "totalToolUseCount":1}}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::ToolUpdate { subagent, .. } = &update_to_events(&update)[0] else {
            panic!("expected a tool update")
        };
        let summary = subagent.as_ref().expect("a summary");
        assert_eq!(summary.agent_type, "general-purpose");
        assert_eq!(summary.tokens, 12479);
        assert_eq!(summary.tool_uses, 1);
        assert_eq!(summary.duration_ms, 4962);
    }

    #[test]
    fn an_ordinary_tools_result_is_not_mistaken_for_a_subagents() {
        // Every tool reports a `toolResponse`, and every field of the subagent
        // shape defaults — so a `Read` returning a file deserialized into a
        // perfectly valid, perfectly empty subagent summary. That put a
        // subagent badge on ordinary tool rows.
        let raw = r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed",
            "_meta":{"claudeCode":{"toolName":"Read","toolResponse":{"type":"text",
            "file":{"filePath":"/tmp/main.rs","content":"fn main(){}","numLines":2}}}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::ToolUpdate { subagent, .. } = &update_to_events(&update)[0] else {
            panic!("expected a tool update")
        };
        assert!(subagent.is_none(), "a Read reported itself as a subagent: {subagent:?}");
    }

    #[test]
    fn an_ordinary_turn_carries_no_parent_at_all() {
        // Old events in SQLite have no parent field. They must keep decoding,
        // and must keep rendering at the top level.
        let raw = r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::Message { parent, .. } = &update_to_events(&update)[0] else {
            panic!("expected a message")
        };
        assert!(parent.is_none());
    }

    #[test]
    fn an_event_without_a_parent_serializes_exactly_as_it_used_to() {
        // Stored transcripts and one-release-behind clients both read this
        // JSON. A new key on every ordinary event would change bytes nothing
        // asked to change, on every row ever written.
        let event = AgentEvent::Message { role: Role::Agent, text: "hi".into(), parent: None };
        let json = serde_json::to_string(&event).expect("serializes");
        assert_eq!(json, r#"{"Message":{"role":"Agent","text":"hi"}}"#);
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
/// farcooler-sample-refactor-api/README.md`, and in a tiled pane that title is
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
            locations: vec!["/tmp/wt/src/main.rs".into()], parent: None, subagent: false, }];
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
            locations: vec!["/etc/hosts".into()], parent: None, subagent: false, }];
        relativize(&mut events, worktree);
        let AgentEvent::ToolCall { title, .. } = &events[0] else { panic!() };
        assert_eq!(title, "Read /etc/hosts");
    }
}
