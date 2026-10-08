//! Bring Here in a terminal-mode claude pane (ov-369, R-28): the draft a
//! person left in claude's own box, read so a native composer can take it,
//! then cleared. Two calls, so the text is never in neither place
//! (`answer_wake::bring`).

use farcooler_protocol::capability::BRING_DRAFT;
use farcooler_protocol::v1::{self as pb, BroughtDraft, request, result};
use uuid::Uuid;

use super::results::wrong;
use super::{Session, SessionError, require};

impl Session {
    /// `terminal.bring_draft`: the box's draft, read with nothing typed; with
    /// `expected`, the text a read answered, the box cleared too, only while
    /// it still reads exactly that.
    pub async fn bring_draft(&self, terminal: Uuid, expected: Option<&str>) -> Result<BroughtDraft, SessionError> {
        require(self.capabilities(), BRING_DRAFT, "terminal.bring_draft")?;
        let payload = request::Payload::BringDraft(pb::BringDraft {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            expected: expected.unwrap_or_default().to_string(),
        });
        // Targeted, so it keeps its order against this terminal's other input.
        match self.value_requiring("terminal.bring_draft", Some(terminal), Some(payload), Vec::new()).await? {
            result::Value::BroughtDraft(brought) => Ok(brought),
            other => Err(wrong("brought_draft", &other)),
        }
    }
}
