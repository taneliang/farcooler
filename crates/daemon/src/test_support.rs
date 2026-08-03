//! Shared test fixtures for the daemon crate.
//!
//! Extracted from `reconcile.rs`'s test module so `service.rs`'s tests can
//! reach the same real-git-repo fixture without a second, drifting copy.

use std::sync::Arc;

use uuid::Uuid;

use crate::git;
use crate::service::Service;

/// A service with one registered repository, on a real git repo on disk.
///
/// Real repositories rather than hand-written fixtures: `reconcile.rs`'s
/// tests are about the shape of `git worktree list --porcelain`, and a
/// hand-written fixture would only prove the author can copy it. Sharing it
/// with `service.rs`'s tests means both get a workspace row that came from
/// the same adoption path a live daemon uses.
pub(crate) async fn fixture() -> (tempfile::TempDir, Arc<Service>, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        git::git(&repo, &args).await.unwrap();
    }

    let svc = Arc::new(Service::open_in(state).await.unwrap());
    svc.add_root(dir.path()).await.unwrap();
    let registered = svc.register_repository(&repo).await.unwrap();
    (dir, svc, registered.id)
}
