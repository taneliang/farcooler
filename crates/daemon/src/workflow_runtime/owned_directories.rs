//! Scratch and gate logs have an explicit owner in the cleanup outbox.
use super::{Binding, CodingCommand, EffectKind, Error, Exec, Result, StageKind, adapters, digest, now};
use farcooler_workflow::cleanup::{CleanupEffect, CleanupResult, OwnedDirectory};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

const LOG_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
pub(super) const DIRECTORY_INTERRUPTED: &str = "directory cleanup interrupted; bytes freed unknown";
pub(super) const SCRATCH_ENV: &str = "FARCOOLER_SCRATCH";
pub(super) const SCRATCH_PROMPT: &str = "Put temporary build output in `$FARCOOLER_SCRATCH`, for example by setting `CARGO_TARGET_DIR=\"$FARCOOLER_SCRATCH/target\"`. Never create build folders elsewhere under `/tmp` or `/private/tmp`. The engine clears this scratch directory when the step ends.";

impl Exec {
    pub(super) fn attach_owned_directory(&self, binding: &Binding, owner: &str, command: &mut CodingCommand) -> Result<()> {
        let (key, retention) = match binding.effect.kind {
            EffectKind::Execute(StageKind::Build | StageKind::Review(_)) => (SCRATCH_ENV, 0),
            EffectKind::Execute(StageKind::Run) => ("FARCOOLER_LANE_GATE_LOG_DIR", LOG_RETENTION_MS),
            _ => return Ok(()),
        };
        let id = digest(&("owned-directory/v1", binding, key));
        let path = if key == SCRATCH_ENV { self.scratch_path(&id)? } else { self.owned_directory_path(&id) };
        if self.root.canonicalize()? != self.root { return Err(Error::Refused("aliased engine root")); }
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        let metadata = path.symlink_metadata()?;
        let path_text = path.to_string_lossy().into_owned();
        let effect = CleanupEffect { id, workspace: binding.workspace.clone(), revision: 0, item: binding.effect.item.clone(),
            run: binding.effect.run.clone(), restart: None, tree: None, folders: Vec::new(), uncertain: false,
            directory: Some(OwnedDirectory { path: path_text.clone(), device: metadata.dev(), inode: metadata.ino(),
                owner_effect: owner.into(), retain_until_ms: now().saturating_add(retention) }) };
        if let Err(error) = self.store.workflow_cleanup_enqueue(&effect) {
            let _ = std::fs::remove_dir(&path);
            return Err(error.into());
        }
        command.environment.insert(key.into(), path_text);
        Ok(())
    }

    fn owned_directory_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("output-{}", adapters::folder_name(id)))
    }

    fn scratch_path(&self, id: &str) -> Result<PathBuf> {
        // Codex workspace-write includes /tmp. Use its canonical spelling on macOS,
        // and namespace each directory by the engine root as well as the effect.
        let name = adapters::folder_name(&digest(&(&self.root, id)));
        Ok(Path::new("/tmp").canonicalize()?.join(format!("farcooler-scratch-{name}")))
    }

    pub(super) async fn release_owned_directory(&self, effect: &CleanupEffect, directory: &OwnedDirectory,
        previous: Option<&CleanupResult>, at: i64) -> Result<bool> {
        if at < directory.retain_until_ms
            || !self.store.workflow_cleanup_owner_settled(effect, &directory.owner_effect)? { return Ok(false); }
        if !self.store.workflow_cleanup_begin(effect, previous)? { return Ok(false); }
        let path = PathBuf::from(&directory.path);
        let mut result = CleanupResult { bytes_freed: 0, skipped: Vec::new(), retry: false };
        let deletion = async {
            // Keep accepting recorded output under the engine root, including scratch
            // created before agent scratch moved to the sandbox's writable area.
            let engine_output = path.parent() == Some(self.root.as_path()) && path == self.owned_directory_path(&effect.id);
            if self.root.canonicalize()? != self.root
                || (!engine_output && !self.scratch_path(&effect.id).is_ok_and(|expected| path == expected)) {
                return Err(std::io::Error::other("foreign engine output directory"));
            }
            if !owned_identity(&path, directory)? { return Ok(0); }
            // Unused scratch has no output to inspect. The atomic empty-directory
            // removal fails if a file appeared since the owner settled.
            match std::fs::remove_dir(&path) {
                Ok(()) => return Ok(0),
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(error) => return Err(error),
            }
            if let Err(why) = super::build_cleanup::check_open_files(&path).await {
                result.retry = true;
                return Err(std::io::Error::other(why));
            }
            let directory = directory.clone();
            tokio::task::spawn_blocking(move || {
                if !owned_identity(&path, &directory)? { return Ok(0); }
                if path.canonicalize()? != path {
                    return Err(std::io::Error::other("engine output path has changed"));
                }
                let bytes = output_bytes(&path, directory.device)?;
                std::fs::remove_dir_all(&path).map_err(|error|
                    std::io::Error::other(format!("{DIRECTORY_INTERRUPTED} ({error})")))?;
                Ok(bytes)
            }).await.map_err(std::io::Error::other)?
        }.await;
        match deletion {
            Ok(bytes) => result.bytes_freed = bytes,
            Err(error) => {
                let reason = error.to_string();
                result.retry |= reason.starts_with(DIRECTORY_INTERRUPTED);
                result.skipped.push(reason);
            }
        }
        if previous.is_some_and(|result| result.skipped.iter().any(|note| note.starts_with(DIRECTORY_INTERRUPTED))) {
            result.bytes_freed = 0;
            result.skipped.push(DIRECTORY_INTERRUPTED.into());
        }
        self.store.workflow_cleanup_result(effect, &result)?;
        Ok(result.retry)
    }
}

fn owned_identity(path: &Path, directory: &OwnedDirectory) -> std::io::Result<bool> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && metadata.dev() == directory.device && metadata.ino() == directory.inode => Ok(true),
        Ok(_) => Err(std::io::Error::other("engine output directory has been replaced")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn output_bytes(path: &Path, device: u64) -> std::io::Result<u64> {
    let metadata = path.symlink_metadata()?;
    if metadata.dev() != device { return Err(std::io::Error::other("output crosses a filesystem boundary")); }
    if !metadata.is_dir() { return Ok(metadata.len()); }
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path)? {
        bytes = bytes.saturating_add(output_bytes(&entry?.path(), device)?);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
