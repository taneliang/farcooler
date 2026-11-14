//! Builders yield to landing through inherited niceness and a runner-owned Cargo cache (ov-1170).
//! Worktree buckets live in one bounded cache, without mixing workspace artifacts from different checkouts.
//! Cargo serializes writers inside a bucket. Leases and retained run state keep LRU eviction away from live agents across daemon recovery.
//! Only recorded engine-owned output is reclaimed; adopted worktrees can contain a person’s build output.
use super::*;
use rustix::fs::{flock, FlockOperation};

const GIB: u64 = 1024 * 1024 * 1024;
const MIN_FREE: u64 = 25 * GIB;
pub(super) const CACHE_CAP: u64 = 20 * GIB;
pub(super) const LOW_DISK: &str = "Build didn't start: less than 25 GB of disk space is free. Free some space and retry.";

fn free_bytes(path: &Path) -> std::io::Result<u64> {
    let stat = rustix::fs::statvfs(path)?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// Never count or traverse symlinks: a build's output is untrusted.
fn disk_bytes(path: &Path) -> std::io::Result<u64> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut bytes = metadata.blocks().saturating_mul(512);
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? { bytes = bytes.saturating_add(disk_bytes(&entry?.path())?); }
    }
    Ok(bytes)
}

fn lease(root: &Path, name: &str) -> std::io::Result<File> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW).open(root.join(format!("{name}.lock")))?;
    if !file.metadata()?.is_file() || file.metadata()?.nlink() != 1 {
        return Err(std::io::Error::other("aliased builder cache lease"));
    }
    Ok(file)
}

/// Evict oldest idle worktree buckets until the cache fits. Active buckets may temporarily exceed the cap;
/// completion trims again after its observation. Retained live state protects surviving agents after a daemon restart.
fn trim(root: &Path, cap: u64, protected: &BTreeSet<PathBuf>) -> std::io::Result<()> {
    let mut buckets = Vec::new();
    let mut total = 0u64;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.len() != 64 || !name.bytes().all(|c| c.is_ascii_hexdigit()) { continue; }
        if !entry.file_type()?.is_dir() { continue; }
        let lock = lease(root, &name)?;
        // Active output counts against the budget, but only idle buckets may be removed.
        let idle = !protected.contains(&entry.path()) && flock(&lock, FlockOperation::NonBlockingLockExclusive).is_ok();
        let bytes = disk_bytes(&entry.path())?;
        total = total.saturating_add(bytes);
        if !idle { continue; }
        buckets.push((lock.metadata()?.modified()?, entry.path(), bytes, lock));
    }
    buckets.sort_by_key(|b| b.0);
    for (_, path, bytes, _lock) in buckets {
        if total <= cap { break; }
        std::fs::remove_dir_all(path)?;
        total = total.saturating_sub(bytes);
    }
    Ok(())
}

/// A build owns its managed bucket and, if it created one, this exact compatibility link.
/// Recording the inode keeps recovery from unlinking a replacement made by the user.
#[derive(Serialize, Deserialize)]
struct TargetOwner { link: Option<(u64, u64)> }

impl Exec {
    fn target_receipt(&self, effect: &Effect, suffix: &str) -> PathBuf {
        self.root.join("receipts").join(format!("{}.target-{suffix}.json", digest(effect)))
    }

    pub(super) fn builder_target(&self, tree: &Path) -> PathBuf {
        // A checkout's Cargo fingerprints must never stand in for another checkout's: this repository has
        // measured stale workspace artifacts with a single undifferentiated CARGO_TARGET_DIR (ov-837).
        self.root.join("builder-targets").join(&digest(&("builder-target/v1", tree))[4..])
    }
    pub(super) fn build_has_space(&self, tree: &Path) -> Result<bool> {
        #[cfg(test)]
        if let Some(bytes) = self.build_free_bytes { return Ok(bytes >= MIN_FREE); }
        Ok(free_bytes(&self.root)? >= MIN_FREE && free_bytes(tree)? >= MIN_FREE)
    }
    pub(super) async fn trim_builder_targets(&self) -> Result<()> {
        let root = self.root.join("builder-targets");
        std::fs::create_dir_all(&root)?;
        if root.canonicalize()? != root { return Err(Error::Refused("aliased builder cache")); }
        // A native PTY can outlive its daemon, and the daemon's file descriptor cannot. Retained live
        // runs are protected even before their ownership has been reconciled on a new open.
        let mut protected = BTreeSet::new();
        for workspace in self.store.workflow_workspaces()? {
            if let Some(state) = self.state(&workspace)? {
                for (id, item) in &state.items {
                    if item.group_of.is_none() && matches!(item.phase, Phase::Running | Phase::Cancelling | Phase::Uncertain) {
                        let tree = item.tree.as_ref().map(|t| PathBuf::from(&t.path)).unwrap_or_else(|| self.item_tree(&workspace, id));
                        protected.insert(self.builder_target(&tree));
                    }
                }
            }
        }
        #[cfg(not(test))]
        let cap = CACHE_CAP;
        #[cfg(test)]
        let cap = self.build_cache_cap;
        tokio::task::spawn_blocking(move || trim(&root, cap, &protected)).await
            .map_err(|_| Error::Refused("builder cache cleanup interrupted"))??;
        Ok(())
    }
    pub(super) async fn prepare_builder_target(&self, tree: &Path, effect: &Effect) -> Result<File> {
        self.trim_builder_targets().await?;
        let target = self.builder_target(tree);
        let root = target.parent().ok_or(Error::Refused("missing builder cache"))?;
        let name = target.file_name().and_then(|n| n.to_str()).ok_or(Error::Refused("missing builder cache key"))?;
        let file = lease(root, name)?;
        // Eviction only takes idle buckets. No await between taking the shared lease and creating the directory.
        loop {
            match flock(&file, FlockOperation::NonBlockingLockShared) {
                Ok(()) => break,
                Err(rustix::io::Errno::WOULDBLOCK) => tokio::time::sleep(Duration::from_millis(50)).await,
                Err(error) => return Err(std::io::Error::from(error).into()),
            }
        }
        std::fs::create_dir_all(&target)?;
        if target.canonicalize()? != target { return Err(Error::Refused("aliased builder target")); }
        // A lease's modification time is its last use, rather than Cargo's arbitrary directory mtime.
        use std::io::Write;
        (&file).write_all(&now().to_le_bytes())?;
        let receipt = self.target_receipt(effect, "owner");
        if matches!(effect.kind, EffectKind::Execute(StageKind::Build)) && !receipt.try_exists()? {
            let link = self.link_builder_target(tree, &target).await?;
            durable(&receipt, &serde_json::to_vec(&TargetOwner { link })?)?;
        }
        Ok(file)
    }
    /// Existing build scripts link libraries and execute binaries through the checkout's target/ path. Keep
    /// that path working while Cargo writes into the managed bucket. Never replace tracked or nonignored files.
    async fn link_builder_target(&self, tree: &Path, cached: &Path) -> Result<Option<(u64, u64)>> {
        let target = tree.join("target");
        // Ignored does not mean owned. Keep pre-existing directories, files and even dangling links.
        match target.symlink_metadata() {
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
        if !crate::git::git(tree, &["check-ignore", "--quiet", "--", "target/"]).await.is_ok_and(|o| o.ok) { return Ok(None); }
        if !crate::git::git(tree, &["ls-files", "-z", "--", "target"]).await.is_ok_and(|o| o.ok && o.stdout.is_empty()) { return Ok(None); }
        // A directory-only ignore rule (target/) doesn't ignore a symlink. Extend that already-ignored build
        // path in Git's private excludes, leaving committed source and its .gitignore unchanged.
        let exclude = crate::git::git(tree, &["rev-parse", "--path-format=absolute", "--git-path", "info/exclude"]).await
            .map_err(|_| Error::Refused("cannot locate build output excludes"))?;
        if !exclude.ok { return Err(Error::Refused("cannot locate build output excludes")); }
        let path = PathBuf::from(exclude.stdout.trim());
        let mut file = OpenOptions::new().append(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&path)?;
        if !file.metadata()?.is_file() || file.metadata()?.nlink() != 1 { return Err(Error::Refused("aliased build output excludes")); }
        let existing = std::fs::read_to_string(&path)?;
        if !existing.lines().any(|l| l == "/target") {
            use std::io::Write;
            file.write_all(b"\n# Engine-managed build output (ov-1170)\n/target\n")?;
            file.sync_all()?;
        }
        // symlink fails with AlreadyExists if somebody created target while Git was being checked.
        std::os::unix::fs::symlink(cached, &target)?;
        let metadata = target.symlink_metadata()?;
        File::open(tree)?.sync_all()?;
        Ok(Some((metadata.dev(), metadata.ino())))
    }
    /// Completed cleanup is durable and scoped to each build effect. Archived tickets may be swept again,
    /// but must never claim a user's later output. Older runs without ownership receipts are preserved.
    pub(super) async fn prune_builder_targets(&self, workspaces: &[String]) -> Result<()> {
        // A named tree can appear on more than one board. Protect all live trees before reclaiming any.
        let mut live = BTreeSet::new();
        let mut finished = Vec::new();
        for workspace in self.store.workflow_workspaces()? {
            let Some(state) = self.state(&workspace)? else { continue };
            for (id, item) in &state.items {
                if item.group_of.is_some() { continue; }
                let tree = item.tree.as_ref().map(|t| PathBuf::from(&t.path)).unwrap_or_else(|| self.item_tree(&workspace, id));
                if !matches!(item.phase, Phase::WaitingJoin | Phase::Joined | Phase::Failed | Phase::Cancelled | Phase::Done) {
                    live.insert(tree);
                }
            }
            if !workspaces.contains(&workspace) { continue; }
            for stored in self.store.workflow_outbox(&workspace)? {
                let effect = stored.effect;
                if !matches!(effect.kind, EffectKind::Execute(StageKind::Build)) { continue; }
                let ended = state.items.get(&effect.item).is_some_and(|i| i.group_of.is_none()
                    && matches!(i.phase, Phase::WaitingJoin | Phase::Joined | Phase::Failed | Phase::Cancelled | Phase::Done))
                    || state.archive.get(&effect.item).is_some_and(|a| a.group.is_none());
                if ended {
                    let tree = effect.tree.as_ref().map(|t| PathBuf::from(&t.path)).unwrap_or_else(|| self.item_tree(&workspace, &effect.item));
                    finished.push((tree, effect));
                }
            }
        }
        for (tree, effect) in finished {
            if live.contains(&tree) { continue; }
            let receipt = self.target_receipt(&effect, "owner");
            let cleaned = self.target_receipt(&effect, "cleaned");
            if cleaned.try_exists()? || !receipt.try_exists()? { continue; }
            let owner: TargetOwner = serde_json::from_slice(&self.bytes(&receipt)?)?;
            let cached = self.builder_target(&tree);
            let root = cached.parent().ok_or(Error::Refused("missing builder cache"))?;
            let name = cached.file_name().and_then(|n| n.to_str()).ok_or(Error::Refused("missing builder cache key"))?;
            let lock = lease(root, name)?;
            if flock(&lock, FlockOperation::NonBlockingLockExclusive).is_err() { continue; }
            if cached.symlink_metadata().is_ok_and(|m| m.is_dir()) { self.discard_builder_target(&cached).await?; }
            let target = tree.join("target");
            if let Some(identity) = owner.link {
                if target.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink() && (m.dev(), m.ino()) == identity)
                    && std::fs::read_link(&target).is_ok_and(|p| p == cached) {
                    // A person may have explicitly tracked the link since launch; source is never reclaimed.
                    if crate::git::git(&tree, &["ls-files", "-z", "--", "target"]).await.is_ok_and(|o| o.ok && o.stdout.is_empty()) {
                        std::fs::remove_file(&target)?;
                        File::open(&tree)?.sync_all()?;
                    }
                }
            }
            durable(&cleaned, b"{}")?;
        }
        Ok(())
    }
    async fn discard_builder_target(&self, target: &Path) -> Result<()> {
        // Reuse the fold-tree trash recovery on restart; renaming releases the run's target immediately.
        let trash = self.root.join(format!("trash-{}", Uuid::now_v7()));
        if let Err(error) = std::fs::rename(target, &trash) {
            if error.raw_os_error() != Some(libc::EXDEV) { return Err(error.into()); }
            // A named worktree may be on another volume. Wait for its deletion off the async executor.
            let target = target.to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(target)).await
                .map_err(|_| Error::Refused("builder output cleanup interrupted"))??;
            return Ok(());
        }
        File::open(target.parent().ok_or(Error::Refused("missing target parent"))?)?.sync_all()?;
        File::open(&self.root)?.sync_all()?;
        tokio::task::spawn_blocking(move || {
            if let Err(error) = std::fs::remove_dir_all(&trash) { tracing::warn!(%error, "could not delete builder output"); }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests;
