//! ACP frames, deserialized only as far as the normalizer needs.
//!
//! Deliberately lenient: unknown fields are ignored and unknown update kinds
//! deserialize to `Unknown` rather than failing the frame. A strict decoder
//! would turn every adapter release into an outage, and the honest response to
//! an update we do not understand is a visible `Gap` — not a dropped
//! connection, and never a silently shorter transcript.

use serde::Deserialize;

/// One JSON-RPC frame from the adapter.
#[derive(Debug, Deserialize)]
pub struct Rpc {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    /// A JSON-RPC error reply.
    ///
    /// Modelled because ignoring it is not a small omission: a caller waiting
    /// for `result` on this id waits forever, and the failure presents as a
    /// hang rather than as an error anyone can read.
    #[serde(default)]
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SessionNotification {
    #[serde(rename = "sessionId", default)]
    pub session_id: String,
    pub update: SessionUpdate,
}

impl Rpc {
    /// The frame as a `session/update` notification, if that is what it is.
    pub fn session_notification(&self) -> Option<SessionNotification> {
        if self.method.as_deref() != Some("session/update") {
            return None;
        }
        serde_json::from_value(self.params.clone()?).ok()
    }
}

#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct WirePlanEntry {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub status: String,
}

/// One block of a tool call's own output.
///
/// Either a nested text block — the console output, fenced as markdown — or a
/// diff describing an edit. Modelled because a tool row without its output is
/// a row that says a command ran and refuses to say what happened.
#[derive(Debug, Deserialize)]
pub struct ToolContent {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub content: Option<ContentBlock>,
    #[serde(default)]
    pub path: String,
    #[serde(rename = "oldText", default)]
    pub old_text: Option<String>,
    #[serde(rename = "newText", default)]
    pub new_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Location {
    #[serde(default)]
    pub path: String,
}

/// One entry of the slash-command menu carried by `available_commands_update`.
///
/// Only `name` is captured. `description` and `input` are real fields on the
/// wire (see the fixtures) but nothing downstream reads them yet, and this
/// file stays lenient by design — unused fields are left for serde to ignore
/// rather than modeled speculatively.
#[derive(Debug, Deserialize)]
pub struct WireAvailableCommand {
    #[serde(default)]
    pub name: String,
    /// What the command does, which the adapter has always sent and this had
    /// always thrown away — so the picker could only ever list names, and a
    /// list of names is a list you have to already know.
    #[serde(default)]
    pub description: String,
}

/// The envelope the adapter hangs its own extensions from.
///
/// Everything Far Cooler needs about subagents arrives here rather than in a
/// modelled field, because ACP has no subagent concept at all — the adapter
/// carries the structure out-of-band and the protocol stays unaware of it.
#[derive(Debug, Default, Deserialize)]
pub struct Meta {
    #[serde(rename = "claudeCode", default)]
    pub claude_code: ClaudeMeta,
}

#[derive(Debug, Default, Deserialize)]
pub struct ClaudeMeta {
    /// This tool call IS a subagent dispatch — the `Task` row.
    #[serde(default)]
    pub subagent: bool,
    /// This frame is a subagent's work. The value is the dispatching call's
    /// `toolCallId`, which is the whole of what nests it.
    #[serde(rename = "parentToolUseId", default)]
    pub parent_tool_use_id: Option<String>,
    /// Present on a dispatch's final update, and nowhere else.
    #[serde(rename = "toolResponse", default)]
    pub tool_response: Option<SubagentResult>,
}

/// What a finished subagent reports about itself.
///
/// Structured on the wire, so read as structure. The same numbers also appear
/// inside the tool's `rawOutput` as a `<usage>` text blob; parsing that would
/// break the first time the adapter reworded a line nobody promised to keep.
#[derive(Debug, Deserialize)]
pub struct SubagentResult {
    #[serde(rename = "agentType", default)]
    pub agent_type: String,
    #[serde(rename = "resolvedModel", default)]
    pub resolved_model: String,
    #[serde(rename = "totalTokens", default)]
    pub total_tokens: u64,
    #[serde(rename = "totalToolUseCount", default)]
    pub total_tool_use_count: u64,
    #[serde(rename = "totalDurationMs", default)]
    pub total_duration_ms: u64,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    AgentMessageChunk {
        content: ContentBlock,
        #[serde(rename = "_meta", default)]
        meta: Meta,
    },
    UserMessageChunk {
        content: ContentBlock,
        #[serde(rename = "_meta", default)]
        meta: Meta,
    },
    AgentThoughtChunk {
        content: ContentBlock,
        #[serde(rename = "_meta", default)]
        meta: Meta,
    },
    /// The full slash-command list, resent once per turn (60+ entries, tens
    /// of KB in a real capture). Session metadata, not conversation content —
    /// see `normalize::update_to_events` for why this must not become a
    /// `Gap`. Without this variant it fell into `Unknown` and every single
    /// turn produced a spurious "history missing" break in the transcript.
    AvailableCommandsUpdate {
        #[serde(rename = "availableCommands", default)]
        available_commands: Vec<WireAvailableCommand>,
    },
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        status: String,
        #[serde(default)]
        locations: Vec<Location>,
        #[serde(default)]
        content: Vec<ToolContent>,
        #[serde(rename = "_meta", default)]
        meta: Meta,
    },
    ToolCallUpdate {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(default)]
        status: String,
        /// Revised as the call resolves: `Terminal` becomes the command, and
        /// `Read File` becomes the file. Ignoring it left every row showing
        /// the generic placeholder it started as.
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        content: Vec<ToolContent>,
        #[serde(default)]
        locations: Vec<Location>,
        /// The tool's own arguments. Carried for the few tools whose title
        /// says nothing on its own — `Skill` is "Skill" until you look at
        /// which skill.
        #[serde(rename = "rawInput", default)]
        raw_input: serde_json::Value,
        #[serde(rename = "_meta", default)]
        meta: Meta,
    },
    Plan {
        #[serde(default)]
        entries: Vec<WirePlanEntry>,
    },
    CurrentModeUpdate {
        #[serde(rename = "currentModeId")]
        current_mode_id: String,
    },
    /// A selector changed, possibly because the AGENT changed it. Plan mode
    /// flipping itself off after producing a plan arrives this way, and a UI
    /// that ignored it would show the wrong mode until the next reload.
    ConfigOptionUpdate {
        #[serde(rename = "configId", default)]
        config_id: String,
        #[serde(default)]
        value: serde_json::Value,
    },
    /// The session's own name, which the adapter derives from the first
    /// prompt and revises as the conversation goes on.
    SessionInfoUpdate {
        #[serde(default)]
        title: String,
    },
    /// Context-window usage, resent as a turn consumes it.
    UsageUpdate {
        #[serde(default)]
        used: u64,
        #[serde(default)]
        size: u64,
    },
    /// Anything this version does not know. Becomes a visible `Gap`.
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod meta_tests {
    use super::*;

    #[test]
    fn a_dispatch_is_recognized_as_one() {
        // The Task row and an ordinary tool row are the same `tool_call` on
        // the wire; `subagent` is the only thing telling them apart, and
        // without it a subagent's block has no row to hang from.
        let raw = r#"{"sessionUpdate":"tool_call","toolCallId":"t1","title":"Task",
            "_meta":{"claudeCode":{"toolName":"Agent","subagent":true}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let SessionUpdate::ToolCall { meta, .. } = update else { panic!("expected a tool call") };
        assert!(meta.claude_code.subagent);
    }

    #[test]
    fn a_subagents_frame_names_the_call_that_dispatched_it() {
        // The whole design rests on this pointer. Discarding it is what
        // attributed a subagent's words to the agent that dispatched it.
        let raw = r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"},
            "_meta":{"claudeCode":{"parentToolUseId":"toolu_01Wnr"}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let SessionUpdate::AgentMessageChunk { meta, .. } = update else {
            panic!("expected a chunk")
        };
        assert_eq!(meta.claude_code.parent_tool_use_id.as_deref(), Some("toolu_01Wnr"));
    }

    #[test]
    fn a_finished_subagent_reports_what_it_cost() {
        // Captured verbatim from a live adapter. These are the fields the
        // collapsed summary line is built from; parsing them out of the
        // `rawOutput` text blob instead would break the first time the
        // adapter reworded it.
        let raw = r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed",
            "_meta":{"claudeCode":{"toolResponse":{"status":"completed","agentType":"general-purpose",
            "resolvedModel":"claude-opus-5[1m]","totalDurationMs":4962,"totalTokens":12479,
            "totalToolUseCount":1}}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let SessionUpdate::ToolCallUpdate { meta, .. } = update else {
            panic!("expected an update")
        };
        let result = meta.claude_code.tool_response.expect("a result");
        assert_eq!(result.agent_type, "general-purpose");
        assert_eq!(result.total_tokens, 12479);
        assert_eq!(result.total_tool_use_count, 1);
        assert_eq!(result.total_duration_ms, 4962);
        assert_eq!(result.resolved_model, "claude-opus-5[1m]");
    }

    #[test]
    fn a_frame_with_no_meta_is_still_a_frame() {
        // Most frames carry no `_meta` at all. Requiring it would fail every
        // ordinary turn.
        let raw = r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let SessionUpdate::AgentMessageChunk { meta, .. } = update else {
            panic!("expected a chunk")
        };
        assert!(!meta.claude_code.subagent);
        assert!(meta.claude_code.parent_tool_use_id.is_none());
    }
}
