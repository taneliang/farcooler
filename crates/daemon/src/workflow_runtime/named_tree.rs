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

/// Two branch names that can't both exist: the same, or one a path prefix of the other (`foo`, `foo/bar`).
fn clashes(a: &str, b: &str) -> bool {
    a == b || a.strip_prefix(b).is_some_and(|r| r.starts_with('/')) || b.strip_prefix(a).is_some_and(|r| r.starts_with('/'))
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
        let fresh = host.dir.join(farcooler_core::names::slug(&repository.display_name)).join(farcooler_core::names::slug(&named.name));
        // An adopted branch is worked on where it is already checked out, or in a new tree on it (ov-968).
        let found = match named.adopt.then(|| adopted_at(&config.repository, &named.branch)) {
            Some(found) => found.await?,
            None => None,
        };
        if named.adopt { self.refuse_unadoptable(&config, board, repository.id, &named.branch, found.as_deref()).await?; }
        let path = found.unwrap_or(fresh);
        let listed = named.adopt && path.try_exists()?;
        if path.try_exists()? && !listed { return Err(Error::Domain(DomainError::WorktreeExists)); }
        // What git itself accepts as a branch name, so a name it would refuse fails here and not at Provision.
        if adapters::git(&config.repository, &["check-ref-format", "--branch", &named.branch]).await.is_err() {
            return Err(Error::Domain(DomainError::InvalidArgument { what: "branch" }));
        }
        // Another run in this workspace already holds the tree or the branch, though it may not be made yet.
        if let Some(state) = self.state(workspace)? {
            let held = |f: &dyn Fn(&RunTree) -> bool| state.items.values().filter_map(|i| i.tree.as_ref()).any(f);
            if held(&|t| t.path == path.to_string_lossy()) { return Err(Error::Domain(DomainError::WorktreeExists)); }
            if held(&|t| clashes(&t.branch, &named.branch)) { return Err(Error::Domain(DomainError::BranchExists)); }
        }
        // Any branch of that name, here or on a remote, or one that clashes with it by prefix (`foo` beside
        // `foo/bar`): the run's branch is new, as `fork_only` makes it.
        let mut patterns = vec!["for-each-ref".to_string(), "--format=%(refname)".into(),
            format!("refs/heads/{}", named.branch), format!("refs/remotes/*/{}", named.branch)];
        let parents: Vec<&str> = named.branch.match_indices('/').map(|(i, _)| &named.branch[..i]).collect();
        for parent in &parents { patterns.push(format!("refs/heads/{parent}")); patterns.push(format!("refs/remotes/*/{parent}")); }
        let refs: Vec<&str> = patterns.iter().map(String::as_str).collect();
        let found = adapters::git(&config.repository, &refs).await?;
        let clash = found.lines().any(|line| {
            let name = line.strip_prefix("refs/heads/").or_else(|| line.strip_prefix("refs/remotes/").and_then(|r| r.split_once('/').map(|x| x.1)));
            name.is_some_and(|n| clashes(n, &named.branch))
        });
        if !named.adopt && clash { return Err(Error::Domain(DomainError::BranchExists)); }
        let base = adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await?;
        let tree = RunTree { name: named.name.clone(), branch: named.branch.clone(), path: path.to_string_lossy().into_owned(), base,
            adopted: named.adopt };
        if !tree.valid() { return Err(Error::Refused("invalid worktree")); }
        Ok(Some(tree))
    }
}

impl Runtime {
    /// What may be adopted (ov-968 review): a plain local branch that is nobody's but the work's. Never the landing
    /// branch or a remote's default, which a build would commit straight onto; never a remote-tracking name; never
    /// the engine's own `wf-` branches or a tree under its root, which are another run's; and only a worktree the
    /// app lists, on this board, with no pane running in it, so the run is the one writer.
    async fn refuse_unadoptable(&self, config: &CodingConfig, board: Uuid, repository: Uuid, branch: &str, found: Option<&Path>) -> Result<()> {
        let refuse = |what: &'static str| Err(Error::Domain(DomainError::InvalidArgument { what }));
        let repo = &config.repository;
        let local = adapters::git(repo, &["for-each-ref", "--format=%(refname)", &format!("refs/heads/{branch}")]).await?;
        if local.lines().all(|l| l != format!("refs/heads/{branch}")) { return refuse("not a local branch"); }
        let mut protected = vec!["main".to_string(), "master".into()];
        protected.extend([config.landing_ref.as_str()].into_iter().chain(config.landing.as_ref().map(|l| l.target.as_str()))
            .map(|r| r.strip_prefix("refs/heads/").unwrap_or(r).to_string()));
        // Each remote's default branch, as `refs/remotes/<remote>/HEAD` names it.
        let heads = adapters::git(repo, &["for-each-ref", "--format=%(symref)", "refs/remotes/*/HEAD"]).await.unwrap_or_default();
        protected.extend(heads.lines().filter_map(|h| h.strip_prefix("refs/remotes/")?.split_once('/').map(|(_, b)| b.to_string())));
        if protected.iter().any(|p| p == branch) { return refuse("a landing or default branch"); }
        let remotes = adapters::git(repo, &["remote"]).await.unwrap_or_default();
        if remotes.lines().any(|r| !r.is_empty() && branch.starts_with(&format!("{r}/"))) { return refuse("a remote-tracking branch"); }
        if branch.starts_with("wf-") { return refuse("another run's branch"); }
        let Some(path) = found else { return Ok(()) };
        if canonical(path).starts_with(canonical(&self.root)) { return refuse("another run's branch"); }
        let row = self.store.list_worktrees_for_repository(repository).map_err(Error::Domain)?
            .into_iter().find(|w| same_path(Path::new(&w.worktree_path), path));
        let Some(row) = row else { return refuse("a worktree the app doesn't list") };
        if row.workspace_id.is_some_and(|w| w != board) { return refuse("a worktree on another board"); }
        let live = self.store.list_terminals_for_worktree(row.id).map_err(Error::Domain)?
            .iter().any(|t| t.exit_code.is_none() && t.exit_signal.is_none());
        if live { return refuse("a worktree with a pane running in it"); }
        Ok(())
    }
}

/// A path with its links resolved, so `/tmp` and `/private/tmp` (as git reports it) are one place.
fn canonical(p: &Path) -> PathBuf { p.canonicalize().unwrap_or_else(|_| p.to_path_buf()) }
fn same_path(a: &Path, b: &Path) -> bool { canonical(a) == canonical(b) }

/// The linked worktree that has `branch` checked out, if one does. The repository's own checkout is where a person
/// works by hand, so a branch checked out there is refused rather than adopted.
async fn adopted_at(repository: &Path, branch: &str) -> Result<Option<PathBuf>> {
    let trees = crate::git::list_worktrees(repository).await.map_err(Error::Domain)?;
    match trees.into_iter().find(|t| t.branch.as_deref() == Some(branch) && !t.prunable) {
        Some(t) if t.is_main => Err(Error::Domain(DomainError::InvalidArgument { what: "branch checked out in the main checkout" })),
        found => Ok(found.map(|t| PathBuf::from(t.path))),
    }
}

/// A named tree is visible, so people may have changes in it. A gate restarting there stops and asks instead of
/// running `reset --hard` and `clean` over them.
pub(super) async fn refuse_foreign_changes(tree: &Path) -> Result<()> {
    if crate::git::is_dirty(tree).await.unwrap_or(true) {
        return Err(Error::Refused("the run's worktree has changes the run didn't make; commit or remove them to continue"));
    }
    Ok(())
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
        if tree.adopted { return self.provision_adopted(board, repository, &host.install_id, tree).await; }
        let path = Path::new(&tree.path);
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        let created = crate::git::create_worktree_with(&self.repository(), &tree.branch, &tree.base, path, true).await
            .map_err(|_| Error::Refused("named worktree provisioning uncertain"))?;
        crate::git::mark_owner(path, &host.install_id).await;
        if created.forked { crate::git::mark_forked(path, &host.install_id).await; }
        // The tree is made, so a failure to record or claim it takes it back out (it is clean and at its base),
        // and a retry can name the same tree and branch again.
        let recorded = self.store.create_worktree(repository, &tree.branch, &tree.path, false)
            .map_err(|_| Error::Refused("named worktree could not be recorded"))
            .and_then(|row| self.store.claim_worktree(row.id, board, farcooler_store::models::ClaimSource::Explicit)
                .map_err(|_| { let _ = self.store.delete_worktree(row.id, row.resource_version); Error::Refused("named worktree could not be claimed") }));
        if let Err(error) = recorded {
            let _ = crate::git::rollback_worktree(&self.repository(), &tree.branch, path, &tree.base).await;
            return Err(error);
        }
        Ok(())
    }
}

impl Exec {
    /// An adopted branch (ov-968): carry on in the worktree that has it checked out, or make one on it. The branch
    /// is never made or deleted here. The tree must be clean and have no pane running in it, so the run is the
    /// one writer; it is recorded and claimed for the board if it isn't already.
    async fn provision_adopted(&self, board: Uuid, repository: Uuid, install_id: &str, tree: &RunTree) -> Result<()> {
        let path = Path::new(&tree.path);
        let row = self.store.list_worktrees_for_repository(repository).map_err(Error::Domain)?
            .into_iter().find(|w| same_path(Path::new(&w.worktree_path), path));
        if path.try_exists()? {
            let head = adapters::git(path, &["symbolic-ref", "HEAD"]).await?;
            if head != format!("refs/heads/{}", tree.branch) { return Err(Error::Refused("the adopted worktree has moved to another branch")); }
            refuse_foreign_changes(path).await?;
            // Start refused a tree the app doesn't list; one that lost its row since is refused here too.
            let Some(row) = &row else { return Err(Error::Refused("the adopted worktree isn't one the app lists")) };
            {
                if row.workspace_id.is_some_and(|w| w != board) { return Err(Error::Refused("the adopted worktree belongs to another board")); }
                let live = self.store.list_terminals_for_worktree(row.id).map_err(Error::Domain)?
                    .iter().any(|t| t.exit_code.is_none() && t.exit_signal.is_none());
                if live { return Err(Error::Refused("a pane is still running in the adopted worktree; close it so the run is its one writer")); }
            }
        } else {
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
            crate::git::create_worktree_from_branch(&self.repository(), &tree.branch, path).await
                .map_err(|_| Error::Refused("adopted worktree provisioning uncertain"))?;
            crate::git::mark_owner(path, install_id).await;
        }
        let row = match row {
            Some(row) => row,
            None => self.store.create_worktree(repository, &tree.branch, &tree.path, false)
                .map_err(|_| Error::Refused("adopted worktree could not be recorded"))?,
        };
        if row.workspace_id.is_none() {
            self.store.claim_worktree(row.id, board, farcooler_store::models::ClaimSource::Explicit)
                .map_err(|_| Error::Refused("adopted worktree could not be claimed"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_dirty_named_tree_is_refused_and_left_alone_and_a_clean_one_passes() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| { let ok = std::process::Command::new("/usr/bin/git").current_dir(dir.path())
            .args(["-c", "user.name=T", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false"]).args(args)
            .status().unwrap().success(); assert!(ok, "{args:?}"); };
        git(&["init", "-q"]);
        git(&["commit", "-q", "--allow-empty", "-m", "base"]);
        refuse_foreign_changes(dir.path()).await.unwrap();
        std::fs::write(dir.path().join("mine.txt"), "someone's work").unwrap();
        assert!(refuse_foreign_changes(dir.path()).await.is_err());
        assert_eq!(std::fs::read_to_string(dir.path().join("mine.txt")).unwrap(), "someone's work");
    }
}
