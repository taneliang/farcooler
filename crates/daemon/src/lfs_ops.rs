//! Which large files a worktree didn't download, and trying again (ov-199).
//!
//! A worktree of a Git LFS repository hydrates its large files best effort
//! (`crate::git_lfs::hydrate`): an object missing from the local store, or a
//! hydration that ran out of time, leaves the pointer file. The runner writes
//! down which paths ([`farcooler_store::lfs_pointers`]) and reports the count
//! (`Worktree.lfs_pointers`); the apps say "Some large files weren't
//! downloaded." and offer `worktree.hydrate_lfs`. The paths stay here: the
//! count is what a card needs, and `farcooler worktree show` can list them
//! later.

use farcooler_core::Result;
use farcooler_store::models;
use uuid::Uuid;

use crate::service::Service;

impl Service {
    /// Record the pointer paths a worktree was made with, and answer the
    /// worktree as it now stands. Best effort: the worktree is made either
    /// way, and a store that can't say is a worktree that says nothing.
    pub(crate) fn record_lfs_pointers(&self, worktree: models::Worktree, paths: &[String]) -> models::Worktree {
        if paths.is_empty() {
            return worktree;
        }
        if let Err(e) = self.store.set_lfs_pointers(worktree.id, paths) {
            tracing::warn!(worktree = %worktree.id, error = ?e, "couldn't record the LFS files left as pointers");
            return worktree;
        }
        self.store.get_worktree(worktree.id).unwrap_or(worktree)
    }

    /// `worktree.hydrate_lfs`: try again to download the large files this
    /// worktree still holds as pointers. Answers the worktree with its fresh
    /// count.
    ///
    /// Held under the repository's lock like every other write to its
    /// worktrees. What it does to the worktree is `git_lfs::rehydrate`'s, which
    /// never touches the agent's index.
    pub async fn hydrate_lfs(&self, id: Uuid) -> Result<models::Worktree> {
        let ws = self.store.get_worktree(id)?;
        let recorded = self.store.lfs_pointer_paths(id)?;
        if recorded.is_empty() {
            return Ok(ws);
        }
        let lock = self.repo_lock(ws.repository_id);
        let _guard = lock.lock().await;
        let left = crate::git_lfs::rehydrate(std::path::Path::new(&ws.worktree_path), &recorded).await;
        self.store.set_lfs_pointers(id, &left)?;
        self.store.get_worktree(id)
    }

    /// Whether the recorded count moved because the files did: an agent ran
    /// `git lfs pull`, or deleted them. Called when a client reads the
    /// worktree's changes, so the count is fresh where its notice is seen. A
    /// stat and a read of at most a kilobyte per recorded path, and nothing
    /// for a worktree with none.
    pub(crate) async fn recheck_lfs_pointers(&self, id: Uuid) -> bool {
        let Ok(recorded) = self.store.lfs_pointer_paths(id) else { return false };
        if recorded.is_empty() {
            return false;
        }
        let Ok(ws) = self.store.get_worktree(id) else { return false };
        let left = crate::git_lfs::pointers(std::path::Path::new(&ws.worktree_path), &recorded).await;
        self.store.set_lfs_pointers(id, &left).unwrap_or(false)
    }
}

#[cfg(test)]
#[path = "service_lfs_tests.rs"]
mod tests;
