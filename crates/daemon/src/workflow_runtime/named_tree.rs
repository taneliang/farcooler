//! A run's worktree, named by the orchestrator at Start (ov-967): a Far Cooler worktree the app lists, claimed
//! for the board, on the branch it named, from main as it is at Start. Without a name a run keeps its private
//! tree under the runtime root.
use super::*;
use farcooler_protocol::{workflow_commands::Action, workflow_config::AdapterConfiguration};
use farcooler_core::error::DomainError;
use std::sync::Arc;

/// The per-repository locks the reconciler and `worktree.create` take, shared so a run's worktree is made and
/// recorded with no gap in which the reconciler could adopt it unclaimed.
pub type RepoLocks = Arc<std::sync::Mutex<std::collections::HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>>;

/// What the runtime needs from the Service to make a worktree the app lists.
#[derive(Clone)]
pub struct WorktreeHost {
    /// Where the app keeps worktrees: one directory per repository, the leaf being the worktree's name.
    pub dir: PathBuf,
    /// Marks the worktree as this install's, so another install on the host doesn't adopt it.
    pub install_id: String,
    pub locks: RepoLocks,
}
impl WorktreeHost {
    fn lock(&self, repository: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(locks.entry(repository).or_default())
    }
}

impl Runtime {
    /// A Start that names a worktree: where it goes, and main's commit now, which the run records. Refuses a
    /// name or branch that is taken, before anything is made. `None` for any other action, a Start that names
    /// none, one already answered (its receipt is replayed) and one that names a workflow no longer published
    /// (refused as stale by the command itself).
    pub(super) async fn prepare_start(&self, workspace: &str, principal: &str, action: &Action) -> Result<Option<RunTree>> {
        let Action::Start { mutation, worktree: Some(named), start_pin, .. } = action else { return Ok(None) };
        let id = format!("wire:{principal}:{}", mutation.id);
        if self.store.workflow_wire_receipt(workspace, &id, &serde_json::to_string(&(principal, action))?)?.is_some() { return Ok(None); }
        let Some(configuration) = self.store.workflow_configuration(workspace)?.filter(|c| &c.pin == start_pin) else { return Ok(None) };
        let host = self.host.as_ref().ok_or(Error::Refused("this runner can't make a named worktree for a run"))?;
        let Some(AdapterConfiguration::Coding(config)) = configuration.adapter else {
            return Err(Error::Refused("a named worktree needs a coding workflow"));
        };
        farcooler_core::validate::worktree_name(&named.name).map_err(Error::Domain)?;
        farcooler_core::validate::branch_name(&named.branch).map_err(Error::Domain)?;
        let board = Uuid::parse_str(workspace).map_err(|_| Error::Refused("invalid workspace"))?;
        let repository = self.store.get_workspace(board).and_then(|w| self.store.get_repository(w.repository_id))
            .map_err(|_| Error::Refused("unknown repository"))?;
        let path = host.dir.join(farcooler_core::names::slug(&repository.display_name)).join(farcooler_core::names::slug(&named.name));
        if path.try_exists()? { return Err(Error::Domain(DomainError::WorktreeExists)); }
        // Any branch of that name, here or on a remote: the run's branch is new, as `fork_only` makes it.
        let taken = adapters::git(&config.repository, &["for-each-ref", "--format=%(refname)",
            &format!("refs/heads/{}", named.branch), &format!("refs/remotes/*/{}", named.branch)]).await?;
        if !taken.trim().is_empty() { return Err(Error::Domain(DomainError::BranchExists)); }
        let base = adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await?;
        let tree = RunTree { name: named.name.clone(), branch: named.branch.clone(), path: path.to_string_lossy().into_owned(), base };
        if !tree.valid() { return Err(Error::Refused("invalid worktree")); }
        Ok(Some(tree))
    }
}

impl Exec {
    /// Make the run's named worktree, record it, and claim it for the board, under the repository's lock.
    pub(super) async fn provision_named(&self, workspace: &str, binding: &Binding, tree: &RunTree) -> Result<()> {
        let host = self.host.as_ref().ok_or(Error::Refused("this runner can't make a named worktree for a run"))?;
        if binding.base != tree.base { return Err(Error::Refused("named worktree base differs from the run's")); }
        let board = Uuid::parse_str(workspace).map_err(|_| Error::Refused("invalid workspace"))?;
        let repository = self.store.get_workspace(board).map_err(|_| Error::Refused("unknown workspace"))?.repository_id;
        let lock = host.lock(repository);
        let _held = lock.lock().await;
        let path = Path::new(&tree.path);
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        let created = crate::git::create_worktree_with(&self.repository(), &tree.branch, &tree.base, path, true).await
            .map_err(|_| Error::Refused("named worktree provisioning uncertain"))?;
        crate::git::mark_owner(path, &host.install_id).await;
        if created.forked { crate::git::mark_forked(path, &host.install_id).await; }
        let row = self.store.create_worktree(repository, &tree.branch, &tree.path, false)
            .map_err(|_| Error::Refused("named worktree could not be recorded"))?;
        self.store.claim_worktree(row.id, board, farcooler_store::models::ClaimSource::Explicit)
            .map_err(|_| Error::Refused("named worktree could not be claimed"))?;
        Ok(())
    }
}
