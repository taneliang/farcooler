//! Non-gating build output release, driven by the engine's durable cleanup outbox.
use super::*;
use farcooler_workflow::cleanup::{CleanupEffect, CleanupResult, valid_folder};
use std::{collections::BTreeMap, sync::{Arc, Mutex, Weak}};
use tokio::sync::{OwnedMutexGuard, OwnedRwLockWriteGuard, RwLock};

const OUTPUT_INTERRUPTED: &str = "build output cleanup interrupted; bytes freed unknown";

pub(super) const GROUP_INTERRUPTED: &str = "group tree cleanup interrupted; bytes freed unknown";

#[derive(Clone, Default)]
pub(super) struct TreeLocks(Arc<Mutex<BTreeMap<PathBuf, Weak<RwLock<()>>>>>);

impl Exec {
    pub(super) fn tree_lock(&self, tree: &Path) -> Arc<RwLock<()>> {
        let mut key = tree.canonicalize().unwrap_or_else(|_| tree.to_path_buf());
        // An old private checkout switches names once removed. Fence both names until pruning finishes.
        if key.parent() == Some(self.root.as_path())
            && let Some(legacy) = key.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_prefix("tree-wf1:"))
        {
            key = self.root.join(format!("tree-wf1-{legacy}"));
        }
        let mut locks = self.tree_locks.0.lock().unwrap();
        locks.retain(|_, lock| lock.strong_count() > 0);
        let entry = locks.entry(key).or_default();
        entry.upgrade().unwrap_or_else(|| {
            let lock = Arc::new(RwLock::new(()));
            *entry = Arc::downgrade(&lock);
            lock
        })
    }
}

/// Also retained by blocking deletion, which can outlive a cancelled async caller.
pub(super) struct CleanupFences {
    _tree: OwnedRwLockWriteGuard<()>,
    _repository: Option<OwnedMutexGuard<()>>,
}

/// Retry on a monotonic deadline, only while a transient cleanup blocker remains.
pub(super) async fn wait_for_retry(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(time) => tokio::time::sleep_until(time).await,
        None => std::future::pending::<()>().await,
    }
}

impl Exec {
    /// True while a transient blocker still needs a later dispatch wakeup.
    pub(super) async fn release_builds(&self, workspaces: &[String]) -> Result<bool> {
        self.release_builds_checked(workspaces, || Ok(())).await
    }
    pub(super) async fn release_builds_checked(&self, workspaces: &[String], mut before_result: impl FnMut() -> Result<()>) -> Result<bool> {
        let mut retry = false;
        for workspace in workspaces {
            self.retain_finished_group_cleanups(workspace)?;
            for record in self.store.workflow_cleanups(workspace)? {
                if record.result.as_ref().is_some_and(|r| !r.retry) { continue; }
                let effect = record.effect;
                let tree = effect.tree.as_ref().map_or_else(|| self.item_tree_for(workspace, &effect.item, effect.restart.as_deref()), |t| PathBuf::from(&t.path));
                // Retry rather than settle a skip. A cancellation can end the item before its
                // already-running future is joined, and a waiting branch can also be in use.
                let Ok(held) = self.tree_lock(&tree).try_write_owned() else {
                    retry = true;
                    continue;
                };
                if !self.store.workflow_cleanup_begin(&effect, record.result.as_ref())? { continue; }
                let reconcile = record.result.as_ref().is_some_and(|r|
                    r.skipped.iter().any(|s| s == GROUP_INTERRUPTED));
                let mut result = match self.release_build_attempt(&effect, &tree, held, || {}, reconcile).await {
                    Ok(result) => result,
                    Err(error) => CleanupResult { retry: false, bytes_freed: 0, skipped: vec![format!("cleanup unavailable: {error}")] },
                };
                // A preflight refusal must not discard the reconciliation mode: .git may be
                // gone already, and only a later safe reconciliation can finish this intent.
                if reconcile && !result.skipped.is_empty() { result.retry = true; }
                let output_unknown = result.skipped.iter().any(|s| s.contains(OUTPUT_INTERRUPTED))
                    || record.result.as_ref().is_some_and(|r| r.skipped.iter().any(|s| s.contains(OUTPUT_INTERRUPTED)));
                // Completed folders are absent on retry; preserve their byte count exactly once.
                if reconcile || result.skipped.iter().any(|s| s.starts_with(GROUP_INTERRUPTED)) {
                    // Neither the remaining size nor zero is a known total for the lost attempt.
                    // Keep the existing wire convention: zero plus an explicit unknown-byte reason.
                    result.bytes_freed = 0;
                    result.skipped.push(GROUP_INTERRUPTED.into());
                } else {
                    result.bytes_freed = result.bytes_freed.saturating_add(record.result.map_or(0, |r| r.bytes_freed));
                }
                if output_unknown {
                    result.bytes_freed = 0;
                    if !result.skipped.iter().any(|s| s.contains(OUTPUT_INTERRUPTED)) {
                        result.skipped.push(OUTPUT_INTERRUPTED.into());
                    }
                }
                before_result()?;
                self.store.workflow_cleanup_result(&effect, &result)?;
                retry |= result.retry;
            }
        }
        Ok(retry)
    }
    #[cfg(test)]
    pub(super) async fn release_build_checked(&self, effect: &CleanupEffect, after_check: impl FnOnce()) -> Result<CleanupResult> {
        let tree = effect.tree.as_ref().map_or_else(|| self.item_tree_for(&effect.workspace, &effect.item, effect.restart.as_deref()), |t| PathBuf::from(&t.path));
        let held = self.tree_lock(&tree).try_write_owned().map_err(|_| Error::Refused("cleanup tree is busy"))?;
        self.release_build_attempt(effect, &tree, held, after_check, false).await
    }
    async fn release_build_attempt(&self, effect: &CleanupEffect, tree: &Path, tree_guard: OwnedRwLockWriteGuard<()>,
        after_check: impl FnOnce(), reconcile: bool) -> Result<CleanupResult>
    {
        let skip = |why: &str| CleanupResult { retry: false, bytes_freed: 0, skipped: vec![why.into()] };
        // The service shares this fence with provision_named, adoption and worktree.create.
        // The per-tree fence also protects against dispatching effects in either runtime.
        // A service missing its shared repository fence must fail closed.
        let repository_guard = if let Some(host) = &self.host {
            let board = Uuid::parse_str(&effect.workspace).map_err(|_| Error::Refused("cleanup workspace unavailable"))?;
            let repository = self.store.get_workspace(board).map_err(|_| Error::Refused("cleanup repository unavailable"))?.repository_id;
            match host.lock(repository).try_lock_owned() {
                Ok(guard) => Some(guard),
                Err(_) => return Ok(CleanupResult { retry: true, ..skip("worktree provision/claim fence is busy") }),
            }
        } else if self.service_mode {
            return Ok(skip("worktree provision/claim fence is unavailable"));
        } else { None };
        if effect.uncertain { return Ok(skip("run is Uncertain")); }
        // An adopted worktree is a person's (ov-968): its build folders may hold their own output, which the run
        // neither made nor may delete (ov-1263).
        if effect.tree.as_ref().is_some_and(|t| t.adopted) { return Ok(skip("worktree is adopted; its output is a person's")); }
        if let Some(why) = self.store.workflow_cleanup_skip(effect)? { return Ok(skip(&why)); }
        let held = Arc::new(CleanupFences { _tree: tree_guard, _repository: repository_guard });
        let group_recovery = reconcile && effect.tree.is_none() && self.ended_group(effect)?;
        let exists = tree.try_exists()?;
        if !exists && !group_recovery { return Ok(skip("worktree is absent")); }
        // Recovery may find .git already removed. Its registration is checked before deletion.
        // A main checkout, symlink or moved binding is still never ours to clean.
        let canonical = if exists { tree.canonicalize()? } else { tree.to_path_buf() };
        let linked = match tree.join(".git").symlink_metadata() {
            Ok(meta) => meta.file_type().is_file(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => group_recovery,
            Err(e) => return Err(e.into()),
        };
        if canonical != tree || !linked { return Ok(skip("worktree is not an exclusive linked tree")); }
        for workspace in &self.store.workflow_workspaces()? {
            let Some(state) = self.state(workspace)? else { continue };
            for (id, item) in &state.items {
                let path = item.tree.as_ref().map_or_else(|| self.item_tree_for(workspace, id, item.restart.as_deref()), |t| PathBuf::from(&t.path));
                let same = path == tree || path.canonicalize().is_ok_and(|p| p == canonical);
                if !same { continue; }
                if item.phase == Phase::Uncertain { return Ok(skip("run is Uncertain")); }
                if workspace == &effect.workspace && id == &effect.item {
                    if item.definition.stages.get(item.stage).is_some_and(|s| matches!(s.kind, StageKind::Build | StageKind::Provision))
                        && !matches!(item.phase, Phase::Done | Phase::Failed | Phase::Cancelled) {
                        return Ok(skip("run is still building"));
                    }
                } else if !matches!(item.phase, Phase::Done | Phase::Failed | Phase::Cancelled) {
                    return Ok(skip("worktree is shared with another live run"));
                }
            }
        }
        after_check();
        if group_recovery {
            return Ok(match self.release_group_tree(tree, held, effect, true).await {
                Ok(_) => CleanupResult { retry: false, bytes_freed: 0, skipped: Vec::new() },
                Err(why) => CleanupResult { retry: true, bytes_freed: 0, skipped: vec![why] },
            });
        }
        let mut result = CleanupResult { retry: false, bytes_freed: 0, skipped: Vec::new() };
        let mut paths = BTreeSet::new();
        for pattern in &effect.folders {
            if !valid_folder(pattern) { result.skipped.push(format!("{pattern}: unsafe folder")); continue; }
            match expand(tree, pattern) {
                Ok(found) => paths.extend(found),
                Err(error) => result.skipped.push(format!("{pattern}: {error}")),
            }
        }
        // Nested/duplicate patterns must not count or remove a folder twice.
        let mut removed: Vec<PathBuf> = Vec::new();
        for path in paths {
            if removed.iter().any(|p| path.starts_with(p)) { continue; }
            let label = path.strip_prefix(tree).unwrap().to_string_lossy().into_owned();
            let mut retry = false;
            let answer = self.release_folder(tree, &path, held.clone(), &mut retry).await;
            result.retry |= retry;
            match answer {
                Ok(bytes) => { result.bytes_freed = result.bytes_freed.saturating_add(bytes); removed.push(path); }
                Err(why) => result.skipped.push(format!("{label}: {why}")),
            }
        }
        if !result.retry && self.ended_group(effect)? && effect.tree.is_none() {
            let released = self.release_group_tree(tree, held, effect, false).await;
            match released {
                Ok(bytes) => result.bytes_freed = result.bytes_freed.saturating_add(bytes),
                Err(why) => { result.retry = true; result.skipped.push(why); }
            }
        }
        if result.skipped.iter().any(|s| s.contains(OUTPUT_INTERRUPTED)) { result.bytes_freed = 0; }
        Ok(result)
    }
    async fn release_folder(&self, tree: &Path, path: &Path, held: Arc<CleanupFences>, retry: &mut bool) -> std::result::Result<u64, String> {
        // A configured output may have become source. Refuse tracked files, even inside a listed folder.
        let relative = path.strip_prefix(tree).map_err(|e| e.to_string())?;
        let spec = format!(":(literal){}", relative.to_string_lossy());
        let tracked = crate::git::git(tree, &["ls-files", "-z", "--", &spec]).await.map_err(|e| e.to_string())?;
        if !tracked.ok || !tracked.stdout.is_empty() { return Err("contains tracked files or git could not check".into()); }
        // No deletion has started: contention or an unavailable open-file check is safe to retry.
        *retry = true;
        #[cfg(test)]
        if let Some(check) = &self.cleanup_check {
            check.0.notify_one();
            check.1.notified().await;
        }
        let mut command = tokio::process::Command::new("lsof");
        command.args(["-nP", "-F", "p", "+D"]).arg(path).kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(30), command.output()).await
            .map_err(|_| "open-file check timed out".to_string())?.map_err(|e| format!("open-file check unavailable: {e}"))?;
        // Some lsof versions return 1 alongside matching records. Any match still means busy.
        if !output.stdout.is_empty() { return Err("open files under folder".into()); }
        if output.status.code() != Some(1) || !output.stderr.is_empty() {
            return Err("open-file check could not establish safety".into());
        }
        *retry = false;
        // Recheck ancestors immediately before removal. remove_dir_all itself does not follow symlinks.
        let mut parent = tree.to_path_buf();
        for part in relative.components() {
            parent.push(part);
            let meta = parent.symlink_metadata().map_err(|e| e.to_string())?;
            if !meta.file_type().is_dir() { return Err("folder or ancestor is a symlink or not a directory".into()); }
        }
        let device = tree.symlink_metadata().map_err(|e| e.to_string())?.dev();
        // Gradle generates reports inside its selected build/ output. Those reports are
        // disposable too; the workspace reports/ evidence and other output stay protected.
        let generated_reports = relative.file_name().is_some_and(|n| n == "build");
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            // Blocking deletion can outlive a cancelled async caller; retain the fence here too.
            let _held = held;
            let bytes = size(&path, device, generated_reports).map_err(|e| e.to_string())?;
            // remove_dir_all can delete some entries before returning an error.
            std::fs::remove_dir_all(&path).map_err(|e| format!("{OUTPUT_INTERRUPTED}: {e}"))?;
            Ok(bytes)
        }).await.map_err(|e| e.to_string())?
    }
}
/// Expand only directories, checking every ancestor without following links. Missing output is already released.
fn expand(tree: &Path, pattern: &str) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = vec![tree.to_path_buf()];
    for part in pattern.split('/') {
        let mut next = Vec::new();
        for path in paths {
            if part == "*" {
                for entry in std::fs::read_dir(path)? {
                    let entry = entry?;
                    if !entry.file_name().eq_ignore_ascii_case(".git") && !entry.file_name().eq_ignore_ascii_case("reports") && entry.file_type()?.is_dir() { next.push(entry.path()); }
                }
            } else {
                let child = path.join(part);
                match child.symlink_metadata() {
                    Ok(meta) if meta.file_type().is_dir() => next.push(child),
                    Ok(_) => return Err(std::io::Error::other("folder or ancestor is a symlink or not a directory")),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                    Err(e) => return Err(e),
                }
            }
        }
        paths = next;
    }
    Ok(paths)
}
fn size(path: &Path, device: u64, generated_reports: bool) -> std::io::Result<u64> {
    let meta = path.symlink_metadata()?;
    if meta.dev() != device { return Err(std::io::Error::other("folder crosses a filesystem boundary")); }
    if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case(".git")) { return Err(std::io::Error::other("contains git metadata")); }
    if !generated_reports && path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("reports")) { return Err(std::io::Error::other("contains reports")); }
    if !meta.file_type().is_dir() { return Ok(meta.len()); }
    // Bare repositories have no `.git` component. Preserve their objects too, even when an
    // untracked cache holds them; a build-folder policy is never permission to delete commits.
    if path.join("HEAD").try_exists()? && path.join("objects").try_exists()? {
        return Err(std::io::Error::other("contains git metadata (bare repository)"));
    }
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path)? { bytes = bytes.saturating_add(size(&entry?.path(), device, generated_reports)?); }
    Ok(bytes)
}
