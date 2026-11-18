//! Finished groups release their private fold/gate checkout; Git references and receipts remain.
use super::*;

impl Exec {
    /// Kept at every former tree-pruning call site, including recovery and cancellation.
    /// Durable cleanup waits for running effects and retains commits before removing a checkout.
    pub(super) async fn prune_trees(&self, workspaces: &[String], _recovering: bool) {
        if let Err(error) = self.prune_process_logs(workspaces) {
            tracing::warn!(%error, "could not prune orphan process logs");
        }
        if let Err(error) = self.release_run_targets().await {
            tracing::warn!(%error, "could not prune ended builders’ output");
        }
        if let Err(error) = self.release_builds(workspaces).await {
            tracing::warn!(%error, "could not release build output");
        }
    }
}

impl Exec {
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
            self.store.workflow_cleanup_enqueue(&farcooler_workflow::cleanup::CleanupEffect {
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
        held: Option<std::sync::Arc<tokio::sync::OwnedMutexGuard<()>>>,
        effect: &farcooler_workflow::cleanup::CleanupEffect) -> std::result::Result<u64, String> {
        if tree != self.item_tree(&effect.workspace, &effect.item) || tree.parent() != Some(self.root.as_path()) {
            return Err("group tree is outside the engine root".into());
        }
        let mut command = tokio::process::Command::new("lsof");
        command.args(["-nP", "-F", "p", "+D"]).arg(tree).kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(30), command.output()).await
            .map_err(|_| "group open-file check timed out".to_string())?.map_err(|e| e.to_string())?;
        if !output.stdout.is_empty() { return Err("open files under group tree".into()); }
        if output.status.code() != Some(1) || !output.stderr.is_empty() {
            return Err("group open-file check could not establish safety".into());
        }
        let common = crate::git::git(tree, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await.map_err(|e| e.to_string())?;
        if !common.ok { return Err("group git directory unavailable".into()); }
        // A failed fold can have a detached HEAD. Keep it reachable even after its worktree
        // metadata goes; normal group branches also remain available for inspection.
        let reference = format!("refs/workflow/finished/{}", adapters::folder_name(&digest(&(&effect.workspace, &effect.item))));
        let anchor = crate::git::git(tree, &["update-ref", &reference, "HEAD"]).await.map_err(|e| e.to_string())?;
        if !anchor.ok { return Err("could not retain group HEAD".into()); }
        let path = tree.to_path_buf();
        let bytes = tokio::task::spawn_blocking(move || {
            // Deletion can outlive cancellation of this future; retain the claim fence too.
            let _held = held;
            let device = path.symlink_metadata().map_err(|e| e.to_string())?.dev();
            let bytes = group_bytes(&path, device, true).map_err(|e| e.to_string())?;
            std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
            Ok::<_, String>(bytes)
        }).await.map_err(|e| e.to_string())??;
        let _ = crate::git::git(&self.root, &["--git-dir", common.stdout.trim(), "worktree", "prune"]).await;
        Ok(bytes)
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
