//! Finished groups release their private fold/gate checkout; Git references and receipts remain.
use super::*;

impl Exec {
    /// Recover cleanup intents and maintain process logs and legacy targets.
    pub(super) async fn prune_trees(&self, workspaces: &[String], recovering: bool) {
        if recovering {
            for workspace in workspaces {
                if let Err(error) = self.recover_group_cleanups(workspace) {
                    tracing::warn!(%error, "could not reconcile interrupted group cleanup");
                }
            }
        }
        if let Err(error) = self.prune_process_logs(workspaces) {
            tracing::warn!(%error, "could not prune orphan process logs");
        }
        if let Err(error) = self.release_run_targets().await {
            tracing::warn!(%error, "could not prune ended builders’ output");
        }
    }
}

impl Exec {
    fn recover_group_cleanups(&self, workspace: &str) -> Result<()> {
        for record in self.store.workflow_cleanups(workspace)? {
            if record.effect.tree.is_none() && !record.effect.uncertain && self.ended_group(&record.effect)? {
                self.store.workflow_cleanup_recover_group(&record.effect)?;
            }
        }
        Ok(())
    }

    /// Older engines settled output cleanup while leaving the source checkout behind. Retain
    /// a separate intent for those known groups, without sweeping unknown directory names.
    pub(super) fn retain_finished_group_cleanups(&self, workspace: &str) -> Result<()> {
        let Some(state) = self.state(workspace)? else { return Ok(()) };
        let records = self.store.workflow_cleanups(workspace)?;
        let ended = state.items.iter().filter(|(_, i)| i.group_of.is_some() && i.tree.is_none()
            && matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled))
            .map(|(id, i)| (id, i.run.clone()))
            .chain(state.archive.iter().filter(|(_, a)| a.group.is_some()).map(|(id, _)| (id, String::new())));
        for (id, run) in ended {
            if !self.item_tree(workspace, id).try_exists()? || records.iter().any(|r| r.effect.item == *id
                && r.result.as_ref().is_none_or(|r| r.retry
                    || r.skipped.iter().any(|s| s.starts_with("cleanup attempt interrupted")))) { continue; }
            self.store.workflow_cleanup_enqueue(&farcooler_workflow::cleanup::CleanupEffect { restart: None,
                id: digest(&("group-tree-release/v1", workspace, id)), workspace: workspace.into(),
                revision: state.revision, item: id.clone(), run, tree: None, folders: Vec::new(), uncertain: false,
            })?;
        }
        Ok(())
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
        let (common, gitdir) = if reconcile {
            self.group_recovery_gitdir(tree, effect).await?
        } else {
            let common = crate::git::git(tree, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await.map_err(|e| e.to_string())?;
            if !common.ok { return Err("group git directory unavailable".into()); }
            (common.stdout.trim().to_string(), None)
        };
        // Git's registration survives partial checkout removal, including loss of .git.
        // Anchor its detached HEAD before pruning that registration, even if the tree is gone.
        if let Some(gitdir) = gitdir {
            let anchor = crate::git::git(&self.root, &["--git-dir", &gitdir, "update-ref", &reference, "HEAD"]).await.map_err(|e| e.to_string())?;
            if !anchor.ok { return Err("could not retain interrupted group HEAD".into()); }
        } else if tree.try_exists().map_err(|e| e.to_string())? {
            let anchor = crate::git::git(tree, &["update-ref", &reference, "HEAD"]).await.map_err(|e| e.to_string())?;
            if !anchor.ok { return Err("could not retain group HEAD".into()); }
        }
        if !tree.try_exists().map_err(|e| e.to_string())? {
            let _ = crate::git::git(&self.root, &["--git-dir", &common, "worktree", "prune"]).await;
            return Ok(0);
        }
        let mut command = tokio::process::Command::new("lsof");
        command.args(["-nP", "-F", "p", "+D"]).arg(tree).kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(30), command.output()).await
            .map_err(|_| "group open-file check timed out".to_string())?.map_err(|e| e.to_string())?;
        if !output.stdout.is_empty() { return Err("open files under group tree".into()); }
        if output.status.code() != Some(1) || !output.stderr.is_empty() {
            return Err("group open-file check could not establish safety".into());
        }
        let path = tree.to_path_buf();
        let deletion_fences = held.clone();
        let bytes = tokio::task::spawn_blocking(move || {
            // Deletion can outlive cancellation of this future; retain the claim fence too.
            let _held = deletion_fences;
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
            for entry in std::fs::read_dir(&registrations).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if !entry.file_type().map_err(|e| e.to_string())?.is_dir() { continue; }
                let link = entry.path().join("gitdir");
                if !link.symlink_metadata().map_err(|e| e.to_string())?.is_file() { continue; }
                let registered = std::fs::read_to_string(link).map_err(|e| e.to_string())?;
                if Path::new(registered.trim_end()) == tree.join(".git") {
                    return Ok((common, Some(entry.path().to_string_lossy().into_owned())));
                }
            }
        }
        // The prior attempt may have removed and pruned the tree before saving its result,
        // or the group may have ended before provisioning. There is nothing left to remove.
        if !tree.try_exists().map_err(|e| e.to_string())? { return Ok((common, None)); }
        Err("interrupted group has no matching Git registration".into())
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
    if path.join("HEAD").try_exists()? && path.join("objects").try_exists()? {
        return Err(std::io::Error::other("group contains a bare repository"));
    }
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path)? { bytes = bytes.saturating_add(group_bytes(&entry?.path(), device, false)?); }
    Ok(bytes)
}
