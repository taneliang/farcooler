//! Builders yield to landing and need 25 GiB free before launch (ov-1214).
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
}

#[cfg(test)]
mod tests;
