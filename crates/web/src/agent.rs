//! An agent's conversation, for the page (ov-776): the same rows, in the same
//! JSON, as the Mac and the phones follow.
//!
//! A terminal-mode agent's transcript is folded by the runner's projector into
//! rows (`crates/core/src/session_log/projector`). The page reads them as the
//! apps do: `agent.rows` for the newest, `agent.rows_follow` for what changed
//! after a revision, held open by the runner while nothing has. It sends a
//! message through `terminal.compose`, which is the call the Mac's composer
//! makes, so the Mac shows it as the person's turn, and stops a turn through
//! `terminal.interrupt`. Nothing here is a protocol of the browser's own.

use uuid::Uuid;

use serde::Serialize;

use crate::error::WebError;
use crate::wire::Runner;

impl Runner {
    /// Up to `limit` of `terminal`'s rows before `before` (the newest when
    /// `None`), as `{epoch, rev, moreBefore, rows}`.
    pub async fn agent_rows(
        &self,
        terminal: Uuid,
        before: Option<u64>,
        limit: u32,
    ) -> Result<serde_json::Value, WebError> {
        let page = self.session.agent_rows(terminal, before, limit, None).await?;
        Ok(farcooler_client::rows_json::rows_page_json(&page))
    }

    /// What changed in `terminal`'s rows after `after_rev` of projection
    /// `epoch`, as `{epoch, rev, reset, changes}`. The runner holds the call
    /// up to `wait_ms` while nothing has.
    pub async fn agent_rows_follow(
        &self,
        terminal: Uuid,
        epoch: u64,
        after_rev: u64,
        wait_ms: u32,
    ) -> Result<serde_json::Value, WebError> {
        let changes = self.session.agent_rows_follow(terminal, epoch, after_rev, wait_ms, None).await?;
        Ok(farcooler_client::rows_json::changes_json(&changes))
    }

    /// Send `text` to the agent as the person's message. True when the agent
    /// was busy and the message waits in its queue.
    pub async fn compose(&self, terminal: Uuid, text: &str) -> Result<bool, WebError> {
        Ok(self.session.compose_with_files(terminal, text, &[], &[]).await?)
    }

    /// `compose`, with `images` (MIME type and bytes) typed into the agent's
    /// box too, as the apps' composers send them. The session refuses images
    /// past the limits (`compose_limits`) before anything is sent.
    pub async fn compose_images(&self, terminal: Uuid, text: &str, images: &[(String, Vec<u8>)]) -> Result<bool, WebError> {
        Ok(self.session.compose(terminal, text, images).await?)
    }

    /// What the runner takes from the composer: whether it takes images at
    /// all, and the most bytes of one image and of all of them together. A
    /// runner with `compose_upload` stages each image in chunks (so the
    /// limits are the paste's and the upload's); an older one carries them
    /// inside the request.
    pub fn compose_limits(&self) -> ComposeLimits {
        use farcooler_protocol::capability::{COMPOSE, COMPOSE_UPLOAD};
        let upload = self.session.can(COMPOSE_UPLOAD);
        ComposeLimits {
            images: self.session.can(COMPOSE),
            each: if upload { farcooler_protocol::MAX_PASTE_FILE_BYTES } else { farcooler_protocol::MAX_COMPOSE_IMAGE_BYTES as u64 },
            total: if upload { farcooler_protocol::MAX_COMPOSE_UPLOAD_BYTES as u64 } else { farcooler_protocol::MAX_COMPOSE_IMAGE_BYTES as u64 },
        }
    }

    /// Stop the turn the agent is working on.
    pub async fn interrupt(&self, terminal: Uuid) -> Result<(), WebError> {
        self.session.interrupt(terminal).await?;
        Ok(())
    }
}

/// What the composer may send (`Runner::compose_limits`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ComposeLimits {
    pub images: bool,
    /// The most bytes in one image.
    pub each: u64,
    /// The most bytes in all of a message's images together.
    pub total: u64,
}
