//! Review, on the daemon side.
//!
//! Holds the change-set cache and its invalidation.
//!
//! `watch.rs` samples tmux panes for agent activity and observes neither file
//! content nor ref movement, so this brings its own: a two-syscall gate,
//! invalidation when the daemon itself changes something under a worktree, and
//! an explicit Refresh because no watcher is perfect.
//!
//! That gate used to run on every tick for every worktree. It does not any
//! more: `fs_watch` watches the same two files and tells the watch loop when
//! they move, and `cheap_gate` is now the key a demand-driven `get` compares
//! against plus the fallback for a worktree the watcher could not register.
//!
//! The sidebar's `+N -M` is computed here but no longer decided here. It costs
//! a `git diff` and a `git status`, which is affordable once per worktree that
//! moved and ruinous once per worktree per client poll, so the watch loop owns
//! when to spend it and everything else reads `counts`.
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
    /// When this daemon last did something to each workspace that its own
    /// caches must not survive.
    ///
    /// The two-syscall gate cannot see a file edited in place — HEAD and the
    /// index both sit still — and that is the MOST common change there is, so
    /// this exists to record the ones the daemon is in a position to know about
    /// first-hand, free and precisely.
    ///
    /// It is worth being honest about how much that is: `invalidate` is the only
    /// writer, and its only caller today is `changes.set_base`. An agent editing
    /// a file writes it with its own tools in its own pane, not through this
    /// process, so nothing here hears about it. What DOES see that is the
    /// filesystem, which `fs_watch` now listens to, and the pane sampler, which
    /// is already reading every pane once a second and knows which of them has
    /// an agent working in it — see `Watcher::probe_change_sets`, where both are
    /// gates that earn the `git` call and this stamp is neither.
    touched: Mutex<HashMap<Uuid, i64>>,
    /// Per-workspace sidebar counts, as the watch loop last computed them.
    ///
    /// Written by `Watcher::probe_change_sets` and read by everything else. The
    /// inbox used to compute these itself, once per RPC call per worktree, which
    /// was affordable only while the numbers were committed-only; a client that
    /// polls an RPC running git per workspace is the thing that does not scale.
    shortstats: Mutex<HashMap<Uuid, CachedShortstat>>,
}

/// The numbers, and what they were computed against.
struct CachedShortstat {
    key: ShortstatKey,
    counts: Counts,
    /// Set when something outside this cache has PROVED the worktree moved.
    ///
    /// The key cannot express that on its own. A change set recomputed because
    /// its digest moved — and the digest reads the contents of every dirty file
    /// — is proof of an edit that left both mtimes exactly where they were, and
    /// there is nothing in a pair of mtimes that can say so.
    ///
    /// The numbers are marked rather than dropped, and that matters twice: they
    /// are what the next probe compares against to decide whether anything is
    /// worth announcing, and a row that blinks to `+0 -0` for the one tick
    /// before that probe lands is a worse answer than a three-second-old one.
    stale: bool,
}

/// What a worktree's counts were computed against.
///
/// The two mtimes alone were enough while the numbers were committed-only:
/// nothing commits without moving `HEAD`. Now that they include uncommitted
/// work, an agent editing a file in place moves neither of them — so a cache
/// keyed on the gate alone would go on serving the numbers from before the edit
/// for as long as the agent worked, which is the exact staleness live counts
/// exist to remove. `touched` is the other half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShortstatKey {
    /// See `cheap_gate`.
    gate: (u128, u128),
    /// The stamp `invalidate` leaves — see `touched` on the cache itself for
    /// what does and does not reach it.
    touched: Option<i64>,
}

/// What this daemon knows about one worktree's `+N -M`.
///
/// Three answers rather than a number with zero standing in for the other two,
/// because neither of those is a number. A worktree nobody has looked at yet and
/// a worktree with no base to compare against have both said nothing, and
/// neither of them has said "nothing changed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counts {
    /// The watch loop has not reached this workspace yet. True for the first
    /// tick after the daemon starts, and after a worktree appears.
    Unknown,
    /// Probed, and nothing resolved as a base. There is no comparison to
    /// report, which is not the same as a comparison that came out empty — see
    /// the base note in `Watcher::probe_change_sets`.
    NoBase,
    /// Files changed, insertions, deletions.
    Known(u32, u32, u32),
}

/// mtimes of the two files that move whenever git does something structural.
///
/// Deliberately NOT a `git status`: this is on the path of every request for a
/// change set, and porcelain output is the expensive call. This is two `stat`s,
/// and it is only a gate — anything it lets through is verified by the real
/// computation behind it.
///
/// It is no longer what the watch loop asks on a clock. `fs_watch` registers a
/// watch on the same two files, so the sampler is TOLD when they move rather
/// than looking every three seconds per worktree; this stays as the key
/// `ReviewCache::get` compares against, which is demand-driven, and as the
/// fallback gate for a worktree the watcher reports it does not cover. See
/// `Watcher::probe_change_sets`.
pub fn cheap_gate(worktree: &Path) -> (u128, u128) {
    fn mtime(p: PathBuf) -> u128 {
        std::fs::metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
    let git_dir = git_dir(worktree);
    (mtime(git_dir.join("HEAD")), mtime(git_dir.join("index")))
}

/// Where this worktree's own `HEAD` and `index` live.
///
/// In a linked worktree `.git` is a FILE pointing at the real directory, so
/// they live beside it there rather than here. Reading the pointer is one open
/// of a very small file.
///
/// Shared with `fs_watch`, which registers a watch on the same directory that
/// `cheap_gate` stats. Two copies of this would be two answers about which
/// directory a worktree's git state is in, and the one that was wrong would be
/// wrong silently — a gate that never fires, or a watch on nothing.
pub fn git_dir(worktree: &Path) -> PathBuf {
    let dot_git = worktree.join(".git");
    if dot_git.is_dir() {
        return dot_git;
    }
    std::fs::read_to_string(&dot_git)
        .ok()
        .and_then(|s| s.strip_prefix("gitdir: ").map(|p| PathBuf::from(p.trim())))
        .unwrap_or(dot_git)
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
        // The sidebar counts go stale with it. Leaving them alone was safe while
        // they were committed-only, because nothing can commit without moving
        // the half of their key that `cheap_gate` reads. They include
        // uncommitted work now, and the one caller here is a base change — which
        // makes every number computed against the old base wrong rather than
        // merely old.
        self.mark_counts_stale(workspace_id);
        self.touched
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .insert(workspace_id, now_millis());
    }

    /// When this workspace was last written to through the daemon.
    pub fn touched_at(&self, workspace_id: Uuid) -> Option<i64> {
        self.touched.lock().unwrap_or_else(|x| x.into_inner()).get(&workspace_id).copied()
    }

    /// Files changed and +/- for one workspace, as last computed.
    ///
    /// Free: this never runs git, and never can. The inbox answers every row
    /// from here, which is what lets a client poll it.
    pub fn counts(&self, workspace_id: Uuid) -> Counts {
        self.shortstats
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .get(&workspace_id)
            .map(|c| c.counts)
            .unwrap_or(Counts::Unknown)
    }

    /// Whether the cached counts still describe this worktree, as far as
    /// anything free can tell.
    ///
    /// Two `stat`s and two map lookups; no git, ever. Asked before anything
    /// else, because even resolving a base can run git for a repository whose
    /// default branch has never been read — and a worktree that has not moved
    /// must cost nothing at all.
    ///
    /// The FALLBACK gate now: the watch loop asks it only for a worktree
    /// `fs_watch` reports it does not cover, because two `stat`s per worktree
    /// every three seconds is still a poll, and an idle fleet is meant to make
    /// no syscall at all. See `counts_unproven`, which is the half of this that
    /// no watcher can replace and which every worktree still pays.
    ///
    /// False for a workspace that has never been probed, which is how the first
    /// pass after the daemon starts reaches every worktree.
    pub fn counts_current(&self, workspace_id: Uuid, worktree: &Path) -> bool {
        let key = self.shortstat_key(workspace_id, worktree);
        self.shortstats
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .get(&workspace_id)
            .is_some_and(|c| !c.stale && c.key == key)
    }

    /// Whether this workspace needs recomputing for a reason no filesystem
    /// event will ever report.
    ///
    /// The half of `counts_current` that costs nothing at all — no `stat`, one
    /// map lookup — and the half a watcher cannot replace. Two things reach it:
    /// a workspace nobody has probed yet, which is how the first pass after a
    /// daemon start reaches every worktree; and a workspace this daemon marked
    /// stale itself, which is a base changing or a change set recomputed for a
    /// client. Neither of those is a write to a file, so neither moves under a
    /// watch.
    ///
    /// Kept apart from `counts_current` because a watched worktree must not pay
    /// its two `stat`s. Two per worktree every three seconds is not much, and
    /// it is still a poll on a fleet whose whole property is that an idle
    /// runner does nothing.
    pub fn counts_unproven(&self, workspace_id: Uuid) -> bool {
        self.shortstats
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .get(&workspace_id)
            .is_none_or(|c| c.stale)
    }

    /// Say that this workspace's counts no longer describe its worktree.
    ///
    /// For the callers that know something the key cannot express — see
    /// `CachedShortstat::stale`. The next probe pass recomputes; nothing here
    /// runs git, because both callers are already on a path that is spending it.
    fn mark_counts_stale(&self, workspace_id: Uuid) {
        if let Some(c) =
            self.shortstats.lock().unwrap_or_else(|x| x.into_inner()).get_mut(&workspace_id)
        {
            c.stale = true;
        }
    }

    /// Recompute one workspace's counts and cache them.
    ///
    /// `base_ref` is `None` for a worktree nothing resolved a base for; that is
    /// recorded rather than treated as a failure, so the inbox can leave the row
    /// out instead of claiming it changed nothing.
    ///
    /// Returns a fresh change-set version when the numbers came out DIFFERENT
    /// from the ones already cached, and `None` otherwise — which is the common
    /// answer, and the reason a fleet where an agent is thinking rather than
    /// writing broadcasts nothing. The first probe of a workspace returns `None`
    /// too: nobody is showing a number for it yet, and the read a client is
    /// already making will carry it.
    pub async fn recompute_counts(
        &self,
        workspace_id: Uuid,
        worktree: &Path,
        base_ref: Option<&str>,
    ) -> Option<u64> {
        let key = self.shortstat_key(workspace_id, worktree);
        let counts = match base_ref {
            Some(base) => match crate::change_set::shortstat(worktree, base).await {
                Ok((files, ins, del)) => Counts::Known(files, ins, del),
                Err(e) => {
                    tracing::warn!(?e, ?worktree, base, "shortstat failed");
                    // Left exactly as it was, key and all, so the next pass tries
                    // again. A call that failed knows nothing, and the last true
                    // numbers beat a confident zero.
                    return None;
                }
            },
            None => Counts::NoBase,
        };

        // Read under the same lock the write takes, so two passes racing on one
        // workspace cannot both decide they were the one that moved it.
        let mut m = self.shortstats.lock().unwrap_or_else(|x| x.into_inner());
        let previous = m.insert(workspace_id, CachedShortstat { key, counts, stale: false });
        drop(m);

        previous.filter(|p| p.counts != counts).map(|_| self.bump())
    }

    /// Forget the counts of workspaces that are no longer there.
    ///
    /// Called from the same pass that computes them, on the same terms as the
    /// watcher's own `state.retain`: a worktree that was removed and later
    /// re-registered would otherwise inherit the numbers of the one it replaced.
    pub fn retain_counts(&self, live: &std::collections::HashSet<Uuid>) {
        self.shortstats
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .retain(|id, _| live.contains(id));
    }

    fn shortstat_key(&self, workspace_id: Uuid, worktree: &Path) -> ShortstatKey {
        ShortstatKey { gate: cheap_gate(worktree), touched: self.touched_at(workspace_id) }
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
        // Reaching here is proof this worktree moved: either the gate said so,
        // or the digest — which reads the CONTENTS of every dirty file — said so
        // when the gate could not. The sidebar's counts are computed from the
        // same worktree on a different clock and have no way to learn that on
        // their own, and a diff pane showing a file next to a row still reporting
        // the numbers from before it is one worktree described two ways. Marking
        // them costs nothing here and the next probe pass picks it up;
        // recomputing them inline would put a second `git diff` and a second
        // `git status` on the path of drawing a diff.
        self.mark_counts_stale(workspace_id);

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

    /// A bare git directory, enough for `cheap_gate` to read.
    fn gated(dir: &Path) -> PathBuf {
        let git = dir.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("index"), "x").unwrap();
        dir.to_path_buf()
    }

    /// The counts are keyed on more than the two mtimes now, and they have to
    /// be: a file edited in place moves neither, which is the most common change
    /// there is. Everything a daemon-served write does to that key is checked
    /// here, because a cache that cannot go stale is a cache that shows the
    /// numbers from before the edit for as long as the work lasts.
    #[tokio::test]
    async fn a_write_the_daemon_served_leaves_no_countable_numbers_behind() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = gated(dir.path());
        let cache = ReviewCache::new();
        let ws = Uuid::now_v7();

        assert!(
            !cache.counts_current(ws, &worktree),
            "a worktree nobody has probed is never current, which is how the first pass reaches it"
        );
        assert_eq!(cache.counts(ws), Counts::Unknown);

        // `None` for the base rather than a real repository: this is a test of
        // the key, and the git half has its own tests against a real one.
        cache.recompute_counts(ws, &worktree, None).await;
        assert_eq!(cache.counts(ws), Counts::NoBase);
        assert!(cache.counts_current(ws, &worktree));

        cache.invalidate(ws);
        assert!(
            !cache.counts_current(ws, &worktree),
            "invalidate cleared the change set and left the counts standing, which under live \
             counts is the staleness it exists to prevent"
        );
        assert_eq!(
            cache.counts(ws),
            Counts::NoBase,
            "and the last numbers are still readable, so a row does not blink to zero for the \
             one tick before the probe lands"
        );
    }

    /// The other half of the key, unchanged in meaning: a commit, a rebase or a
    /// checkout still has to be enough on its own.
    #[tokio::test]
    async fn a_commit_leaves_the_counts_stale_without_anything_telling_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = gated(dir.path());
        let cache = ReviewCache::new();
        let ws = Uuid::now_v7();

        cache.recompute_counts(ws, &worktree, None).await;
        assert!(cache.counts_current(ws, &worktree));

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(worktree.join(".git").join("HEAD"), "ref: refs/heads/other\n").unwrap();
        assert!(!cache.counts_current(ws, &worktree));
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
