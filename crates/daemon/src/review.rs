//! Review, on the daemon side.
//!
//! Holds the change-set cache and its invalidation.
//!
//! `watch.rs` samples tmux panes for agent activity and observes neither file
//! content nor ref movement, so this brings its own: a two-syscall gate on every
//! tick, precise invalidation when an agent writes through the daemon's own
//! filesystem service, and an explicit Refresh because no watcher is perfect.
//!
//! Prompt composition and attachments used to live here too, and went with the
//! review buffer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use farcooler_core::Result;
use uuid::Uuid;

use crate::change_set::{BaseSource, ChangeSet, change_set, worktree_digest};

pub fn now_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A cached change set plus the cheap facts that decide whether it is still true.
#[derive(Debug, Clone)]
pub struct CachedChangeSet {
    pub set: ChangeSet,
    pub version: u64,
    /// mtime of `HEAD` and of the index, in nanoseconds. The two-syscall gate:
    /// a commit, a rebase, a checkout or a `git add` all move one of them, and
    /// checking them costs nothing on a quiet fleet.
    pub gate: (u128, u128),
    pub computed_at: i64,
}

/// The change-set cache.
///
/// Keyed by workspace and branch, because a workspace can be asked about a
/// stack link it does not have checked out.
#[derive(Default)]
pub struct ReviewCache {
    entries: Mutex<HashMap<(Uuid, String), CachedChangeSet>>,
    next_version: Mutex<u64>,
    /// When an agent last wrote into each workspace.
    ///
    /// The two-syscall gate cannot see a file edited in place — HEAD and the
    /// index both sit still — and that is the MOST common change there is. The
    /// inbox needs to know about it without running `git status` per workspace
    /// per tick, so the daemon records the moment it serves an agent's write
    /// through its own filesystem service. Precise, free, and it covers the case
    /// that matters: the agents doing the work are ACP clients of this daemon.
    touched: Mutex<HashMap<Uuid, i64>>,
    /// Per-workspace `(gate, files, insertions, deletions)` for the sidebar.
    ///
    /// Keyed by the cheap gate, so a quiet worktree costs two stats and no git
    /// at all. This is what makes "diff status across every worktree at a glance"
    /// affordable rather than a fleet-wide `git` loop on a timer.
    shortstats: Mutex<HashMap<Uuid, CachedShortstat>>,
}

/// The gate the numbers were computed at, and the numbers.
///
/// Named rather than written inline: nested tuples that deep say nothing about
/// which `u32` is which, and the gate half is a pair whose meaning lives on
/// `cheap_gate` below.
type CachedShortstat = ((u128, u128), (u32, u32, u32));

/// mtimes of the two files that move whenever git does something structural.
///
/// Deliberately NOT a `git status`: this runs on every tick for every workspace
/// with a review surface open, and porcelain output is the expensive call. This
/// is two `stat`s, and it is only a gate — anything it lets through is verified
/// by the real computation behind it.
pub fn cheap_gate(worktree: &Path) -> (u128, u128) {
    fn mtime(p: PathBuf) -> u128 {
        std::fs::metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
    // In a linked worktree `.git` is a FILE pointing at the real directory, so
    // HEAD and index live beside it there rather than here. Reading the pointer
    // is one open of a very small file, and only on the miss path.
    let dot_git = worktree.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        std::fs::read_to_string(&dot_git)
            .ok()
            .and_then(|s| s.strip_prefix("gitdir: ").map(|p| PathBuf::from(p.trim())))
            .unwrap_or(dot_git)
    };
    (mtime(git_dir.join("HEAD")), mtime(git_dir.join("index")))
}

impl ReviewCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn bump(&self) -> u64 {
        let mut v = self.next_version.lock().unwrap_or_else(|e| e.into_inner());
        *v += 1;
        *v
    }

    /// Drop a workspace's cached sets.
    ///
    /// Called when an agent writes a file through the daemon's own filesystem
    /// service — precise, immediate, and it covers the common case, because the
    /// agents doing the work are ACP clients of this daemon.
    pub fn invalidate(&self, workspace_id: Uuid) {
        let mut e = self.entries.lock().unwrap_or_else(|x| x.into_inner());
        e.retain(|(ws, _), _| *ws != workspace_id);
        drop(e);
        self.touched
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .insert(workspace_id, now_millis());
    }

    /// When this workspace was last written to through the daemon.
    pub fn touched_at(&self, workspace_id: Uuid) -> Option<i64> {
        self.touched.lock().unwrap_or_else(|x| x.into_inner()).get(&workspace_id).copied()
    }

    /// Files changed and +/- for one workspace, recomputed only when the cheap
    /// gate says something moved.
    pub async fn shortstat(
        &self,
        workspace_id: Uuid,
        worktree: &Path,
        base_ref: &str,
    ) -> Option<(u32, u32, u32)> {
        let gate = cheap_gate(worktree);
        {
            let m = self.shortstats.lock().unwrap_or_else(|x| x.into_inner());
            if let Some((cached_gate, stats)) = m.get(&workspace_id) {
                if *cached_gate == gate {
                    return Some(*stats);
                }
            }
        }
        let stats = match crate::change_set::shortstat(worktree, base_ref).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(?e, ?worktree, base_ref, "shortstat failed");
                return None;
            }
        };
        self.shortstats
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .insert(workspace_id, (gate, stats));
        Some(stats)
    }

    /// The digest of an already-cached change set, if there is one.
    ///
    /// Free: no git runs. Lets the inbox notice an in-place edit for any
    /// workspace someone has actually been looking at, without paying for the
    /// ones nobody has opened.
    pub fn cached_digest(&self, workspace_id: Uuid, branch: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .get(&(workspace_id, branch.to_string()))
            .map(|c| c.set.worktree_digest.clone())
    }

    /// The change set, computed only if something moved.
    pub async fn get(
        &self,
        workspace_id: Uuid,
        worktree: &Path,
        branch: &str,
        base_ref: &str,
        base_source: BaseSource,
        fresh: bool,
    ) -> Result<CachedChangeSet> {
        let key = (workspace_id, branch.to_string());
        let gate = cheap_gate(worktree);

        if !fresh {
            let cached = {
                let e = self.entries.lock().unwrap_or_else(|x| x.into_inner());
                e.get(&key).cloned()
            };
            if let Some(c) = cached {
                if c.gate == gate {
                    // The gate cannot see a file edited in place, so it is not
                    // trusted on its own for long: the digest is recomputed and
                    // compared, which reads only the files git already calls
                    // dirty.
                    let digest = worktree_digest(worktree, &c.set.head_commit).await?;
                    if digest == c.set.worktree_digest {
                        return Ok(c);
                    }
                }
            }
        }

        let set = change_set(worktree, branch, base_ref, base_source).await?;
        let cached = CachedChangeSet {
            set,
            version: self.bump(),
            gate,
            computed_at: now_millis(),
        };
        let mut e = self.entries.lock().unwrap_or_else(|x| x.into_inner());
        e.insert(key, cached.clone());
        Ok(cached)
    }
}

/// How much of one workspace's snapshots the daemon will hold.
///
/// A per-entry cap alone is not a budget: sixty anchored comments in a heavy
/// review is fifteen megabytes of file copies, on top of the attachments and on
/// top of the replay buffers `TODOS.md` already names as the dominant term in
/// daemon memory.
pub const MAX_SNAPSHOT_BYTES_PER_WORKSPACE: usize = 8 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cheap_gate_moves_when_head_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("index"), "x").unwrap();

        let before = cheap_gate(dir.path());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(git.join("HEAD"), "ref: refs/heads/other\n").unwrap();
        let after = cheap_gate(dir.path());

        assert_ne!(before, after, "a checkout must be visible to the gate");
    }

    /// A worktree's `.git` is a FILE. Without following it the gate reads two
    /// missing files, returns (0, 0) forever, and every linked worktree — that
    /// is, every workspace this product creates — caches a stale change set.
    #[test]
    fn the_cheap_gate_follows_a_linked_worktrees_gitdir_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real-git-dir");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(real.join("index"), "x").unwrap();

        let wt = dir.path().join("worktree");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();

        assert_ne!(cheap_gate(&wt), (0, 0), "the pointer must be followed");
    }
}
