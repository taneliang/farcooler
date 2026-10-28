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

    /// Stop the turn the agent is working on.
    pub async fn interrupt(&self, terminal: Uuid) -> Result<(), WebError> {
        self.session.interrupt(terminal).await?;
        Ok(())
    }
}
