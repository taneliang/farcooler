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
            let held = |f: &dyn Fn(&RunTree) -> bool| state.items.values()
                // A run that has ended holds its name only while its tree is still there (ov-979).
                .filter(|i| !matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled) || i.tree.as_ref().is_some_and(|t| Path::new(&t.path).exists()))
                .filter_map(|i| i.tree.as_ref()).any(f);
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
        let local = adapters::git(repo, &["for-each-ref", "--format=%(refname) %(symref)", &format!("refs/heads/{branch}")]).await?;
        let Some(found_ref) = local.lines().find(|l| l.split(' ').next() == Some(&format!("refs/heads/{branch}"))) else {
            return refuse("adopt_not_local");
        };
        // A symbolic ref is a second name for another branch, which may be the landing branch itself.
        if found_ref.split_once(' ').is_some_and(|(_, to)| !to.is_empty()) { return refuse("adopt_alias"); }
        let mut protected = vec!["main".to_string(), "master".into()];
        protected.extend([config.landing_ref.as_str()].into_iter().chain(config.landing.as_ref().map(|l| l.target.as_str()))
            .map(|r| r.strip_prefix("refs/heads/").unwrap_or(r).to_string()));
        // Each remote's default branch, as `refs/remotes/<remote>/HEAD` names it.
        let heads = adapters::git(repo, &["for-each-ref", "--format=%(symref)", "refs/remotes/*/HEAD"]).await.unwrap_or_default();
        protected.extend(heads.lines().filter_map(|h| h.strip_prefix("refs/remotes/")?.split_once('/').map(|(_, b)| b.to_string())));
        if protected.iter().any(|p| p == branch) { return refuse("adopt_protected"); }
        let remotes = adapters::git(repo, &["remote"]).await.unwrap_or_default();
        if remotes.lines().any(|r| !r.is_empty() && branch.starts_with(&format!("{r}/"))) { return refuse("adopt_remote_name"); }
        if branch.starts_with("wf-") { return refuse("adopt_engine"); }
        let Some(path) = found else { return Ok(()) };
        if canonical(path).starts_with(canonical(&self.root)) { return refuse("adopt_engine"); }
        let row = self.store.list_worktrees_for_repository(repository).map_err(Error::Domain)?
            .into_iter().find(|w| same_path(Path::new(&w.worktree_path), path));
        let Some(row) = row else { return refuse("adopt_unlisted") };
        if row.workspace_id.is_some_and(|w| w != board) { return refuse("adopt_other_board"); }
        let live = self.store.list_terminals_for_worktree(row.id).map_err(Error::Domain)?
            .iter().any(|t| t.exit_code.is_none() && t.exit_signal.is_none());
        if live { return refuse("adopt_busy"); }
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

/// What a named tree's Run gate changed, noted by the daemon while the gate ran (ov-979): each path with the hash
/// of what was there at the last look (`None`: deleted). A Run gate can be any command, so the daemon looks for
/// it, and a look can't tell the gate's writes from a person's, so the ledger only names what changed while a
/// crashed gate ran, for the person deciding; it never decides. It is kept in the daemon's receipts, not in the tree.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct GateFiles {
    run: String,
    candidate: Option<String>,
    /// Changed already when the gate began, though the tree was just checked clean: never the gate's.
    before: std::collections::BTreeSet<String>,
    files: std::collections::BTreeMap<String, Option<String>>,
}

/// How often a running gate's changes are noted. A file written after the last look is unlabeled.
pub(super) const GATE_LOOK: std::time::Duration = std::time::Duration::from_secs(1);

fn write_ledger(ledger: &Path, files: &GateFiles) -> Result<()> {
    let temporary = ledger.with_extension(format!("{}.tmp", Uuid::now_v7()));
    durable(&temporary, &serde_json::to_vec(files)?)?;
    std::fs::rename(&temporary, ledger)?;
    Ok(())
}
fn read_ledger(ledger: &Path) -> Result<Option<GateFiles>> {
    match std::fs::read(ledger) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
/// What is at a path now: a file's bytes, a link's target, or `None` when nothing is there.
fn content_hash(path: &Path) -> Result<Option<String>> {
    use sha2::Digest;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let (kind, bytes) = if meta.file_type().is_symlink() {
        ("link", std::fs::read_link(path)?.into_os_string().into_encoded_bytes())
    } else if meta.is_file() { ("file", std::fs::read(path)?) } else { return Ok(Some("other".into())) };
    let mut hash = sha2::Sha256::new();
    hash.update(kind.as_bytes());
    hash.update([0]);
    hash.update(&bytes);
    Ok(Some(format!("{:x}", hash.finalize())))
}

/// Every path the tree has changed now, as git sees it.
async fn changed(tree: &Path) -> Result<crate::change_set::WorkingTree> {
    crate::change_set::working_tree(tree).await.map_err(|_| Error::Refused("the run's worktree could not be read"))
}
/// A Run gate is about to start in a named tree that was just checked clean: open its ledger. Anything changed
/// all the same (a person, in the moment since the check) is set aside as never the gate's.
pub(super) async fn begin_gate(tree: &Path, ledger: &Path, run: &str, candidate: Option<&str>) -> Result<()> {
    let now = changed(tree).await?;
    let before = now.staged.iter().chain(&now.unstaged).chain(&now.untracked).map(|c| c.path.clone())
        .chain(now.conflicted.iter().cloned()).collect();
    write_ledger(ledger, &GateFiles { run: run.into(), candidate: candidate.map(Into::into), before, ..Default::default() })
}
/// Note what the running gate has changed so far, as it is now. A path changed again since the last look gets
/// its new hash; one the gate has put back stays listed, but unchanged it is in no stop's list.
pub(super) async fn note_gate_files(tree: &Path, ledger: &Path) -> Result<()> {
    let Some(mut files) = read_ledger(ledger)? else { return Ok(()) };
    let changes = changed(tree).await?;
    for change in changes.unstaged.iter().chain(&changes.untracked).filter(|c| !c.submodule && !files.before.contains(&c.path)) {
        files.files.insert(change.path.clone(), content_hash(&tree.join(&change.path))?);
    }
    write_ledger(ledger, &files)
}
/// The gate is over, however it ended: what is in the tree now is no longer a stopped gate's leftover. Also
/// takes away a stop once the tree is clean.
pub(super) fn end_gate(ledger: &Path) -> Result<()> {
    match std::fs::remove_file(ledger) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

/// Why a Run gate stopped a run in its named tree, kept for the app beside the run's state (ov-979).
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct TreeStop { pub run: String, pub text: String }

/// Why the run stopped, with the changes it found, as the person will read it. `still` are the ones the stopped
/// gate's ledger noted and that are exactly as noted: changed while the gate ran, by it or by a person, since a
/// look can't tell which. The copy says only that, so it never invites a person to discard their own work.
fn kept(paths: &[&str], still: &[&str]) -> Error {
    const SHOWN: usize = 10;
    let list = |paths: &[&str]| {
        let mut list = paths.iter().take(SHOWN).copied().collect::<Vec<_>>().join(", ");
        if paths.len() > SHOWN { list.push_str(&format!(" and {} more", paths.len() - SHOWN)); }
        list
    };
    let still = if still.is_empty() { String::new() }
        else { format!(" Changed while its gate was running, and not since: {}.", list(still)) };
    Error::Kept(format!("The run stopped because its worktree has changes, and it left them as they are: {}.{still} \
        Keep them by copying them somewhere else, or discard them, then retry the run.", list(paths)))
}

/// A named tree is visible, so people may have changes in it, and a Run gate restarting there must never wipe
/// them (ov-967). After a crash the tree may also hold the dead gate's leftovers, but no look from outside can
/// tell who wrote a file, so nothing is removed or put back on the run's own say (ov-979). Any change stops the
/// run with the list of what changed; the files the gate's ledger noted, still exactly as noted, are named as
/// changed while the gate ran, to help the person choose. A clean tree drops the ledger and lets the gate run.
pub(super) async fn stop_on_changes(tree: &Path, ledger: &Path, run: &str, candidate: Option<&str>) -> Result<()> {
    let changes = changed(tree).await?;
    if !changes.is_dirty() { return end_gate(ledger); }
    // Only this run and candidate's ledger labels anything; the ledger stays for the next try.
    let files = read_ledger(ledger)?.filter(|f| f.run == run && f.candidate.as_deref() == candidate).unwrap_or_default();
    let mut paths: Vec<&str> = changes.staged.iter().chain(&changes.unstaged).chain(&changes.untracked)
        .map(|c| c.path.as_str()).chain(changes.conflicted.iter().map(String::as_str)).collect();
    paths.sort_unstable();
    paths.dedup();
    let still: Vec<&str> = changes.unstaged.iter().chain(&changes.untracked)
        .filter(|c| !c.submodule && !changes.staged.iter().any(|s| s.path == c.path)
            && content_hash(&tree.join(&c.path)).is_ok_and(|now| files.files.get(&c.path) == Some(&now)))
        .map(|c| c.path.as_str()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    Err(kept(&paths, &still))
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

/// An adopted tree must be clean (ov-968), so the run is its one writer. Changes in it are a person's, so the run
/// refuses and leaves them as they are; nothing here resets or cleans (ov-979).
pub(super) async fn refuse_adopting_changes(tree: &Path) -> Result<()> {
    if crate::git::is_dirty(tree).await.unwrap_or(true) {
        return Err(Error::Refused("the adopted worktree has changes; commit them to its branch or discard them, then start the run again"));
    }
    Ok(())
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
            refuse_adopting_changes(path).await?;
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

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for args in [&["init", "-q"][..], &["commit", "-q", "--allow-empty", "-m", "base"]] { git(dir.path(), args); }
        std::fs::write(dir.path().join("tracked.txt"), "one").unwrap();
        git(dir.path(), &["add", "tracked.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "tracked"]);
        dir
    }
    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("/usr/bin/git").current_dir(dir)
            .args(["-c", "user.name=T", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false"]).args(args)
            .status().unwrap().success();
        assert!(ok, "{args:?}");
    }
    #[tokio::test]
    async fn an_adopted_tree_with_changes_is_refused_and_left_as_it_is() {
        let dir = repo();
        refuse_adopting_changes(dir.path()).await.unwrap();
        std::fs::write(dir.path().join("mine.txt"), "someone's work").unwrap();
        assert!(refuse_adopting_changes(dir.path()).await.is_err());
        assert_eq!(read(&dir, "mine.txt"), "someone's work");
    }
    fn ledger(dir: &tempfile::TempDir) -> PathBuf { dir.path().join(".git").join("gate-files.json") }
    async fn gate(dir: &tempfile::TempDir) -> PathBuf {
        let ledger = ledger(dir);
        begin_gate(dir.path(), &ledger, "run", Some("c")).await.unwrap();
        ledger
    }
    async fn restart(dir: &tempfile::TempDir) -> Result<()> { stop_on_changes(dir.path(), &ledger(dir), "run", Some("c")).await }
    fn read(dir: &tempfile::TempDir, path: &str) -> String { std::fs::read_to_string(dir.path().join(path)).unwrap() }
    /// What a stop says: every change it found, and the ones it names as changed while the gate ran.
    async fn stopped(dir: &tempfile::TempDir) -> (String, String) {
        let error = restart(dir).await.unwrap_err().to_string();
        assert!(error.contains("Keep them") && error.contains("discard them") && !error.contains("commit"), "{error}");
        let (changed, rest) = error.split_once("left them as they are: ").and_then(|(_, r)| r.split_once(". ")).expect(&error);
        let still = rest.strip_prefix("Changed while its gate was running, and not since: ").and_then(|r| r.split_once(". ")).map_or("", |l| l.0);
        (changed.into(), still.into())
    }

    #[tokio::test]
    async fn a_clean_tree_passes_and_a_person_s_changes_are_left_alone() {
        let dir = repo();
        restart(&dir).await.unwrap();
        // Their work is there before the gate began, so the gate's ledger can't list it.
        std::fs::write(dir.path().join("mine.txt"), "someone's work").unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "someone's edit").unwrap();
        let ledger = gate(&dir).await;
        std::fs::write(dir.path().join("half-built.txt"), "the gate's").unwrap();
        note_gate_files(dir.path(), &ledger).await.unwrap();
        assert_eq!(stopped(&dir).await, ("half-built.txt, mine.txt, tracked.txt".into(), "half-built.txt".into()));
        assert_eq!(read(&dir, "mine.txt"), "someone's work");
        assert_eq!(read(&dir, "tracked.txt"), "someone's edit");
        assert_eq!(read(&dir, "half-built.txt"), "the gate's");
        assert!(ledger.exists(), "the ledger stays to mark the same files on the next try");
    }

    /// Even what the gate's ledger lists is only named: a look from outside can't tell who wrote it. Once the
    /// person has discarded it, the run goes on.
    #[tokio::test]
    async fn a_dead_gate_s_leftovers_are_marked_and_left_for_the_person() {
        let dir = repo();
        std::fs::write(dir.path().join("gone.txt"), "kept by git").unwrap();
        git(dir.path(), &["add", "gone.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "gone"]);
        std::fs::create_dir(dir.path().join("out")).unwrap();
        let ledger = gate(&dir).await;
        std::fs::write(dir.path().join("out/half-built.txt"), "the gate's").unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "the gate rewrote this").unwrap();
        std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
        note_gate_files(dir.path(), &ledger).await.unwrap();
        // The daemon dies here; a restart stops and asks.
        let all = "gone.txt, out/half-built.txt, tracked.txt";
        assert_eq!(stopped(&dir).await, (all.into(), all.into()));
        assert_eq!(read(&dir, "out/half-built.txt"), "the gate's");
        assert_eq!(read(&dir, "tracked.txt"), "the gate rewrote this");
        assert!(!dir.path().join("gone.txt").exists());
        // The person discards them.
        git(dir.path(), &["checkout", "--", "tracked.txt", "gone.txt"]);
        std::fs::remove_file(dir.path().join("out/half-built.txt")).unwrap();
        restart(&dir).await.unwrap();
        assert!(!ledger.exists());
    }

    /// The blocker in the first ov-979 review: the daemon dies mid-gate and a person keeps working in the tree.
    #[tokio::test]
    async fn a_person_s_edit_after_the_gate_began_survives_a_crash_restart() {
        let dir = repo();
        let ledger = gate(&dir).await;
        std::fs::write(dir.path().join("out.txt"), "the gate's").unwrap();
        std::fs::write(dir.path().join("log.txt"), "the gate's").unwrap();
        note_gate_files(dir.path(), &ledger).await.unwrap();
        // The daemon is dead: nothing more is noted, and a person works on.
        std::fs::write(dir.path().join("mine.txt"), "someone's work").unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "someone's edit").unwrap();
        std::fs::write(dir.path().join("out.txt"), "someone's fix to the gate's file").unwrap();
        assert_eq!(stopped(&dir).await, ("log.txt, mine.txt, out.txt, tracked.txt".into(), "log.txt".into()));
        assert_eq!(read(&dir, "mine.txt"), "someone's work");
        assert_eq!(read(&dir, "tracked.txt"), "someone's edit");
        assert_eq!(read(&dir, "out.txt"), "someone's fix to the gate's file");
        assert_eq!(read(&dir, "log.txt"), "the gate's");
    }

    /// The blocker in the ov-979 re-review: a person edits while the gate runs and the daemon is alive, so the
    /// ledger notes their edit as if the gate made it. A crash restart must still keep it.
    #[tokio::test]
    async fn a_person_s_edit_during_the_gate_survives_a_crash_restart() {
        let dir = repo();
        let ledger = gate(&dir).await;
        std::fs::write(dir.path().join("out.txt"), "the gate's").unwrap();
        std::fs::write(dir.path().join("mine.txt"), "someone's work").unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "someone's edit").unwrap();
        note_gate_files(dir.path(), &ledger).await.unwrap();
        // The daemon dies here; a restart finds the tree changed.
        assert!(restart(&dir).await.is_err(), "the run stops and asks");
        assert_eq!(read(&dir, "mine.txt"), "someone's work");
        assert_eq!(read(&dir, "tracked.txt"), "someone's edit");
        assert_eq!(read(&dir, "out.txt"), "the gate's", "not even the gate's own file goes on its own");
    }

    #[tokio::test]
    async fn a_ledger_from_another_run_or_candidate_marks_nothing() {
        for (run, candidate) in [("another run", Some("c")), ("run", Some("an older candidate")), ("run", None)] {
            let dir = repo();
            let ledger = gate(&dir).await;
            std::fs::write(dir.path().join("out.txt"), "the gate's").unwrap();
            note_gate_files(dir.path(), &ledger).await.unwrap();
            let error = stop_on_changes(dir.path(), &ledger, run, candidate).await.unwrap_err().to_string();
            assert!(error.contains("out.txt") && !error.contains("while its gate"), "{run} {candidate:?}: {error}");
            assert!(dir.path().join("out.txt").exists());
        }
    }

    #[tokio::test]
    async fn a_gate_that_ends_takes_its_ledger_with_it() {
        let dir = repo();
        let ledger = gate(&dir).await;
        std::fs::write(dir.path().join("out.txt"), "the gate's").unwrap();
        note_gate_files(dir.path(), &ledger).await.unwrap();
        end_gate(&ledger).unwrap();
        end_gate(&ledger).unwrap();
        assert_eq!(stopped(&dir).await, ("out.txt".into(), "".into()), "what a finished gate left is no longer marked");
        assert!(dir.path().join("out.txt").exists());
    }

    #[tokio::test]
    async fn changes_with_no_ledger_are_a_person_s() {
        let dir = repo();
        std::fs::write(dir.path().join("mine.txt"), "someone's work").unwrap();
        assert_eq!(stopped(&dir).await, ("mine.txt".into(), "".into()));
        assert_eq!(read(&dir, "mine.txt"), "someone's work");
    }
}
