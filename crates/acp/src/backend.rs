//! `AgentBackend`, as ACP performs it.

use farcooler_agent_core::backend::{AgentBackend, BackendError, Capabilities};
use farcooler_agent_core::event::{AgentEvent, PromptImage};

use crate::conn::AcpError;
use crate::session::{RunningSession, SessionError};

/// A started ACP session, as a backend.
///
/// Thin by design: `RunningSession` already does the work, and everything this
/// adds is the shape `ChatSession` speaks to. The one piece of real state is
/// `replay`, which cannot be a constant because ACP decides it per connection —
/// an adapter advertises `agentCapabilities.loadSession` at `initialize` or it
/// does not, and that answer differs between the four adapters shipped today.
pub struct AcpBackend {
    session: RunningSession,
    replay: bool,
}

impl AcpBackend {
    pub fn new(session: RunningSession, replay: bool) -> Self {
        AcpBackend { session, replay }
    }

    pub fn session_id(&self) -> &str {
        &self.session.session_id
    }

    /// The frame receiver, for the caller that races it in a `select!`.
    ///
    /// `next_events` must never be used in a `select!` — it awaits `respond`
    /// after `handle_fs_write` has already touched the disk, so cancellation
    /// between them leaves the file written and the agent waiting forever on
    /// an answer that is never coming. Callers that race take `recv_frame`
    /// here and `handle` the result outside the select.
    pub fn session_mut(&mut self) -> &mut RunningSession {
        &mut self.session
    }
}

impl From<SessionError> for BackendError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Acp(inner) => inner.into(),
            // Both mean the same thing to a caller: this session cannot
            // proceed. `Refused` carries no message here because neither
            // variant has one — the path refusal is ours, not the agent's.
            SessionError::Refused => {
                BackendError::Refused("the path is outside the workspace worktree".into())
            }
            SessionError::Rejected => {
                BackendError::Refused("the agent did not accept the session".into())
            }
        }
    }
}

impl AgentBackend for AcpBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities { replay: self.replay, ..Capabilities::acp() }
    }

    async fn prompt(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        Ok(self.session.prompt(text, images).await?)
    }

    async fn steer(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        Ok(self.session.steer(text, images).await?)
    }

    async fn answer(&mut self, request_id: &str, option_id: &str) -> Result<(), BackendError> {
        // The id travels as the JSON text the adapter sent, because inventing
        // a new one here would make the answer unroutable. Parsing it back is
        // this backend's business rather than the caller's.
        let id: serde_json::Value =
            serde_json::from_str(request_id).unwrap_or(serde_json::Value::Null);
        Ok(self.session.answer(id, option_id).await?)
    }

    async fn set_config_option(&mut self, id: &str, value: &str) -> Result<(), BackendError> {
        // `mode` and `model` are well-known ids rather than methods, and the
        // adapter in use still answers `Method not found` to the generic
        // `session/set_config_option` while accepting the dedicated ones. So
        // the collapse happens here, at the boundary that knows which wire it
        // is talking to, rather than in the trait.
        match id {
            "mode" => Ok(self.session.set_mode(value).await?),
            "model" => Ok(self.session.set_model(value).await?),
            other => Ok(self.session.set_config_option(other, value).await?),
        }
    }

    async fn cancel(&mut self) -> Result<(), BackendError> {
        Ok(self.session.cancel().await?)
    }

    async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError> {
        Ok(self.session.next_events().await?)
    }
}

impl From<AcpError> for BackendError {
    fn from(e: AcpError) -> Self {
        match e {
            AcpError::Spawn => BackendError::Spawn,
            AcpError::Closed => BackendError::Closed,
            // Folded into Closed deliberately. A frame we cannot parse means
            // this conversation cannot continue, which is the same outcome as
            // a closed pipe — and two names for one outcome would make callers
            // branch on a distinction with no different action behind it.
            AcpError::Malformed => BackendError::Closed,
            AcpError::Refused(message) => BackendError::Refused(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_keeps_the_adapters_own_words_across_the_boundary() {
        // The whole value of Refused is that it carries what the agent said.
        // A conversion that dropped the message would leave the caller with
        // nothing more useful than "it failed".
        let mapped: BackendError = AcpError::Refused("Session not found".into()).into();
        assert!(mapped.to_string().contains("Session not found"), "{mapped}");
    }

    #[test]
    fn a_malformed_frame_reads_as_a_closed_connection() {
        // Not a lossy mapping by accident: there is no recovery from a frame
        // this build cannot read, so the honest report is that the
        // conversation is over.
        assert!(matches!(BackendError::from(AcpError::Malformed), BackendError::Closed));
    }
}
