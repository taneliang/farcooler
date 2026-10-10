//! A terminal's agent rows on a session (ov-366): a page, and a follow by
//! revision, served while the runner's projector is on: its setting
//! (`set_projector`, ov-372) or `FARCOOLER_PROJECTOR=1`.

use super::*;

impl Session {
    /// Up to `limit` of `terminal`'s rows before `before` (`agent.rows`),
    /// oldest first, with the epoch and revision to follow from.
    pub async fn agent_rows(
        &self,
        terminal: Uuid,
        before: Option<u64>,
        limit: u32,
    ) -> Result<farcooler_protocol::v1::AgentRowPage, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::AGENT_ROWS, "agent.rows")?;
        let payload = request::Payload::AgentRowsPage(farcooler_protocol::v1::AgentRowsPage {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            before,
            limit,
            hint_rows: true,
        });
        match self.value("agent.rows", None, Some(payload)).await? {
            result::Value::AgentRowPage(page) => Ok(page),
            other => Err(wrong("agent_row_page", &other)),
        }
    }

    /// What changed in `terminal`'s rows after `after_rev` of projection
    /// `epoch` (`agent.rows_follow`), the runner holding the call up to
    /// `wait_ms` while nothing has.
    pub async fn agent_rows_follow(
        &self,
        terminal: Uuid,
        epoch: u64,
        after_rev: u64,
        wait_ms: u32,
    ) -> Result<farcooler_protocol::v1::AgentRowChanges, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::AGENT_ROWS, "agent.rows_follow")?;
        let payload = request::Payload::AgentRowsFollow(farcooler_protocol::v1::AgentRowsFollow {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            epoch,
            after_rev,
            wait_ms,
            hint_rows: true,
        });
        match self.value("agent.rows_follow", None, Some(payload)).await? {
            result::Value::AgentRowChanges(changes) => Ok(changes),
            other => Err(wrong("agent_row_changes", &other)),
        }
    }

    /// Image `index` of the prompt on turn row `row` (`agent.image`, ov-454),
    /// its MIME type and bytes, asked for a piece at a time until whole.
    pub async fn agent_image(&self, terminal: Uuid, row: &str, index: u32) -> Result<(String, Vec<u8>), SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::AGENT_IMAGES, "agent.image")?;
        let mut bytes = Vec::new();
        loop {
            let payload = request::Payload::AgentImage(farcooler_protocol::v1::AgentImageRequest {
                terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
                row_id: row.to_string(),
                index,
                offset: bytes.len() as u64,
            });
            let piece = match self.value("agent.image", None, Some(payload)).await? {
                result::Value::AgentImage(piece) => piece,
                other => return Err(wrong("agent_image", &other)),
            };
            // A piece from anywhere but where this left off, or none past
            // it, is a runner that changed its mind about the image.
            if piece.offset != bytes.len() as u64 || (piece.chunk.is_empty() && (bytes.len() as u64) < piece.total_size) {
                return Err(SessionError::Protocol("agent.image answered out of order".into()));
            }
            bytes.extend_from_slice(&piece.chunk);
            if bytes.len() as u64 >= piece.total_size {
                return Ok((piece.mime_type, bytes));
            }
        }
    }

    /// Turn the runner's projector on or off (`settings.set_projector`,
    /// ov-372): `[agents] projector` in its config.toml, live at once. A
    /// hello offers `agent_rows` only while it's on, and a hello is made once
    /// per connection, so a client reconnects to read rows after turning it
    /// on (ov-373, a phone's settings row). Needs `host_admin`.
    pub async fn set_projector(&self, on: bool) -> Result<(), SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::PROJECTOR_SETTING, "settings.set_projector")?;
        let payload = request::Payload::HostSettings(farcooler_protocol::v1::HostSettings { branch_prefix: String::new(), projector: on });
        match self.value("settings.set_projector", None, Some(payload)).await? {
            result::Value::Empty(_) => Ok(()),
            other => Err(wrong("empty", &other)),
        }
    }
}
