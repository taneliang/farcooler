//! A landing that reached main lands itself (ov-503, ov-872).
//!
//! The CI watch (`ci_watch.rs`) moves a pushed landing to green when its SHA's
//! CI passed. In a direct-mode workspace that SHA was pushed to the base
//! branch, so a green landing whose SHA is an ancestor of the base branch has
//! landed, and nobody should have to say so. Each turn, after the reads, this
//! moves such a landing to `landed`; the store lands its tickets with it.
//!
//! Pull-request workspaces are left alone. A landing there is a rehearsal that
//! is never pushed, and a squash merge never makes its SHA an ancestor.
//!
//! The check reads only refs this clone already has (`merge-base
//! --is-ancestor` against the remote-tracking base branch), never the network:
//! a push from this runner's checkout moves that ref at once, and a clone that
//! someone else pushed from sees the landing land on the turn after its next
//! fetch.

use std::path::Path;

use farcooler_store::landing::LandingMode;
use farcooler_store::models::Actor;
use farcooler_store::landings::{Landing, LandingState, LandingUpdate, landing_title};
use uuid::Uuid;

use crate::service::Service;
use crate::watch::Watcher;

/// The branch a workspace lands on, as a ref this clone has: the workspace's
/// own base when it names one, else what `origin/HEAD` says, else `main`.
async fn base_ref(svc: &Service, workspace: Uuid, tree: &Path) -> String {
    let named = svc.store.get_landing(workspace).ok().and_then(|l| l.base);
    let name = match named {
        Some(base) => base,
        None => crate::change_set::default_branch_local(tree).await.unwrap_or_else(|| "main".into()),
    };
    let bare = name.strip_prefix("origin/").unwrap_or(&name).to_string();
    crate::change_set::remote_qualified(tree, &bare).await
}

/// Whether `sha` is on `base` in `tree`: false for a commit this clone has
/// never seen. `None` when git could not run at all (a repository path that is
/// gone, a git that times out), which says nothing about the commit.
pub async fn is_on_base(tree: &Path, base: &str, sha: &str) -> Option<bool> {
    crate::git::git(tree, &["merge-base", "--is-ancestor", sha, base]).await.ok().map(|out| out.ok)
}

/// Landings whose check failed to run, so each is logged once and not on
/// every turn. A landing that checks fine again is forgotten, to be logged if
/// it breaks again.
fn unreadable() -> &'static std::sync::Mutex<std::collections::HashSet<Uuid>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<Uuid>>> = std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
}

/// Note that `landing`'s check ran (`ok`) or could not; true when this is the
/// first failure since it last ran, which is when to say so.
pub fn note_check(landing: Uuid, ok: bool) -> bool {
    let mut set = unreadable().lock().unwrap_or_else(|e| e.into_inner());
    if ok {
        set.remove(&landing);
        false
    } else {
        set.insert(landing)
    }
}

/// Whether a green landing has reached its workspace's base branch.
async fn reached_base(svc: &Service, landing: &Landing, tree: &Path) -> bool {
    let Some(sha) = landing.pushed_sha.as_deref() else { return false };
    if svc.store.get_landing(landing.workspace_id).ok().and_then(|l| l.mode) == Some(LandingMode::PullRequests) {
        return false;
    }
    let base = base_ref(svc, landing.workspace_id, tree).await;
    let on = is_on_base(tree, &base, sha).await;
    if note_check(landing.id, on.is_some()) {
        let called = landing_title(landing);
        tracing::warn!(landing = %called, path = %tree.display(), "could not ask git whether a green landing is on its base branch");
    }
    on == Some(true)
}

/// Land every green landing that is on its base branch, and announce each
/// board it changed. The landings landed, by id.
pub async fn land_green_landings(svc: &Service, watcher: &Watcher) -> Vec<Uuid> {
    // Landing is the engine's after release, and the landing tables are fenced (ov-947).
    if !svc.store.legacy_authority() { return Vec::new(); }
    let green: Vec<Landing> =
        svc.store.landings_following_ci().unwrap_or_default().into_iter().filter(|l| l.state == LandingState::Green).collect();
    let mut landed = Vec::new();
    for landing in green {
        let Some((_, tree)) = crate::ci_watch::tree_of(svc, landing.workspace_id) else { continue };
        if !reached_base(svc, &landing, &tree).await {
            continue;
        }
        // Only if it is still the green landing that was checked: a new SHA
        // since is one CI has not passed.
        let Some(sha) = landing.pushed_sha.clone() else { continue };
        let update = LandingUpdate { state: Some(LandingState::Landed), expect: Some((LandingState::Green, sha)), ..Default::default() };
        match svc.store.update_landing(landing.id, &update, Actor::Runner) {
            Err(farcooler_core::DomainError::InvalidArgument { what: "landing_changed" }) => {}
            Ok(_) => {
                landed.push(landing.id);
                watcher.announce_plan_changed(landing.workspace_id, Actor::Runner);
                // Landing may have sent the collecting landing off.
                watcher.tell_departures();
            }
            Err(e) => tracing::warn!(error = %e, landing = %landing_title(&landing), "could not land a landing that reached its base"),
        }
    }
    landed
}
