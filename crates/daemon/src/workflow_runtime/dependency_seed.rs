//! Builders start from landed main's build output instead of from nothing (ov-1516).
//!
//! Four builders each compiling every third-party crate cold took 30 to 50 GB apiece. When the adapter names a
//! `dependency_seed` command, the engine runs it after each landing in a private checkout of the landed commit,
//! with `FARCOOLER_SEED` set to an empty folder under the engine root. Builders never get that folder: agents can
//! write anywhere under /tmp, so the seed is made and kept where only the engine writes. Each Build or Review step's
//! scratch then starts as an APFS clone of the newest seed, which shares its blocks until a builder writes.
//!
//! Every folder here is named in the ledger before it is made, and only a folder the ledger names is ever deleted,
//! by that exact path, once no process has a file open in it.
use super::{CodingCommand, Error, Exec, Result, adapters, coding, durable};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SEEDS: &str = "dependency-seed";
const LEDGER: &str = "ledger.json";

#[derive(Clone, Default)]
pub(super) struct Seeds(std::sync::Arc<std::sync::Mutex<SeedState>>);

#[derive(Default)]
struct SeedState {
    /// A seed build is under way; one at a time per engine.
    building: bool,
    /// The last commit whose seed failed, so the next landing tries again but the same commit doesn't.
    failed: Option<String>,
}

/// What the engine made under its seed folder: the seed builders clone, and folders still to remove.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Ledger {
    /// The commit whose finished seed builders start from.
    pub current: Option<String>,
    /// Folders this engine made and no longer needs: an earlier seed, or a build that stopped part-way.
    pub retired: Vec<String>,
}

impl Exec {
    fn seeds_dir(&self) -> PathBuf { self.root.join(SEEDS) }
    fn seed_path(&self, commit: &str) -> PathBuf { self.seeds_dir().join(commit) }
    fn seed_partial(&self, commit: &str) -> PathBuf { self.seeds_dir().join(format!("{commit}.partial")) }
    fn seed_checkout(&self, commit: &str) -> PathBuf { self.seeds_dir().join(format!("{commit}.checkout")) }

    pub(super) fn seed_ledger(&self) -> Result<Ledger> {
        match std::fs::read(self.seeds_dir().join(LEDGER)) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
            Err(error) => Err(error.into()),
        }
    }
    fn write_seed_ledger(&self, ledger: &Ledger) -> Result<()> {
        let next = self.seeds_dir().join(format!("{LEDGER}.next"));
        let _ = std::fs::remove_file(&next);
        durable(&next, &serde_json::to_vec(ledger)?)?;
        std::fs::rename(&next, self.seeds_dir().join(LEDGER))?;
        Ok(())
    }

    /// Start a seed of `commit` in the background, unless one is under way, `commit` already has one or its
    /// seed already failed. Called when a landing push lands, and when a build starts with no seed at all.
    pub(super) fn refresh_seed(&self, commit: &str) -> Option<tokio::task::JoinHandle<()>> {
        let command = self.coding.as_ref()?.dependency_seed.clone()?;
        if !cfg!(target_os = "macos") || !coding::sha(commit) { return None; }
        {
            let mut state = self.seeds.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = self.seed_ledger().ok()?.current;
            if state.building || current.as_deref() == Some(commit) || state.failed.as_deref() == Some(commit) { return None; }
            state.building = true;
        }
        let (exec, commit) = (self.clone(), commit.to_string());
        Some(tokio::spawn(async move {
            let result = exec.build_seed(&command, &commit).await;
            let mut state = exec.seeds.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.building = false;
            match result {
                Ok(true) => tracing::info!(%commit, "builders now start from this commit's dependency seed"),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(%commit, %error, "the dependency seed failed; builders start cold until the next landing");
                    state.failed = Some(commit);
                }
            }
        }))
    }

    /// Build the seed for `commit`. `Ok(false)` when there isn't the disk for it yet.
    async fn build_seed(&self, command: &CodingCommand, commit: &str) -> Result<bool> {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(self.seeds_dir())?;
        self.remove_retired_seeds().await;
        // Cold every time, a seed writes its whole peak before it strips, and keeps it while it pauses for a
        // landing check: it starts only when that check would still have its need with the seed at full size.
        if let Some(free) = self.short_of_space(&self.root, super::builder_resources::SEED_MIN_FREE)? {
            tracing::info!(%commit, free, "no dependency seed until a landing check would still have room beside it");
            return Ok(false);
        }
        let (partial, checkout) = (self.seed_partial(commit), self.seed_checkout(commit));
        // Recorded before they exist, so a crash part-way leaves only folders the ledger names.
        let mut ledger = self.seed_ledger()?;
        for path in [&partial, &checkout] {
            let path = path.to_string_lossy().into_owned();
            if !ledger.retired.contains(&path) { ledger.retired.push(path); }
        }
        self.write_seed_ledger(&ledger)?;
        let name = checkout.to_str().ok_or(Error::Refused("invalid seed checkout path"))?;
        adapters::git(&self.repository(), &["worktree", "add", "--detach", name, commit]).await?;
        let built = async {
            std::fs::DirBuilder::new().mode(0o700).create(&partial)?;
            let mut environment = command.environment.clone();
            environment.insert("FARCOOLER_SEED".into(), partial.to_string_lossy().into_owned());
            let log = self.seeds_dir().join(format!("{commit}.log"));
            let _ = std::fs::remove_file(&log);
            let (_keep, cancel) = tokio::sync::watch::channel(false);
            let exit = crate::effect_runner::owned_yielding(&checkout, &command.argv, &environment, &log, Duration::from_millis(command.timeout_ms),
                cancel, None, Some(&self.yielding)).await.map_err(|failure| Error::Io(failure.error))?;
            if exit.code != Some(0) { return Err(Error::Refused("the dependency seed command failed; its log is beside the seed")); }
            Ok(())
        }.await;
        let _ = adapters::git(&self.repository(), &["worktree", "remove", "--force", name]).await;
        if let Err(error) = built {
            self.remove_retired_seeds().await;
            return Err(error);
        }
        let seed = self.seed_path(commit);
        std::fs::rename(&partial, &seed)?;
        let mut ledger = self.seed_ledger()?;
        {
            // Builders read `current` and clone under this lock, so an old seed is never removed mid-clone.
            let _state = self.seeds.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(old) = ledger.current.replace(commit.into()) { ledger.retired.push(self.seed_path(&old).to_string_lossy().into_owned()); }
            ledger.retired.retain(|path| Path::new(path) != partial);
            self.write_seed_ledger(&ledger)?;
        }
        self.remove_retired_seeds().await;
        Ok(true)
    }

    /// Remove the folders the ledger retired, each by its exact path, skipping any a process has files open in.
    /// Only the one seed build under way calls this, so it never races a build's own folders.
    async fn remove_retired_seeds(&self) {
        let Ok(ledger) = self.seed_ledger() else { return };
        let mut kept = Vec::new();
        for path in &ledger.retired {
            let folder = PathBuf::from(path);
            // Only this engine's own seed folders: never a path the ledger was edited to name.
            if folder.parent() != Some(self.seeds_dir().as_path()) {
                tracing::warn!(path, "a dependency seed ledger entry is outside the seed folder; left alone");
                continue;
            }
            match folder.symlink_metadata() {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Ok(metadata) if !metadata.is_dir() => {
                    tracing::warn!(path, "a dependency seed ledger entry isn't a folder; left alone");
                    continue;
                }
                _ => {}
            }
            if let Err(why) = self.check_open_files(&folder).await {
                tracing::info!(path, why, "a retired dependency seed is still open; removing it later");
                kept.push(path.clone());
                continue;
            }
            // No lock: builders clone only `current`, and a folder is retired only after `current` moved off it
            // under the lock. A failed seed's partial output can be tens of GB, so it goes off the async workers.
            let removed = match tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&folder)).await {
                Ok(Err(error)) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
                Ok(_) => Ok(()),
            };
            if let Err(error) = removed {
                tracing::warn!(path, %error, "could not remove a retired dependency seed");
                kept.push(path.clone());
            }
        }
        if kept != ledger.retired {
            // Read again under the lock: a build may have finished meanwhile.
            let _state = self.seeds.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Ok(mut now) = self.seed_ledger() {
                now.retired.retain(|path| kept.contains(path) || !ledger.retired.contains(path));
                if let Err(error) = self.write_seed_ledger(&now) { tracing::warn!(%error, "could not record removed seeds"); }
            }
        }
    }

    /// Clone the current seed into a new, empty scratch folder. Best effort: without a seed, or off APFS, the
    /// builder simply starts cold. Starts a seed of `base` when there is none yet.
    pub(super) async fn seed_scratch(&self, scratch: &Path, base: &str) {
        if self.coding.as_ref().is_none_or(|c| c.dependency_seed.is_none()) { return; }
        let (exec, scratch) = (self.clone(), scratch.to_path_buf());
        let cloned = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            let _state = exec.seeds.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(commit) = exec.seed_ledger()?.current else { return Ok(None) };
            let seed = exec.seed_path(&commit);
            for entry in std::fs::read_dir(&seed)? {
                let entry = entry?;
                clone_tree(&entry.path(), &scratch.join(entry.file_name()))?;
            }
            Ok(Some(commit))
        }).await;
        let current = match cloned {
            Ok(Ok(current)) => current,
            Ok(Err(error)) => { tracing::warn!(%error, "could not clone the dependency seed; this build starts cold"); None }
            Err(error) => { tracing::warn!(%error, "the dependency seed clone stopped"); None }
        };
        // Only when there is none: a rebuild after a send-back keeps an older base, and its seed
        // would replace a newer landing's. Landings refresh it from then on.
        if current.is_none() { drop(self.refresh_seed(base)); }
    }
}

/// An APFS clone of `from` at `to`, which must not exist. A directory is cloned whole.
#[cfg(target_os = "macos")]
fn clone_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())?;
    // clonefile(2)'s CLONE_NOFOLLOW, which libc doesn't name: a symlink is cloned as itself.
    const CLONE_NOFOLLOW: u32 = 0x0001;
    // SAFETY: both are NUL-terminated paths that live across the call.
    if unsafe { libc::clonefile(from.as_ptr(), to.as_ptr(), CLONE_NOFOLLOW) } == 0 { Ok(()) }
    else { Err(std::io::Error::last_os_error()) }
}
#[cfg(not(target_os = "macos"))]
fn clone_tree(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("dependency seeds need APFS clones"))
}

#[cfg(test)]
mod tests;
