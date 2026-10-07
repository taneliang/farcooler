//! Stop and Send Now in a terminal-mode claude pane (ov-368): one Esc, or
//! claude's ctrl+x ctrl+s, pressed by the runner past its typing gate, and
//! answered once claude says it took. Refused with a word, nothing pressed,
//! when the runner can't make it safe (`answer_wake::interrupt`).

use farcooler_protocol::capability::TERMINAL_INTERRUPT;
use farcooler_protocol::v1::{self as pb, Terminal, request, result};
use uuid::Uuid;

use super::results::wrong;
use super::{Session, SessionError, require};

impl Session {
    /// `terminal.interrupt`: stop the turn claude is working on.
    pub async fn interrupt(&self, terminal: Uuid) -> Result<Terminal, SessionError> {
        self.press("terminal.interrupt", terminal).await
    }

    /// `terminal.send_now`: send the messages waiting in claude's queue now.
    pub async fn send_now(&self, terminal: Uuid) -> Result<Terminal, SessionError> {
        self.press("terminal.send_now", terminal).await
    }

    async fn press(&self, method: &'static str, terminal: Uuid) -> Result<Terminal, SessionError> {
        require(self.capabilities(), TERMINAL_INTERRUPT, method)?;
        let payload =
            request::Payload::AgentCancel(pb::AgentCancel { terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()) });
        // Targeted, so it keeps its order against this terminal's other input.
        match self.value_requiring(method, Some(terminal), Some(payload), Vec::new()).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }
}
