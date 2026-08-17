//! Shared test fixtures for the daemon crate.
//!
//! Extracted from `reconcile.rs`'s test module so `service.rs`'s tests can
//! reach the same real-git-repo fixture without a second, drifting copy.

use std::sync::Arc;

use uuid::Uuid;

use crate::git;
use crate::service::Service;

/// The fixture's temporary directory, plus the tmux server that came with it.
///
/// A `Service` starts a private tmux server the first time a test asks it for a
/// terminal, and nothing ever stopped one. The `TempDir` went away when the test
/// ended and the server did not: it holds a session, a pane and an interactive
/// shell, on a socket whose name died with the directory that produced it, so
/// nothing could ever find it again to clean it up.
///
/// They accumulate. On the machine this was written for there were 362 live tmux
/// servers and 2 454 sockets under `/tmp`, and they are not inert — tmux is
/// single-threaded per server and they compete for the same CPU. `capture-pane`
/// against the real fleet measured 50ms at rest and 740ms with that crowd
/// running, which is slow enough to have broken timing assumptions in the tmux
/// crate's own suite.
///
/// Deref rather than a new type at every call site: two dozen tests already say
/// `dir.path()`, and none of them should have to know this exists.
pub(crate) struct ScratchDir {
    dir: tempfile::TempDir,
    socket: String,
}

impl std::ops::Deref for ScratchDir {
    type Target = tempfile::TempDir;
    fn deref(&self) -> &tempfile::TempDir {
        &self.dir
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // Blocking, and not the async `kill_server`. `Drop` cannot await, and a
        // task spawned here would want a runtime that is being torn down to poll
        // it. Killing a server that never started fails harmlessly and costs a
        // couple of milliseconds; leaving one running costs the machine for as
        // long as it stays on.
        let Some(tmux) = farcooler_core::programs::find("tmux") else { return };
        let _ = std::process::Command::new(tmux)
            .args(["-L", &self.socket, "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// A service with one registered repository, on a real git repo on disk.
///
/// Real repositories rather than hand-written fixtures: `reconcile.rs`'s
/// tests are about the shape of `git worktree list --porcelain`, and a
/// hand-written fixture would only prove the author can copy it. Sharing it
/// with `service.rs`'s tests means both get a workspace row that came from
/// the same adoption path a live daemon uses.
pub(crate) async fn fixture() -> (ScratchDir, Arc<Service>, Uuid) {
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
    // Read from the service rather than recomputed, so the reaper cannot end up
    // aiming at a socket name the service does not actually use.
    let socket = svc.tmux.socket().to_string();
    (ScratchDir { dir, socket }, svc, registered.id)
}

/// Two services at two roots, both registering the SAME repository.
///
/// This is what two channels on one host are. A channel's only job is to
/// choose a runtime directory, so two roots is two channels — and it is the
/// same shape `rpc_over_socket.rs` already relies on, where every test gets "a
/// daemon on a private socket with a private database" at an explicit
/// directory rather than through the process-global `FARCOOLER_HOME`.
///
/// One repository on purpose. Two installs that never touched the same repo
/// would be isolated by having nothing in common, which proves nothing; git
/// reports every worktree of a repository regardless of which daemon made it,
/// and that is the case worth testing.
pub(crate) async fn two_daemons()
-> (tempfile::TempDir, Arc<Service>, Arc<Service>, Uuid, Uuid) {
    let dir = tempfile::tempdir().unwrap();
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

    let mut svcs = Vec::new();
    let mut repos = Vec::new();
    for tag in ["a", "b"] {
        let state = dir.path().join(tag);
        std::fs::create_dir_all(&state).unwrap();
        let svc = Arc::new(Service::open_in(state).await.unwrap());
        svc.add_root(dir.path()).await.unwrap();
        let registered = svc.register_repository(&repo).await.unwrap();
        svcs.push(svc);
        repos.push(registered.id);
    }

    let b = svcs.pop().unwrap();
    let a = svcs.pop().unwrap();
    (dir, a, b, repos[0], repos[1])
}
