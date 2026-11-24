//! Non-gating build output release, driven by the engine's durable cleanup outbox.
use super::*;
use farcooler_workflow::cleanup::{CleanupEffect, CleanupResult, DEFAULT_BUILD_FOLDERS, valid_folder};
use std::{collections::BTreeMap, sync::{Arc, Mutex, Weak}};
use tokio::sync::{OwnedMutexGuard, OwnedRwLockWriteGuard, RwLock};
use tokio::time::Instant;

mod generated;

// Older landing gates and local builders used these paths as well as the defaults.
const LEGACY_BUILD_FOLDERS: &[&str] = &["crates/web/target", "apps/macos/.build", "apps/shared/AgentKit/.build"];
const LEGACY_PUBLISHED_DEFAULTS: &[&str] = &["target", "apps/android/app/build", "apps/*/build", ".build"];

const OUTPUT_INTERRUPTED: &str = "build output cleanup interrupted; bytes freed unknown";
pub(super) const CLEANUP_RETRY: Duration = Duration::from_secs(1);
const CHECKOUT_RETRY: Duration = Duration::from_secs(5 * 60);

pub(super) const GROUP_INTERRUPTED: &str = "group tree cleanup interrupted; bytes freed unknown";

#[derive(Clone, Default)]
pub(super) struct TreeLocks(Arc<Mutex<BTreeMap<PathBuf, Weak<RwLock<()>>>>>);

impl Exec {
    fn tree_key(&self, tree: &Path) -> PathBuf {
        let mut key = tree.canonicalize().unwrap_or_else(|_| tree.to_path_buf());
        // An old private checkout switches names once removed. Fence both names until pruning finishes.
        if key.parent() == Some(self.root.as_path())
            && let Some(legacy) = key.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_prefix("tree-wf1:"))
        {
            key = self.root.join(format!("tree-wf1-{legacy}"));
        }
        key
    }

    pub(super) fn tree_lock(&self, tree: &Path) -> Arc<RwLock<()>> {
        let key = self.tree_key(tree);
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

/// Retry on a monotonic deadline, only while a cleanup blocker remains.
pub(super) async fn wait_for_retry(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(time) => tokio::time::sleep_until(time).await,
        None => std::future::pending::<()>().await,
    }
}

impl Exec {
    pub(super) fn private_trees(&self) -> std::io::Result<Vec<PathBuf>> {
        let mut trees = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let digest = name.to_str().and_then(|name|
                name.strip_prefix("tree-wf1-").or_else(|| name.strip_prefix("tree-wf1:")));
            if digest.is_some_and(|key| key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()))
                && entry.file_type()?.is_dir() {
                trees.push(entry.path());
            }
        }
        Ok(trees)
    }

    /// A legacy checkout can outlast all its run records. The private namespace
    /// identifies candidates, but only linked checkouts with no live user qualify.
    pub(super) async fn release_orphan_builds(&self, trees: &[PathBuf]) -> Result<BTreeMap<PathBuf, Instant>> {
        let mut bytes_freed = 0u64;
        let mut pending = BTreeMap::new();
        for tree in trees {
            let paths: BTreeSet<_> = DEFAULT_BUILD_FOLDERS.iter().chain(LEGACY_BUILD_FOLDERS)
                .filter_map(|folder| checked_folder(tree, folder).ok().flatten())
                .chain(cargo_cache_folders(tree)).collect();
            if paths.is_empty() { continue; }
            let Ok(guard) = self.tree_lock(tree).try_write_owned() else {
                pending.insert(tree.clone(), Instant::now() + CLEANUP_RETRY);
                continue;
            };
            if !self.store.workflow_cleanup_has_authority()? { break; }
            if self.private_tree_in_use(tree)? || tree.canonicalize().ok().as_deref() != Some(tree.as_path())
                || !tree.join(".git").symlink_metadata().is_ok_and(|m| m.is_file()) {
                continue;
            }
            let Ok(top) = crate::git::git(tree, &["rev-parse", "--show-toplevel"]).await else { continue };
            if !top.ok || Path::new(top.stdout.trim_end()) != tree { continue; }
            let held = Arc::new(CleanupFences { _tree: guard, _repository: None });
            for path in paths {
                let mut retry = None;
                match self.release_folder(tree, &path, held.clone(), &mut retry, true).await {
                    Ok(bytes) => bytes_freed = bytes_freed.saturating_add(bytes),
                    Err(reason) => tracing::debug!(?path, %reason, "legacy build output retained"),
                }
                if let Some(delay) = retry {
                    pending.insert(tree.clone(), Instant::now() + delay);
                    break;
                }
            }
        }
        if bytes_freed > 0 {
            tracing::info!(bytes_freed, "released legacy workflow build output");
        }
        Ok(pending)
    }

    fn private_tree_in_use(&self, tree: &Path) -> Result<bool> {
        let key = self.tree_key(tree);
        for workspace in self.store.workflow_workspaces()? {
            if let Some(state) = self.state(&workspace)? {
                for (id, item) in &state.items {
                    if matches!(item.phase, Phase::Done | Phase::Failed | Phase::Cancelled) { continue; }
                    let path = item.tree.as_ref().map_or_else(|| self.item_tree_for(&workspace, id, item.restart.as_deref()),
                        |tree| PathBuf::from(&tree.path));
                    if self.tree_key(&path) == key { return Ok(true); }
                }
            }
            for effect in self.store.workflow_unsettled_writers(&workspace)? {
                if self.tree_key(&self.effect_tree(&workspace, &effect)) == key { return Ok(true); }
            }
        }
        Ok(false)
    }

    /// Return the earliest dispatch wakeup needed by a pending cleanup.
    pub(super) async fn release_builds(&self, workspaces: &[String]) -> Result<Option<Instant>> {
        self.release_builds_checked(workspaces, || Ok(())).await
    }
    pub(super) async fn release_builds_checked(&self, workspaces: &[String], mut before_result: impl FnMut() -> Result<()>) -> Result<Option<Instant>> {
        self.release_builds_at(workspaces, now(), &mut before_result).await
    }
    pub(super) async fn release_builds_at(&self, workspaces: &[String], at: i64, mut before_result: impl FnMut() -> Result<()>) -> Result<Option<Instant>> {
        let start = Instant::now();
        let mut deadline = None;
        let mut schedule = |time| deadline = deadline.into_iter().chain(Some(time)).min();
        for workspace in workspaces {
            for record in self.store.workflow_pending_cleanups(workspace)? {
                let effect = record.effect;
                if let Some(until) = record.result.as_ref().and_then(|result| result.retry_at_ms).filter(|until| *until > at) {
                    schedule(start + Duration::from_millis(until.saturating_sub(at) as u64));
                    continue;
                }
                if let Some(directory) = &effect.directory {
                    if self.release_owned_directory(&effect, directory, record.result.as_ref(), at).await? {
                        schedule(Instant::now() + CLEANUP_RETRY);
                    }
                    continue;
                }
                let tree = effect.tree.as_ref().map_or_else(|| self.item_tree_for(workspace, &effect.item, effect.restart.as_deref()), |t| PathBuf::from(&t.path));
                // Retry rather than settle a skip. A cancellation can end the item before its
                // already-running future is joined, and a waiting branch can also be in use.
                let Ok(held) = self.tree_lock(&tree).try_write_owned() else {
                    schedule(Instant::now() + CLEANUP_RETRY);
                    continue;
                };
                if !self.store.workflow_cleanup_begin(&effect, record.result.as_ref())? { continue; }
                let reconcile = record.result.as_ref().is_some_and(|r|
                    r.skipped.iter().any(|s| s == GROUP_INTERRUPTED));
                let mut result = match self.release_build_attempt(&effect, &tree, held, || {}, reconcile).await {
                    Ok(result) => result,
                    Err(error) => CleanupResult { skipped: vec![format!("cleanup unavailable: {error}")], ..CleanupResult::default() },
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
                if result.retry {
                    let delay = result.retry_at_ms.map_or(CLEANUP_RETRY, |until|
                        Duration::from_millis(until.saturating_sub(now()).max(0) as u64));
                    schedule(Instant::now() + delay);
                }
            }
        }
        Ok(deadline)
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
        let skip = |why: &str| CleanupResult { skipped: vec![why.into()], ..CleanupResult::default() };
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
        let adopted = effect.tree.as_ref().filter(|t| t.adopted);
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
                        && !matches!(item.phase, Phase::Done | Phase::Failed | Phase::Cancelled | Phase::WaitingApproval) {
                        return Ok(skip("run is still building"));
                    }
                } else if !matches!(item.phase, Phase::Done | Phase::Failed | Phase::Cancelled) {
                    return Ok(skip("worktree is shared with another live run"));
                }
            }
            // A prior writer can survive daemon restart after its item has left live state.
            for stored in self.store.workflow_unsettled_writers(workspace)? {
                let path = self.effect_tree(workspace, &stored);
                if path == tree || path.canonicalize().is_ok_and(|p| p == canonical) {
                    return Ok(CleanupResult { retry: true, ..skip("worktree has an unsettled writer") });
                }
            }
        }
        if let Some(adopted) = adopted {
            // The cleanup records the stage that just ended. Entering the waiting
            // stage changes the run ID, while restart identifies the same start.
            let waiting = self.state(&effect.workspace)?.is_some_and(|state| state.items.get(&effect.item).is_some_and(|item|
                farcooler_workflow::cleanup::waiting_at_rest(&item.phase)
                    && item.restart == effect.restart && item.tree.as_ref() == Some(adopted)));
            if !waiting { return Ok(skip("adopted worktree has no waiting run")); }
            let branch = crate::git::git(tree, &["symbolic-ref", "--quiet", "HEAD"]).await.map_err(Error::Domain)?;
            if !branch.ok || branch.stdout.trim_end() != format!("refs/heads/{}", adopted.branch) {
                return Ok(skip("adopted worktree branch has changed"));
            }
        }
        after_check();
        if group_recovery || effect.tree.is_none() && self.finished_landing_group(effect)? {
            return Ok(match self.release_group_tree(tree, held, effect, group_recovery).await {
                Ok(bytes_freed) => CleanupResult { bytes_freed, ..CleanupResult::default() },
                Err(why) => CleanupResult { retry: true, skipped: vec![why], ..CleanupResult::default() },
            });
        }
        let mut result = CleanupResult::default();
        let mut paths = BTreeSet::new();
        for folder in build_folders(&effect.folders, adopted.is_some()) {
            for path in folder_paths(tree, folder) {
                match path {
                    Ok(Some(path)) => { paths.insert(path); }
                    Ok(None) => {}
                    Err(error) => result.skipped.push(format!("{folder}: {error}")),
                }
            }
        }
        let discovered = if default_build_folders(&effect.folders) {
            cargo_cache_folders(tree).filter(|path| !paths.contains(path)).collect()
        } else {
            BTreeSet::new()
        };
        paths.extend(discovered.iter().cloned());
        // Nested/duplicate patterns must not count or remove a folder twice.
        let mut removed: Vec<PathBuf> = Vec::new();
        for path in paths {
            if removed.iter().any(|p| path.starts_with(p)) { continue; }
            let label = path.strip_prefix(tree).unwrap().to_string_lossy().into_owned();
            let mut retry = None;
            let selective = adopted.is_some() || discovered.contains(&path);
            let answer = self.release_folder(tree, &path, held.clone(), &mut retry, selective).await;
            result.retry |= retry.is_some();
            match answer {
                Ok(bytes) => { result.bytes_freed = result.bytes_freed.saturating_add(bytes); removed.push(path); }
                Err(why) => result.skipped.push(format!("{label}: {why}")),
            }
            if retry == Some(CHECKOUT_RETRY) {
                result.retry_at_ms = Some(now().saturating_add(CHECKOUT_RETRY.as_millis() as i64));
                break;
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
    async fn release_folder(&self, tree: &Path, path: &Path, held: Arc<CleanupFences>, retry: &mut Option<Duration>,
        check_whole_checkout: bool) -> std::result::Result<u64, String> {
        let relative = path.strip_prefix(tree).map_err(|e| e.to_string())?;
        check_output_index(tree, relative, check_whole_checkout).await?;
        let metadata = path.symlink_metadata().map_err(|e| e.to_string())?;
        let device = tree.symlink_metadata().map_err(|e| e.to_string())?.dev();
        let generated_reports = relative.file_name().is_some_and(|n| n == "build");
        let artifacts = if check_whole_checkout {
            // A source reader or a shell's cwd can hold the tree. Check before
            // opening candidate files and walking potentially large outputs.
            check_open_files(tree).await.inspect_err(|_| *retry = Some(CHECKOUT_RETRY))?;
            let output = path.to_path_buf();
            let checkout = tree.to_path_buf();
            let plan = tokio::task::spawn_blocking(move || generated::ArtifactPlan::inspect(&checkout, &output, device))
                .await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
            if plan.is_empty() { return Err("no verified build artifacts".into()); }
            Some(plan)
        } else {
            None
        };
        #[cfg(test)]
        if let Some(check) = &self.cleanup_check {
            check.0.notify_one();
            check.1.notified().await;
        }
        // Recheck after inventory in case a process started using the tree
        // during the scan. Keep the short retry for output-folder checks.
        let (scope, delay) = if check_whole_checkout { (tree, CHECKOUT_RETRY) } else { (path, CLEANUP_RETRY) };
        check_open_files(scope).await.inspect_err(|_| *retry = Some(delay))?;
        // A completed git add can leave both the artifact inode and the
        // open-file check unchanged. Recheck the index before unlinking.
        check_output_index(tree, relative, check_whole_checkout).await?;
        if check_whole_checkout && !self.store.workflow_cleanup_has_authority().map_err(|e| e.to_string())? {
            return Err("engine authority unavailable".into());
        }
        let tree = tree.to_path_buf();
        let relative = relative.to_string_lossy().into_owned();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            // Blocking deletion can outlive a cancelled async caller; retain the fence here too.
            let _held = held;
            let current = path.symlink_metadata().map_err(|e| e.to_string())?;
            if (current.dev(), current.ino()) != (metadata.dev(), metadata.ino())
                || checked_folder(&tree, &relative).map_err(|e| e.to_string())?.is_none()
                || tree.canonicalize().map_err(|e| e.to_string())? != tree {
                return Err("build output path has changed".into());
            }
            // Either deletion can remove some entries before returning an error.
            let released = match artifacts {
                Some(plan) => plan.release(&path),
                None => {
                    // The open-file check can take seconds. Validate descendants
                    // afterward so an inserted repository or report is protected.
                    let bytes = size(&path, device, generated_reports).map_err(|e| e.to_string())?;
                    std::fs::remove_dir_all(&path).map(|()| bytes)
                }
            };
            released.map_err(|e| format!("{OUTPUT_INTERRUPTED}: {e}"))
        }).await.map_err(|e| e.to_string())?
    }
}

async fn check_output_index(tree: &Path, relative: &Path, check_staged: bool) -> std::result::Result<(), String> {
    let spec = format!(":(icase,literal){}", relative.to_string_lossy());
    let tracked = crate::git::git(tree, &["ls-files", "-z", "--cached", "--", &spec])
        .await.map_err(|e| e.to_string())?;
    if !tracked.ok || !tracked.stdout.is_empty() {
        return Err("contains tracked files or git could not check".into());
    }
    if check_staged {
        // A staged removal can hide still-present source from the index.
        let staged = crate::git::git(tree, &["diff", "--cached", "--name-only", "-z", "HEAD", "--", &spec])
            .await.map_err(|e| e.to_string())?;
        if !staged.ok || !staged.stdout.is_empty() {
            return Err("contains staged changes or git could not check".into());
        }
    }
    Ok(())
}

pub(super) fn default_build_folders(folders: &[String]) -> bool {
    [DEFAULT_BUILD_FOLDERS, LEGACY_PUBLISHED_DEFAULTS].into_iter()
        .any(|defaults| folders.iter().map(String::as_str).eq(defaults.iter().copied()))
}

pub(super) fn build_folders(folders: &[String], include_legacy: bool) -> impl Iterator<Item = &str> {
    // Preserve an explicit policy. Only augment the stock lists.
    let legacy = include_legacy && default_build_folders(folders);
    folders.iter().map(String::as_str).chain(LEGACY_BUILD_FOLDERS.iter().copied().filter(move |_| legacy))
}

/// Discover Cargo targets set by an agent's environment. Nested targets can
/// also be named explicitly in the workflow build-folder policy.
pub(super) fn cargo_cache_folders(tree: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    std::fs::read_dir(tree).into_iter().flatten().filter_map(|entry| {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_dir() { return None; }
        let name = entry.file_name();
        let name = name.to_str().filter(|name| valid_folder(name))?;
        let path = checked_folder(tree, name).ok()??;
        generated::cargo_cache(&path).then_some(path)
    })
}

pub(super) fn folder_paths<'a>(tree: &'a Path, folder: &'a str) -> impl Iterator<Item = std::io::Result<Option<PathBuf>>> + 'a {
    // Older defaults stored a wildcard. Only its known literal replacements may be reclaimed.
    let legacy = folder == "apps/*/build";
    let replacements = if legacy { &DEFAULT_BUILD_FOLDERS[2..6] } else { &[] };
    replacements.iter().copied().chain((!legacy).then_some(folder)).map(move |folder| {
        if !valid_folder(folder) || folder.contains('*') {
            return Err(std::io::Error::other("cleanup requires a literal folder"));
        }
        checked_folder(tree, folder)
    })
}
pub(super) async fn check_open_files(path: &Path) -> std::result::Result<(), String> {
    // macOS launch agents may run without /usr/sbin on PATH.
    let mut command = tokio::process::Command::new(if cfg!(target_os = "macos") { "/usr/sbin/lsof" } else { "lsof" });
    command.args(["-nP", "-F", "p", "+D"]).arg(path).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(120), command.output()).await
        .map_err(|_| "open-file check timed out".to_string())?.map_err(|e| format!("open-file check unavailable: {e}"))?;
    if !output.stdout.is_empty() { return Err("open files under folder".into()); }
    if output.status.code() != Some(1) || !output.stderr.is_empty() {
        return Err("open-file check could not establish safety".into());
    }
    Ok(())
}

/// Check every ancestor without following links. Missing output is already released.
pub(super) fn checked_folder(tree: &Path, folder: &str) -> std::io::Result<Option<PathBuf>> {
    let mut path = tree.to_path_buf();
    for part in folder.split('/') {
        path.push(part);
        match path.symlink_metadata() {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(std::io::Error::other("folder or ancestor is a symlink or not a directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(Some(path))
}
fn size(path: &Path, device: u64, generated_reports: bool) -> std::io::Result<u64> {
    let meta = path.symlink_metadata()?;
    if meta.dev() != device { return Err(std::io::Error::other("folder crosses a filesystem boundary")); }
    if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case(".git")) { return Err(std::io::Error::other("contains git metadata")); }
    if !generated_reports && path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("reports")) { return Err(std::io::Error::other("contains reports")); }
    if !meta.file_type().is_dir() { return Ok(meta.len()); }
    // Bare repositories have no `.git` component. Preserve their objects too, even when an
    // untracked cache holds them; a build-folder policy is never permission to delete commits.
    if path.join("HEAD").symlink_metadata().is_ok() && path.join("objects").symlink_metadata().is_ok() {
        return Err(std::io::Error::other("contains git metadata (bare repository)"));
    }
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path)? { bytes = bytes.saturating_add(size(&entry?.path(), device, generated_reports)?); }
    Ok(bytes)
}
