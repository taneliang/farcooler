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
/// names. `SDKCommandsChangedMessage` is a third caller and takes the first
/// shape again.
///
/// "A session is built from the first and refined by the second" is what this
/// comment used to claim, and it was not true: the only caller was
/// `ClaudeBackend::start`, which sees the initialize response and nothing else,
/// so the refinement never happened. `ClaudeBackend::handle` now calls this on
/// `system` frames too, which is what makes the sentence honest — but note that
/// only the COMMANDS are taken from those, as `AgentEvent::CommandsAvailable`.
/// A session still starts exactly once.
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

/// Where a record came from, which decides whether the words in it have
/// already been drawn by someone else.
///
/// Live, the client has already drawn what you typed — see
/// `Transcript.appendLocalUserMessage` — so taking the CLI's echo would render
/// every prompt twice. History has no local echo behind it, so the same records
/// are the only place the prompts exist. The identical split codex needed, for
/// the identical reason.
///
/// The AGENT's words split the same way once `--include-partial-messages` is
/// on, and this is the trap that flag sets: the CLI sends the `stream_event`
/// deltas AND the finished `assistant` message afterwards, both carrying the
/// whole answer. Measured on 2.1.226, a two-token reply arrived as `text_delta`
/// "h", `text_delta` "i", then `assistant` `[{"type":"text","text":"hi"}]`.
/// Reading both renders every answer twice — once as it streams and again
/// underneath. So live, an assistant text or thinking block is DROPPED, because
/// the deltas already said it. On replay there are no deltas — the on-disk
/// transcript stores finished blocks — so those same blocks are the only place
/// the answer exists. Tool calls are taken either way: nothing streams them.
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
        "assistant" => assistant_to_events(frame, origin),
        "user" => user_to_events(frame, origin),
        "stream_event" => stream_to_events(frame),
        "result" => {
            let reason = end_reason(frame["stop_reason"].as_str().unwrap_or_default());
            vec![AgentEvent::TurnEnded { reason }]
        }
        // Everything else is judged by whether it CARRIES A MESSAGE, not by
        // whether its name is on a list.
        //
        // The list was the bug, twice. Claude Code's record vocabulary is open
        // and mostly metadata: a real transcript here holds `last-prompt`,
        // `mode`, `ai-title`, `pr-link`, `permission-mode`, `file-history-delta`,
        // `bridge-session`, `relocated`, `worktree-state`, `agent-name` and
        // more, none of which is conversation. Enumerating them meant every
        // name I had not seen became a Gap — a scissors icon reading "something
        // happened here that this version cannot show" over a session where
        // nothing had been lost.
        //
        // A record with no `message` is bookkeeping, and bookkeeping is not
        // missing content. One that HAS a message and is still not understood
        // is the case a Gap exists for, and that stays visible — so a genuinely
        // new kind of conversation content is still reported rather than
        // silently dropped, which is the property the whole derived transcript
        // rests on.
        _ => {
            if frame["message"]["content"].is_null() {
                Vec::new()
            } else {
                vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]
            }
        }
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

/// Words said, unless there were none.
///
/// An empty block is not silence worth an entry — 2.1.226 sends
/// `{"type":"thinking","thinking":""}` on turns with no visible reasoning, and
/// each one drew a blank thought bubble.
fn spoken_words(role: Role, text: &serde_json::Value, parent: &Option<String>) -> Vec<AgentEvent> {
    let text = text.as_str().unwrap_or_default();
    if text.is_empty() {
        return Vec::new();
    }
    vec![AgentEvent::Message { role, text: text.to_string(), parent: parent.clone() }]
}

/// The agent's own content blocks.
///
/// Live, the text and thinking blocks are deliberately skipped: the
/// `stream_event` deltas already delivered them word by word, and taking the
/// finished block on top prints every answer a second time. See `Origin`.
fn assistant_to_events(frame: &serde_json::Value, origin: Origin) -> Vec<AgentEvent> {
    let parent = parent_of(frame);
    let Some(blocks) = frame["message"]["content"].as_array() else { return Vec::new() };
    let already_streamed = origin == Origin::Live;

    blocks
        .iter()
        .flat_map(|block| match block["type"].as_str().unwrap_or_default() {
            "text" if !already_streamed => spoken_words(Role::Agent, &block["text"], &parent),
            "thinking" if !already_streamed => {
                spoken_words(Role::Thought, &block["thinking"], &parent)
            }
            "tool_use" => {
                let name = block["name"].as_str().unwrap_or_default();
                let mut out = vec![AgentEvent::ToolCall {
                    id: block["id"].as_str().unwrap_or_default().to_string(),
                    title: tool_title(name, &block["input"]),
                    kind: tool_kind(name).to_string(),
                    status: ToolStatus::InProgress,
                    locations: tool_locations(&block["input"]),
                    parent: parent.clone(),
                    // The dispatch itself, which owns a block rather than
                    // being a row inside one.
                    subagent: name == "Task" || name == "Agent",
                }];
                // A todo write is BOTH: the row is how you see the call
                // happened, and the plan is what it said. The row comes first
                // because the plan is derived from it — and both are sent,
                // because dropping the row would make the one tool call the
                // agent runs most often invisible in the transcript.
                if name == "TodoWrite"
                    && let Some(entries) = plan_from_todo(&block["input"])
                {
                    out.push(AgentEvent::Plan { entries });
                }
                out
            }
            _ => Vec::new(),
        })
        .collect()
}

/// A `stream_event` frame, as the answer arriving a piece at a time.
///
/// `SDKPartialAssistantMessage` wraps an Anthropic `BetaRawMessageStreamEvent`
/// under `event` — a type `vendor/claude-sdk.d.ts` only imports, so the shapes
/// below were read off a live 2.1.226 rather than out of the declarations.
/// Only `content_block_delta` carries words; `message_start`,
/// `content_block_start/stop`, `message_delta` and `message_stop` are framing
/// and correctly produce nothing.
///
/// One event per delta, with no buffering, because the client coalesces
/// consecutive same-role messages — see `Transcript.swift`. Buffering here
/// would only delay what the flag was added to make immediate.
fn stream_to_events(frame: &serde_json::Value) -> Vec<AgentEvent> {
    let event = &frame["event"];
    if event["type"].as_str() != Some("content_block_delta") {
        return Vec::new();
    }
    let delta = &event["delta"];
    let (role, text) = match delta["type"].as_str().unwrap_or_default() {
        "text_delta" => (Role::Agent, delta["text"].as_str().unwrap_or_default()),
        "thinking_delta" => (Role::Thought, delta["thinking"].as_str().unwrap_or_default()),
        // `input_json_delta` streams a tool call's arguments. Deliberately
        // ignored: the finished `tool_use` block carries the whole input, and a
        // row titled with half-parsed JSON is worse than one that appears a
        // moment later.
        _ => return Vec::new(),
    };
    if text.is_empty() {
        return Vec::new();
    }
    vec![AgentEvent::Message { role, text: text.to_string(), parent: parent_of(frame) }]
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
///
/// That claim was false for as long as nothing called this: `PlanPanel` was
/// fed by ACP and by codex, and a native Claude chat showed no plan at all
/// while the agent wrote todo lists all turn. `assistant_to_events` calls it
/// now, on a `tool_use` block named `TodoWrite`.
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

    /// One assistant text block, as both a live frame and a restored one.
    fn said(text: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "text", "text": text }] }
        })
    }

    #[test]
    fn the_agents_words_come_from_assistant_text_blocks_on_replay() {
        // On replay only. Live they come from the deltas — see below.
        assert!(matches!(
            frame_to_events_from(&said("hi"), Origin::Replay).as_slice(),
            [AgentEvent::Message { role: Role::Agent, text, parent: None }] if text == "hi"
        ));
    }

    #[test]
    fn an_answer_arrives_a_piece_at_a_time_while_it_is_being_written() {
        // The whole point of `--include-partial-messages`: without these the
        // pane sits on Working for the length of the answer and then prints it
        // whole. The shape is 2.1.226's, captured rather than composed.
        let delta = serde_json::json!({
            "type": "stream_event", "parent_tool_use_id": null,
            "event": { "type": "content_block_delta", "index": 0,
                       "delta": { "type": "text_delta", "text": "h" } }
        });
        assert!(matches!(
            frame_to_events(&delta).as_slice(),
            [AgentEvent::Message { role: Role::Agent, text, .. }] if text == "h"
        ));

        let thought = serde_json::json!({
            "type": "stream_event", "parent_tool_use_id": null,
            "event": { "type": "content_block_delta", "index": 0,
                       "delta": { "type": "thinking_delta", "thinking": "hmm" } }
        });
        assert!(matches!(
            frame_to_events(&thought).as_slice(),
            [AgentEvent::Message { role: Role::Thought, text, .. }] if text == "hmm"
        ));
    }

    #[test]
    fn a_streamed_answer_is_not_printed_again_when_the_block_lands() {
        // The doubling trap `--include-partial-messages` sets: 2.1.226 sends
        // BOTH the deltas and the finished `assistant` message, each carrying
        // the whole answer. Taking both renders every reply twice, once
        // streamed and once underneath.
        assert!(frame_to_events(&said("hi")).is_empty(), "the deltas already said it");
    }

    #[test]
    fn a_restored_answer_survives_because_history_has_no_deltas() {
        // The other half of the same rule. `stream_event` frames are never
        // written to the on-disk transcript, so suppressing the finished block
        // on replay too would restore a conversation with the agent's side
        // missing.
        let raw = serde_json::to_string(&said("ok")).expect("json");
        assert!(matches!(
            history_to_events(&raw).as_slice(),
            [AgentEvent::Message { role: Role::Agent, text, .. }] if text == "ok"
        ));
    }

    #[test]
    fn a_tool_call_is_reported_whether_it_is_live_or_restored() {
        // Nothing streams a tool_use block, so the live/replay split that
        // silences text must not touch it.
        let frame = serde_json::json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "tool_use", "id": "t1", "name": "Bash",
                                       "input": { "command": "ls" } }] }
        });
        for origin in [Origin::Live, Origin::Replay] {
            assert!(
                matches!(
                    frame_to_events_from(&frame, origin).as_slice(),
                    [AgentEvent::ToolCall { title, .. }] if title == "ls"
                ),
                "{origin:?} lost the tool row"
            );
        }
    }

    #[test]
    fn a_todo_write_draws_the_plan_as_well_as_the_row() {
        // Both, not either: the row is how you see the call happened, the plan
        // is what it said. `plan_from_todo` existed and was tested and nothing
        // called it, so a native Claude chat showed no plan panel at all while
        // ACP and codex both fed one.
        let frame = serde_json::json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "tool_use", "id": "t1", "name": "TodoWrite",
                "input": { "todos": [{ "content": "write it", "status": "in_progress" }] } }] }
        });
        let events = frame_to_events(&frame);
        assert!(
            matches!(
                events.as_slice(),
                [AgentEvent::ToolCall { .. }, AgentEvent::Plan { entries }]
                    if entries.len() == 1 && entries[0].content == "write it"
            ),
            "{events:?}"
        );
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
        // ACP has to smuggle this through _meta.claudeCode. Native has a field,
        // and the streaming deltas carry it too.
        let frame = serde_json::json!({
            "type": "assistant", "parent_tool_use_id": "t1",
            "message": { "content": [{ "type": "text", "text": "inside" }] }
        });
        assert!(matches!(
            frame_to_events_from(&frame, Origin::Replay).as_slice(),
            [AgentEvent::Message { parent: Some(p), .. }] if p == "t1"
        ));

        let delta = serde_json::json!({
            "type": "stream_event", "parent_tool_use_id": "t1",
            "event": { "type": "content_block_delta",
                       "delta": { "type": "text_delta", "text": "inside" } }
        });
        assert!(matches!(
            frame_to_events(&delta).as_slice(),
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
    fn an_unknown_record_carrying_a_message_is_a_visible_gap() {
        // The property the derived transcript rests on: a new kind of
        // conversation content is reported, never silently dropped.
        assert!(matches!(
            frame_to_events(&serde_json::json!({
                "type": "something_new",
                "message": { "content": [{ "type": "text", "text": "words" }] }
            }))
            .as_slice(),
            [AgentEvent::Gap { reason: AgentGapReason::Unparsed }]
        ));
    }

    #[test]
    fn unknown_bookkeeping_is_silent_because_nothing_was_lost() {
        // These are real record types out of real transcripts. Judging them by
        // name meant every one I had not seen drew "something happened here
        // that this version cannot show" over a session where nothing had.
        for kind in [
            "last-prompt",
            "mode",
            "ai-title",
            "pr-link",
            "permission-mode",
            "file-history-delta",
            "bridge-session",
            "relocated",
            "worktree-state",
            "agent-name",
            "queue-operation",
            "attachment",
        ] {
            assert!(
                frame_to_events(&serde_json::json!({ "type": kind })).is_empty(),
                "{kind} is bookkeeping, not lost content"
            );
        }
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
