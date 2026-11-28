//! Empty engine scratch folders that no pending record names are swept (ov-1528).
//!
//! A recorded folder is released by its cleanup record (`owned_directories`), which also covers an
//! orphan with output in it. This sweep is the other rule: the engine's own scratch name, empty, and
//! untouched for a day. It never deletes anything with something in it.
use super::owned_directories::{ORPHAN_AGE_MS, SCRATCH_PREFIX};
use super::{Exec, Result};
use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Folders per open-file check, well inside the argument limit. A thousand take about half a second.
const BATCH: usize = 500;

impl Exec {
    /// One sweep as the runner makes it, keeping every folder a pending record names.
    pub(super) async fn sweep_scratch(&self) -> Result<usize> {
        if !self.store.workflow_cleanup_has_authority()? { return Ok(0); }
        let keep = self.store.workflow_pending_output_paths()?.into_iter().map(PathBuf::from).collect();
        self.sweep_empty_scratch(&keep, SystemTime::now()).await
    }

    /// Remove the empty scratch folders, a day old at `at`, that `keep` doesn't name and no process
    /// has open. When the open-file check can't answer for a batch, all of the batch stays.
    pub(super) async fn sweep_empty_scratch(&self, keep: &BTreeSet<PathBuf>, at: SystemTime) -> Result<usize> {
        let base = self.scratch_base()?;
        let mut candidates = tokio::task::spawn_blocking(move || stale_empty_scratch(&base, at))
            .await.map_err(std::io::Error::other)??;
        candidates.retain(|path| !keep.contains(path));
        let mut removed = 0;
        for batch in candidates.chunks(BATCH) {
            let held = match self.held_folders(batch).await {
                Ok(held) => held,
                Err(reason) => {
                    tracing::warn!(%reason, folders = batch.len(), "kept empty scratch folders");
                    continue;
                }
            };
            let free: Vec<_> = batch.iter().filter(|path| !held.contains(*path)).cloned().collect();
            removed += tokio::task::spawn_blocking(move || {
                // `remove_dir` refuses a folder that gained a file since it was listed.
                free.iter().filter(|path| std::fs::remove_dir(path).is_ok()).count()
            }).await.map_err(std::io::Error::other)?;
        }
        Ok(removed)
    }
}

/// The engine's scratch name for any engine root: the prefix and one whole digest.
fn engine_scratch_name(name: &str) -> bool {
    name.strip_prefix(SCRATCH_PREFIX).and_then(|rest| rest.strip_prefix("wf1-"))
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Real folders of this user's, with a scratch name, empty and unchanged for a day. No links.
fn stale_empty_scratch(base: &Path, at: SystemTime) -> std::io::Result<Vec<PathBuf>> {
    let age = Duration::from_millis(ORPHAN_AGE_MS as u64);
    let user = rustix::process::geteuid().as_raw();
    let mut found = Vec::new();
    for entry in std::fs::read_dir(base)? {
        let Ok(entry) = entry else { continue };
        if !entry.file_name().to_str().is_some_and(engine_scratch_name) { continue; }
        let path = entry.path();
        let Ok(metadata) = path.symlink_metadata() else { continue };
        if metadata.is_dir() && metadata.uid() == user
            && metadata.modified().is_ok_and(|changed| at.duration_since(changed).is_ok_and(|idle| idle >= age))
            && std::fs::read_dir(&path).is_ok_and(|mut entries| entries.next().is_none()) {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests;
