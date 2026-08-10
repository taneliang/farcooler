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
    AgentEvent, AgentGapReason, Diff, EndReason, PermissionOption, PlanEntry, Role, ToolStatus,
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
        // The conversation's own title, which every GUI renders — the Mac app
        // puts it in the pane header. ACP feeds `SessionInfo` and codex did
        // not, so a codex pane's header stayed blank for the whole session
        // while the server was sending the name the whole time.
        //
        // `threadName` is nullable: that is codex CLEARING the name, not
        // naming it, and a header reading "null" would be worse than a blank
        // one.
        "thread/name/updated" => {
            let title = params["threadName"].as_str().unwrap_or_default();
            if title.is_empty() {
                return Vec::new();
            }
            vec![AgentEvent::SessionInfo { title: title.to_string() }]
        }
        // A patch as it is being applied, carrying the same `FileUpdateChange`
        // array the item does.
        //
        // Read for completeness rather than because it was observed: over
        // several real edits against 0.147.0 this never fired — the `fileChange`
        // item's own `item/started` already carried the changes, and the
        // streaming came as `turn/diff/updated`. It is in the union, it costs
        // nothing to read, and a version that does send it would otherwise have
        // drawn a break.
        "item/fileChange/patchUpdated" => {
            let id = params["itemId"].as_str().unwrap_or_default().to_string();
            let (diff, content) = change_view(&params["changes"]);
            if diff.is_none() && content.is_none() {
                return Vec::new();
            }
            vec![AgentEvent::ToolUpdate {
                id,
                status: ToolStatus::InProgress,
                title: None,
                content,
                diff,
                locations: change_paths(&params["changes"]),
                parent: None,
                subagent: None,
            }]
        }
        // A failure, which is the OPPOSITE of a gap. Left unhandled this fell
        // to the `_` arm and drew "Something happened here that this version
        // cannot show" — telling the user history was missing when in fact
        // nothing was missing and something had gone wrong.
        //
        // A `Message` rather than a `TurnEnded`, for two reasons. `TurnStatus`
        // has its own `failed` and `turn/completed` still arrives to carry it,
        // so ending the turn here would end it twice. And `willRetry` is
        // exactly the case where the turn is NOT over — stopping the pane on a
        // retryable stream disconnect would abandon a turn codex is still
        // working on. The neutral vocabulary has no error event, so the
        // server's own words in the transcript is the honest surface.
        "error" => {
            let error = &params["error"];
            let mut text = format!(
                "Error: {}",
                error["message"]
                    .as_str()
                    .filter(|m| !m.is_empty())
                    .unwrap_or("codex reported an error without describing it")
            );
            if let Some(details) = error["additionalDetails"].as_str().filter(|d| !d.is_empty()) {
                text.push_str("\n\n");
                text.push_str(details);
            }
            if params["willRetry"].as_bool().unwrap_or(false) {
                text.push_str("\n\nRetrying.");
            }
            vec![AgentEvent::Message { role: Role::Agent, text, parent: None }]
        }
        // Known, and deliberately silent. Each of these is real and none of
        // them belongs in a transcript, so an empty vector is the correct
        // answer rather than a Gap.
        //
        // Still an enumerated list, where the sibling Claude crate deliberately
        // gave one up. The two vocabularies are not alike: Claude Code's record
        // types are open and mostly metadata, so any name unseen became a false
        // Gap and a "does this carry a message" rule was the only stable
        // answer. `ServerNotification` here is a CLOSED union in a schema this
        // repo vendors, and `check_version` refuses a codex that does not match
        // `vendor/PINNED` — so the set is knowable rather than guessable. Nor
        // is there a shape to fall back on: these payloads share no content
        // field to test, only `threadId`, so a shape rule would have to guess
        // where a list can simply be checked against the schema. That checking
        // is `every_notification_the_pinned_schema_declares_is_read_or_silent`,
        // which fails the moment a codex bump adds a name — at test time,
        // rather than as scissors in someone's transcript.
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
        | "guardianWarning"
        | "thread/deleted"
        // The turn's AGGREGATED diff, across every file it touched, resent
        // whole on every change — three times for one two-file edit, observed.
        // Silent because it carries no `itemId`: there is no tool row to hang
        // it on, and inventing one would put a growing second copy of every
        // edit at the bottom of the transcript. The `fileChange` item carries
        // the same bytes per file against a row that exists, which is where the
        // diffs now come from. The worktree-wide view is the review pane's job,
        // not the transcript's.
        | "turn/diff/updated"
        // The guardian deciding whether to answer an approval on the user's
        // behalf. The approval itself is already a `Permission` event, and a
        // row narrating the deliberation over it would say nothing the answer
        // does not.
        | "item/autoApprovalReview/started"
        | "item/autoApprovalReview/completed"
        // The connection-scoped process channel, from `command/exec` and
        // `process/spawn`. Far Cooler calls neither — codex's own commands
        // arrive as `commandExecution` items — so these can only be another
        // client's output on a shared connection, and are certainly not this
        // transcript's missing history.
        | "command/exec/outputDelta"
        | "process/outputDelta"
        | "process/exited"
        // A command asking for the terminal (a pager, a prompt). Far Cooler
        // answers none of it; the output still arrives on the item.
        | "item/commandExecution/terminalInteraction"
        // The schema marks this deprecated and says the server no longer emits
        // it at all. Listed so that a codex old enough to still send it does
        // not draw a break for a notification its successor replaced.
        | "item/fileChange/outputDelta"
        // Progress counters on an MCP call. The call's own item carries the
        // result, which is the part worth showing.
        | "item/mcpToolCall/progress"
        // Only `itemId` and `summaryIndex` — a marker that the next summary
        // begins, with no text on it. The words arrive as
        // `item/reasoning/summaryTextDelta`, which is already read above.
        | "item/reasoning/summaryPartAdded"
        // Deprecated in favor of a `ContextCompaction` item, which the generic
        // tool arm below renders. Nothing is lost by ignoring the announcement.
        | "thread/compacted"
        | "model/verification"
        | "turn/moderationMetadata"
        | "model/safetyBuffering/updated"
        // Answers to a `fuzzyFileSearch/start` this client never makes.
        | "fuzzyFileSearch/sessionUpdated"
        | "fuzzyFileSearch/sessionCompleted"
        // The realtime voice channel, which Far Cooler never opens. Eight
        // methods that cannot arrive here — listed rather than left to the `_`
        // arm because "cannot happen" is a bad reason to draw eight scissors
        // breaks if it does.
        | "thread/realtime/started"
        | "thread/realtime/itemAdded"
        | "thread/realtime/transcript/delta"
        | "thread/realtime/transcript/done"
        | "thread/realtime/outputAudio/delta"
        | "thread/realtime/sdp"
        | "thread/realtime/error"
        | "thread/realtime/closed"
        // Account, app, and machine bookkeeping. Real, and none of it is
        // conversation: a login finishing or an MCP server's OAuth completing
        // says nothing about what the agent did.
        | "mcpServer/oauthLogin/completed"
        | "account/login/completed"
        | "app/list/updated"
        | "externalAgentConfig/import/progress"
        | "externalAgentConfig/import/completed"
        | "fs/changed"
        | "windows/worldWritableWarning"
        | "windowsSandbox/setupCompleted" => Vec::new(),
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
            let paths = change_paths(&item["changes"]);
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
            let (diff, content) = change_view(&item["changes"]);
            vec![AgentEvent::ToolUpdate {
                id,
                // `PatchApplyStatus`, not an assumption. A patch can be
                // `declined` or `failed`, and reporting either as Completed
                // drew a finished green row for an edit that never landed.
                status: match item["status"].as_str().unwrap_or_default() {
                    "failed" | "declined" => ToolStatus::Failed,
                    "inProgress" => ToolStatus::InProgress,
                    _ => ToolStatus::Completed,
                },
                title: Some(title),
                content,
                diff,
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

/// The files a `changes` array touches, in the order it lists them.
fn change_paths(changes: &serde_json::Value) -> Vec<String> {
    changes
        .as_array()
        .map(|c| {
            c.iter().filter_map(|change| change["path"].as_str().map(str::to_string)).collect()
        })
        .unwrap_or_default()
}

/// A `changes` array, as the one diff and the one body a tool row can hold.
///
/// A row holds exactly ONE diff — every client applies an update with
/// `if let diff { row.diff = diff }`, last write wins — so a two-file patch
/// sent as two updates would render the second file and quietly hide the
/// first. One change therefore becomes a `Diff`; several become the patch text
/// in `content`, which is not rendered side by side but does not drop a file
/// on the floor either.
fn change_view(changes: &serde_json::Value) -> (Option<Diff>, Option<String>) {
    let Some(changes) = changes.as_array().filter(|c| !c.is_empty()) else {
        return (None, None);
    };
    if let [only] = changes.as_slice() {
        return (diff_of(only), None);
    }
    let body: String = changes
        .iter()
        .map(|change| {
            format!(
                "{}\n{}\n",
                change["path"].as_str().unwrap_or("(unnamed file)"),
                change["diff"].as_str().unwrap_or_default()
            )
        })
        .collect();
    (None, Some(body))
}

/// One `FileUpdateChange`, as both sides of the edit.
///
/// The comment this replaces said there was no before-and-after on the wire and
/// that reconstructing one would be a guess. Half of that is still true: the
/// client cannot read the file to recover what is missing, because by the time
/// this arrives codex has already written it — the disk holds the AFTER image,
/// and diffing against that would report every edit as a no-op.
///
/// The other half was simply wrong, and how wrong depends on `kind`. Observed
/// against codex-cli 0.147.0, where `diff` is three different things:
///
/// - `add`: NOT a patch at all. The new file's entire content, verbatim, with
///   no `@@` and no `+` prefixes — `"one\ntwo\n"` for a two-line file.
/// - `delete`: the same, for the content the file HAD. Both sides are exact.
/// - `update`: a unified diff, hunks only, with no `---`/`+++` header on it.
///
/// Reading all three as patches was the first version of this and it silently
/// produced nothing for every new file — the `@@` gate never opened, so a
/// created file showed no diff at all. Each kind is now read as what it is.
///
/// For `update` both sides come out of the hunks: old is the context and `-`
/// lines, new is the context and `+` lines. That is FRAGMENTS of the file
/// rather than the file — the unchanged spans between hunks are not on the wire
/// and are not invented here — which is what a diff view wants anyway, since it
/// computes its own alignment from the two strings it is handed.
fn diff_of(change: &serde_json::Value) -> Option<Diff> {
    let path = change["path"].as_str()?.to_string();
    let body = change["diff"].as_str().unwrap_or_default().to_string();

    match change["kind"]["type"].as_str().unwrap_or_default() {
        // `old_text: None` is the vocabulary's word for "this file did not
        // exist", which is exactly what `add` means.
        "add" => Some(Diff { path, old_text: None, new_text: body }),
        "delete" => Some(Diff { path, old_text: Some(body), new_text: String::new() }),
        _ => diff_from_patch(path, &body),
    }
}

/// The two sides of an `update`, out of its hunks.
///
/// `None` when neither side has a line in it, so a patch shaped in some way
/// this does not read renders as no diff rather than as an empty one: an empty
/// diff view is a claim that nothing changed, which is a stronger and more
/// wrong statement than showing nothing.
fn diff_from_patch(path: String, patch: &str) -> Option<Diff> {
    let mut old = String::new();
    let mut new = String::new();
    let mut in_hunk = false;
    for line in patch.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            continue;
        }
        // Nothing before the first `@@` is content. 0.147.0 sends no header
        // here, but a `--- a/x` and `+++ b/x` pair from some other version
        // would otherwise read as a removed line and an added one, putting the
        // file's own name inside its diff.
        if !in_hunk {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                new.push_str(&line[1..]);
                new.push('\n');
            }
            Some(b'-') => {
                old.push_str(&line[1..]);
                old.push('\n');
            }
            // `\ No newline at end of file` is a note about the line above it,
            // not a line of either side.
            Some(b'\\') => {}
            // Context, on both sides. A bare empty line is context too — some
            // patch writers strip the trailing space rather than emit " ".
            _ => {
                let text = line.strip_prefix(' ').unwrap_or(line);
                old.push_str(text);
                old.push('\n');
                new.push_str(text);
                new.push('\n');
            }
        }
    }
    if old.is_empty() && new.is_empty() {
        return None;
    }
    // `Some`, even when empty: a pure insertion has no old side either, and
    // calling that a new file would claim the rest of the file never existed.
    Some(Diff { path, old_text: Some(old), new_text: new })
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
    fn the_notifications_that_used_to_litter_a_session_with_scissors_are_silent() {
        // Thirty-five of the schema's seventy methods reached the `_` arm, and
        // several fire in every ordinary session — so a normal conversation
        // came out dotted with "something happened here that this version
        // cannot show" over places where nothing at all had been lost.
        for method in [
            "turn/diff/updated",
            "item/autoApprovalReview/started",
            "item/mcpToolCall/progress",
            "item/reasoning/summaryPartAdded",
            "process/exited",
            "fs/changed",
            "thread/realtime/transcript/delta",
            "account/login/completed",
            "model/safetyBuffering/updated",
            "windowsSandbox/setupCompleted",
        ] {
            assert!(
                frame_to_events(method, &serde_json::json!({}), Origin::Live).is_empty(),
                "{method} should be silent"
            );
        }
    }

    #[test]
    fn every_notification_the_pinned_schema_declares_is_read_or_silent() {
        // The list stays a list only because it can be checked against the
        // schema instead of against my memory — the same argument
        // `every_permission_mode_the_cli_accepts_is_offered` makes on the
        // Claude side. When a codex bump adds a method, this fails here rather
        // than drawing a break in someone's transcript.
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../vendor/codex-app-server.schema.json"))
                .expect("the vendored schema is JSON");
        let variants = schema["definitions"]["ServerNotification"]["oneOf"]
            .as_array()
            .expect("ServerNotification is a union");
        assert!(variants.len() >= 70, "only {} notifications — wrong node?", variants.len());

        let unread: Vec<&str> = variants
            .iter()
            .filter_map(|v| v["properties"]["method"]["enum"][0].as_str())
            .filter(|method| {
                // `item/*` frames are routed on the item they carry, so those
                // two need one; every other method decides on its name alone.
                let params = serde_json::json!({
                    "item": { "type": "agentMessage", "id": "m", "text": "t" }
                });
                frame_to_events(method, &params, Origin::Replay)
                    .iter()
                    .any(|e| matches!(e, AgentEvent::Gap { .. }))
            })
            .collect();
        assert!(unread.is_empty(), "notifications that would draw a break: {unread:?}");
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
    fn the_thread_name_becomes_the_title_every_gui_puts_in_its_header() {
        // ACP fed `SessionInfo` and codex did not, so a codex pane's header
        // stayed blank while the server was sending the name all along.
        assert!(matches!(
            frame_to_events(
                "thread/name/updated",
                &serde_json::json!({ "threadId": "t", "threadName": "Fix the picker" }),
                Origin::Live)
            .as_slice(),
            [AgentEvent::SessionInfo { title }] if title == "Fix the picker"
        ));
        // Nullable, and null means cleared. A header reading "null" would be
        // worse than a blank one.
        assert!(
            frame_to_events(
                "thread/name/updated",
                &serde_json::json!({ "threadId": "t", "threadName": null }),
                Origin::Live)
            .is_empty()
        );
    }

    /// One file's change, as `FileUpdateChange` puts it on the wire.
    fn change(path: &str, kind: &str, diff: &str) -> serde_json::Value {
        serde_json::json!({ "path": path, "kind": { "type": kind }, "diff": diff })
    }

    #[test]
    fn an_edit_carries_both_sides_of_the_patch_rather_than_no_diff_at_all() {
        // The comment this replaces claimed there was no before-and-after on
        // the wire. There is: the patch states both sides of every line it
        // touches, and neither side is invented here. The exact string is what
        // codex-cli 0.147.0 sent for a one-line edit — hunks only, no header.
        let events = frame_to_events(
            "item/completed",
            &serde_json::json!({ "item": { "type": "fileChange", "id": "f",
                "status": "completed", "changes": [change("/w/hello.txt", "update",
                    "@@ -1,3 +1,3 @@\n alpha\n-bravo\n+BRAVO\n charlie\n")] } }),
            Origin::Live,
        );
        let [AgentEvent::ToolUpdate { diff: Some(diff), .. }] = events.as_slice() else {
            panic!("expected one update carrying a diff: {events:?}");
        };
        assert_eq!(diff.path, "/w/hello.txt");
        assert_eq!(
            diff.old_text.as_deref(),
            Some("alpha\nbravo\ncharlie\n"),
            "context and the - lines"
        );
        assert_eq!(diff.new_text, "alpha\nBRAVO\ncharlie\n", "context and the + lines");
    }

    #[test]
    fn a_created_file_is_read_as_content_because_that_is_what_the_wire_sends() {
        // The bug the first version of this shipped with: `add` does not carry
        // a patch. It carries the whole new file, verbatim, with no `@@` and no
        // `+` prefixes — so parsing it as a diff found no hunk, produced
        // nothing, and every file codex created showed no diff at all.
        // Observed against 0.147.0, not assumed from the schema, which types
        // the field as a plain string and says nothing about this.
        let added = diff_of(&change("/w/fresh.txt", "add", "one\ntwo\n")).expect("a diff");
        assert_eq!(added.old_text, None, "the file did not exist");
        assert_eq!(added.new_text, "one\ntwo\n");

        // And `delete` is the mirror of it: the content the file HAD.
        let deleted =
            diff_of(&change("/w/doomed.txt", "delete", "gamma\ndelta\n")).expect("a diff");
        assert_eq!(deleted.old_text.as_deref(), Some("gamma\ndelta\n"));
        assert_eq!(deleted.new_text, "", "nothing is left of it");
    }

    #[test]
    fn an_insertion_into_an_existing_file_is_not_reported_as_a_new_file() {
        // `old_text: None` says the file did not exist. A pure insertion also
        // has an empty old side, and letting that read as None would claim the
        // rest of the file was never there.
        let inserted =
            diff_of(&change("/w/old.txt", "update", "@@ -1,0 +1,1 @@\n+one\n")).expect("a diff");
        assert_eq!(inserted.old_text.as_deref(), Some(""), "empty, but it existed");
    }

    #[test]
    fn a_patch_this_cannot_read_produces_no_diff_rather_than_an_empty_one() {
        // An empty diff view is a claim that nothing changed, which is a
        // stronger and more wrong statement than showing no diff at all.
        assert!(diff_of(&change("x.txt", "update", "")).is_none());
        assert!(diff_of(&change("x.txt", "update", "diff --git a/x.txt b/x.txt\n")).is_none());
    }

    #[test]
    fn a_patch_arriving_mid_apply_updates_the_row_it_names() {
        // Without this an edit appeared only once it was finished. `itemId` is
        // what attaches it to a row that already exists.
        let events = frame_to_events(
            "item/fileChange/patchUpdated",
            &serde_json::json!({ "itemId": "f", "threadId": "t", "turnId": "u",
                "changes": [change("a.rs", "update", "@@ -1 +1 @@\n-a\n+b\n")] }),
            Origin::Live,
        );
        let [AgentEvent::ToolUpdate { id, status, diff: Some(diff), locations, .. }] =
            events.as_slice()
        else {
            panic!("expected one update carrying a diff: {events:?}");
        };
        assert_eq!(id, "f", "or it hangs off nothing");
        assert_eq!(*status, ToolStatus::InProgress, "the patch is still being applied");
        assert_eq!(diff.new_text, "b\n");
        assert_eq!(locations, &["a.rs"]);
    }

    #[test]
    fn a_multi_file_patch_keeps_every_file_instead_of_showing_only_the_last() {
        // A row holds one diff and every client applies updates last-wins, so
        // two files sent as two diffs would render the second and silently
        // hide the first.
        let events = frame_to_events(
            "item/completed",
            &serde_json::json!({ "item": { "type": "fileChange", "id": "f",
                "status": "completed", "changes": [
                    change("a.rs", "update", "@@ -1 +1 @@\n-a\n+b\n"),
                    change("b.rs", "update", "@@ -1 +1 @@\n-c\n+d\n")] } }),
            Origin::Live,
        );
        let [AgentEvent::ToolUpdate { diff, content: Some(content), locations, .. }] =
            events.as_slice()
        else {
            panic!("expected one update: {events:?}");
        };
        assert!(diff.is_none(), "no single diff can stand for two files");
        assert!(content.contains("a.rs") && content.contains("b.rs"), "both are in the body");
        assert_eq!(locations, &["a.rs", "b.rs"]);
    }

    #[test]
    fn a_declined_patch_is_not_reported_as_a_finished_edit() {
        // `PatchApplyStatus` has `declined` and `failed`. Reporting either as
        // Completed drew a finished green row for an edit that never landed.
        for status in ["declined", "failed"] {
            let events = frame_to_events(
                "item/completed",
                &serde_json::json!({ "item": { "type": "fileChange", "id": "f",
                    "status": status, "changes": [] } }),
                Origin::Live,
            );
            assert!(
                matches!(
                    events.as_slice(),
                    [AgentEvent::ToolUpdate { status: ToolStatus::Failed, .. }]
                ),
                "{status} should not look finished: {events:?}"
            );
        }
    }

    #[test]
    fn a_server_error_reads_as_a_failure_rather_than_as_missing_history() {
        // Unhandled, this drew "Something happened here that this version
        // cannot show" — telling the user history was missing when nothing was
        // missing and something had gone wrong.
        let events = frame_to_events(
            "error",
            &serde_json::json!({ "threadId": "t", "turnId": "u", "willRetry": true,
                "error": { "message": "stream disconnected",
                           "additionalDetails": "upstream closed the connection" } }),
            Origin::Live,
        );
        let [AgentEvent::Message { role: Role::Agent, text, .. }] = events.as_slice() else {
            panic!("expected the server's own words: {events:?}");
        };
        assert!(text.starts_with("Error: stream disconnected"), "{text}");
        assert!(text.contains("upstream closed the connection"), "{text}");
        assert!(text.contains("Retrying."), "willRetry means the turn is not over: {text}");
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. })),
            "turn/completed carries the end; ending it here too would end it twice"
        );
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
