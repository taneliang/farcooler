//! A workspace's orchestrator pages, for the page: in their own file, as the
//! page layer's rule asks (ov-269, scripts/page-layer-lint.py).

use uuid::Uuid;

use crate::error::WebError;
use crate::wire::Runner;

impl Runner {
    /// One of a workspace's orchestrator pages, in the phones' shape
    /// (`page_json`, ov-768).
    pub async fn page(&self, workspace: Uuid, slot: &str) -> Result<serde_json::Value, WebError> {
        Ok(farcooler_client::page_json::page_json(&self.session.page(workspace, slot).await?))
    }
}
