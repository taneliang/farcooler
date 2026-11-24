//! Finished groups release their private fold/gate checkout; Git references and receipts remain.
use super::*;

impl Exec {
    /// Recover cleanup intents and maintain retained process evidence.
    pub(super) fn prune_trees(&self, workspaces: &[String], recovering: bool) {
        if recovering {
            for workspace in workspaces {
                if let Err(error) = self.recover_disk_cleanups(workspace) {
                    tracing::warn!(%error, "could not retain recovery cleanup intents");
                }
            }
        }
        if let Err(error) = self.prune_process_logs(workspaces) {
            tracing::warn!(%error, "could not prune orphan process logs");
        }
    }
}

impl Exec {
    fn recover_disk_cleanups(&self, workspace: &str) -> Result<()> {
        let records = self.store.workflow_cleanups(workspace)?;
        for record in &records {
            if !record.result.as_ref().is_some_and(|result| result.skipped.iter()
                .any(|note| note.starts_with("cleanup attempt interrupted"))) { continue; }
            if record.effect.directory.is_some() {
                self.store.workflow_cleanup_recover(&record.effect, owned_directories::DIRECTORY_INTERRUPTED)?;
            } else if record.effect.tree.is_none() && !record.effect.uncertain && self.ended_group(&record.effect)? {
                self.store.workflow_cleanup_recover(&record.effect, build_cleanup::GROUP_INTERRUPTED)?;
            }
        }
        self.retain_finished_group_cleanups(workspace, &records)?;
        self.retain_idle_run_cleanups(workspace, &records)?;
        Ok(())
    }

    /// Older engines settled output cleanup while leaving the source checkout behind. Retain
    /// a separate intent for those known groups, without sweeping unknown directory names.
    fn retain_finished_group_cleanups(&self, workspace: &str, records: &[farcooler_store::workflow_cleanup::CleanupRecord]) -> Result<()> {
        let Some(state) = self.state(workspace)? else { return Ok(()) };
        let ended = state.items.iter().filter(|(_, i)| i.group_of.is_some() && i.tree.is_none()
            && matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled))
            .map(|(id, i)| (id, i.run.clone()))
            .chain(state.archive.iter().filter(|(_, a)| a.group.is_some()).map(|(id, _)| (id, String::new())));
        for (id, run) in ended {
            if !self.item_tree(workspace, id).try_exists()? || records.iter().any(|r| r.effect.directory.is_none() && r.effect.item == *id
                && r.result.as_ref().is_none_or(|r| r.retry
                    || r.skipped.iter().any(|s| s.starts_with("cleanup attempt interrupted")))) { continue; }
            self.store.workflow_cleanup_enqueue(&farcooler_workflow::cleanup::CleanupEffect { restart: None,
                id: digest(&("group-tree-release/v1", workspace, id)), workspace: workspace.into(),
                revision: state.revision, item: id.clone(), run, tree: None, folders: Vec::new(), uncertain: false, directory: None,
            })?;
        }
        Ok(())
    }

    fn retain_idle_run_cleanups(&self, workspace: &str, records: &[farcooler_store::workflow_cleanup::CleanupRecord]) -> Result<()> {
        use farcooler_workflow::cleanup::{CleanupEffect, waiting_at_rest};
        let Some(state) = self.state(workspace)? else { return Ok(()) };
        for (id, item) in &state.items {
            let ended_owned_run = matches!(item.phase, Phase::Failed | Phase::Cancelled)
                && !item.tree.as_ref().is_some_and(|tree| tree.adopted);
            if item.group_of.is_some() || item.definition.is_ticket() || item.follows.is_some()
                || !(waiting_at_rest(&item.phase) || ended_owned_run) {
                continue;
            }
            if records.iter().any(|r| r.effect.directory.is_none() && r.effect.item == *id
                && r.effect.run == item.run && r.result.as_ref().is_none_or(|result| result.retry)) {
                continue;
            }
            let tree = item.tree.as_ref().map_or_else(|| self.item_tree_for(workspace, id, item.restart.as_deref()),
                |tree| PathBuf::from(&tree.path));
            let folders = item.definition.build_folders();
            let adopted = item.tree.as_ref().is_some_and(|tree| tree.adopted);
            if !build_cleanup::build_folders(&folders, adopted).any(|folder| build_cleanup::folder_paths(&tree, folder)
                .any(|path| matches!(path, Ok(Some(_)))))
                && !(build_cleanup::default_build_folders(&folders) && build_cleanup::cargo_cache_folders(&tree).next().is_some()) { continue; }
            // The expanded artifact inventory can release output a previous
            // engine left after settling its recovery attempt for this run.
            let policy = if adopted { "idle-adopted-build-release/v3" } else { "idle-build-release/v1" };
            self.store.workflow_cleanup_enqueue(&CleanupEffect {
                id: digest(&(policy, workspace, id, &item.run, item.fence)),
                workspace: workspace.into(), revision: state.revision, item: id.clone(), run: item.run.clone(),
                restart: item.restart.clone(), tree: item.tree.clone(), folders, uncertain: false, directory: None,
            })?;
        }
        Ok(())
    }

    pub(super) fn finished_landing_group(&self, effect: &farcooler_workflow::cleanup::CleanupEffect) -> Result<bool> {
        let Some(state) = self.state(&effect.workspace)? else { return Ok(false) };
        Ok(state.items.get(&effect.item).is_some_and(|i| i.group_of.is_some() && matches!(i.phase, Phase::Done | Phase::Failed))
            || state.archive.get(&effect.item).is_some_and(|a| a.group.is_some() && matches!(a.phase, Phase::Done | Phase::Failed)))
    }

    pub(super) fn ended_group(&self, effect: &farcooler_workflow::cleanup::CleanupEffect) -> Result<bool> {
        let Some(state) = self.state(&effect.workspace)? else { return Ok(false) };
        Ok(state.items.get(&effect.item).is_some_and(|i| i.group_of.is_some()
            && matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled))
            || state.archive.get(&effect.item).is_some_and(|a| a.group.is_some()))
    }

    /// Called only after the cleanup's ownership, quarantine, active-effect and claim-fence checks.
    /// Groups own disposable engine paths, never adopted or named worktrees. The same checkout
    /// is used for folding and gates; no output is parked for the next group.
    pub(super) async fn release_group_tree(&self, tree: &Path,
        held: std::sync::Arc<build_cleanup::CleanupFences>,
        effect: &farcooler_workflow::cleanup::CleanupEffect, reconcile: bool) -> std::result::Result<u64, String> {
        if tree != self.item_tree(&effect.workspace, &effect.item) || tree.parent() != Some(self.root.as_path()) {
            return Err("group tree is outside the engine root".into());
        }
        let reference = format!("refs/workflow/finished/{}", adapters::folder_name(&digest(&(&effect.workspace, &effect.item))));
        let (common, gitdir) = self.group_recovery_gitdir(tree, effect).await?;
        // Git's registration survives partial checkout removal, including loss of .git.
        // Anchor its detached HEAD before pruning that registration, even if the tree is gone.
        if let Some(gitdir) = gitdir {
            if !reconcile {
                let actual = crate::git::git(tree, &["rev-parse", "--absolute-git-dir"]).await.map_err(|e| e.to_string())?;
                if !actual.ok || Path::new(actual.stdout.trim()) != Path::new(&gitdir) {
                    return Err("group worktree registration has changed".into());
                }
            }
            let anchor = crate::git::git(&self.root, &["--git-dir", &gitdir, "update-ref", &reference, "HEAD"]).await.map_err(|e| e.to_string())?;
            if !anchor.ok { return Err("could not retain interrupted group HEAD".into()); }
        }
        if !tree.try_exists().map_err(|e| e.to_string())? {
            let _ = crate::git::git(&self.root, &["--git-dir", &common, "worktree", "prune"]).await;
            return Ok(0);
        }
        build_cleanup::check_open_files(tree).await?;
        let path = tree.to_path_buf();
        let deletion_fences = held.clone();
        let bytes = tokio::task::spawn_blocking(move || {
            // Deletion can outlive cancellation of this future; retain the claim fence too.
            let _held = deletion_fences;
            if path.canonicalize().map_err(|e| e.to_string())? != path {
                return Err("group worktree path has changed".into());
            }
            let device = path.symlink_metadata().map_err(|e| e.to_string())?.dev();
            let bytes = group_bytes(&path, device, true).map_err(|e| e.to_string())?;
            // An I/O error can leave a partially removed tree, just like cancellation.
            // The next attempt must use its Git registration and cannot reconstruct bytes.
            std::fs::remove_dir_all(&path).map_err(|e| format!("{}: {e}", build_cleanup::GROUP_INTERRUPTED))?;
            Ok::<_, String>(bytes)
        }).await.map_err(|e| e.to_string())??;
        let _ = crate::git::git(&self.root, &["--git-dir", &common, "worktree", "prune"]).await;
        Ok(bytes)
    }

    /// Recover only a registered engine checkout in the workspace's repository. Never guess
    /// ownership from a directory name, or use a partly deleted checkout to find its repository.
    async fn group_recovery_gitdir(&self, tree: &Path, effect: &farcooler_workflow::cleanup::CleanupEffect) -> std::result::Result<(String, Option<String>), String> {
        let common = if self.service_mode {
            let board = Uuid::parse_str(&effect.workspace).map_err(|e| e.to_string())?;
            let workspace = self.store.get_workspace(board).map_err(|e| e.to_string())?;
            self.store.get_repository(workspace.repository_id).map_err(|e| e.to_string())?.canonical_git_dir
        } else {
            let common = crate::git::git(&self.repository(), &["rev-parse", "--path-format=absolute", "--git-common-dir"])
                .await.map_err(|e| e.to_string())?;
            if !common.ok { return Err("group repository unavailable".into()); }
            common.stdout.trim().to_string()
        };
        let registrations = Path::new(&common).join("worktrees");
        if registrations.try_exists().map_err(|e| e.to_string())? {
            if registrations.canonicalize().map_err(|e| e.to_string())? != registrations {
                return Err("aliased group Git registrations".into());
            }
            for entry in std::fs::read_dir(&registrations).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if !entry.file_type().map_err(|e| e.to_string())?.is_dir() { continue; }
                let link = entry.path().join("gitdir");
                if !link.symlink_metadata().is_ok_and(|metadata| metadata.is_file()) { continue; }
                let registered = std::fs::read_to_string(link).map_err(|e| e.to_string())?;
                if Path::new(registered.trim_end()) == tree.join(".git") {
                    return Ok((common, Some(entry.path().to_string_lossy().into_owned())));
                }
            }
        }
        // The prior attempt may have removed and pruned the tree before saving its result,
        // or the group may have ended before provisioning. There is nothing left to remove.
        if !tree.try_exists().map_err(|e| e.to_string())? { return Ok((common, None)); }
        Err("group has no matching Git registration".into())
    }

}

/// Do not traverse links, filesystem boundaries or nested repositories. Group reports are
/// disposable checkout files; durable gate logs and receipts live outside this tree.
fn group_bytes(path: &Path, device: u64, root: bool) -> std::io::Result<u64> {
    let meta = path.symlink_metadata()?;
    if meta.dev() != device { return Err(std::io::Error::other("group tree crosses a filesystem boundary")); }
    if !root && path.file_name().is_some_and(|n| n.eq_ignore_ascii_case(".git")) && meta.is_dir() {
        return Err(std::io::Error::other("group contains a nested repository"));
    }
    if !meta.is_dir() { return Ok(meta.len()); }
    if !root && path.join(".git").symlink_metadata().is_ok() {
        return Err(std::io::Error::other("group contains a nested worktree"));
    }
    if path.join("HEAD").symlink_metadata().is_ok() && path.join("objects").symlink_metadata().is_ok() {
        return Err(std::io::Error::other("group contains a bare repository"));
    }
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path)? { bytes = bytes.saturating_add(group_bytes(&entry?.path(), device, false)?); }
    Ok(bytes)
}
