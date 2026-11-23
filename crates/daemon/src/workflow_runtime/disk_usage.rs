//! Sample allocated bytes off the command path. A cold inventory can take
//! longer than the workflow.get RPC budget on a busy runner.
use super::{Exec, Result, build_cleanup, now};
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub(super) struct DiskUsage(Arc<Mutex<Cache>>);

#[derive(Default)]
struct Cache {
    running: bool,
    last_sample: Option<Instant>,
    sample: Option<Sample>,
}

struct Sample {
    sampled_at_ms: i64,
    engine_tree_bytes: Option<u64>,
    waiting_build_bytes: BTreeMap<String, Option<u64>>,
}

impl Exec {
    pub(super) fn refresh_disk_usage(&self) -> Option<tokio::task::JoinHandle<()>> {
        {
            let mut cache = self.disk_usage.0.lock().unwrap();
            if cache.running || cache.last_sample.is_some_and(|at| at.elapsed() < SAMPLE_INTERVAL) {
                return None;
            }
            cache.running = true;
        }
        let exec = self.clone();
        Some(tokio::task::spawn_blocking(move || {
            let sample = exec.sample_disk_usage();
            let mut cache = exec.disk_usage.0.lock().unwrap();
            cache.running = false;
            cache.last_sample = Some(Instant::now());
            match sample {
                Ok(sample) => cache.sample = Some(sample),
                Err(error) => tracing::warn!(%error, "could not sample workflow disk usage"),
            }
        }))
    }

    pub(super) fn disk_usage_reply(&self, workspace: &str) -> serde_json::Value {
        let cache = self.disk_usage.0.lock().unwrap();
        let sample = cache.sample.as_ref();
        serde_json::json!({
            "sampled_at_ms": sample.map(|s| s.sampled_at_ms),
            "engine_tree_bytes": sample.and_then(|s| s.engine_tree_bytes),
            "waiting_build_bytes": sample.and_then(|s| s.waiting_build_bytes.get(workspace).copied().unwrap_or(Some(0))),
        })
    }

    fn sample_disk_usage(&self) -> Result<Sample> {
        let sampled_at_ms = now();
        let mut waiting_build_bytes = BTreeMap::new();
        for workspace in self.store.workflow_workspaces()? {
            let mut folders = BTreeSet::new();
            if let Some(state) = self.state(&workspace)? {
                for (id, item) in &state.items {
                    if !farcooler_workflow::cleanup::waiting_at_rest(&item.phase)
                        || item.definition.is_ticket() || item.group_of.is_some() || item.follows.is_some() {
                        continue;
                    }
                    let tree = item.tree.as_ref().map_or_else(|| self.item_tree_for(&workspace, id, item.restart.as_deref()),
                        |tree| PathBuf::from(&tree.path));
                    if tree.canonicalize().ok().as_deref() != Some(tree.as_path()) { continue; }
                    let policy = item.definition.build_folders();
                    if build_cleanup::default_build_folders(&policy) {
                        folders.extend(build_cleanup::cargo_cache_folders(&tree));
                    }
                    for folder in build_cleanup::build_folders(&policy, true) {
                        folders.extend(build_cleanup::folder_paths(&tree, folder).filter_map(|path| path.ok().flatten()));
                    }
                }
            }
            waiting_build_bytes.insert(workspace, total_bytes(folders));
        }
        Ok(Sample { sampled_at_ms, engine_tree_bytes: total_bytes(self.private_trees()?), waiting_build_bytes })
    }
}

fn total_bytes(paths: impl IntoIterator<Item = PathBuf>) -> Option<u64> {
    let mut bytes = 0u64;
    let mut seen = BTreeSet::new();
    for path in paths {
        match path.symlink_metadata() {
            Ok(meta) if meta.is_dir() => bytes = bytes.saturating_add(allocated_bytes(&path, meta.dev(), &mut seen).ok()?),
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        }
    }
    Some(bytes)
}

fn allocated_bytes(path: &Path, device: u64, seen: &mut BTreeSet<(u64, u64)>) -> std::io::Result<u64> {
    let meta = match path.symlink_metadata() {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if meta.dev() != device { return Ok(0); }
    if (meta.is_dir() || meta.nlink() > 1) && !seen.insert((device, meta.ino())) { return Ok(0); }
    let mut bytes = meta.blocks().saturating_mul(512);
    if meta.is_dir() {
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        for entry in entries {
            bytes = bytes.saturating_add(allocated_bytes(&entry?.path(), device, seen)?);
        }
    }
    Ok(bytes)
}
