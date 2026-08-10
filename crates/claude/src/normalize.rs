//! Claude stream-json frames, as `AgentEvent`.
//!
//! Lenient for the reason `acp/wire.rs` gives: a strict decoder turns every CLI
//! release into an outage, and the honest answer to a frame we do not know is a
//! visible `Gap`.
//!
//! Every shape was read off a real turn against claude 2.1.226, saved as
//! `tests/fixtures/turn_basic.jsonl` by `tests/fixtures/capture_turn.py`.

use farcooler_agent_core::event::{
    AgentChoice, AgentEvent, AgentGapReason, EndReason, PermissionOption, PlanEntry, Role,
    ToolStatus,
};

/// One model the session offers, and the reasoning depths it supports.
///
/// Efforts are per MODEL — Opus offers up to `max`, others stop short — so a
/// single hardcoded list would offer settings the model would reject. The same
/// shape codex turned out to need.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// What `set_model` wants. `default` is a real value meaning "the session
    /// default", not an absence.
    pub value: String,
    pub name: String,
    pub description: String,
    pub efforts: Vec<String>,
}

/// What the session says it is.
#[derive(Debug, Default, Clone)]
pub struct Init {
    pub session_id: String,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub commands: Vec<AgentChoice>,
    pub models: Vec<ModelInfo>,
    pub output_style: Option<String>,
    pub output_styles: Vec<String>,
}

/// Read the session announcement.
///
/// Handles BOTH shapes, because the two carry the same facts under different
/// names and arrive at different times. The `control_response` to `initialize`
/// answers immediately with `commands` as objects; the `system: init` frame
/// answers only once a prompt has been sent, with `slash_commands` as bare
/// names. A session is built from the first and refined by the second.
pub fn init_from(frame: &serde_json::Value) -> Init {
    let commands = if let Some(list) = frame["commands"].as_array() {
        list.iter()
            .filter_map(|c| {
                let name = c["name"].as_str()?.to_string();
                Some(AgentChoice {
                    description: c["description"].as_str().unwrap_or_default().to_string(),
                    id: name.clone(),
                    name,
                })
            })
            .collect()
    } else {
        // `system: init` sends names only. A picker you have to already know
        // still beats no picker.
        frame["slash_commands"]
            .as_array()
            .map(|names| {
                names
                    .iter()
                    .filter_map(|n| n.as_str())
                    .map(|n| AgentChoice {
                        id: n.to_string(),
                        name: n.to_string(),
                        description: String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let models: Vec<ModelInfo> = frame["models"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|m| {
                    let value = m["value"].as_str()?.to_string();
                    Some(ModelInfo {
                        name: m["displayName"].as_str().unwrap_or(&value).to_string(),
                        description: m["description"].as_str().unwrap_or_default().to_string(),
                        efforts: m["supportedEffortLevels"]
                            .as_array()
                            .map(|e| {
                                e.iter().filter_map(|x| x.as_str()).map(str::to_string).collect()
                            })
                            .unwrap_or_default(),
                        value,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Init {
        session_id: frame["session_id"].as_str().unwrap_or_default().to_string(),
        // The initialize response names no current model, so the first entry
        // stands in — it is `default`, which IS the session's model until
        // something changes it.
        model: frame["model"]
            .as_str()
            .map(str::to_string)
            .or_else(|| models.first().map(|m| m.value.clone())),
        permission_mode: frame["permissionMode"]
            .as_str()
            .or_else(|| frame["current_permission_mode"].as_str())
            .map(str::to_string),
        commands,
        models,
        output_style: frame["output_style"].as_str().map(str::to_string),
        output_styles: frame["available_output_styles"]
            .as_array()
            .map(|s| s.iter().filter_map(|x| x.as_str()).map(str::to_string).collect())
            .unwrap_or_default(),
    }
}

/// Where a record came from, which decides whether the user's own words are
/// part of it.
///
/// Live, the client has already drawn what you typed — see
/// `Transcript.appendLocalUserMessage` — so taking the CLI's echo would render
/// every prompt twice. History has no local echo behind it, so the same records
/// are the only place the prompts exist. The identical split codex needed, for
/// the identical reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Live,
    Replay,
}

/// What one stream-json frame means, if anything.
pub fn frame_to_events(frame: &serde_json::Value) -> Vec<AgentEvent> {
    frame_to_events_from(frame, Origin::Live)
}

/// As `frame_to_events`, for a caller that knows where the record came from.
pub fn frame_to_events_from(frame: &serde_json::Value, origin: Origin) -> Vec<AgentEvent> {
    match frame["type"].as_str().unwrap_or_default() {
        "assistant" => assistant_to_events(frame),
        "user" => user_to_events(frame, origin),
        "result" => {
            let reason = end_reason(frame["stop_reason"].as_str().unwrap_or_default());
            vec![AgentEvent::TurnEnded { reason }]
        }
        // Bookkeeping. `init` is read separately by the backend, and hooks
        // firing are not transcript material — a Gap for one would draw
        // "history missing" over an ordinary session start.
        //
        // The last four appear only in the on-disk transcript, never on the
        // wire: the file records queue bookkeeping and hook attachments that
        // the stream never sends. Left unlisted they became a Gap per record,
        // which is a scissors icon between every restored turn.
        "system" | "rate_limit_event" | "control_response" | "control_request"
        | "control_cancel_request" | "stream_event" | "queue-operation" | "attachment"
        | "summary" | "file-history-snapshot" => Vec::new(),
        _ => vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }],
    }
}

/// A conversation restored from the transcript Claude Code writes to disk.
///
/// `--resume` does NOT replay it. Measured against 2.1.226: resuming attaches
/// the session so the agent still has its context, and sends nothing at all —
/// four hook frames and the control response, no `assistant`, no `user`. So a
/// pane that waited for a replay showed an empty conversation, which is what it
/// did.
///
/// The transcript is a JSONL file whose `user` and `assistant` records carry
/// the same `message.content` shape the wire does, so the same normalizer reads
/// them. Two things differ and both would have been silent: the parent field is
/// `parentToolUseID` on disk against `parent_tool_use_id` on the wire, and the
/// file interleaves record types the stream never sends.
pub fn history_to_events(transcript: &str) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    for line in transcript.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            // A truncated last line is normal in a file being appended to.
            continue;
        };
        events.extend(frame_to_events_from(&record, Origin::Replay));
    }
    events
}

/// The dispatch a record belongs to, when a subagent produced it.
///
/// ACP smuggles this through `_meta.claudeCode`; here it is a first-class
/// field, which is one of the concrete things the native path buys.
///
/// Spelled BOTH ways, because the two sources disagree: the wire sends
/// `parent_tool_use_id` and the on-disk transcript writes `parentToolUseID`.
/// Reading only one silently flattens every restored subagent into the parent
/// conversation.
fn parent_of(record: &serde_json::Value) -> Option<String> {
    record["parent_tool_use_id"]
        .as_str()
        .or_else(|| record["parentToolUseID"].as_str())
        .map(str::to_string)
}

/// The agent's own content blocks.
fn assistant_to_events(frame: &serde_json::Value) -> Vec<AgentEvent> {
    let parent = parent_of(frame);
    let Some(blocks) = frame["message"]["content"].as_array() else { return Vec::new() };

    blocks
        .iter()
        .filter_map(|block| match block["type"].as_str().unwrap_or_default() {
            "text" => {
                let text = block["text"].as_str().unwrap_or_default().to_string();
                (!text.is_empty()).then(|| AgentEvent::Message {
                    role: Role::Agent,
                    text,
                    parent: parent.clone(),
                })
            }
            "thinking" => {
                let text = block["thinking"].as_str().unwrap_or_default().to_string();
                (!text.is_empty()).then(|| AgentEvent::Message {
                    role: Role::Thought,
                    text,
                    parent: parent.clone(),
                })
            }
            "tool_use" => {
                let name = block["name"].as_str().unwrap_or_default();
                Some(AgentEvent::ToolCall {
                    id: block["id"].as_str().unwrap_or_default().to_string(),
                    title: tool_title(name, &block["input"]),
                    kind: tool_kind(name).to_string(),
                    status: ToolStatus::InProgress,
                    locations: tool_locations(&block["input"]),
                    parent: parent.clone(),
                    // The dispatch itself, which owns a block rather than
                    // being a row inside one.
                    subagent: name == "Task" || name == "Agent",
                })
            }
            _ => None,
        })
        .collect()
}

/// A `user` record: tool results, and the prompt itself.
fn user_to_events(frame: &serde_json::Value, origin: Origin) -> Vec<AgentEvent> {
    let parent = parent_of(frame);
    // The on-disk transcript writes a bare string for a plain prompt, where the
    // wire always sends blocks. Reading only the array shape loses every
    // restored prompt that had no tool result beside it.
    if let Some(text) = frame["message"]["content"].as_str() {
        if origin == Origin::Live || text.is_empty() {
            return Vec::new();
        }
        return vec![AgentEvent::Message {
            role: Role::User,
            text: text.to_string(),
            parent,
        }];
    }
    let Some(blocks) = frame["message"]["content"].as_array() else { return Vec::new() };

    blocks
        .iter()
        .filter_map(|block| {
            if block["type"].as_str() != Some("tool_result") {
                // A text block here is the prompt. Live, the client already
                // drew it and taking it again is the doubling bug; restoring
                // history, this is the only place it exists.
                if origin == Origin::Replay && block["type"].as_str() == Some("text") {
                    let text = block["text"].as_str().unwrap_or_default().to_string();
                    return (!text.is_empty()).then(|| AgentEvent::Message {
                        role: Role::User,
                        text,
                        parent: parent.clone(),
                    });
                }
                return None;
            }
            let failed = block["is_error"].as_bool().unwrap_or(false);
            Some(AgentEvent::ToolUpdate {
                id: block["tool_use_id"].as_str().unwrap_or_default().to_string(),
                status: if failed { ToolStatus::Failed } else { ToolStatus::Completed },
                title: None,
                content: result_text(&block["content"]),
                diff: None,
                locations: Vec::new(),
                parent: parent.clone(),
                subagent: None,
            })
        })
        .collect()
}

/// A tool result's text, whether it came as a string or as blocks.
fn result_text(content: &serde_json::Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return (!text.is_empty()).then(|| text.to_string());
    }
    let joined: String = content
        .as_array()?
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (!joined.is_empty()).then_some(joined)
}

/// What a tool row should say it is doing.
///
/// The command for a shell, the path for a file operation, the tool's own name
/// otherwise. A row reading "Bash" tells you less than the command it ran.
fn tool_title(name: &str, input: &serde_json::Value) -> String {
    let field = |key: &str| input[key].as_str().map(str::to_string);
    match name {
        "Bash" | "BashOutput" => field("command").unwrap_or_else(|| name.to_string()),
        "Read" | "Write" | "Edit" | "NotebookEdit" => {
            field("file_path").unwrap_or_else(|| name.to_string())
        }
        "Glob" | "Grep" => field("pattern").unwrap_or_else(|| name.to_string()),
        "WebFetch" => field("url").unwrap_or_else(|| name.to_string()),
        "WebSearch" => field("query").unwrap_or_else(|| name.to_string()),
        "Task" | "Agent" => field("description").unwrap_or_else(|| name.to_string()),
        _ => name.to_string(),
    }
}

/// The generic kind a client renders an icon from.
fn tool_kind(name: &str) -> &'static str {
    match name {
        "Bash" | "BashOutput" | "KillShell" => "execute",
        "Read" | "NotebookRead" => "read",
        "Write" | "Edit" | "NotebookEdit" => "edit",
        "Glob" | "Grep" => "search",
        "WebFetch" | "WebSearch" => "fetch",
        "Task" | "Agent" => "delegate",
        _ => "other",
    }
}

/// Paths a tool call names, so a client can offer to open them.
fn tool_locations(input: &serde_json::Value) -> Vec<String> {
    input["file_path"]
        .as_str()
        .map(|p| vec![p.to_string()])
        .unwrap_or_default()
}

/// A turn's `stop_reason`, as the reason it ended.
///
/// An unrecognized reason is `EndTurn` rather than an error: the turn IS over
/// whatever it was called, and refusing to admit that leaves the pane on
/// Working forever.
pub fn end_reason(stop_reason: &str) -> EndReason {
    match stop_reason {
        "cancelled" | "canceled" | "interrupted" | "abort" => EndReason::Cancelled,
        "refusal" => EndReason::Refusal,
        "max_tokens" | "max_output_tokens" => EndReason::MaxTokens,
        _ => EndReason::EndTurn,
    }
}

/// A `can_use_tool` control request, as the event that blocks a fleet row.
pub fn permission_event(request_id: &str, request: &serde_json::Value) -> AgentEvent {
    let name = request["tool_name"].as_str().unwrap_or("this tool");
    AgentEvent::Permission {
        id: request_id.to_string(),
        tool_call: request["tool_use_id"].as_str().unwrap_or_default().to_string(),
        options: vec![
            PermissionOption {
                id: "allow".into(),
                name: format!("Allow {}", tool_title(name, &request["input"])),
                kind: "allow_once".into(),
            },
            PermissionOption {
                id: "deny".into(),
                name: "Deny".into(),
                kind: "reject_once".into(),
            },
        ],
    }
}

/// The plan, when a `TodoWrite` names one.
///
/// Claude has no plan frame of its own — the todo list IS the plan, and it
/// arrives as an ordinary tool call. Reading it here is what lets the same plan
/// UI serve both agents.
pub fn plan_from_todo(input: &serde_json::Value) -> Option<Vec<PlanEntry>> {
    let todos = input["todos"].as_array()?;
    Some(
        todos
            .iter()
            .map(|t| PlanEntry {
                content: t["content"].as_str().unwrap_or_default().to_string(),
                priority: String::new(),
                status: t["status"].as_str().unwrap_or_default().to_string(),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agents_words_come_from_assistant_text_blocks() {
        let frame = serde_json::json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "text", "text": "hi" }] }
        });
        assert!(matches!(
            frame_to_events(&frame).as_slice(),
            [AgentEvent::Message { role: Role::Agent, text, parent: None }] if text == "hi"
        ));
    }

    #[test]
    fn a_prompt_coming_back_is_not_drawn_again() {
        // The doubling that showed up on codex, prevented here by construction:
        // only tool_result blocks are taken from a `user` frame.
        let frame = serde_json::json!({
            "type": "user", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "text", "text": "hello" }] }
        });
        assert!(frame_to_events(&frame).is_empty());
    }

    #[test]
    fn a_tool_row_says_what_it_is_doing_not_which_tool_did_it() {
        // "Bash" tells you less than the command it ran.
        assert_eq!(tool_title("Bash", &serde_json::json!({ "command": "ls -la" })), "ls -la");
        assert_eq!(tool_title("Read", &serde_json::json!({ "file_path": "/a/b.rs" })), "/a/b.rs");
        assert_eq!(tool_title("Mystery", &serde_json::json!({})), "Mystery");
    }

    #[test]
    fn a_subagent_dispatch_is_marked_as_one() {
        // It owns a block rather than being a row inside one.
        let frame = serde_json::json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "tool_use", "id": "t1", "name": "Task",
                                       "input": { "description": "find the bug" } }] }
        });
        assert!(matches!(
            frame_to_events(&frame).as_slice(),
            [AgentEvent::ToolCall { subagent: true, title, .. }] if title == "find the bug"
        ));
    }

    #[test]
    fn a_subagents_words_carry_the_dispatch_they_belong_to() {
        // ACP has to smuggle this through _meta.claudeCode. Native has a field.
        let frame = serde_json::json!({
            "type": "assistant", "parent_tool_use_id": "t1",
            "message": { "content": [{ "type": "text", "text": "inside" }] }
        });
        assert!(matches!(
            frame_to_events(&frame).as_slice(),
            [AgentEvent::Message { parent: Some(p), .. }] if p == "t1"
        ));
    }

    #[test]
    fn a_failed_tool_result_is_marked_failed() {
        let frame = serde_json::json!({
            "type": "user", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1",
                                       "is_error": true, "content": "boom" }] }
        });
        assert!(matches!(
            frame_to_events(&frame).as_slice(),
            [AgentEvent::ToolUpdate { status: ToolStatus::Failed, content: Some(c), .. }]
                if c == "boom"
        ));
    }

    #[test]
    fn housekeeping_frames_are_silent_rather_than_gaps() {
        for kind in ["system", "rate_limit_event", "control_response", "stream_event"] {
            assert!(
                frame_to_events(&serde_json::json!({ "type": kind })).is_empty(),
                "{kind} should be silent"
            );
        }
    }

    #[test]
    fn an_unknown_frame_is_a_visible_gap() {
        assert!(matches!(
            frame_to_events(&serde_json::json!({ "type": "something_new" })).as_slice(),
            [AgentEvent::Gap { reason: AgentGapReason::Unparsed }]
        ));
    }

    #[test]
    fn an_unfamiliar_stop_reason_still_ends_the_turn() {
        assert!(matches!(end_reason("something"), EndReason::EndTurn));
        assert!(matches!(end_reason("interrupted"), EndReason::Cancelled));
        assert!(matches!(end_reason("max_output_tokens"), EndReason::MaxTokens));
    }

    #[test]
    fn the_todo_list_is_the_plan() {
        // Claude has no plan frame; the todo write IS one, which is what lets
        // the same plan UI serve both agents.
        let entries = plan_from_todo(&serde_json::json!({
            "todos": [{ "content": "write it", "status": "in_progress" }]
        }))
        .expect("a plan");
        assert_eq!(entries[0].content, "write it");
        assert_eq!(entries[0].status, "in_progress");
        assert!(plan_from_todo(&serde_json::json!({})).is_none());
    }
}
