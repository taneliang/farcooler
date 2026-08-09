//! Codex app-server frames, as `AgentEvent`.
//!
//! Deliberately lenient, for the reason `acp/wire.rs` gives: a strict decoder
//! would turn every codex release into an outage, and the honest response to a
//! frame we do not understand is a visible `Gap` — not a dropped connection,
//! and never a silently shorter transcript.
//!
//! Every shape here was read off a real turn against codex-cli 0.146.0, saved
//! as `tests/fixtures/turn_basic.jsonl` by `tests/fixtures/capture_turn.py`.
//! The notification names are not guessable and the item shapes are not
//! documented in prose; both were observed.

use farcooler_agent_core::event::{
    AgentEvent, AgentGapReason, EndReason, PermissionOption, PlanEntry, Role, ToolStatus,
};

/// Where a frame came from, which decides whether the user's own words are
/// part of it.
///
/// Codex echoes a `userMessage` item on every turn, including the live one you
/// just typed. The client has already put that on screen itself — see
/// `Transcript.appendLocalUserMessage` — so taking the echo as well renders
/// every prompt twice. ACP never had this problem because its adapters send
/// `user_message_chunk` only while replaying `session/load`.
///
/// Replay is the case that genuinely needs it: nothing else will produce the
/// prompts in a resumed conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A turn happening now. The client already showed what was typed.
    Live,
    /// History being restored, where the prompts exist nowhere else.
    Replay,
}

/// What one frame means, if anything.
///
/// Most frames mean nothing to a transcript — `mcpServer/startupStatus/updated`,
/// `account/rateLimits/updated`, `hook/started`. Those return an empty vector
/// rather than a `Gap`: nothing was lost, and drawing a "history missing" break
/// for a hook firing would make every turn look broken.
pub fn frame_to_events(
    method: &str,
    params: &serde_json::Value,
    origin: Origin,
) -> Vec<AgentEvent> {
    match method {
        "item/started" | "item/completed" => item_to_events(method, &params["item"], origin),
        "item/agentMessage/delta" => {
            let text = params["delta"].as_str().unwrap_or_default();
            if text.is_empty() {
                return Vec::new();
            }
            vec![AgentEvent::Message { role: Role::Agent, text: text.to_string(), parent: None }]
        }
        "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => {
            let text = params["delta"].as_str().unwrap_or_default();
            if text.is_empty() {
                return Vec::new();
            }
            vec![AgentEvent::Message { role: Role::Thought, text: text.to_string(), parent: None }]
        }
        "turn/completed" => {
            // `status` is the turn's own word for how it ended. Anything
            // unrecognized is still an ended turn — refusing to admit that
            // leaves the pane on Working forever, which is the failure
            // `end_reason` already reasons about on the ACP side.
            let status = params["turn"]["status"].as_str().unwrap_or_default();
            vec![AgentEvent::TurnEnded { reason: end_reason(status) }]
        }
        "thread/tokenUsage/updated" => {
            let used = params["usage"]["totalTokens"].as_u64();
            let size = params["usage"]["contextWindow"].as_u64();
            match (used, size) {
                (Some(used), Some(size)) if size > 0 => vec![AgentEvent::Usage { used, size }],
                // Reported without a window to measure against is not a gap —
                // there is simply nothing to draw.
                _ => Vec::new(),
            }
        }
        "turn/plan/updated" | "item/plan/delta" => {
            let entries = params["plan"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|e| PlanEntry {
                            content: e["text"]
                                .as_str()
                                .or_else(|| e["content"].as_str())
                                .unwrap_or_default()
                                .to_string(),
                            priority: e["priority"].as_str().unwrap_or_default().to_string(),
                            status: e["status"].as_str().unwrap_or_default().to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            vec![AgentEvent::Plan { entries }]
        }
        // Known, and deliberately silent. Each of these is real and none of
        // them belongs in a transcript, so an empty vector is the correct
        // answer rather than a Gap.
        "turn/started"
        | "thread/started"
        | "thread/status/changed"
        | "mcpServer/startupStatus/updated"
        | "account/rateLimits/updated"
        | "account/updated"
        | "remoteControl/status/changed"
        | "hook/started"
        | "hook/completed"
        | "warning"
        | "thread/name/updated"
        | "item/commandExecution/outputDelta"
        | "serverRequest/resolved"
        // Session bookkeeping a resume emits. `thread/goal/cleared` in
        // particular was drawing "Something happened here that this version
        // cannot show" over an otherwise empty restored conversation — a
        // scissors icon reporting loss where nothing had been lost.
        | "thread/goal/cleared"
        | "thread/goal/updated"
        | "thread/settings/updated"
        | "thread/environment/connected"
        | "thread/environment/disconnected"
        | "thread/archived"
        | "thread/unarchived"
        | "thread/closed"
        | "skills/changed"
        | "model/rerouted"
        | "deprecationNotice"
        | "configWarning"
        | "guardianWarning" => Vec::new(),
        _ => vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }],
    }
}

/// One `ThreadItem`, as rows.
fn item_to_events(method: &str, item: &serde_json::Value, origin: Origin) -> Vec<AgentEvent> {
    let started = method == "item/started";
    let id = item["id"].as_str().unwrap_or_default().to_string();
    match item["type"].as_str().unwrap_or_default() {
        // The echo of what the user sent. Only worth taking when restoring
        // history — during a live turn the client already put it on screen, and
        // taking it again renders every prompt twice. See `Origin`.
        "userMessage" => {
            if started || origin == Origin::Live {
                return Vec::new();
            }
            let text = content_text(&item["content"]);
            if text.is_empty() {
                return Vec::new();
            }
            vec![AgentEvent::Message { role: Role::User, text, parent: None }]
        }
        // Live, the deltas already carried this text and taking it again would
        // print the whole answer a second time underneath the streamed one.
        // In replay there ARE no deltas — history is items only — so this is
        // the only place the agent's words exist.
        "agentMessage" => {
            if origin == Origin::Live {
                return Vec::new();
            }
            let text = item["text"].as_str().unwrap_or_default().to_string();
            if text.is_empty() {
                return Vec::new();
            }
            vec![AgentEvent::Message { role: Role::Agent, text, parent: None }]
        }
        "reasoning" => {
            if origin == Origin::Live {
                return Vec::new();
            }
            // `summary` when there is one, `content` otherwise: a restored
            // transcript should show what the agent showed, not its full
            // internal trace.
            let text = content_text(&item["summary"]);
            let text = if text.is_empty() { content_text(&item["content"]) } else { text };
            if text.is_empty() {
                return Vec::new();
            }
            vec![AgentEvent::Message { role: Role::Thought, text, parent: None }]
        }
        "commandExecution" => {
            let command = item["command"].as_str().unwrap_or_default().to_string();
            if started {
                return vec![AgentEvent::ToolCall {
                    id,
                    title: command,
                    kind: "execute".into(),
                    status: ToolStatus::InProgress,
                    locations: Vec::new(),
                    parent: None,
                    subagent: false,
                }];
            }
            let failed = item["exitCode"].as_i64().is_some_and(|c| c != 0);
            vec![AgentEvent::ToolUpdate {
                id,
                status: if failed { ToolStatus::Failed } else { ToolStatus::Completed },
                title: Some(command),
                content: item["aggregatedOutput"].as_str().map(str::to_string),
                diff: None,
                locations: Vec::new(),
                parent: None,
                subagent: None,
            }]
        }
        "fileChange" => {
            let paths: Vec<String> = item["changes"]
                .as_array()
                .map(|c| {
                    c.iter()
                        .filter_map(|change| change["path"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let title = match paths.len() {
                0 => "Edit".to_string(),
                1 => paths[0].clone(),
                n => format!("{n} files"),
            };
            if started {
                return vec![AgentEvent::ToolCall {
                    id,
                    title,
                    kind: "edit".into(),
                    status: ToolStatus::InProgress,
                    locations: paths,
                    parent: None,
                    subagent: false,
                }];
            }
            vec![AgentEvent::ToolUpdate {
                id,
                status: ToolStatus::Completed,
                title: Some(title),
                content: None,
                // No before-and-after on the wire, and reconstructing one from
                // a patch would be a guess. The client answers no fs requests
                // here — codex writes its own files — so unlike ACP there is
                // nothing to build a Diff from honestly.
                diff: None,
                locations: paths,
                parent: None,
                subagent: None,
            }]
        }
        // Everything else that IS a tool of some kind: web search, MCP calls,
        // image generation, sub-agent activity. Rendered generically rather
        // than dropped, because a row saying something happened beats silence.
        other if !other.is_empty() => {
            let title = item["title"]
                .as_str()
                .or_else(|| item["query"].as_str())
                .or_else(|| item["text"].as_str())
                .unwrap_or(other)
                .to_string();
            if started {
                return vec![AgentEvent::ToolCall {
                    id,
                    title,
                    kind: other.to_string(),
                    status: ToolStatus::InProgress,
                    locations: Vec::new(),
                    parent: None,
                    subagent: false,
                }];
            }
            vec![AgentEvent::ToolUpdate {
                id,
                status: ToolStatus::Completed,
                title: Some(title),
                content: None,
                diff: None,
                locations: Vec::new(),
                parent: None,
                subagent: None,
            }]
        }
        _ => vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }],
    }
}

/// A restored conversation, from a `thread/read` result.
///
/// History does NOT arrive as notifications. `thread/resume` attaches to the
/// thread and streams only bookkeeping; the conversation has to be asked for
/// with `thread/read { includeTurns: true }`, which answers with
/// `thread.turns[].items[]`. Reading it as `Replay` is what makes the prompts
/// and the agent's finished answers appear at all — in history there are no
/// deltas to carry them.
pub fn history_to_events(result: &serde_json::Value) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    let Some(turns) = result["thread"]["turns"].as_array() else { return events };
    for turn in turns {
        let Some(items) = turn["items"].as_array() else { continue };
        for item in items {
            // `item/completed` because a restored item is finished by
            // definition: nothing in history is still in flight.
            events.extend(item_to_events("item/completed", item, Origin::Replay));
        }
    }
    events
}

/// The text of a `content` array, joined.
fn content_text(content: &serde_json::Value) -> String {
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// A turn's `status`, as the reason it ended.
pub fn end_reason(status: &str) -> EndReason {
    match status {
        "cancelled" | "interrupted" => EndReason::Cancelled,
        "refused" | "refusal" => EndReason::Refusal,
        "maxTokens" | "max_tokens" => EndReason::MaxTokens,
        _ => EndReason::EndTurn,
    }
}

/// A server-to-client approval request, as the event that blocks a fleet row.
///
/// Codex asks three different ways — a command, a patch, a permission profile —
/// and they carry different context but need the same answer, so they become
/// one event. `id` is the JSON-RPC request id as text, because that is what has
/// to be handed back for the answer to be routable.
pub fn approval_event(request_id: &str, method: &str, params: &serde_json::Value) -> AgentEvent {
    let what = match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => params["command"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| "Run a command".into()),
        "item/fileChange/requestApproval" | "applyPatchApproval" => "Apply a patch".into(),
        _ => "Continue".into(),
    };
    AgentEvent::Permission {
        id: request_id.to_string(),
        tool_call: params["itemId"].as_str().unwrap_or_default().to_string(),
        // The three decisions the wire accepts. `accept` and `decline` are what
        // a person means; `acceptForSession` is codex's own "don't ask again"
        // and is offered rather than hidden, because the alternative is being
        // asked once per command for the rest of the turn.
        options: vec![
            PermissionOption {
                id: "accept".into(),
                name: format!("Allow: {what}"),
                kind: "allow_once".into(),
            },
            PermissionOption {
                id: "acceptForSession".into(),
                name: "Allow for this session".into(),
                kind: "allow_always".into(),
            },
            PermissionOption {
                id: "decline".into(),
                name: "Decline".into(),
                kind: "reject_once".into(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_message_arrives_as_deltas_and_is_not_repeated_on_completion() {
        // Both would render the whole answer twice: once streamed, once again
        // underneath it.
        let delta = frame_to_events(
            "item/agentMessage/delta",
            &serde_json::json!({ "delta": "hi" }),
            Origin::Live,
        );
        assert!(matches!(
            delta.as_slice(),
            [AgentEvent::Message { role: Role::Agent, text, .. }] if text == "hi"
        ));

        let completed = frame_to_events(
            "item/completed",
            &serde_json::json!({ "item": { "type": "agentMessage", "id": "m", "text": "hi" } }), Origin::Live);
        assert!(completed.is_empty(), "the deltas already carried it: {completed:?}");
    }

    #[test]
    fn a_live_prompt_is_not_echoed_because_the_client_already_showed_it() {
        // Observed on screen: every message appeared twice. The client appends
        // what you typed the moment you send it, and codex echoes the same text
        // back as a userMessage item — so taking the echo renders it again.
        let item = serde_json::json!({
            "item": { "type": "userMessage", "id": "u",
                      "content": [{ "type": "text", "text": "hello" }] }
        });
        assert!(frame_to_events("item/started", &item, Origin::Live).is_empty());
        assert!(
            frame_to_events("item/completed", &item, Origin::Live).is_empty(),
            "the client put this on screen itself"
        );
    }

    #[test]
    fn a_replayed_prompt_is_taken_because_nothing_else_will_produce_it() {
        // The other half: a resumed conversation has no local echo behind it,
        // so dropping these would restore an agent talking to itself.
        let item = serde_json::json!({
            "item": { "type": "userMessage", "id": "u",
                      "content": [{ "type": "text", "text": "hello" }] }
        });
        assert!(matches!(
            frame_to_events("item/completed", &item, Origin::Replay).as_slice(),
            [AgentEvent::Message { role: Role::User, text, .. }] if text == "hello"
        ));
        assert!(
            frame_to_events("item/started", &item, Origin::Replay).is_empty(),
            "once, on completion, not twice"
        );
    }

    #[test]
    fn a_command_becomes_a_tool_row_that_fails_when_it_exits_nonzero() {
        let started = frame_to_events(
            "item/started",
            &serde_json::json!({ "item": { "type": "commandExecution", "id": "c",
                                           "command": "ls -la" } }), Origin::Live);
        assert!(matches!(
            started.as_slice(),
            [AgentEvent::ToolCall { title, status: ToolStatus::InProgress, .. }] if title == "ls -la"
        ));

        let failed = frame_to_events(
            "item/completed",
            &serde_json::json!({ "item": { "type": "commandExecution", "id": "c",
                                           "command": "false", "exitCode": 1 } }), Origin::Live);
        assert!(matches!(
            failed.as_slice(),
            [AgentEvent::ToolUpdate { status: ToolStatus::Failed, .. }]
        ));
    }

    #[test]
    fn housekeeping_frames_are_silent_rather_than_gaps() {
        // A Gap draws a "history missing" break. Drawing one because a hook
        // fired would make every turn look broken.
        for method in [
            "hook/started",
            "mcpServer/startupStatus/updated",
            "account/rateLimits/updated",
            "thread/status/changed",
            "warning",
        ] {
            assert!(
                frame_to_events(method, &serde_json::json!({}), Origin::Live).is_empty(),
                "{method} should be silent"
            );
        }
    }

    #[test]
    fn an_unknown_frame_is_a_visible_gap_rather_than_a_silent_drop() {
        // The contract the whole derived transcript rests on: it can say where
        // it is incomplete.
        assert!(matches!(
            frame_to_events("turn/somethingNew", &serde_json::json!({}), Origin::Live).as_slice(),
            [AgentEvent::Gap { reason: AgentGapReason::Unparsed }]
        ));
    }

    #[test]
    fn an_unfamiliar_turn_status_still_ends_the_turn() {
        // Refusing to admit a turn ended leaves the pane on Working forever.
        assert!(matches!(
            frame_to_events(
                "turn/completed",
                &serde_json::json!({ "turn": { "status": "somethingElse" } }), Origin::Live)
            .as_slice(),
            [AgentEvent::TurnEnded { reason: EndReason::EndTurn }]
        ));
        assert!(matches!(
            frame_to_events(
                "turn/completed",
                &serde_json::json!({ "turn": { "status": "interrupted" } }), Origin::Live)
            .as_slice(),
            [AgentEvent::TurnEnded { reason: EndReason::Cancelled }]
        ));
    }

    #[test]
    fn an_approval_offers_the_three_decisions_the_wire_accepts() {
        let event = approval_event(
            "7",
            "item/commandExecution/requestApproval",
            &serde_json::json!({ "command": "rm -rf build", "itemId": "c1" }),
        );
        let AgentEvent::Permission { id, tool_call, options } = event else {
            panic!("expected a Permission");
        };
        assert_eq!(id, "7", "the request id has to survive or the answer is unroutable");
        assert_eq!(tool_call, "c1");
        let ids: Vec<_> = options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["accept", "acceptForSession", "decline"]);
        assert!(options[0].name.contains("rm -rf build"), "names what it would run");
    }
}
