//! `terminal.draft_prompt`: Ask the Orchestrator's paste, from a phone (ov-241),
//! held behind a dialog where the runner can (ov-385).

use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, request, result};
use serde_json::json;
use uuid::Uuid;

use super::results::wrong;
use super::{Session, SessionError, require};

/// What the runner did with a draft.
#[derive(Debug)]
pub enum Drafted {
    /// Pasted into the box, unsent.
    Pasted,
    /// Held behind a dialog, to be pasted once it closes (`draft_hold`).
    Held(pb::DraftHold),
}

impl Session {
    /// Ask the runner to paste `text` into a terminal orchestrator's box,
    /// pressing no Enter. The daemon refuses, typing nothing, unless the pane
    /// is provably an agent with an empty box; the refusal comes back as the
    /// usual error and the phone copies the text instead, as the Mac does.
    /// A runner with `draft_hold` holds it behind a dialog rather than refuse
    /// for one, and says so with the hold.
    pub async fn draft_prompt(&self, terminal: Uuid, text: &str) -> Result<Drafted, SessionError> {
        let hold = self.can(farcooler_protocol::capability::DRAFT_HOLD);
        let payload = request::Payload::AgentPrompt(pb::AgentPrompt {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            blocks: vec![pb::AgentPromptBlock { content: Some(Content::Text(text.to_string())) }],
            hold_behind_dialog: hold,
        });
        // Named, so a runner that lost the capability refuses rather than
        // dropping the flag and refusing for the dialog with no word why.
        let required = if hold { vec![farcooler_protocol::capability::DRAFT_HOLD.to_string()] } else { Vec::new() };
        match self.value_requiring("terminal.draft_prompt", None, Some(payload), required).await? {
            result::Value::Terminal(_) => Ok(Drafted::Pasted),
            result::Value::DraftHold(h) => Ok(Drafted::Held(h)),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// Withdraw the draft `hold` held on `terminal`: answers with the hold as
    /// it now is.
    pub async fn draft_withdraw(&self, terminal: Uuid, hold: Uuid) -> Result<pb::DraftHold, SessionError> {
        let payload = request::Payload::DraftWithdraw(pb::DraftWithdraw {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            hold_id: bytes::Bytes::copy_from_slice(hold.as_bytes()),
        });
        let required = vec![farcooler_protocol::capability::DRAFT_HOLD.to_string()];
        match self.value_requiring("terminal.draft_withdraw", None, Some(payload), required).await? {
            result::Value::DraftHold(h) => Ok(h),
            other => Err(wrong("draft_hold", &other)),
        }
    }

    /// `terminal.compose` (ov-372): `text` typed into a terminal-mode agent
    /// pane's box on one line and submitted past the same gate as
    /// `terminal tell`, or refused with nothing typed. True when the agent
    /// was working and its own queue took it.
    pub async fn compose(&self, terminal: Uuid, text: &str) -> Result<bool, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::AGENT_ROWS, "terminal.compose")?;
        let payload = request::Payload::AgentPrompt(pb::AgentPrompt {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            blocks: vec![pb::AgentPromptBlock { content: Some(Content::Text(text.to_string())) }],
            hold_behind_dialog: false,
        });
        match self.value("terminal.compose", None, Some(payload)).await? {
            result::Value::TerminalTold(told) => Ok(told.queued),
            other => Err(wrong("terminal_told", &other)),
        }
    }
}

/// A hold as both phones read it: `id` as a uuid string, `state` as a word
/// (`waiting`, `sent`, `withdrawn`, `expired`, `failed`), and its times in Unix ms.
pub fn draft_hold_json(hold: &pb::DraftHold) -> serde_json::Value {
    let id = Uuid::from_slice(&hold.id).map(|id| id.to_string()).unwrap_or_default();
    json!({
        "id": id,
        "state": draft_hold_state(hold.state),
        "heldMs": hold.held_ms,
        "expiresMs": hold.expires_ms,
        "endedMs": hold.ended_ms,
    })
}

/// The word for a hold's state; an unknown one from a newer runner reads as
/// ended, never as waiting, so no client waits on it forever.
pub fn draft_hold_state(state: i32) -> &'static str {
    match pb::DraftHoldState::try_from(state) {
        Ok(pb::DraftHoldState::Waiting) => "waiting",
        Ok(pb::DraftHoldState::Sent) => "sent",
        Ok(pb::DraftHoldState::Withdrawn) => "withdrawn",
        Ok(pb::DraftHoldState::Failed) => "failed",
        _ => "expired",
    }
}
