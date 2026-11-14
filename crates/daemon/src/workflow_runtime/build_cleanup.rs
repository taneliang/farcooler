//! Non-gating build output release, driven by the engine's durable cleanup outbox.
use super::*;
use farcooler_workflow::cleanup::{CleanupEffect, CleanupResult, valid_folder};

impl Exec {
    pub(super) async fn release_builds(&self, workspaces: &[String]) -> Result<()> {
        self.release_builds_checked(workspaces, || Ok(())).await
    }
    pub(super) async fn release_builds_checked(&self, workspaces: &[String], mut before_result: impl FnMut() -> Result<()>) -> Result<()> {
        for workspace in workspaces {
            for record in self.store.workflow_cleanups(workspace)? {
                if record.result.as_ref().is_some_and(|r| !r.retry) { continue; }
                let effect = record.effect;
                let tree = effect.tree.as_ref().map_or_else(|| self.item_tree(workspace, &effect.item), |t| PathBuf::from(&t.path));
                // Defer, rather than settle a skip: a cancellation can end the item before its
                // already-running future is joined. Its output is released on the next pass.
                if self.active_trees.lock().unwrap().iter().any(|p| p == &tree
                    || p.canonicalize().ok().zip(tree.canonicalize().ok()).is_some_and(|(a,b)| a == b)) { continue; }
                if !self.store.workflow_cleanup_begin(&effect, record.result.as_ref())? { continue; }
                let mut result = match self.release_build(&effect).await {
                    Ok(result) => result,
                    Err(error) => CleanupResult { retry: false, bytes_freed: 0, skipped: vec![format!("cleanup unavailable: {error}")] },
                };
                // Completed folders are absent on retry; preserve their byte count exactly once.
                result.bytes_freed = result.bytes_freed.saturating_add(record.result.map_or(0, |r| r.bytes_freed));
                before_result()?;
                self.store.workflow_cleanup_result(&effect, &result)?;
            }
        }
        Ok(())
    }
    async fn release_build(&self, effect: &CleanupEffect) -> Result<CleanupResult> {
        self.release_build_checked(effect, || {}).await
    }
    pub(super) async fn release_build_checked(&self, effect: &CleanupEffect, after_check: impl FnOnce()) -> Result<CleanupResult> {
        let skip = |why: &str| CleanupResult { retry: false, bytes_freed: 0, skipped: vec![why.into()] };
        // The service shares this fence with provision_named, adoption and worktree.create.
        // The isolated runtime instead owns writer.lock for its lifetime and serializes claims
        // on its dispatch loop. A service missing its shared fence must fail closed.
        let held = if let Some(host) = &self.host {
            let board = Uuid::parse_str(&effect.workspace).map_err(|_| Error::Refused("cleanup workspace unavailable"))?;
            let repository = self.store.get_workspace(board).map_err(|_| Error::Refused("cleanup repository unavailable"))?.repository_id;
            match host.lock(repository).try_lock_owned() {
                Ok(guard) => Some(std::sync::Arc::new(guard)),
                Err(_) => return Ok(CleanupResult { retry: true, ..skip("worktree provision/claim fence is busy") }),
            }
        } else if self.service_mode {
            return Ok(skip("worktree provision/claim fence is unavailable"));
        } else { None };
        if effect.uncertain { return Ok(skip("run is Uncertain")); }
        if let Some(why) = self.store.workflow_cleanup_skip(effect)? { return Ok(skip(&why)); }
        let tree = effect.tree.as_ref().map_or_else(|| self.item_tree(&effect.workspace, &effect.item), |t| PathBuf::from(&t.path));
        if !tree.try_exists()? { return Ok(skip("worktree is absent")); }
        // A main checkout, a symlink, or a path which moved out from under its binding is never ours to clean.
        let canonical = tree.canonicalize()?;
        if canonical != tree || !tree.join(".git").symlink_metadata()?.file_type().is_file() {
            return Ok(skip("worktree is not an exclusive linked tree"));
        }
        for workspace in &self.store.workflow_workspaces()? {
            let Some(state) = self.state(workspace)? else { continue };
            for (id, item) in &state.items {
                let path = item.tree.as_ref().map_or_else(|| self.item_tree(workspace, id), |t| PathBuf::from(&t.path));
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
        let mut result = CleanupResult { retry: false, bytes_freed: 0, skipped: Vec::new() };
        let mut paths = BTreeSet::new();
        for pattern in &effect.folders {
            if !valid_folder(pattern) { result.skipped.push(format!("{pattern}: unsafe folder")); continue; }
            match expand(&tree, pattern) {
                Ok(found) => paths.extend(found),
                Err(error) => result.skipped.push(format!("{pattern}: {error}")),
            }
        }
        // Nested/duplicate patterns must not count or remove a folder twice.
        let mut removed: Vec<PathBuf> = Vec::new();
        for path in paths {
            if removed.iter().any(|p| path.starts_with(p)) { continue; }
            let label = path.strip_prefix(&tree).unwrap().to_string_lossy().into_owned();
            let mut retry = false;
            let answer = self.release_folder(&tree, &path, held.clone(), &mut retry).await;
            result.retry |= retry;
            match answer {
                Ok(bytes) => { result.bytes_freed = result.bytes_freed.saturating_add(bytes); removed.push(path); }
                Err(why) => result.skipped.push(format!("{label}: {why}")),
            }
        }
        Ok(result)
    }
    async fn release_folder(&self, tree: &Path, path: &Path, held: Option<std::sync::Arc<tokio::sync::OwnedMutexGuard<()>>>, retry: &mut bool) -> std::result::Result<u64, String> {
        // A configured output may have become source. Refuse tracked files, even inside a listed folder.
        let relative = path.strip_prefix(tree).map_err(|e| e.to_string())?;
        let spec = format!(":(literal){}", relative.to_string_lossy());
        let tracked = crate::git::git(tree, &["ls-files", "-z", "--", &spec]).await.map_err(|e| e.to_string())?;
        if !tracked.ok || !tracked.stdout.is_empty() { return Err("contains tracked files or git could not check".into()); }
        // No deletion has started: contention or an unavailable open-file check is safe to retry.
        *retry = true;
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
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            // Blocking deletion can outlive a cancelled async caller; retain the fence here too.
            let _held = held;
            let bytes = size(&path, device).map_err(|e| e.to_string())?;
            std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
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
fn size(path: &Path, device: u64) -> std::io::Result<u64> {
    let meta = path.symlink_metadata()?;
    if meta.dev() != device { return Err(std::io::Error::other("folder crosses a filesystem boundary")); }
    if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case(".git")) { return Err(std::io::Error::other("contains git metadata")); }
    if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("reports")) { return Err(std::io::Error::other("contains reports")); }
    if !meta.file_type().is_dir() { return Ok(meta.len()); }
    // Bare repositories have no `.git` component. Preserve their objects too, even when an
    // untracked cache holds them; a build-folder policy is never permission to delete commits.
    if path.join("HEAD").try_exists()? && path.join("objects").try_exists()? {
        return Err(std::io::Error::other("contains git metadata (bare repository)"));
    }
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path)? { bytes = bytes.saturating_add(size(&entry?.path(), device)?); }
    Ok(bytes)
}
