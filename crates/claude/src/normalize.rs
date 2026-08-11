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
/// The AGENT's words need the same care once `--include-partial-messages` is
/// on, but NOT on this flag — see `Live`, which decides that from evidence
/// rather than from which side of the wire a frame came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Live,
    Replay,
}

/// A live conversation, and which of its messages the deltas already drew.
///
/// This exists because `--include-partial-messages` sets a trap: the CLI sends
/// the `stream_event` deltas AND the finished `assistant` message, both
/// carrying the whole answer. Measured on 2.1.226, a two-token reply arrived as
/// `text_delta` "h", `text_delta` "i", then `assistant`
/// `[{"type":"text","text":"hi"}]`. Taking both renders every answer twice.
///
/// The first fix was to drop text and thinking from every LIVE assistant frame,
/// which was wrong in a way this codebase specifically refuses. It assumed the
/// deltas rather than observing them, so anything that cost them — the flag
/// regressing, a user `args` entry, a CLI path that finishes a message without
/// streaming it — deleted the agent's entire side of the conversation with no
/// Gap, no error and nothing on screen to say so. A derived transcript is
/// supposed to be able to say where it is incomplete; silent total text loss is
/// the exact failure it is built to refuse.
///
/// So the drop is keyed to EVIDENCE: a message's words are suppressed only if a
/// `text_delta` or `thinking_delta` was actually seen for that message. No
/// deltas, no suppression, and the finished block renders as it always did.
/// Every uncertainty here errs toward drawing an answer twice, which is visible
/// and reportable, rather than toward dropping it, which is neither.
///
/// Correlated by `message.id`, which `message_start` and the finished
/// `assistant` frame both carry and which was confirmed identical across them.
/// A set rather than a take: one `message_start` can be followed by SEVERAL
/// assistant frames sharing its id — a real dispatch sent the thinking block
/// and the tool_use block as two frames, one message — so consuming the entry
/// on the first would un-suppress the rest.
#[derive(Debug, Default)]
pub struct Live {
    /// The message currently being streamed, from the last `message_start`.
    ///
    /// A `content_block_delta` does not name its message, so the deltas are
    /// attributed to the message most recently started. That is sound only
    /// while one stream runs at a time, which holds on this pin: a real
    /// subagent dispatch produced NO `stream_event` frame with a
    /// `parent_tool_use_id` at all, so there is no second stream to interleave
    /// with. If that ever changes the misattribution shows up as an answer
    /// drawn twice, not as one lost.
    streaming: Option<String>,
    /// Messages whose words have actually arrived as deltas.
    drawn: std::collections::HashSet<String>,
}

impl Live {
    /// What one frame off the wire means, remembering what streamed.
    pub fn frame_to_events(&mut self, frame: &serde_json::Value) -> Vec<AgentEvent> {
        match frame["type"].as_str().unwrap_or_default() {
            "stream_event" => {
                self.remember(frame);
                stream_to_events(frame)
            }
            "assistant" => {
                let drawn = frame["message"]["id"]
                    .as_str()
                    .is_some_and(|id| self.drawn.contains(id));
                assistant_to_events(frame, drawn)
            }
            "result" => {
                // A turn is the natural lifetime: nothing streams after the
                // result, so holding these any longer only grows the set.
                self.streaming = None;
                self.drawn.clear();
                frame_to_events_from(frame, Origin::Live)
            }
            _ => frame_to_events_from(frame, Origin::Live),
        }
    }

    /// Note what a `stream_event` proves about the message in flight.
    fn remember(&mut self, frame: &serde_json::Value) {
        let event = &frame["event"];
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                self.streaming = event["message"]["id"].as_str().map(str::to_string);
            }
            // Only words count as evidence. `input_json_delta` and
            // `signature_delta` are real deltas that draw nothing — the first
            // streams a tool call's arguments, the second an encrypted thinking
            // signature — so treating either as proof would suppress a block
            // nobody had seen.
            "content_block_delta" => {
                let delta = &event["delta"];
                let words = match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => delta["text"].as_str(),
                    "thinking_delta" => delta["thinking"].as_str(),
                    _ => None,
                };
                if words.is_some_and(|w| !w.is_empty())
                    && let Some(id) = &self.streaming
                {
                    self.drawn.insert(id.clone());
                }
            }
            _ => {}
        }
    }
}

/// What one stream-json frame means, if anything.
///
/// Suppresses NOTHING on its own: with no record of what streamed, the only
/// safe answer is to render what the frame carries. `Live` is what knows
/// better, and it is the only thing that may drop an answer.
pub fn frame_to_events_from(frame: &serde_json::Value, origin: Origin) -> Vec<AgentEvent> {
    match frame["type"].as_str().unwrap_or_default() {
        "assistant" => assistant_to_events(frame, false),
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
    history_to_events_with(transcript, &mut Tasks::default())
}

/// The same, against a task list the caller goes on using.
///
/// `Tasks` is the one piece of this normalizer whose state has to OUTLIVE the
/// restore. A `TaskUpdate` arriving after a resume names an id that was handed
/// out before it, so a pane that rebuilt its list into a throwaway tracker
/// would draw the restored tasks once and then ignore every update to them —
/// see `Tasks` for why an unknown id is ignored rather than invented.
/// `ClaudeBackend::start` passes its own.
pub fn history_to_events_with(transcript: &str, tasks: &mut Tasks) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    for line in transcript.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            // A truncated last line is normal in a file being appended to.
            continue;
        };
        let mut from_record = frame_to_events_from(&record, Origin::Replay);
        tasks.fold(&record, &mut from_record);
        events.extend(from_record);
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
/// An empty block is not silence worth an entry. 2.1.226 really does send
/// `{"type":"thinking","thinking":"","signature":"…"}` — encrypted reasoning,
/// where the signature is the whole payload and there is no text at all — and
/// every one of those drew a blank thought bubble. Reachable from both
/// directions: on replay, and live whenever a block arrives with no deltas
/// behind it.
fn spoken_words(role: Role, text: &serde_json::Value, parent: &Option<String>) -> Vec<AgentEvent> {
    let text = text.as_str().unwrap_or_default();
    if text.is_empty() {
        return Vec::new();
    }
    vec![AgentEvent::Message { role, text: text.to_string(), parent: parent.clone() }]
}

/// The agent's own content blocks.
///
/// `already_drawn` says the deltas have already delivered this message's words
/// word by word, so taking the finished blocks on top would print the answer a
/// second time. Only `Live` can know that, and only from having watched the
/// deltas arrive — see `Live` for why it is never assumed.
///
/// Tool calls are taken either way. They DO stream — 2.1.226 opens a
/// `content_block_start` carrying `{"type":"tool_use",…}` and then chunks the
/// arguments as `input_json_delta` — but `stream_to_events` deliberately
/// ignores both, so the finished block is the only place a tool row ever comes
/// from and suppressing it would lose the call entirely.
fn assistant_to_events(frame: &serde_json::Value, already_drawn: bool) -> Vec<AgentEvent> {
    let parent = parent_of(frame);
    let Some(blocks) = frame["message"]["content"].as_array() else { return Vec::new() };

    blocks
        .iter()
        .flat_map(|block| match block["type"].as_str().unwrap_or_default() {
            "text" if !already_drawn => spoken_words(Role::Agent, &block["text"], &parent),
            "thinking" if !already_drawn => {
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
///
/// The task tools are here because of what the fallback did to them: a session
/// that called `TaskCreate` five times drew five rows every one of which said
/// literally "TaskCreate", which is the worst version of this — five rows that
/// are not merely uninformative but indistinguishable. `TaskUpdate` has the
/// same shape of problem and less to work with, since an update that only moves
/// a status carries nothing but `taskId`.
///
/// `TaskList` deliberately gets no case: it takes no parameters at all, so its
/// own name is the whole truth about it.
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
        // `Task` and `Agent` are the SUBAGENT dispatch, unrelated to the
        // `Task*` tools below despite the shared prefix.
        "Task" | "Agent" => field("description").unwrap_or_else(|| name.to_string()),
        "TaskCreate" => field("subject").unwrap_or_else(|| name.to_string()),
        "TaskUpdate" => task_update_title(input).unwrap_or_else(|| name.to_string()),
        _ => name.to_string(),
    }
}

/// A `TaskUpdate` row, named for whatever it actually says.
///
/// A new `subject` is a rename and IS the row. Otherwise the only required
/// field is `taskId`, so the row is the task it moved plus the state it moved
/// it to — because the update most often made is a status change, and without
/// the status three of those in a row would read identically, which is the
/// complaint that brought this whole function here.
fn task_update_title(input: &serde_json::Value) -> Option<String> {
    if let Some(subject) = input["subject"].as_str().filter(|s| !s.is_empty()) {
        return Some(subject.to_string());
    }
    let id = task_key(input["taskId"].as_str()?);
    Some(match input["status"].as_str().filter(|s| !s.is_empty()) {
        Some(status) => format!("Task #{id}: {status}"),
        None => format!("Task #{id}"),
    })
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

/// A task id as this crate keys tasks by.
///
/// `#` is display sugar. The result sentence writes `Task #2` and `TaskUpdate`
/// is documented to take `"2"`, so both are trimmed to the same key. That costs
/// nothing and means a model that writes `taskId: "#2"` still hits the task it
/// meant, instead of being discarded as an id nobody created.
fn task_key(id: &str) -> String {
    id.trim().trim_start_matches('#').trim().to_string()
}

/// The id a `TaskCreate` was given, read out of the sentence announcing it.
///
/// THIS IS STRING PARSING OF A HUMAN-READABLE TOOL RESULT, and it is fragile on
/// purpose rather than by oversight — there is no other source. `TaskCreate`'s
/// input carries no id at all: it takes a `subject` and a `description`, and
/// every task is created `pending`. `TaskUpdate` addresses tasks by `taskId`.
/// The only place those two facts are ever joined is the result text,
/// `Task #2 created successfully: <subject>`. So a CLI that rewords that
/// sentence silently costs every later update its target, and nothing in the
/// protocol will say so. That is exactly why an unparsed create still lands in
/// the panel under a key of its own — see `Tasks::create`. The failure this can
/// cause is "that task stops responding to updates", never "that task is
/// invisible".
///
/// Anchored on `Task #` AND on the words after the id, because "created
/// successfully" alone is not distinctive: on this same pin a `Write` answers
/// "File created successfully at: /a/b.rs".
///
/// The sentence was written here from the tool's declaration and has since been
/// MEASURED against a live call, which answered exactly
/// `Task #1 created successfully: Verify the TaskCreate result string format`.
/// `TaskUpdate` takes the bare number — `"1"`, no `#` — and answers
/// `Updated task #1 status`, so the `#` really is display sugar on one side of
/// the pair and absent on the other, which is why `task_key` trims it.
fn created_task_id(result: &str) -> Option<String> {
    let (_, after) = result.split_once("Task #")?;
    let (id, rest) = after.split_once(' ')?;
    (!id.is_empty() && rest.starts_with("created successfully")).then(|| task_key(id))
}

/// One task, as the panel draws it.
#[derive(Debug)]
struct Task {
    /// `TaskUpdate`'s `taskId`, or a stand-in when the result could not be read.
    key: String,
    subject: String,
    status: String,
}

/// A `TaskCreate` or `TaskUpdate` whose result has not come back yet.
#[derive(Debug)]
struct PendingCall {
    name: String,
    input: serde_json::Value,
}

/// The task list `TaskCreate` and `TaskUpdate` build, as the plan panel.
///
/// Claude has no plan frame; `plan_from_todo` already reads a `TodoWrite` as
/// one. The task tools are the second way the same agent keeps the same kind of
/// list, and until this existed they were five rows titled "TaskCreate" and no
/// panel at all.
///
/// Stateful, and here rather than in `backend.rs` for the reason `Live` is here:
/// `history_to_events` has to rebuild the same list off a restored transcript,
/// and it can only do that if the machine that builds it is reachable from this
/// module. `ClaudeBackend` owns the instance, exactly as it owns `Live`.
///
/// # Why the list changes on the RESULT and never on the call
///
/// A `TaskUpdate` could be applied the moment its `tool_use` block lands, and
/// that would draw sooner. It is not, and the reason is not tidiness:
///
/// - `TaskCreate` has no choice. Its id exists only in its result, so a task
///   cannot be listed under the id later updates will use until the result
///   arrives.
/// - Mixing the two therefore REORDERS them. One assistant message can carry
///   several tool_use blocks — the report that prompted this had five creates
///   in a row — so an optimistic `TaskUpdate` would be applied before the
///   create it follows had been confirmed, hit an id this tracker had not
///   learned yet, and be dropped by the rule two paragraphs down. A dropped
///   update is silent and permanent; a late one costs milliseconds, because
///   these tools are local and their results follow immediately.
/// - A call can also be DENIED. An optimistic panel would show a task moved to
///   `completed` that the agent was never allowed to move, and nothing would
///   ever correct it.
///
/// So: confirmed, uniformly. Consistency is worth more than the latency here.
///
/// # The rules that make it safe
///
/// - An update naming an id this pane never watched being created is IGNORED,
///   not invented. A subagent owns tasks of its own that this pane never saw
///   created, and a row conjured out of an id with no subject would be a blank
///   line in the panel.
/// - `status: "deleted"` removes a task rather than rendering as a state. The
///   Mac app's `PlanStatus` knows `pending` / `in_progress` / `completed`,
///   which is TodoWrite's vocabulary and happens to be the task tools' too;
///   `deleted` is the one word that is not a state a task can be shown in.
/// - `Plan` is emitted only when the list actually moved, for the reason
///   `commands_sent` exists: this rides the same ring to every subscriber, and
///   an unchanged list re-announced is traffic for nothing.
///
/// # Two writers, one panel
///
/// `TodoWrite` and the task tools can both be live in one session, and they
/// draw the same surface. The task tools win WHILE THEY HAVE A LIST, and the
/// asymmetry is what decides it rather than a preference: a `TodoWrite` always
/// sends its whole list, so it can re-assert itself on its very next call, but
/// a task list is ACCUMULATED across many calls and cannot be re-derived — once
/// a todo list has replaced it, nothing short of the agent re-creating every
/// task brings it back. So the recoverable writer yields to the unrecoverable
/// one. It is not sticky: when the last task is deleted the list is empty, the
/// task tools stop suppressing anything, and `TodoWrite` owns the panel again
/// rather than leaving it dead.
///
/// # This list is not the tool's list, deliberately
///
/// Measured against a live pair of calls: a task moved to `completed` DISAPPEARS
/// from `TaskList` — it answered "No tasks found" with one completed task
/// outstanding — and a later `TaskUpdate` naming it answers "Task not found".
/// So the tool's own list is work REMAINING, and this one is not: a completed
/// task stays on the panel, struck through, exactly as a finished `TodoWrite`
/// entry does. That divergence is the point of the panel. A plan you are
/// watching should show what has been done, and a list that empties itself as
/// the agent succeeds would read as the plan being lost rather than finished.
///
/// It also means the "Task not found" answer to an update of a completed task
/// is an ERROR result, which changes nothing here — the same rule that already
/// protects the panel from a denied call.
#[derive(Debug, Default)]
pub struct Tasks {
    /// Every task, in the order it was created — which is the order the panel
    /// draws, so updates edit in place and never reorder.
    tasks: Vec<Task>,
    /// Task calls awaiting their result, by `tool_use` id — the only thing that
    /// correlates a call with the result carrying its id.
    pending: std::collections::HashMap<String, PendingCall>,
    /// The entries last handed out, so an unchanged list is not re-announced.
    sent: Vec<PlanEntry>,
}

impl Tasks {
    /// Fold one frame's task bookkeeping into the events it produced.
    ///
    /// Takes the events rather than returning some, because the two-writers
    /// rule above can only be applied where a `TodoWrite`'s plan and the task
    /// list are both in view — and this is the one place they are.
    pub fn fold(&mut self, frame: &serde_json::Value, events: &mut Vec<AgentEvent>) {
        match frame["type"].as_str().unwrap_or_default() {
            "assistant" => self.remember_calls(frame),
            "user" => self.apply_results(frame),
            "result" => {
                // A tool call and its result always land inside one turn, so
                // anything still waiting when the turn ends — a cancel with a
                // call in flight — is never going to be answered. The same
                // bound `permission_inputs` gets, for the same reason: without
                // it the map only grows.
                self.pending.clear();
                return;
            }
            _ => return,
        }

        if !self.tasks.is_empty() {
            // The task tools hold the panel. See "Two writers, one panel".
            events.retain(|e| !matches!(e, AgentEvent::Plan { .. }));
        }

        let entries = self.entries();
        if entries != self.sent {
            self.sent = entries.clone();
            events.push(AgentEvent::Plan { entries });
        }
    }

    /// The list, as the panel's own shape.
    fn entries(&self) -> Vec<PlanEntry> {
        self.tasks
            .iter()
            .map(|t| PlanEntry {
                content: t.subject.clone(),
                priority: String::new(),
                status: t.status.clone(),
            })
            .collect()
    }

    /// Note the task calls an assistant message makes, to be applied when they
    /// come back.
    fn remember_calls(&mut self, frame: &serde_json::Value) {
        let Some(blocks) = frame["message"]["content"].as_array() else { return };
        for block in blocks {
            if block["type"].as_str() != Some("tool_use") {
                continue;
            }
            let name = block["name"].as_str().unwrap_or_default();
            // `TaskList` is deliberately absent: it takes no parameters and
            // changes nothing, so watching it would only be a way to get the
            // list wrong.
            if name != "TaskCreate" && name != "TaskUpdate" {
                continue;
            }
            let Some(id) = block["id"].as_str() else { continue };
            self.pending.insert(
                id.to_string(),
                PendingCall { name: name.to_string(), input: block["input"].clone() },
            );
        }
    }

    /// Apply the calls whose results have now landed.
    fn apply_results(&mut self, frame: &serde_json::Value) {
        let Some(blocks) = frame["message"]["content"].as_array() else { return };
        for block in blocks {
            if block["type"].as_str() != Some("tool_result") {
                continue;
            }
            let Some(tool_use_id) = block["tool_use_id"].as_str() else { continue };
            let Some(call) = self.pending.remove(tool_use_id) else { continue };
            // A failed or denied call did nothing, so the list did nothing
            // either. This is NOT the unparsed-result case below: an error
            // result is the CLI saying the task was not created, and drawing
            // one anyway would put a task in the panel that the agent does not
            // have and cannot be told about.
            if block["is_error"].as_bool().unwrap_or(false) {
                continue;
            }
            let result = result_text(&block["content"]).unwrap_or_default();
            match call.name.as_str() {
                "TaskCreate" => self.create(tool_use_id, &call.input, &result),
                _ => self.update(&call.input),
            }
        }
    }

    /// A confirmed `TaskCreate`, as a row.
    fn create(&mut self, tool_use_id: &str, input: &serde_json::Value, result: &str) {
        // The required field, falling back to the other required field. A row
        // with no words is a blank line in the panel, which is a worse way to
        // be wrong than a long one.
        let subject = ["subject", "description"]
            .iter()
            .find_map(|k| input[*k].as_str().filter(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_string();

        // An unreadable result costs this task its ID, never its ROW. A list
        // that quietly omits a task is worse than one that cannot later update
        // it: the first is a lie about what the agent is doing, the second is
        // one stale line.
        //
        // The stand-in cannot collide with a real id. A real one is whatever
        // `Task #<id> created successfully` names, and this is a `tool_use` id
        // under a prefix no such sentence has ever carried — so a `TaskUpdate`
        // can never accidentally address a task whose id was never learned.
        let key = created_task_id(result)
            .unwrap_or_else(|| format!("unidentified:{tool_use_id}"));

        match self.tasks.iter_mut().find(|t| t.key == key) {
            Some(existing) => existing.subject = subject,
            None => self.tasks.push(Task { key, subject, status: "pending".to_string() }),
        }
    }

    /// A confirmed `TaskUpdate`, applied in place so the order holds.
    fn update(&mut self, input: &serde_json::Value) {
        let Some(key) = input["taskId"].as_str().map(task_key) else { return };
        // Ignored, not invented — a subagent may own tasks this pane never
        // watched being created, and so may a session resumed from a transcript
        // that was truncated before them.
        let Some(at) = self.tasks.iter().position(|t| t.key == key) else { return };

        if input["status"].as_str() == Some("deleted") {
            self.tasks.remove(at);
            return;
        }
        if let Some(subject) = input["subject"].as_str().filter(|s| !s.is_empty()) {
            self.tasks[at].subject = subject.to_string();
        }
        if let Some(status) = input["status"].as_str().filter(|s| !s.is_empty()) {
            self.tasks[at].status = status.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame off the wire, into a conversation with no history behind it.
    ///
    /// A fresh `Live` per call, so nothing has streamed and nothing is
    /// suppressed — which is the right default for every test below that is
    /// not about streaming.
    fn once(frame: &serde_json::Value) -> Vec<AgentEvent> {
        Live::default().frame_to_events(frame)
    }

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
            once(&delta).as_slice(),
            [AgentEvent::Message { role: Role::Agent, text, .. }] if text == "h"
        ));

        let thought = serde_json::json!({
            "type": "stream_event", "parent_tool_use_id": null,
            "event": { "type": "content_block_delta", "index": 0,
                       "delta": { "type": "thinking_delta", "thinking": "hmm" } }
        });
        assert!(matches!(
            once(&thought).as_slice(),
            [AgentEvent::Message { role: Role::Thought, text, .. }] if text == "hmm"
        ));
    }

    /// The frames 2.1.226 sends for one streamed message, in order.
    ///
    /// `message_start` names the message; the deltas carry the words but NOT
    /// the id, which is why `Live` has to remember which message is in flight.
    fn streamed(id: &str, text: &str) -> Vec<serde_json::Value> {
        let mut frames = vec![serde_json::json!({
            "type": "stream_event", "parent_tool_use_id": null,
            "event": { "type": "message_start", "message": { "id": id, "content": [] } }
        })];
        frames.extend(text.chars().map(|c| {
            serde_json::json!({
                "type": "stream_event", "parent_tool_use_id": null,
                "event": { "type": "content_block_delta", "index": 0,
                           "delta": { "type": "text_delta", "text": c.to_string() } }
            })
        }));
        frames
    }

    /// An `assistant` frame naming the message it completes.
    fn finished(id: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": { "id": id, "content": [{ "type": "text", "text": text }] }
        })
    }

    #[test]
    fn a_streamed_answer_is_not_printed_again_when_the_block_lands() {
        // The doubling trap `--include-partial-messages` sets: 2.1.226 sends
        // BOTH the deltas and the finished `assistant` message, each carrying
        // the whole answer. Taking both renders every reply twice, once
        // streamed and once underneath.
        let mut live = Live::default();
        let spoken: String = streamed("msg_1", "hi")
            .iter()
            .chain(std::iter::once(&finished("msg_1", "hi")))
            .flat_map(|f| live.frame_to_events(f))
            .filter_map(|e| match e {
                AgentEvent::Message { role: Role::Agent, text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(spoken, "hi", "not hihi");
    }

    #[test]
    fn an_answer_with_no_deltas_behind_it_is_still_drawn() {
        // The failure the first version of this had, and the one that matters
        // most: suppression keyed on Origin::Live alone assumed the deltas
        // rather than observing them, so anything that cost them — the flag
        // regressing, a user `args` entry, a CLI path that finishes a message
        // without streaming it — deleted the agent's whole side of the
        // conversation with no Gap and no error. Silent total text loss is the
        // exact failure a derived transcript exists to refuse.
        let mut live = Live::default();
        assert!(
            matches!(
                live.frame_to_events(&finished("msg_1", "hi")).as_slice(),
                [AgentEvent::Message { role: Role::Agent, text, .. }] if text == "hi"
            ),
            "an unstreamed answer must render, not vanish"
        );
    }

    #[test]
    fn only_the_message_that_actually_streamed_is_suppressed() {
        // Evidence is per message, not per session: one answer streaming must
        // not silence the next one that does not.
        let mut live = Live::default();
        for frame in streamed("msg_1", "hi") {
            live.frame_to_events(&frame);
        }
        assert!(live.frame_to_events(&finished("msg_1", "hi")).is_empty());
        assert_eq!(live.frame_to_events(&finished("msg_2", "later")).len(), 1);
    }

    #[test]
    fn a_delta_that_draws_nothing_is_not_evidence_that_anything_was_drawn() {
        // `input_json_delta` streams a tool call's arguments and
        // `signature_delta` an encrypted thinking signature. Both are real
        // deltas that put no words on screen, so counting either as proof
        // would suppress a block nobody had seen. Both were captured off a
        // live dispatch.
        let mut live = Live::default();
        live.frame_to_events(&serde_json::json!({
            "type": "stream_event", "parent_tool_use_id": null,
            "event": { "type": "message_start", "message": { "id": "msg_1" } }
        }));
        for kind in ["input_json_delta", "signature_delta"] {
            live.frame_to_events(&serde_json::json!({
                "type": "stream_event", "parent_tool_use_id": null,
                "event": { "type": "content_block_delta", "index": 0,
                           "delta": { "type": kind, "partial_json": "{\"a\":1}" } }
            }));
        }
        assert_eq!(
            live.frame_to_events(&finished("msg_1", "hi")).len(),
            1,
            "no words were drawn, so the block is still the only source"
        );
    }

    #[test]
    fn several_assistant_frames_can_share_one_streamed_message() {
        // A real dispatch sent the thinking block and the tool_use block as two
        // `assistant` frames under ONE message id and one `message_start`. So
        // the evidence is a set, not something consumed on first use —
        // consuming it would un-suppress every frame after the first.
        let mut live = Live::default();
        for frame in streamed("msg_1", "hi") {
            live.frame_to_events(&frame);
        }
        assert!(live.frame_to_events(&finished("msg_1", "hi")).is_empty());
        assert!(
            live.frame_to_events(&finished("msg_1", "hi")).is_empty(),
            "still the same message"
        );
    }

    #[test]
    fn what_streamed_is_forgotten_when_the_turn_ends() {
        // Nothing streams after the result, so holding the ids past it only
        // grows the set for the life of the session.
        let mut live = Live::default();
        for frame in streamed("msg_1", "hi") {
            live.frame_to_events(&frame);
        }
        live.frame_to_events(&serde_json::json!({ "type": "result", "stop_reason": "end_turn" }));
        assert_eq!(live.frame_to_events(&finished("msg_1", "hi")).len(), 1);
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
    fn a_tool_call_survives_the_suppression_that_silences_text() {
        // A tool block DOES stream — `content_block_start` with
        // `{"type":"tool_use",…}` then `input_json_delta` chunks — but
        // `stream_to_events` ignores both, so the finished block is the only
        // place a tool row ever comes from. Suppressing it alongside the text
        // would lose the call itself.
        let frame = serde_json::json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": { "id": "msg_1",
                         "content": [{ "type": "text", "text": "running it" },
                                     { "type": "tool_use", "id": "t1", "name": "Bash",
                                       "input": { "command": "ls" } }] }
        });

        // Live, with the text already streamed: the row survives alone.
        let mut live = Live::default();
        for f in streamed("msg_1", "running it") {
            live.frame_to_events(&f);
        }
        assert!(
            matches!(
                live.frame_to_events(&frame).as_slice(),
                [AgentEvent::ToolCall { title, .. }] if title == "ls"
            ),
            "the streamed text goes, the tool row stays"
        );

        // And with nothing streamed, and on replay, both come through.
        assert_eq!(once(&frame).len(), 2);
        assert_eq!(frame_to_events_from(&frame, Origin::Replay).len(), 2);
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
        let events = once(&frame);
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
        assert!(once(&frame).is_empty());
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
            once(&frame).as_slice(),
            [AgentEvent::ToolCall { subagent: true, title, .. }] if title == "find the bug"
        ));
    }

    #[test]
    fn a_parented_record_is_read_as_the_dispatch_it_belongs_to() {
        // ACP has to smuggle this through `_meta.claudeCode`; native has a
        // field, and `parent_of` reads it wherever it appears.
        //
        // NOT a claim about what 2.1.226 sends. A real `Agent` dispatch was run
        // through this pin with these exact flags and produced NO parented
        // `assistant` frame and NO parented `stream_event` at all: the subagent
        // surfaced as `system: task_started`/`task_updated`/`task_notification`
        // (all silent), ONE parented `user` frame carrying the prompt it was
        // given, and the dispatch's own `tool_result`. Only the `user` case
        // below is a shape this pin was observed to send. The rest is
        // deliberate leniency — the field is documented, costs nothing to read,
        // and a CLI that starts sending it should not need a code change to be
        // understood.
        let dispatched = serde_json::json!({
            "type": "user", "parentToolUseID": "toolu_01C8VVm16DcpURTrdPqWNtAN",
            "message": { "content": [{ "type": "text", "text": "Reply with: banana" }] }
        });
        assert!(
            matches!(
                frame_to_events_from(&dispatched, Origin::Replay).as_slice(),
                [AgentEvent::Message { parent: Some(p), .. }]
                    if p == "toolu_01C8VVm16DcpURTrdPqWNtAN"
            ),
            "the one parented shape this pin actually sends"
        );

        // Unobserved on this pin, read anyway.
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
            once(&delta).as_slice(),
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
            once(&frame).as_slice(),
            [AgentEvent::ToolUpdate { status: ToolStatus::Failed, content: Some(c), .. }]
                if c == "boom"
        ));
    }

    #[test]
    fn housekeeping_frames_are_silent_rather_than_gaps() {
        for kind in ["system", "rate_limit_event", "control_response", "stream_event"] {
            assert!(
                once(&serde_json::json!({ "type": kind })).is_empty(),
                "{kind} should be silent"
            );
        }
    }

    #[test]
    fn an_unknown_record_carrying_a_message_is_a_visible_gap() {
        // The property the derived transcript rests on: a new kind of
        // conversation content is reported, never silently dropped.
        assert!(matches!(
            once(&serde_json::json!({
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
                once(&serde_json::json!({ "type": kind })).is_empty(),
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

    /// An assistant message making one task tool call.
    fn task_call(id: &str, name: &str, input: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "tool_use", "id": id, "name": name,
                                       "input": input }] }
        })
    }

    /// The `tool_result` that call comes back as.
    fn task_result(id: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "user", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "tool_result", "tool_use_id": id,
                                       "is_error": false, "content": text }] }
        })
    }

    /// Every frame in a create, folded, as the plan the panel ends up drawing.
    ///
    /// Returns the LAST plan rather than all of them, because `Plan` replaces
    /// the panel wholesale — the last one is what a person sees.
    fn plan_after(tasks: &mut Tasks, frames: &[serde_json::Value]) -> Option<Vec<PlanEntry>> {
        let mut last = None;
        for frame in frames {
            let mut events = frame_to_events_from(frame, Origin::Live);
            tasks.fold(frame, &mut events);
            for event in events {
                if let AgentEvent::Plan { entries } = event {
                    last = Some(entries);
                }
            }
        }
        last
    }

    /// A whole task, created and confirmed, as two frames.
    fn created(tool_use: &str, subject: &str, task_id: &str) -> Vec<serde_json::Value> {
        vec![
            task_call(tool_use, "TaskCreate", serde_json::json!({ "subject": subject,
                                                                  "description": "because" })),
            task_result(tool_use, &format!("Task #{task_id} created successfully: {subject}")),
        ]
    }

    #[test]
    fn a_task_row_is_named_for_the_work_and_not_for_the_tool() {
        // Five creates drew five rows reading literally "TaskCreate" —
        // indistinguishable from one another, which is worse than merely
        // uninformative.
        assert_eq!(
            tool_title("TaskCreate", &serde_json::json!({ "subject": "Wire the panel" })),
            "Wire the panel"
        );
        // An update that renames says the new name.
        assert_eq!(
            tool_title("TaskUpdate", &serde_json::json!({ "taskId": "2",
                                                          "subject": "Wire it properly" })),
            "Wire it properly"
        );
        // An update that only moves a status has nothing but the id, so it says
        // the id AND the state — three status changes in a row would otherwise
        // read identically, which is the original complaint again.
        assert_eq!(
            tool_title("TaskUpdate", &serde_json::json!({ "taskId": "2",
                                                          "status": "in_progress" })),
            "Task #2: in_progress"
        );
        assert_eq!(tool_title("TaskUpdate", &serde_json::json!({ "taskId": "#2" })), "Task #2");
        // `TaskList` takes no parameters, so its own name is the whole truth.
        assert_eq!(tool_title("TaskList", &serde_json::json!({})), "TaskList");
    }

    #[test]
    fn the_task_tools_build_the_same_plan_panel_the_todo_list_does() {
        // The report that brought this here: five TaskCreates, repeated
        // TaskUpdates, five opaque rows and no panel at all.
        let mut tasks = Tasks::default();
        let mut frames = created("t1", "Read the schemas", "1");
        frames.extend(created("t2", "Wire the panel", "2"));
        frames.push(task_call("t3", "TaskUpdate", serde_json::json!({ "taskId": "2",
                                                                      "status": "in_progress" })));
        frames.push(task_result("t3", "Task #2 updated"));

        let plan = plan_after(&mut tasks, &frames).expect("a plan");
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].content, "Read the schemas");
        assert_eq!(plan[0].status, "pending", "every task is created pending");
        assert_eq!(plan[1].content, "Wire the panel");
        assert_eq!(plan[1].status, "in_progress");
    }

    #[test]
    fn a_task_list_keeps_the_order_it_was_created_in() {
        // The panel draws them in list order, so an update must edit in place.
        // Moving the updated task to the end would shuffle the list under
        // someone reading it.
        let mut tasks = Tasks::default();
        let mut frames = created("t1", "first", "1");
        frames.extend(created("t2", "second", "2"));
        frames.extend(created("t3", "third", "3"));
        frames.push(task_call("t4", "TaskUpdate", serde_json::json!({ "taskId": "1",
                                                                      "status": "completed" })));
        frames.push(task_result("t4", "Task #1 updated"));

        let plan = plan_after(&mut tasks, &frames).expect("a plan");
        let order: Vec<_> = plan.iter().map(|e| e.content.as_str()).collect();
        assert_eq!(order, ["first", "second", "third"]);
    }

    #[test]
    fn a_task_whose_result_nobody_could_parse_is_still_in_the_list() {
        // The id lives ONLY in the result sentence, so a reworded CLI costs the
        // correlation. A list that quietly omits a task is a lie about what the
        // agent is doing; one that cannot later update a task is a stale line.
        let mut tasks = Tasks::default();
        let plan = plan_after(
            &mut tasks,
            &[
                task_call("t1", "TaskCreate", serde_json::json!({ "subject": "Ship it" })),
                task_result("t1", "Created."),
            ],
        )
        .expect("the task appears anyway");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].content, "Ship it");

        // And its key cannot be hit by accident — an update for a real id does
        // not land on it.
        let after = plan_after(
            &mut tasks,
            &[
                task_call("t2", "TaskUpdate", serde_json::json!({ "taskId": "t1",
                                                                  "status": "completed" })),
                task_result("t2", "Task #t1 updated"),
            ],
        );
        assert!(after.is_none(), "nothing changed, so nothing was announced: {after:?}");
    }

    #[test]
    fn an_update_naming_a_task_this_pane_never_saw_created_is_ignored() {
        // Ignored, not invented: a subagent owns tasks of its own, and a row
        // conjured out of a bare id would be a blank line in the panel.
        let mut tasks = Tasks::default();
        let plan = plan_after(
            &mut tasks,
            &[
                task_call("t1", "TaskUpdate", serde_json::json!({ "taskId": "9",
                                                                  "status": "completed" })),
                task_result("t1", "Task #9 updated"),
            ],
        );
        assert!(plan.is_none(), "{plan:?}");
    }

    #[test]
    fn a_deleted_task_leaves_the_list_rather_than_showing_a_state() {
        // `PlanStatus` in the Mac app knows pending, in_progress and completed.
        // `deleted` is the one word in the task tools' vocabulary that is not a
        // state a task can be drawn in.
        let mut tasks = Tasks::default();
        let mut frames = created("t1", "keep", "1");
        frames.extend(created("t2", "drop", "2"));
        frames.push(task_call("t3", "TaskUpdate", serde_json::json!({ "taskId": "2",
                                                                      "status": "deleted" })));
        frames.push(task_result("t3", "Task #2 deleted"));

        let plan = plan_after(&mut tasks, &frames).expect("a plan");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].content, "keep");
    }

    #[test]
    fn nothing_joins_the_list_until_its_result_confirms_it() {
        // The emit-timing decision, and the reason it is uniform: a TaskCreate
        // has no id before its result, so applying a TaskUpdate at call time
        // instead would reorder the two and drop updates that arrive in the
        // same assistant message as the create they follow.
        let mut tasks = Tasks::default();
        let call = task_call("t1", "TaskCreate", serde_json::json!({ "subject": "Ship it" }));
        assert!(plan_after(&mut tasks, &[call]).is_none(), "not yet — it has no id yet");

        let confirm = task_result("t1", "Task #1 created successfully: Ship it");
        assert_eq!(plan_after(&mut tasks, &[confirm]).expect("now").len(), 1);
    }

    #[test]
    fn a_task_call_that_failed_changes_nothing() {
        // A denied or errored call did nothing, so the list did nothing. This
        // is not the unparsed-result case: an error result is the CLI saying
        // the task was never created, and drawing it anyway would put a task in
        // the panel that the agent does not have.
        let mut tasks = Tasks::default();
        let plan = plan_after(
            &mut tasks,
            &[
                task_call("t1", "TaskCreate", serde_json::json!({ "subject": "Ship it" })),
                serde_json::json!({
                    "type": "user", "parent_tool_use_id": null,
                    "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1",
                                               "is_error": true,
                                               "content": "The user doesn't want to proceed" }] }
                }),
            ],
        );
        assert!(plan.is_none(), "{plan:?}");
    }

    #[test]
    fn a_task_list_that_did_not_move_is_not_announced_again() {
        // The same rule `commands_sent` follows: this rides the ring to every
        // subscriber, and a re-announced identical list is traffic for nothing.
        let mut tasks = Tasks::default();
        assert!(plan_after(&mut tasks, &created("t1", "one", "1")).is_some());

        // A TaskList changes nothing, and neither does an update that sets the
        // status a task already had.
        let quiet = plan_after(
            &mut tasks,
            &[
                task_call("t2", "TaskList", serde_json::json!({})),
                task_result("t2", "1 task: #1 one (pending)"),
                task_call("t3", "TaskUpdate", serde_json::json!({ "taskId": "1",
                                                                  "status": "pending" })),
                task_result("t3", "Task #1 updated"),
            ],
        );
        assert!(quiet.is_none(), "{quiet:?}");
    }

    #[test]
    fn a_todo_write_does_not_take_a_panel_a_task_list_is_holding() {
        // Two writers, one surface. The task tools win while they have a list,
        // because a TodoWrite always sends its whole list and can re-assert
        // itself next call, while an accumulated task list cannot be rebuilt
        // once something has replaced it.
        let mut tasks = Tasks::default();
        plan_after(&mut tasks, &created("t1", "the real work", "1")).expect("a task list");

        let todo = serde_json::json!({
            "type": "assistant", "parent_tool_use_id": null,
            "message": { "content": [{ "type": "tool_use", "id": "t2", "name": "TodoWrite",
                "input": { "todos": [{ "content": "something else", "status": "pending" }] } }] }
        });
        let mut events = frame_to_events_from(&todo, Origin::Live);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Plan { .. })), "the todo made one");
        tasks.fold(&todo, &mut events);
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::Plan { .. })),
            "and the task list keeps the panel: {events:?}"
        );
        // The row itself survives — only the plan is dropped.
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolCall { .. })));

        // Once the last task is deleted the panel is free again, rather than
        // dead: this rule is not sticky.
        plan_after(
            &mut tasks,
            &[
                task_call("t3", "TaskUpdate", serde_json::json!({ "taskId": "1",
                                                                  "status": "deleted" })),
                task_result("t3", "Task #1 deleted"),
            ],
        )
        .expect("an emptied list is itself a change");
        let mut events = frame_to_events_from(&todo, Origin::Live);
        tasks.fold(&todo, &mut events);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Plan { .. })), "{events:?}");
    }

    #[test]
    fn a_restored_transcript_rebuilds_the_task_list_it_ended_with() {
        // Replay reads the same records through the same normalizer, and the
        // on-disk transcript carries the same tool_use and tool_result shapes
        // the wire does — so a resumed pane gets its panel back rather than an
        // empty one under a conversation full of task rows.
        let mut frames = created("t1", "first", "1");
        frames.extend(created("t2", "second", "2"));
        frames.push(task_call("t3", "TaskUpdate", serde_json::json!({ "taskId": "1",
                                                                      "status": "completed" })));
        frames.push(task_result("t3", "Task #1 updated"));
        let transcript: String = frames
            .iter()
            .map(|f| serde_json::to_string(f).expect("json") + "\n")
            .collect();

        // Through a tracker the caller keeps, which is what lets an update
        // arriving AFTER the resume still find the task it names.
        let mut tasks = Tasks::default();
        let events = history_to_events_with(&transcript, &mut tasks);
        let plan = events
            .iter()
            .rev()
            .find_map(|e| match e {
                AgentEvent::Plan { entries } => Some(entries.clone()),
                _ => None,
            })
            .expect("the restored plan");
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].status, "completed");

        let live = plan_after(
            &mut tasks,
            &[
                task_call("t4", "TaskUpdate", serde_json::json!({ "taskId": "2",
                                                                  "status": "in_progress" })),
                task_result("t4", "Task #2 updated"),
            ],
        )
        .expect("a task created before the resume is still updatable");
        assert_eq!(live[1].status, "in_progress");
    }

    #[test]
    fn a_created_task_id_is_read_only_out_of_the_sentence_that_announces_one() {
        // String parsing of a human-readable result, because the id is nowhere
        // else — TaskCreate's input has none. Anchored on both halves of the
        // sentence, because "created successfully" alone is not distinctive: a
        // `Write` on this same pin answers "File created successfully at: …".
        assert_eq!(
            created_task_id("Task #2 created successfully: Wire the panel").as_deref(),
            Some("2")
        );
        assert_eq!(
            created_task_id("File created successfully at: /a/b.rs (file state is current)"),
            None
        );
        assert_eq!(created_task_id("Task #2 was made"), None);
        assert_eq!(created_task_id(""), None);
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
