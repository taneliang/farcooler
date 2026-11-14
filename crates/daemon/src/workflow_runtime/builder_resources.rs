//! Builders yield to landing, and a run's build output does not outlive it (ov-1214).
//! Builders yield to a running landing gate: they are paused while one runs (see `yielding.rs`) and start niced
//! (see `bound_command`), which their compilers and test runners inherit. A build is refused below 25 GB free. A run's own `target/` is deleted
//! when the run reaches WaitingJoin, Failed or Cancelled. Nothing here ever touches a tree a person works in.
use super::*;

const GIB: u64 = 1024 * 1024 * 1024;
pub(super) const MIN_FREE: u64 = 25 * GIB;
pub(super) const LOW_DISK: &str = "Build didn't start: less than 25 GB of disk space is free. Free some space and retry.";

fn free_bytes(path: &Path) -> std::io::Result<u64> {
    let stat = rustix::fs::statvfs(path)?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

impl Exec {
    /// Whether both the engine's volume and the build's worktree have room for a build.
    pub(super) fn build_has_space(&self, tree: &Path) -> Result<bool> {
        #[cfg(test)]
        if let Some(bytes) = self.build_free_bytes { return Ok(bytes >= MIN_FREE); }
        Ok(free_bytes(&self.root)? >= MIN_FREE && free_bytes(tree)? >= MIN_FREE)
    }

    /// Delete the `target/` of every run that has stopped building: WaitingJoin, Failed or Cancelled. Only the engine's
    /// own item trees are swept, never a named tree, and only a real, untracked directory: a symlink is never followed
    /// and committed files are never removed. A tree any live run still uses is left alone.
    pub(super) async fn release_run_targets(&self) -> Result<()> {
        let mut live = BTreeSet::new();
        let mut ended = Vec::new();
        for workspace in self.store.workflow_workspaces()? {
            let Some(state) = self.state(&workspace)? else { continue };
            for (id, item) in &state.items {
                if item.group_of.is_some() { continue; }
                let tree = item.tree.as_ref().map(|t| PathBuf::from(&t.path)).unwrap_or_else(|| self.item_tree(&workspace, id));
                if matches!(item.phase, Phase::WaitingJoin | Phase::Failed | Phase::Cancelled) {
                    if item.tree.is_none() { ended.push(tree); }
                } else { live.insert(tree); }
            }
        }
        for tree in ended {
            if live.contains(&tree) { continue; }
            let target = tree.join("target");
            if !target.symlink_metadata().is_ok_and(|m| m.is_dir()) { continue; }
            if !crate::git::git(&tree, &["ls-files", "-z", "--", "target"]).await.is_ok_and(|o| o.ok && o.stdout.is_empty()) { continue; }
            self.discard_target(&target).await?;
        }
        Ok(())
    }

    async fn discard_target(&self, target: &Path) -> Result<()> {
        // Renaming releases the path at once; the fold-tree trash recovery deletes what a restart finds.
        let trash = self.root.join(format!("trash-{}", Uuid::now_v7()));
        if let Err(error) = std::fs::rename(target, &trash) {
            if error.raw_os_error() != Some(libc::EXDEV) { return Err(error.into()); }
            let target = target.to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(target)).await
                .map_err(|_| Error::Refused("build output cleanup interrupted"))??;
            return Ok(());
        }
        tokio::task::spawn_blocking(move || {
            if let Err(error) = std::fs::remove_dir_all(&trash) { tracing::warn!(%error, "could not delete build output"); }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests;
