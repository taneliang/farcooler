//! `terminal.draft_prompt`: Ask the Orchestrator's paste, from a phone (ov-241).

use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, request, result};
use uuid::Uuid;

use super::results::wrong;
use super::{Session, SessionError};

impl Session {
    /// Ask the runner to paste `text` into a terminal orchestrator's box,
    /// pressing no Enter. The daemon refuses, typing nothing, unless the pane
    /// is provably an idle agent with an empty box; the refusal comes back as
    /// the usual error and the phone copies the text instead, as the Mac does.
    pub async fn draft_prompt(&self, terminal: Uuid, text: &str) -> Result<pb::Terminal, SessionError> {
        let payload = request::Payload::AgentPrompt(pb::AgentPrompt {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            blocks: vec![pb::AgentPromptBlock { content: Some(Content::Text(text.to_string())) }],
        });
        match self.value("terminal.draft_prompt", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }
}
