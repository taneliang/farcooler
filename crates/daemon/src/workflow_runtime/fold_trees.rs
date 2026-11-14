//! Groups release only build output. Source trees, reports and Git references remain available.
use super::*;

impl Exec {
    /// Kept at every former tree-pruning call site, including recovery and cancellation.
    /// A finished group's checkout can still hold the owner's source and commits (ov-1203).
    pub(super) async fn prune_trees(&self, workspaces: &[String], _recovering: bool) {
        if let Err(error) = self.prune_process_logs(workspaces) {
            tracing::warn!(%error, "could not prune orphan process logs");
        }
        if let Err(error) = self.release_builds(workspaces).await {
            tracing::warn!(%error, "could not release build output");
        }
    }
}
