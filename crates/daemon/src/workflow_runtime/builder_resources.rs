//! Builders yield to landing (ov-1214), and a new build waits while starting it could leave a
//! landing check without the disk it needs (ov-1516).
use super::*;

const GIB: u64 = 1024 * 1024 * 1024;
/// What `scripts/gate_disk.py` admits a Rust landing check with: its 36 GB budget and the
/// daemon's 25 GB reserve.
pub(super) const LANDING_NEED: u64 = 61 * GIB;
/// Room for the builds already running to grow while the new one starts.
pub(super) const BUILD_MARGIN: u64 = 10 * GIB;
pub(super) const MIN_FREE: u64 = LANDING_NEED + BUILD_MARGIN;
/// The most a dependency seed writes before it strips the workspace's own output: a cold
/// full-workspace test and binary build, measured at 17.5 GiB with line
/// tables only (26.1 GiB with full debug info).
pub(super) const SEED_PEAK: u64 = 20 * GIB;
/// A seed holds its whole peak while it pauses for a landing check, so it starts only when the
/// check would still have its need with the seed at full size.
pub(super) const SEED_MIN_FREE: u64 = LANDING_NEED + SEED_PEAK + BUILD_MARGIN;
pub(super) const LOW_DISK_PREFIX: &str = "resource hold: Waiting for disk space to start this build.";

fn free_bytes(path: &Path) -> std::io::Result<u64> {
    let stat = rustix::fs::statvfs(path)?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// What the ticket says while its build waits for space. The runner checks again about once a minute.
pub(super) fn low_disk(free: u64) -> String {
    format!("{LOW_DISK_PREFIX} {} GB is free, and builds start at {} GB so a landing check keeps the {} GB it needs. \
        It starts by itself once space is free.", free / GIB, MIN_FREE / GIB, LANDING_NEED / GIB)
}

impl Exec {
    /// The free bytes on the fuller of the engine's volume and the build's worktree, when they're
    /// under what a new build needs; `None` when there's room.
    pub(super) fn build_short_of_space(&self, tree: &Path) -> Result<Option<u64>> { self.short_of_space(tree, MIN_FREE) }

    /// The same, for a build that needs `need` free to start.
    pub(super) fn short_of_space(&self, tree: &Path, need: u64) -> Result<Option<u64>> {
        #[cfg(test)]
        if let Some(bytes) = self.build_free_bytes { return Ok((bytes < need).then_some(bytes)); }
        let free = free_bytes(&self.root)?.min(free_bytes(tree)?);
        Ok((free < need).then_some(free))
    }
}

#[cfg(test)]
mod tests;
