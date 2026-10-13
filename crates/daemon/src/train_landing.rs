//! A train that reached main lands itself (ov-503).
//!
//! The CI watch (`ci_watch.rs`) moves a pushed train to green when its SHA's
//! CI passed. In a direct-mode workspace that SHA was pushed to the base
//! branch, so a green train whose SHA is an ancestor of the base branch has
//! landed, and nobody should have to say so: trains 81 to 87 stayed
//! "integrating" until the owner saw them. Each turn, after the reads, this
//! moves such a train to `landed`; the store lands its lanes with it.
//!
//! Pull-request workspaces are left alone. A train there is a rehearsal that
//! is never pushed, and a squash merge never makes its SHA an ancestor.
//!
//! The check reads only refs this clone already has (`merge-base
//! --is-ancestor` against the remote-tracking base branch), never the network:
//! a push from this runner's checkout moves that ref at once, and a clone that
//! someone else pushed from sees the train land on the turn after its next
//! fetch.

use std::path::Path;

use farcooler_store::landing::LandingMode;
use farcooler_store::models::Actor;
use farcooler_store::trains::{Train, TrainState, TrainUpdate};
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
/// never seen, and for any git failure.
pub async fn is_on_base(tree: &Path, base: &str, sha: &str) -> bool {
    matches!(crate::git::git(tree, &["merge-base", "--is-ancestor", sha, base]).await, Ok(out) if out.ok)
}

/// Whether a green train has reached its workspace's base branch.
async fn reached_base(svc: &Service, train: &Train, tree: &Path) -> bool {
    let Some(sha) = train.pushed_sha.as_deref() else { return false };
    if svc.store.get_landing(train.workspace_id).ok().and_then(|l| l.mode) == Some(LandingMode::PullRequests) {
        return false;
    }
    let base = base_ref(svc, train.workspace_id, tree).await;
    is_on_base(tree, &base, sha).await
}

/// Land every green train that is on its base branch, and announce each board
/// it changed. The trains landed, by id.
pub async fn land_green_trains(svc: &Service, watcher: &Watcher) -> Vec<Uuid> {
    let green: Vec<Train> =
        svc.store.trains_following_ci().unwrap_or_default().into_iter().filter(|t| t.state == TrainState::Green).collect();
    let mut landed = Vec::new();
    for train in green {
        let Some((_, tree)) = crate::ci_watch::tree_of(svc, train.workspace_id) else { continue };
        if !reached_base(svc, &train, &tree).await {
            continue;
        }
        let update = TrainUpdate { state: Some(TrainState::Landed), ..Default::default() };
        match svc.store.set_train(train.id, &update, Actor::Runner) {
            Ok(_) => {
                landed.push(train.id);
                watcher.announce_plan_changed(train.workspace_id, Actor::Runner);
            }
            Err(e) => tracing::warn!(error = %e, train = %train.name, "could not land a train that reached its base"),
        }
    }
    landed
}
