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

#[derive(Debug, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    AgentMessageChunk {
        content: ContentBlock,
    },
    UserMessageChunk {
        content: ContentBlock,
    },
    AgentThoughtChunk {
        content: ContentBlock,
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
