//! A terminal's agent rows on a session (ov-366): a page, and a follow by
//! revision, served by every runner from ov-830 (one from before only
//! while its projector setting is on).

use super::*;

impl Session {
    /// Up to `limit` of `terminal`'s rows before `before` (`agent.rows`),
    /// oldest first, with the epoch and revision to follow from. With
    /// `agent`, that subagent's own rows instead (ov-453).
    pub async fn agent_rows(
        &self,
        terminal: Uuid,
        before: Option<u64>,
        limit: u32,
        agent: Option<&str>,
    ) -> Result<farcooler_protocol::v1::AgentRowPage, SessionError> {
        let required = self.rows_required(agent, "agent.rows")?;
        let payload = request::Payload::AgentRowsPage(farcooler_protocol::v1::AgentRowsPage {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            before,
            limit,
            hint_rows: true,
            agent_id: agent.unwrap_or_default().to_string(),
        });
        match self.value_requiring("agent.rows", None, Some(payload), required).await? {
            result::Value::AgentRowPage(page) => Ok(page),
            other => Err(wrong("agent_row_page", &other)),
        }
    }

    /// What changed in `terminal`'s rows after `after_rev` of projection
    /// `epoch` (`agent.rows_follow`), the runner holding the call up to
    /// `wait_ms` while nothing has. With `agent`, that subagent's own rows.
    pub async fn agent_rows_follow(
        &self,
        terminal: Uuid,
        epoch: u64,
        after_rev: u64,
        wait_ms: u32,
        agent: Option<&str>,
    ) -> Result<farcooler_protocol::v1::AgentRowChanges, SessionError> {
        let required = self.rows_required(agent, "agent.rows_follow")?;
        let payload = request::Payload::AgentRowsFollow(farcooler_protocol::v1::AgentRowsFollow {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            epoch,
            after_rev,
            wait_ms,
            hint_rows: true,
            agent_id: agent.unwrap_or_default().to_string(),
        });
        match self.value_requiring("agent.rows_follow", None, Some(payload), required).await? {
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

    /// What a rows call needs the runner to have: `agent_rows`, and for a
    /// subagent's own rows `subagent_rows` too, named on the request as well,
    /// since a runner without it would drop the id and answer with the
    /// pane's rows as though they were the agent's.
    fn rows_required(&self, agent: Option<&str>, method: &str) -> Result<Vec<String>, SessionError> {
        use farcooler_protocol::capability::{AGENT_ROWS, SUBAGENT_ROWS};
        require(self.capabilities(), AGENT_ROWS, method)?;
        if agent.is_none() {
            return Ok(Vec::new());
        }
        require(self.capabilities(), SUBAGENT_ROWS, method)?;
        Ok(vec![SUBAGENT_ROWS.to_string()])
    }
}
