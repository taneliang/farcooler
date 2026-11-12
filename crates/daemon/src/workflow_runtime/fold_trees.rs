//! A group's fold tree goes when the group ends, and its build output waits for the next fold (ov-1084).
//!
//! Each group folds into a private tree, `tree-wf1-<hex>`, and its gate builds a full `target/` there: 16 to 35 GB
//! for this repository's own gate. A group that finished or failed kept all of it, and the disk fell to 16 GB free.
//! Now:
//! - when a group ends (done, failed or cancelled), its tree and its branch go, at the next sweep;
//! - a tree under the name from before ov-1081, `tree-wf1:<hex>`, goes unless a live run still works in it;
//! - the tree's `target/` doesn't go with it: it waits, one per repository, and the next fold starts with it, so a
//!   gate rebuilds what changed instead of every dependency.
//!
//! A shared `CARGO_TARGET_DIR` was not used: gate scripts read `target/` by path, and two checkouts building into one
//! folder at once let one stand in for the other (scripts/heavy_lock.py). Here the folder moves with the folds, one
//! tree at a time, and a fresh checkout's files are all newer than what was built, so cargo rebuilds every crate of
//! the workspace and keeps the dependencies.
use super::*;

/// What a sweep does with a private tree it finds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate { Keep, Discard }

impl Exec {
    /// Where a repository's build output waits between one group's fold tree and the next.
    fn parked_build(&self, repository: &Path) -> PathBuf {
        let repository = repository.canonicalize().unwrap_or_else(|_| repository.to_path_buf());
        self.root.join(format!("build-{}", adapters::folder_name(&digest(&("parked-build/v1", repository)))))
    }
    /// Move `path` out of the way at once and delete it in the background: a 35 GB `target/` takes minutes to delete.
    fn discard(&self, path: &Path) -> std::io::Result<()> {
        let trash = self.root.join(format!("trash-{}", Uuid::now_v7()));
        std::fs::rename(path, &trash)?;
        tokio::task::spawn_blocking(move || {
            if let Err(error) = std::fs::remove_dir_all(&trash) { tracing::warn!(path = %trash.display(), %error, "could not delete a discarded tree"); }
        });
        Ok(())
    }
    /// Keep a tree's `target/` for the next fold in `repository`. The newest wins: an older one waiting is deleted.
    pub(super) fn park_build(&self, tree: &Path, repository: &Path) {
        let target = tree.join("target");
        // A link points at somebody else's folder: never move or delete what it points to.
        if !target.symlink_metadata().is_ok_and(|m| m.file_type().is_dir()) { return; }
        let parked = self.parked_build(repository);
        if parked.symlink_metadata().is_ok() {
            if let Err(error) = self.discard(&parked) { tracing::warn!(%error, "could not replace parked build output"); return; }
        }
        if let Err(error) = std::fs::rename(&target, &parked) { tracing::warn!(%error, "could not keep a fold tree's build output"); }
    }
    /// Give a fresh fold tree the build output the last fold in its repository left, where git ignores `target/`, so
    /// it never shows as a change in the tree.
    pub(super) async fn unpark_build(&self, binding: &Binding, tree: &Path, repository: &Path) -> Result<()> {
        let parked = self.parked_build(repository);
        if !parked.symlink_metadata().is_ok_and(|m| m.file_type().is_dir()) || tree.join("target").symlink_metadata().is_ok() {
            return Ok(());
        }
        if !self.try_git(binding, tree, &["check-ignore", "--quiet", "--no-index", "target/"]).await?.0 { return Ok(()); }
        if let Err(error) = std::fs::rename(&parked, tree.join("target")) { tracing::warn!(%error, "could not reuse parked build output"); }
        Ok(())
    }
    /// Delete the private trees nothing will run in again: an ended group's, and an old-name one no live run has.
    /// Errors are logged, never raised: a tree left behind costs disk, not correctness. `recovering` also deletes what
    /// an earlier daemon was deleting when it stopped.
    pub(super) async fn prune_trees(&self, workspaces: &[String], recovering: bool) {
        if let Err(error) = self.prune_trees_or_fail(workspaces, recovering).await {
            tracing::warn!(%error, "could not prune ended fold trees");
        }
    }
    async fn prune_trees_or_fail(&self, workspaces: &[String], recovering: bool) -> Result<()> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if recovering && name.starts_with("trash-") {
                let path = self.root.join(&name);
                tokio::task::spawn_blocking(move || { let _ = std::fs::remove_dir_all(path); });
            } else if name.starts_with("tree-") { found.push(name); }
        }
        if found.is_empty() { return Ok(()); }
        // Every tree name a run has, by the name it would have: the current one and the one before ov-1081.
        let mut owners: std::collections::BTreeMap<String, (Fate, String, String)> = std::collections::BTreeMap::new();
        for workspace in workspaces {
            let Some(state) = self.state(workspace)? else { continue };
            let ended = |i: &Item| i.group_of.is_some() && matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled);
            let runs = state.items.iter().map(|(id, i)| (id, if ended(i) { Fate::Discard } else { Fate::Keep }))
                .chain(state.archive.iter().filter(|(_, a)| a.group.is_some()).map(|(id, _)| (id, Fate::Discard)));
            for (id, fate) in runs {
                let key = digest(&(workspace, id));
                for name in [format!("tree-{}", adapters::folder_name(&key)), format!("tree-{key}")] {
                    owners.insert(name, (fate, workspace.clone(), id.clone()));
                }
            }
        }
        for name in found {
            let owner = owners.get(&name);
            let legacy = name.contains(':');
            let discard = match owner {
                Some((fate, ..)) => *fate == Fate::Discard,
                // An old-name tree nobody runs in any more; a current-name one the state doesn't know stays.
                None => legacy,
            };
            if !discard { continue; }
            let tree = self.root.join(&name);
            self.remove_tree(&tree, owner.map(|(_, workspace, item)| (workspace.as_str(), item.as_str()))).await;
        }
        Ok(())
    }
    /// Remove one private tree: keep its build output, move it away, and tell its repository it's gone. An ended
    /// group's branch goes too; a run's from before ov-1081 may hold work and stays.
    async fn remove_tree(&self, tree: &Path, group: Option<(&str, &str)>) {
        let common = crate::git::git(tree, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await.ok()
            .filter(|o| o.ok).map(|o| PathBuf::from(o.stdout.trim()));
        let repository = common.as_ref().and_then(|c| c.parent()).map(Path::to_path_buf);
        if let Some(repository) = &repository { self.park_build(tree, repository); }
        if let Err(error) = self.discard(tree) {
            tracing::warn!(tree = %tree.display(), %error, "could not remove an ended fold tree");
            return;
        }
        tracing::info!(tree = %tree.display(), "removed a fold tree nothing will run in again");
        let Some(repository) = repository else { return };
        let _ = crate::git::git(&repository, &["worktree", "prune"]).await;
        if let Some((workspace, item)) = group {
            let branch = format!("wf-{}", &digest(&(workspace, item))[4..]);
            let _ = crate::git::git(&repository, &["branch", "-D", &branch]).await;
        }
    }
}
