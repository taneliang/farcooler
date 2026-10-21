//! Noticing that a worktree moved, without asking git.
//!
//! `log_watch` made this argument one layer up and it is the same argument
//! here: the daemon used to learn that a repository had moved by stat-ing
//! `.git/HEAD` and `.git/index` once every three seconds per worktree, and
//! that a worktree had been added or removed by stat-ing
//! `.git/worktrees` once a second per repository, forever — work spent even
//! while the fleet sits idle overnight. The filesystem can say when something
//! moved, so it says it.
//!
//! **This replaces the GATE, not the PROBE.** An event says a path under a
//! worktree changed; it does not say `+82 −13`. `probe_change_sets` still
//! spends its three git processes to produce the numbers. What goes away is
//! the guessing about *when* to spend them, and with it the polling that
//! guessing needed.
//!
//! Two disjoint signals, and neither implies the other. An agent editing a
//! file writes only that file, so `.git` never moves. A commit writes only
//! into `.git`, so the working tree is byte-identical before and after. The
//! working-tree half is what closes the gap `probe_change_sets` used to name
//! out loud: an edit made in a worktree with no pane open, nothing running,
//! by something this daemon never served.
//!
//! A sibling of `log_watch` rather than an extension of it. The log roots are
//! three fixed paths that exist for the life of the process; worktrees are
//! discovered from the store, come and go while the daemon runs, and each one
//! needs a registration walk of its own.
//!
//! ## The Linux watch ceiling, which is what shapes this module
//!
//! inotify takes **one watch per directory**, against a per-user ceiling of
//! `/proc/sys/fs/inotify/max_user_watches` — commonly 8192 on older systems
//! and 65536 or more on newer ones. A Rust `target/` or a node
//! `node_modules/` is tens of thousands of directories, multiplied by the
//! number of worktrees on the runner. So registration is **gitignore-aware**:
//! `git ls-files` names every path git cares about, and only the directories
//! on those paths are registered. The directories that would exhaust the
//! ceiling are precisely the ones git already ignores, which is why this one
//! move solves the watch count and the event flood together — and `target/`
//! churns hardest exactly while an agent is working, the busiest moment and
//! the one where a flood would hurt most.
//!
//! Filtering events after the fact instead would still pay the per-directory
//! watch cost and still receive the flood. It only hides it.
//!
//! ## Why the registration strategy is per-platform
//!
//! macOS FSEvents watches a whole tree for one registration and has no such
//! ceiling, and registering per directory there would be thousands of streams
//! (under `notify`, whose `FsEventWatcher::watch` rebuilds its one stream on
//! every call, it was thousands of stream restarts). Windows
//! `ReadDirectoryChangesW` is per-tree in the same way.
//! So on those platforms one recursive registration covers the worktree.
//!
//! On macOS that registration is a stream of our own rather than `notify`'s,
//! because one recursive registration means fseventsd sends everything under
//! the worktree — a `cargo build`'s `target/`, and in a main checkout every
//! nested `.claude/worktrees/*` with its own `target/` — and this module used
//! to drop it only after delivery. `notify` can neither name paths a stream
//! leaves out nor set its latency above zero. So each watched path is its own
//! FSEvents stream (`fs_watch/fsevents.rs`) that leaves out up to eight
//! directories git says are not part of the worktree's change set
//! (`fs_watch/exclusions.rs`), and batches what remains a quarter second at a
//! time.
//!
//! Everywhere else — inotify, and kqueue, which is worse still at one file
//! descriptor per FILE — the directory set is what gets registered, one
//! non-recursive watch at a time.
//!
//! **The directory set is kept and consulted either way**, as `Scope::Filtered`.
//! On the per-tree backends it is the ONLY thing making them gitignore-aware,
//! since their one registration receives the whole subtree. On the
//! per-directory ones almost nothing reaches it that was not registered — but
//! a main worktree's `.git` sits inside the worktree and is registered
//! separately for `HEAD` and `index`, and without the filter its own churn
//! would come back as the worktree having moved. That is not a hypothetical:
//! see `Scope::Entries`. What this module does NOT do is filter INSTEAD of
//! registering carefully, which would pay the per-directory watch cost and
//! receive the flood anyway.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use uuid::Uuid;

mod backend;
mod exclusions;
mod offload;
#[cfg(target_os = "macos")]
mod fsevents;
#[cfg(all(test, target_os = "macos"))]
mod exclusion_tests;
#[cfg(test)]
mod settle;
#[cfg(test)]
mod nested_tests;
#[cfg(test)]
mod offload_tests;

use backend::Backend;
pub(crate) use offload::ClearOnDrop;

/// Whether this platform charges per directory for a watch.
///
/// See the module header. A `const` rather than `cfg` around the call sites so
/// both strategies are type-checked on every platform — a registration path
/// that only compiles on Linux is a registration path nobody here can build.
#[cfg(any(target_os = "macos", target_os = "ios", windows))]
const PER_DIRECTORY: bool = false;
#[cfg(not(any(target_os = "macos", target_os = "ios", windows)))]
const PER_DIRECTORY: bool = true;

/// The most directories one worktree may contribute.
///
/// A bound on this module's own memory as much as on the kernel's watch table:
/// the directory set is kept for the life of the registration, on both
/// strategies, because it is also the filter. A worktree past this is not a
/// worktree this feature can be afforded on, and saying so out loud and
/// falling back is the whole point of `covered`.
const MAX_DIRS_PER_WORKTREE: usize = 20_000;

/// The fleet-wide ceiling where the platform does not tell us one.
///
/// Only reached on the per-tree backends, where a "watch" costs one stream and
/// this number is bounding memory rather than a kernel table.
const DEFAULT_FLEET_BUDGET: usize = 100_000;

/// Half of what the kernel will give this user, or a fixed bound off Linux.
///
/// Half rather than all: the daemon is not the only thing on the runner that
/// watches files — the user's editor, a dev server's reloader, and this
/// process's own `LogWatcher` are all drawing on the same per-user table — and
/// a daemon that took the last watch would break them to serve a sidebar
/// number.
fn fleet_budget() -> usize {
    match std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches") {
        Ok(raw) => match raw.trim().parse::<usize>() {
            // A floor, because a machine configured down to a few hundred
            // would otherwise register nothing at all and never say why. It
            // will hit the ceiling instead, which reports itself.
            Ok(n) => (n / 2).max(1024),
            Err(_) => DEFAULT_FLEET_BUDGET,
        },
        Err(_) => DEFAULT_FLEET_BUDGET,
    }
}

/// What a registered path belongs to.
///
/// Repositories and worktrees are separate subjects because they answer
/// different questions on different clocks: a repository's `worktrees`
/// directory drives `reconcile_worktrees`, and a worktree's tree and `.git`
/// drive `probe_change_sets`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Subject {
    /// A worktree's working tree, or the `.git` directory that worktree reads
    /// `HEAD` and `index` out of.
    Worktree(Uuid),
    /// A repository's `worktrees` directory — where a linked worktree appears
    /// and disappears.
    Repository(Uuid),
}

/// Which paths under `at` belong to this route's subject.
enum Scope {
    /// `at` itself and its immediate children, and nothing deeper.
    ///
    /// `<common>/worktrees` is registered non-recursively, but each linked
    /// worktree's own `<common>/worktrees/<name>` is registered separately for
    /// its `HEAD` and `index`. Without this bound, every commit in a linked
    /// worktree would also read as "the set of worktrees changed" and spend a
    /// reconcile on it.
    Shallow,
    /// Anything at or under `at` whose parent directory git cares about.
    ///
    /// What a worktree claims, on BOTH strategies. The per-tree one needs it
    /// to be gitignore-aware at all, since its single recursive registration
    /// receives the whole subtree. The per-directory one needs it for a
    /// narrower reason that is just as load-bearing: a main worktree's `.git`
    /// sits INSIDE the worktree and is registered separately, so a route that
    /// took everything under the root would claim `.git/index.lock` — and the
    /// `Entries` note below is what that costs.
    ///
    /// Paths are held RELATIVE to `at`, so one set answers for a route
    /// registered under a raw path and one registered under its canonicalized
    /// twin: macOS hands back `/private/var/...` for a watch registered on
    /// `/var/...`.
    Filtered(Arc<HashSet<PathBuf>>),
    /// Only these entries of `at`, by name.
    ///
    /// What the `.git` route claims, and it is deliberately the exact pair
    /// `review::cheap_gate` used to `stat`: this replaces that gate, so it
    /// answers for what that gate answered for and not a byte more.
    ///
    /// Being wider is not merely noisy here, it is a feedback loop. **git
    /// takes `index.lock` for the duration of a `git status`** — created and
    /// removed, both events on the `.git` directory — so a route that claimed
    /// every entry would make this daemon's own probe the event that triggers
    /// the next probe three seconds later, forever. That was measured before
    /// this existed: four worktrees, nothing happening, 400 git processes a
    /// minute. A real index change is not lost to the narrowing, because git
    /// writes `index.lock` and then RENAMES it over `index`, which is an event
    /// on `index`.
    ///
    /// It also keeps a repository's main worktree out of its linked worktrees'
    /// business: `<common>/worktrees/<name>/index` is under `<common>`, and a
    /// route that took everything under `<common>` would report every linked
    /// worktree's commit as a change to the main one.
    Entries(&'static [&'static str]),
}

/// The two files in a git directory whose mtimes gated everything before this.
///
/// See `Scope::Entries` and `review::cheap_gate`. `HEAD` moves on a checkout, a
/// rebase and a detach; `index` moves on a commit, a `git add`, a reset, and an
/// index refresh.
const GIT_GATE_ENTRIES: &[&str] = &["HEAD", "index"];

/// The parts of a git directory no route here reads, left out of its stream.
///
/// A git directory is watched non-recursively, but an FSEvents stream has no
/// such mode and receives the whole subtree — every loose object a commit or a
/// fetch writes. Only `HEAD`, `index` and the creation of `worktrees` matter,
/// and FSEvents still delivers an excluded path's own creation. The same list
/// for every git directory, so a main worktree's `.git` and its repository's
/// common directory, which are one path, share one stream.
const GIT_DIR_NOISE: &[&str] = &["objects", "logs", "refs", "lfs", "modules", "worktrees"];

/// `GIT_DIR_NOISE` under `git_dir`, spelled the way FSEvents compares them.
///
/// Empty where the backend has no exclusions to give: everywhere but macOS.
fn git_dir_exclusions(git_dir: &Path) -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let base = canonical_ish(git_dir).unwrap_or_else(|| git_dir.to_path_buf());
    GIT_DIR_NOISE.iter().map(|name| base.join(name)).collect()
}

struct Route {
    at: PathBuf,
    scope: Scope,
    subject: Subject,
}

impl Route {
    fn covers(&self, path: &Path) -> bool {
        match &self.scope {
            Scope::Shallow => path == self.at || path.parent() == Some(self.at.as_path()),
            Scope::Filtered(dirs) => match path.strip_prefix(&self.at) {
                // The root itself is in the set as the empty path, so a write
                // directly into the worktree root matches like any other.
                Ok(rel) => dirs.contains(rel.parent().unwrap_or(Path::new(""))),
                Err(_) => false,
            },
            Scope::Entries(names) => {
                path.parent() == Some(self.at.as_path())
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| names.contains(&n))
            }
        }
    }
}

/// What changed, coalesced in the data structure rather than by a timer.
///
/// The same shape as `LogWatcher`'s set and for the same reason: every event
/// inserts, and ten writes to one file during a burst are ten insertions of
/// one key with one entry to show for it. The window a debounce timer would
/// have to invent is just the gap between two drains, which is the caller's
/// cadence — three seconds for the change-set pass, one for the reconcile.
///
/// Worktrees keep their paths and repositories do not, because the two
/// callers ask different things of them: `probe_change_sets` needs the paths
/// to notice a directory git cares about that did not exist when the
/// registration walk ran, and `reconcile_worktrees` needs only the fact that
/// something under `worktrees` moved.
#[derive(Default)]
struct Changed {
    worktrees: HashMap<Uuid, HashSet<PathBuf>>,
    repositories: HashSet<Uuid>,
}

/// One subject's registration.
struct Registration {
    /// Every path handed to `watch`, so `unwatch` can hand back exactly those.
    watched: Vec<PathBuf>,
    /// The directories git cares about, relative to `root`. Also the filter
    /// set on the per-tree backends, and what `needs_rewalk` compares against.
    dirs: Arc<HashSet<PathBuf>>,
    /// The worktree root, and its canonicalized twin when they differ.
    roots: Vec<PathBuf>,
    /// What the root's watch leaves out, so a re-walk can tell whether it
    /// changed. Empty for a repository, and everywhere but macOS.
    excluded: Vec<PathBuf>,
    /// False when ANY part of this subject's registration did not take — the
    /// backend would not build, a `watch` call failed, the ceiling was hit, or
    /// git could not enumerate the tree. The caller reads this and keeps its
    /// own gates live for that subject: a daemon that silently stopped
    /// noticing edits is worse than the known, bounded gap it replaced.
    covered: bool,
}

impl Registration {
    fn relative<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        self.roots.iter().find_map(|root| path.strip_prefix(root).ok())
    }
}

/// The filesystem telling this daemon that a worktree, or its `.git`, moved.
pub struct TreeWatcher {
    /// Read by the backend's callback on its own thread, so nothing may hold
    /// it across a `watch` or `unwatch` call: those block on the backend's
    /// event thread (on macOS, stopping a stream waits on its delivery queue),
    /// and that thread is what is inside the callback.
    routes: Arc<Mutex<Vec<Route>>>,
    /// Written by the callback, drained by the two passes.
    changed: Arc<Mutex<Changed>>,
    /// Held, not read — a backend stops watching the moment it is dropped.
    /// Behind a mutex because unlike `LogWatcher`'s this one is registered
    /// against for as long as the daemon runs. `None` when the platform
    /// backend itself would not build, which is not a per-subject failure and
    /// has nothing left to retry: every subject is then uncovered and every
    /// caller falls back.
    /// Shared so `offload` can take it on a blocking thread.
    watcher: Arc<Mutex<Option<Backend>>>,
    book: Mutex<Book>,
    budget: usize,
    /// Every path the backend delivered, before routing, so a test can prove
    /// a path was never sent rather than merely dropped. macOS only, where
    /// the exclusions it checks exist.
    #[cfg(all(test, target_os = "macos"))]
    delivered: Arc<Mutex<Vec<PathBuf>>>,
}

#[derive(Default)]
struct Book {
    worktrees: HashMap<Uuid, Registration>,
    repositories: HashMap<Uuid, Registration>,
    /// Directories charged against `budget` across the whole fleet.
    spent: usize,
}

impl TreeWatcher {
    /// Build the backend, watching nothing.
    ///
    /// Deliberately registers no paths: on the per-tree backends every `watch`
    /// call restarts the event stream, and doing a fleet's worth of that
    /// inside `Watcher::new` would put it on the daemon's startup path.
    /// Registration happens from the passes that already enumerate the fleet.
    pub fn start() -> TreeWatcher {
        let routes: Arc<Mutex<Vec<Route>>> = Arc::new(Mutex::new(Vec::new()));
        let changed: Arc<Mutex<Changed>> = Arc::new(Mutex::new(Changed::default()));

        let table = routes.clone();
        let sink = changed.clone();
        #[cfg(all(test, target_os = "macos"))]
        let delivered: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        #[cfg(all(test, target_os = "macos"))]
        let record = delivered.clone();
        let built = Backend::new(Arc::new(move |paths: &[PathBuf]| {
            #[cfg(all(test, target_os = "macos"))]
            record.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(paths);
            route(&table, &sink, paths);
        }));

        let watcher = match built {
            Ok(w) => Some(w),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "could not start a worktree watcher; every worktree falls back to \
                     stat-polling its .git and to the activity gate"
                );
                None
            }
        };

        TreeWatcher {
            routes,
            changed,
            watcher: Arc::new(Mutex::new(watcher)),
            book: Mutex::new(Book::default()),
            budget: fleet_budget(),
            #[cfg(all(test, target_os = "macos"))]
            delivered,
        }
    }

    /// Whether this worktree's tree and `.git` are actually being watched.
    ///
    /// False keeps the caller's own gates live for it — see `Registration::covered`.
    pub fn covers_worktree(&self, id: Uuid) -> bool {
        self.book
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .worktrees
            .get(&id)
            .is_some_and(|r| r.covered)
    }

    /// Whether this repository's `worktrees` directory is actually being watched.
    pub fn covers_repository(&self, id: Uuid) -> bool {
        self.book
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .repositories
            .get(&id)
            .is_some_and(|r| r.covered)
    }

    /// Directories registered across the whole fleet.
    ///
    /// On inotify this IS the number of kernel watches this daemon holds — the
    /// backend adds one per directory whether they were registered one at a
    /// time or as one recursive walk — so it is the number to weigh against
    /// `/proc/sys/fs/inotify/max_user_watches`. Logged beside every
    /// registration for that reason: a runner that is running out should be
    /// able to see it coming in its own log rather than by reading this file.
    pub fn watched_directories(&self) -> usize {
        self.book.lock().unwrap_or_else(|e| e.into_inner()).spent
    }

    /// The worktrees something moved under, and which paths moved.
    ///
    /// Idempotent in the way `LogWatcher::drain` is: draining empties the map,
    /// so a second call with nothing written in between reports nothing.
    pub fn drain_worktrees(&self) -> HashMap<Uuid, HashSet<PathBuf>> {
        std::mem::take(&mut self.changed.lock().unwrap_or_else(|e| e.into_inner()).worktrees)
    }

    /// The repositories whose set of worktrees moved.
    pub fn drain_repositories(&self) -> HashSet<Uuid> {
        std::mem::take(&mut self.changed.lock().unwrap_or_else(|e| e.into_inner()).repositories)
    }

    /// Whether any of these paths is a directory this registration does not know.
    ///
    /// The one thing the registration walk cannot answer for on its own. A
    /// directory created after the walk ran is not in the set, so nothing
    /// written INSIDE it would ever surface — the creation of the directory
    /// itself does surface, because its parent is watched, and that is the
    /// event this reads. Answering yes costs the caller one `git ls-files` and
    /// buys back everything under the new directory.
    ///
    /// A `metadata` call per unknown path, which is a path this daemon was
    /// going to look at anyway: the flood that would make this expensive lives
    /// in the directories git ignores, and those are not watched.
    pub fn needs_rewalk(&self, id: Uuid, paths: &HashSet<PathBuf>) -> bool {
        let book = self.book.lock().unwrap_or_else(|e| e.into_inner());
        let Some(reg) = book.worktrees.get(&id) else { return false };
        paths.iter().any(|path| {
            let Some(rel) = reg.relative(path) else { return false };
            !reg.dirs.contains(rel) && path.is_dir()
        })
    }

    /// Bring the registered worktrees in line with the fleet.
    ///
    /// `rewalk` names worktrees whose directory set is known to be out of
    /// date — see `needs_rewalk`. Everything else is a set comparison and
    /// spends nothing: this is called from a pass that runs every three
    /// seconds, and an idle fleet must cost neither git nor syscalls here.
    ///
    /// When it DOES have work it does two slow things — a `git ls-files` per
    /// worktree, and a `watch` call per directory — so the caller runs it
    /// somewhere those are affordable. See `Watcher::probe_change_sets`, which
    /// is a detached pass for exactly this class of reason. The `watch` calls
    /// block, and run on a blocking thread rather than the caller's worker:
    /// see `offload`.
    pub async fn sync_worktrees(&self, fleet: &[(Uuid, PathBuf)], rewalk: &HashSet<Uuid>) {
        let live: HashSet<Uuid> = fleet.iter().map(|(id, _)| *id).collect();
        let gone: Vec<Uuid> = {
            let book = self.book.lock().unwrap_or_else(|e| e.into_inner());
            book.worktrees.keys().copied().filter(|id| !live.contains(id)).collect()
        };
        for id in gone {
            self.forget_offloaded(Subject::Worktree(id)).await;
        }

        for (id, worktree) in fleet {
            let stale = {
                let book = self.book.lock().unwrap_or_else(|e| e.into_inner());
                match book.worktrees.get(id) {
                    // A worktree that moved on disk is a different tree under
                    // the same row, and the old registration answers for a
                    // path nothing writes to any more.
                    Some(reg) => rewalk.contains(id) || reg.roots.first() != Some(worktree),
                    None => true,
                }
            };
            if stale {
                self.register_worktree(*id, worktree).await;
            }
        }
    }

    /// Bring the registered repositories in line with the fleet.
    ///
    /// `awake` names repositories that just reported an event, which is when
    /// it is worth re-checking whether `<common>/worktrees` has come into
    /// existence: it is created by the FIRST linked worktree, and its creation
    /// is an event on `<common>`, which is registered from the start. Checking
    /// unconditionally would be one `stat` per repository per second — the
    /// poll this module exists to retire.
    ///
    /// Blocking, and worth knowing where from: a `watch` is a synchronous call
    /// into the platform backend, which on macOS starts an FSEvents stream and
    /// stops one on the stream's own delivery queue. The caller runs this off
    /// the executor for that reason.
    pub fn sync_repositories(&self, fleet: &[(Uuid, PathBuf)], awake: &HashSet<Uuid>) {
        let live: HashSet<Uuid> = fleet.iter().map(|(id, _)| *id).collect();
        let gone: Vec<Uuid> = {
            let book = self.book.lock().unwrap_or_else(|e| e.into_inner());
            book.repositories.keys().copied().filter(|id| !live.contains(id)).collect()
        };
        for id in gone {
            self.forget(Subject::Repository(id));
        }

        for (id, common) in fleet {
            let needs = {
                let book = self.book.lock().unwrap_or_else(|e| e.into_inner());
                match book.repositories.get(id) {
                    Some(reg) => {
                        reg.roots.first() != Some(common)
                            // Registered against `<common>` alone, waiting for
                            // `worktrees` to appear.
                            || (awake.contains(id) && reg.watched.len() < 2)
                    }
                    None => true,
                }
            };
            if needs {
                self.register_repository(*id, common);
            }
        }
    }

    /// Watch one repository's common git directory, and `worktrees` under it.
    ///
    /// `<common>` non-recursively rather than recursively, which is the whole
    /// difference between two watches and a few hundred: `.git/objects` is up
    /// to 256 fan-out directories plus a pack directory, none of which says
    /// anything this daemon reads. `HEAD`, `index` and `worktrees` are all
    /// direct children, and a non-recursive watch on a directory reports
    /// exactly its own entries.
    fn register_repository(&self, id: Uuid, common: &Path) {
        let previous = self.take(Subject::Repository(id));

        let worktrees = common.join("worktrees");
        let mut paths = vec![common.to_path_buf()];
        if worktrees.is_dir() {
            paths.push(worktrees.clone());
        }

        // Routes before watches, so an event that arrives mid-registration is
        // attributed rather than dropped. Only `worktrees` and its immediate
        // children are claimed for the repository: `<common>` is WATCHED so
        // that `worktrees` being created is seen, but a write to
        // `<common>/HEAD` belongs to a worktree, not to the set of worktrees.
        self.add_routes(
            both_names(&worktrees)
                .into_iter()
                .map(|at| Route { at, scope: Scope::Shallow, subject: Subject::Repository(id) })
                .collect(),
        );

        let held: HashSet<PathBuf> =
            previous.as_ref().map(|p| p.watched.iter().cloned().collect()).unwrap_or_default();
        // `<common>` with the git-directory exclusions, `worktrees` without:
        // its children appearing and going is the whole point of it.
        let (mut watched, mut covered) =
            self.watch_all(&paths[..1], false, &git_dir_exclusions(common), "repository", &held);
        let (mut rest, all) = self.watch_all(&paths[1..], false, &[], "repository", &held);
        watched.append(&mut rest);
        covered &= all;
        if !covered {
            tracing::warn!(
                repository = %id,
                common = %common.display(),
                "could not watch a repository's git directory; its worktrees will be found by \
                 stat-polling instead"
            );
        }
        self.retire(previous, &watched);
        self.book.lock().unwrap_or_else(|e| e.into_inner()).repositories.insert(
            id,
            Registration {
                watched,
                dirs: Arc::new(HashSet::new()),
                roots: vec![common.to_path_buf()],
                excluded: Vec::new(),
                covered,
            },
        );
    }

    /// Walk one worktree and register the directories git cares about.
    ///
    /// The previous registration is lifted out of the book WITHOUT handing its
    /// watches back, and retired at the end against what this one ended up
    /// holding. A re-walk — which is what a new directory appearing costs —
    /// shares nearly every path with the walk before it, and dropping them all
    /// to take them all again would be tens of thousands of syscalls on a
    /// large repository and a window, however short, in which this worktree is
    /// watched by nothing at all.
    async fn register_worktree(&self, id: Uuid, worktree: &Path) {
        let previous = self.take(Subject::Worktree(id));
        let held: Arc<HashSet<PathBuf>> =
            Arc::new(previous.as_ref().map(|p| p.watched.iter().cloned().collect()).unwrap_or_default());

        // The `.git` half first and unconditionally: it is one watch, it is
        // what sees a commit land, and it is worth having even for a worktree
        // whose working tree turns out to be too large to watch.
        let git_dir = crate::review::git_dir(worktree);
        let (mut paths, mut covered) = self
            .watch_all_offloaded(
                vec![git_dir.clone()],
                false,
                git_dir_exclusions(&git_dir),
                "git directory",
                held.clone(),
            )
            .await;

        let roots = both_names(worktree);
        let (dirs, nested) = match tracked_directories(worktree).await {
            Ok(walked) => walked,
            Err(why) => {
                tracing::warn!(
                    worktree_id = %id,
                    worktree = %worktree.display(),
                    reason = why,
                    "could not enumerate a worktree's directories; its edits will be found by \
                     the activity gate and the cheap gate instead"
                );
                self.retire_offloaded(previous, &paths).await;
                self.finish_worktree(id, paths, Arc::new(HashSet::new()), roots, Vec::new(), false);
                return;
            }
        };

        // The ceiling, reported as a number rather than as a failure to
        // notice anything. Charged fleet-wide because the kernel's table is
        // per-user, not per-worktree: twenty worktrees of three thousand
        // directories is sixty thousand watches, which is over the whole
        // budget on a machine whose `max_user_watches` is the older 8192 and
        // most of it on a newer 65536.
        let spent = self.book.lock().unwrap_or_else(|e| e.into_inner()).spent;
        if dirs.len() > MAX_DIRS_PER_WORKTREE || spent + dirs.len() > self.budget {
            tracing::warn!(
                worktree_id = %id,
                worktree = %worktree.display(),
                directories = dirs.len(),
                already_watching = spent,
                budget = self.budget,
                per_worktree_limit = MAX_DIRS_PER_WORKTREE,
                "a worktree needs more watches than are left; its working tree is NOT being \
                 watched and its edits will be found by the activity gate and the cheap gate \
                 instead"
            );
            self.retire_offloaded(previous, &paths).await;
            self.finish_worktree(id, paths, Arc::new(HashSet::new()), roots, Vec::new(), false);
            return;
        }

        // Only where one registration covers the whole tree: per directory,
        // nothing git ignores was registered in the first place.
        let excluded = if PER_DIRECTORY {
            Vec::new()
        } else {
            let fresh = exclusions::for_worktree(worktree, &git_dir, &dirs, &nested).await;
            let base = canonical_ish(worktree).unwrap_or_else(|| worktree.to_path_buf());
            kept_or(fresh, previous.as_ref().map(|p| p.excluded.as_slice()), &base, &dirs)
        };
        let dirs = Arc::new(dirs);
        let mut routes: Vec<Route> = roots
            .iter()
            .map(|root| Route {
                at: root.clone(),
                scope: Scope::Filtered(dirs.clone()),
                subject: Subject::Worktree(id),
            })
            .collect();
        // A linked worktree's `.git` is a FILE pointing at
        // `<common>/worktrees/<name>`, which is nowhere under the worktree
        // root — so `HEAD` and `index` need a route of their own. Named
        // entries rather than the whole directory: see `Scope::Entries`, which
        // is where the reason is, and it is a load-bearing one.
        routes.extend(both_names(&git_dir).into_iter().map(|at| Route {
            at,
            scope: Scope::Entries(GIT_GATE_ENTRIES),
            subject: Subject::Worktree(id),
        }));
        self.add_routes(routes);

        let (mut tree, all) = if PER_DIRECTORY {
            let want: Vec<PathBuf> = dirs.iter().map(|rel| worktree.join(rel)).collect();
            self.watch_all_offloaded(want, false, Vec::new(), "worktree directory", held.clone()).await
        } else {
            let want = vec![worktree.to_path_buf()];
            // A root this subject already held keeps its stream, unless what
            // it should leave out has changed: the first build in a fresh
            // worktree makes a `target/` the walk before it never saw.
            let before = previous.as_ref().map(|p| p.excluded.as_slice());
            if held.contains(worktree) && before != Some(excluded.as_slice()) {
                self.reconfigure_offloaded(worktree.to_path_buf(), excluded.clone()).await;
            }
            self.watch_all_offloaded(want, true, excluded.clone(), "worktree", held.clone()).await
        };
        covered &= all;
        paths.append(&mut tree);

        let registered = dirs.len();
        let leaving_out = excluded.len();
        self.retire_offloaded(previous, &paths).await;
        self.finish_worktree(id, paths, dirs, roots, excluded, covered);

        // Logged AFTER the registration is booked, so `fleet_total` includes
        // this worktree. That total is what a runner running out of inotify
        // watches would see climbing.
        if covered {
            tracing::debug!(
                worktree_id = %id,
                worktree = %worktree.display(),
                directories = registered,
                excluded = leaving_out,
                fleet_total = self.watched_directories(),
                budget = self.budget,
                "watching a worktree"
            );
        } else {
            tracing::warn!(
                worktree_id = %id,
                worktree = %worktree.display(),
                directories = registered,
                "a worktree could not be fully watched; its edits will be found by the activity \
                 gate and the cheap gate instead"
            );
        }
    }

    fn finish_worktree(
        &self,
        id: Uuid,
        watched: Vec<PathBuf>,
        dirs: Arc<HashSet<PathBuf>>,
        roots: Vec<PathBuf>,
        excluded: Vec<PathBuf>,
        covered: bool,
    ) {
        let mut book = self.book.lock().unwrap_or_else(|e| e.into_inner());
        book.spent += dirs.len();
        book.worktrees.insert(id, Registration { watched, dirs, roots, excluded, covered });
    }

    /// Drop a subject's routes and its place in the book, keeping its watches.
    ///
    /// Routes go first and outside the lock the callback takes, which is what
    /// makes an event landing mid-registration attributable to nothing and
    /// dropped — the right answer for a subject that is going away, and
    /// harmless for one being re-registered, which is about to be probed for
    /// having moved anyway.
    ///
    /// The watches come back to the caller rather than being handed to the
    /// kernel, because the caller is the only one that knows whether this is a
    /// removal or a re-walk. `retire` is the other half.
    fn take(&self, subject: Subject) -> Option<Registration> {
        {
            let mut routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
            routes.retain(|r| r.subject != subject);
        }
        let mut book = self.book.lock().unwrap_or_else(|e| e.into_inner());
        let gone = match subject {
            Subject::Worktree(id) => book.worktrees.remove(&id),
            Subject::Repository(id) => book.repositories.remove(&id),
        };
        if let Some(reg) = &gone {
            book.spent = book.spent.saturating_sub(reg.dirs.len());
        }
        gone
    }

    /// Hand back the watches a previous registration held and the new one does not.
    ///
    /// Blocking, like every call into the backend: an async caller uses
    /// `retire_offloaded`. See `offload`.
    fn retire(&self, previous: Option<Registration>, kept: &[PathBuf]) {
        let stale = offload::stale(previous, kept);
        if stale.is_empty() {
            return;
        }
        if let Some(backend) = self.watcher.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            offload::unwatch_all(backend, &stale);
        }
    }

    /// Drop everything a subject holds, watches included.
    fn forget(&self, subject: Subject) {
        let previous = self.take(subject);
        self.retire(previous, &[]);
    }

    fn add_routes(&self, mut new: Vec<Route>) {
        let mut routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        routes.append(&mut new);
    }

    /// `offload::watch_all` from a caller already on a blocking thread; an
    /// async caller uses `watch_all_offloaded`.
    fn watch_all(
        &self,
        paths: &[PathBuf],
        recursive: bool,
        exclude: &[PathBuf],
        what: &str,
        held: &HashSet<PathBuf>,
    ) -> (Vec<PathBuf>, bool) {
        let mut watcher = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        let Some(backend) = watcher.as_mut() else { return (Vec::new(), false) };
        offload::watch_all(backend, paths, recursive, exclude, what, held)
    }
}

/// The exclusions to use: git's fresh answer, or, when git had none, what the
/// previous registration used. See `exclusions::for_worktree`.
///
/// The previous list is re-checked against `dirs`, the walk that just ran:
/// a directory excluded then may hold a wanted file now (`git add -f`, a
/// `.gitignore` edit), and reusing it blind would hide that file's edits —
/// the unsafe direction. Anything now in `dirs`, or not under `base` (the
/// worktree root, spelled as the exclusions are), is dropped.
fn kept_or(
    fresh: Option<Vec<PathBuf>>,
    previous: Option<&[PathBuf]>,
    base: &Path,
    dirs: &HashSet<PathBuf>,
) -> Vec<PathBuf> {
    if let Some(fresh) = fresh {
        return fresh;
    }
    previous
        .unwrap_or_default()
        .iter()
        .filter(|path| path.strip_prefix(base).is_ok_and(|rel| !rel.as_os_str().is_empty() && !dirs.contains(rel)))
        .cloned()
        .collect()
}

/// Attribute a batch of delivered paths to the subjects whose routes claim them.
///
/// Runs on the backend's delivery thread, so it takes the route and change
/// locks and nothing else — see `TreeWatcher::routes`.
fn route(table: &Mutex<Vec<Route>>, sink: &Mutex<Changed>, paths: &[PathBuf]) {
    let table = table.lock().unwrap_or_else(|e| e.into_inner());
    if table.is_empty() {
        return;
    }
    let mut sink = sink.lock().unwrap_or_else(|e| e.into_inner());
    for path in paths {
        for route in table.iter() {
            if !route.covers(path) {
                continue;
            }
            match route.subject {
                Subject::Worktree(id) => {
                    sink.worktrees.entry(id).or_default().insert(path.clone());
                }
                Subject::Repository(id) => {
                    sink.repositories.insert(id);
                }
            }
        }
    }
}

/// A path and, when it differs, its canonicalized twin.
///
/// Every route gets both. macOS reports events under `/private/var` for a
/// watch registered on `/var`, and `/tmp` is a symlink into it — so a route
/// that only knows the name it was registered under matches nothing at all in
/// a scratch directory, and would match nothing on any runner whose worktrees
/// sit behind a symlinked mount, which is what `/home` is on a good few Linux
/// installs.
///
/// The canonicalization is done once at registration and never on the event
/// path: an event is often the LAST thing to mention a path that no longer
/// exists, and `canonicalize` fails on those.
fn both_names(path: &Path) -> Vec<PathBuf> {
    let mut names = vec![path.to_path_buf()];
    if let Some(real) = canonical_ish(path) {
        if real != path {
            names.push(real);
        }
    }
    names
}

/// Canonicalize as much of a path as exists, and keep the rest as written.
///
/// `canonicalize` fails outright on a path that is not there, and one of the
/// paths this module routes on is deliberately not there yet:
/// `<common>/worktrees` does not exist until the first linked worktree creates
/// it, and the event announcing that creation is the whole reason the route is
/// registered ahead of time. Resolving the deepest ancestor that DOES exist and
/// re-appending the rest gives the name the backend will report it under.
fn canonical_ish(path: &Path) -> Option<PathBuf> {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut here = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(here) {
            let mut out = real;
            out.extend(tail.iter().rev());
            return Some(out);
        }
        tail.push(here.file_name()?.to_os_string());
        here = here.parent()?;
    }
}

/// Every directory git cares about in this worktree, relative to its root.
///
/// `git ls-files` rather than a gitignore parser: git's own answer about what
/// git ignores cannot disagree with git, and a second implementation of
/// `.gitignore` precedence — repository, global, `.git/info/exclude`, nested
/// files, negations — is a second implementation that will. `--cached` covers
/// tracked files, `--others --exclude-standard` covers untracked ones that are
/// not ignored, and the directories on those paths are exactly the set wanted.
///
/// One git process, spent when a worktree is first seen and when a directory
/// appears inside one — not on a clock. An idle fleet never reaches here.
///
/// An empty directory is not in the set, because git does not track one. That
/// is not a gap: the first file written into it is a write to a directory
/// whose PARENT is watched, which surfaces, and the pass that reads that event
/// asks for a re-walk.
///
/// Also hands back the nested repositories and worktrees git found: `--others`
/// names each as one `dir/` record and never looks inside, which is what makes
/// them safe to leave out of a watch. See `exclusions`.
async fn tracked_directories(
    worktree: &Path,
) -> Result<(HashSet<PathBuf>, Vec<PathBuf>), &'static str> {
    let out = crate::git::git_bytes(
        worktree,
        &["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
    )
    .await
    .map_err(|_| "git could not be run")?;
    if !out.ok {
        return Err("git ls-files failed");
    }

    let mut dirs: HashSet<PathBuf> = HashSet::new();
    let mut nested: Vec<PathBuf> = Vec::new();
    // The root itself, which no listed path names: `README.md` has no parent
    // component, and a write directly into the worktree root has to land
    // somewhere.
    dirs.insert(PathBuf::new());
    for record in out.stdout.split(|b| *b == 0) {
        if record.is_empty() {
            continue;
        }
        // A nested repository or worktree, `dir/`: recorded for the
        // exclusions, and then walked like any other record, so its ancestors
        // are still watched — a directory holding only nested checkouts is
        // where the next one appears. The loop stops before the last
        // component, so the checkout itself never enters the set.
        let record = match record.strip_suffix(b"/") {
            Some(repository) => {
                nested.extend(os_path(repository));
                repository
            }
            None => record,
        };
        let Some(relative) = os_path(record) else { continue };
        // Every ancestor, not just the immediate parent: `a/b/c.rs` puts `a`
        // and `a/b` in the set, and a repository whose only file at a level is
        // deep still gets its intermediate directories watched.
        let mut here = PathBuf::new();
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            // The last component is the file, and a file is not a directory to
            // watch. `-z` output is never quoted and never `.` or `..`.
            if components.peek().is_none() {
                break;
            }
            here.push(component);
            dirs.insert(here.clone());
        }
        if dirs.len() > MAX_DIRS_PER_WORKTREE {
            // Stopped early rather than run to completion: the caller is going
            // to refuse this worktree either way, and a repository with a
            // million directories should not also cost a million insertions to
            // find that out. The count is no longer exact past here, and the
            // caller only compares it against the limit.
            break;
        }
    }
    Ok((dirs, nested))
}

/// A path exactly as git wrote it.
///
/// Not `from_utf8_lossy`: on Linux a filename is any byte sequence that is not
/// NUL or `/`, and a path that has been through lossy substitution names a
/// directory that does not exist — which would register a watch on nothing and
/// silently stop covering everything under it.
#[cfg(unix)]
fn os_path(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

#[cfg(not(unix))]
fn os_path(bytes: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(bytes).ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::settle::{settle, wait_for_retrying};
    use super::*;
    use std::time::{Duration, Instant};

    pub(super) async fn repo(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new().prefix(name).tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main", "."],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
            vec!["config", "commit.gpgsign", "false"],
            // The registration walk asks git what git ignores, and
            // `--exclude-standard` reads the developer's own global excludes
            // file. A fixture whose directory count depends on what somebody
            // put in `~/.config/git/ignore` is a fixture that fails on one
            // machine and nobody else's.
            vec!["config", "core.excludesFile", "/dev/null"],
        ] {
            crate::git::git(&root, &args).await.unwrap();
        }
        (dir, root)
    }

    /// Drain until the watcher has stopped producing, or the deadline passes.
    ///
    /// **A single drain is not a barrier**, and this is `log_watch`'s
    /// `wait_until_quiet` for the same species of reason. What a drain says is
    /// that nothing had arrived *by then*; what a test needs before asserting
    /// a negative is that everything the fixture caused has already arrived.
    /// Between the two sits the whole setup: `repo()` runs five git processes
    /// that write `.git/HEAD` and `.git/config`, and the fixture writes a file
    /// into the worktree root. `.git/HEAD` and a file in the root are both
    /// paths the routes under test deliberately claim — `Scope::Entries` and
    /// `Scope::Filtered` respectively — so either one arriving late is a
    /// failure. (`git ls-files`, which registration runs between the two
    /// watches, is not a third: it writes nothing into `.git`.)
    ///
    /// Those events are not safely in the past at the drain, on this backend
    /// least of all. FSEvents takes its `SinceNow` boundary inside the
    /// `watch()` call rather than at the write, and measurably BEFORE that
    /// call returns: writing a file every 25ms across a `watch()` and asking
    /// which ones the new stream delivered, the earliest was written 4ms to
    /// 10ms before `watch()` handed control back. A write from just before
    /// the stream existed is delivered to it all the same, asynchronously —
    /// after `sync_worktrees` has returned, and after the drain that follows
    /// it. On a runner loaded enough to stretch that delivery it lands inside
    /// the window below and fails an assertion that is about something else
    /// entirely, which is exactly how this arrived:
    /// green on the push that added it, red on macOS days later with nothing
    /// in between having touched this file.
    ///
    /// Half a second of continuous quiet rather than one empty drain, again
    /// for `log_watch`'s reason: an empty drain only says nothing arrived in
    /// the last 50ms, which is also true in the gap between two batches, and
    /// the gap is the thing that has to be outlasted.
    ///
    /// The deadline is a bound, not a wait: on a quiet machine this returns in
    /// half a second. It does NOT assert — a watcher that never goes quiet is
    /// the caller's assertion to make, and every caller here makes one.
    pub(super) fn wait_until_quiet(watcher: &TreeWatcher, deadline: Duration) {
        let start = Instant::now();
        let mut empties = 0;
        while empties < 10 && start.elapsed() < deadline {
            if watcher.drain_worktrees().is_empty() {
                empties += 1;
            } else {
                empties = 0;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Every path that surfaced for `id` inside `window`, for asserting none did.
    ///
    /// Hands back the paths rather than a bool because this is the one place a
    /// failure is unreadable without them: "something surfaced" names neither
    /// which route claimed a path it should not have, nor whether the fixture
    /// simply leaked. Returns as soon as anything arrives — a negative that is
    /// already false has nothing left to prove — and otherwise costs the full
    /// window, which is the point of it.
    fn surfaced_within(watcher: &TreeWatcher, id: Uuid, window: Duration) -> HashSet<PathBuf> {
        let start = Instant::now();
        loop {
            if let Some(paths) = watcher.drain_worktrees().remove(&id) {
                return paths;
            }
            if start.elapsed() > window {
                return HashSet::new();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[tokio::test]
    async fn an_ignored_directory_is_never_registered() {
        let (_dir, root) = repo("fc-fsw-ignored").await;
        std::fs::write(root.join(".gitignore"), "target\n").unwrap();
        std::fs::create_dir_all(root.join("src/deep/deeper")).unwrap();
        std::fs::write(root.join("src/deep/deeper/a.rs"), "").unwrap();
        for i in 0..50 {
            let d = root.join(format!("target/debug/incremental/x{i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("junk.o"), "").unwrap();
        }

        let (dirs, _) = tracked_directories(&root).await.unwrap();
        assert!(dirs.contains(Path::new("src/deep/deeper")), "git's own files must be in: {dirs:?}");
        assert!(
            !dirs.iter().any(|d| d.starts_with("target")),
            "the directories that exhaust the ceiling are the ones git ignores: {dirs:?}"
        );
        // The root, `src`, `src/deep`, `src/deep/deeper`. `.gitignore` is a
        // file, not a directory.
        assert_eq!(dirs.len(), 4, "{dirs:?}");
    }

    #[tokio::test]
    async fn an_edit_with_nothing_running_surfaces_its_worktree() {
        let (_dir, root) = repo("fc-fsw-edit").await;
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        crate::git::git(&root, &["add", "-A"]).await.unwrap();
        crate::git::git(&root, &["commit", "-q", "-m", "base"]).await.unwrap();

        let watcher = TreeWatcher::start();
        let id = Uuid::now_v7();
        watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
        assert!(watcher.covers_worktree(id), "a plain repository must be watchable");
        // The `add` and the `commit` above wrote into `.git`, which this
        // worktree's own route claims. Quiet, so what surfaces below is the
        // edit and not the fixture: see `wait_until_quiet`.
        settle(&watcher, &root);

        // No pane, no agent, nothing this daemon served — the gap the working
        // tree half exists to close.
        let edit = |n: u32| std::fs::write(root.join("src/main.rs"), format!("fn main() {{ let x = {n}; }}")).unwrap();
        edit(1);
        let mut n = 1;
        let seen = wait_for_retrying(&watcher, id, Duration::from_secs(60), || {
            n += 1;
            edit(n);
        });
        assert!(seen, "an in-place edit must surface");
    }

    #[tokio::test]
    async fn a_commit_surfaces_its_worktree_through_the_git_directory() {
        let (_dir, root) = repo("fc-fsw-commit").await;
        std::fs::write(root.join("a.txt"), "one").unwrap();

        let watcher = TreeWatcher::start();
        let id = Uuid::now_v7();
        watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
        settle(&watcher, &root);

        crate::git::git(&root, &["add", "-A"]).await.unwrap();
        crate::git::git(&root, &["commit", "-q", "-m", "one"]).await.unwrap();
        // Again: the index swapped as git does, the route a commit relies on.
        let git_dir = crate::review::git_dir(&root);
        let seen = wait_for_retrying(&watcher, id, Duration::from_secs(60), || {
            std::fs::write(git_dir.join("index.lock"), "").unwrap();
            std::fs::rename(git_dir.join("index.lock"), git_dir.join("index")).unwrap();
        });
        assert!(seen, "a commit must surface");
    }

    /// The feedback loop, kept shut.
    ///
    /// `git status --porcelain=v2` — one of the three processes the probe this
    /// gate guards spends — takes `index.lock` for the duration. If that
    /// counted as the worktree moving, every probe would schedule the next
    /// one three seconds later and an idle fleet would spend git forever. It
    /// did: four worktrees with nothing happening spawned 400 git processes a
    /// minute before this route was narrowed.
    #[tokio::test]
    async fn the_index_lock_a_probe_takes_is_not_the_worktree_moving() {
        let (_dir, root) = repo("fc-fsw-lock").await;
        std::fs::write(root.join("a.txt"), "one").unwrap();

        let watcher = TreeWatcher::start();
        let id = Uuid::now_v7();
        watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
        assert!(watcher.covers_worktree(id));
        // Quiet, not one drain: the fixture's own writes are still in flight
        // here and `.git/HEAD` is a path this route claims. See
        // `wait_until_quiet`, which is where the whole reason lives.
        settle(&watcher, &root);

        // Exactly what git does around an index it does not end up changing.
        let git_dir = crate::review::git_dir(&root);
        let lock = git_dir.join("index.lock");
        std::fs::write(&lock, "").unwrap();
        std::fs::remove_file(&lock).unwrap();
        let leaked = surfaced_within(&watcher, id, Duration::from_secs(3));
        assert!(
            leaked.is_empty(),
            "a lock file the daemon's own probe took must not schedule the next probe: {leaked:?}"
        );

        // And the index itself still does, which is the half that must survive
        // the narrowing: git writes the lock and RENAMES it over `index`.
        let swap = || {
            std::fs::write(&lock, "").unwrap();
            std::fs::rename(&lock, git_dir.join("index")).unwrap();
        };
        swap();
        assert!(
            wait_for_retrying(&watcher, id, Duration::from_secs(60), swap),
            "the index actually changing is what the .git half is for"
        );
    }

    #[tokio::test]
    async fn a_new_directory_asks_for_a_rewalk() {
        let (_dir, root) = repo("fc-fsw-rewalk").await;
        std::fs::write(root.join("a.txt"), "one").unwrap();

        let watcher = TreeWatcher::start();
        let id = Uuid::now_v7();
        watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
        settle(&watcher, &root);

        let fresh = root.join("added");
        std::fs::create_dir_all(&fresh).unwrap();
        let seen = wait_for_retrying(&watcher, id, Duration::from_secs(60), || {
            // Gone and made again: a second creation to deliver.
            let _ = std::fs::remove_dir_all(&fresh);
            std::fs::create_dir_all(&fresh).unwrap();
        });
        assert!(seen, "a new directory must surface");

        let mut paths = HashSet::new();
        paths.insert(fresh.clone());
        assert!(watcher.needs_rewalk(id, &paths), "a directory the walk never saw is what a re-walk is for");

        // And after the re-walk it is watched, so what lands inside it is seen.
        std::fs::write(fresh.join("b.txt"), "two").unwrap();
        let one = HashSet::from([id]);
        watcher.sync_worktrees(&[(id, root.clone())], &one).await;
        // The write above is what would make this pass without the re-walk
        // having registered anything, so it has to be out of the way first.
        settle(&watcher, &root);
        let mut n = 0;
        let mut write = || {
            n += 1;
            std::fs::write(fresh.join("b.txt"), format!("three{n}")).unwrap();
        };
        write();
        assert!(
            wait_for_retrying(&watcher, id, Duration::from_secs(60), write),
            "the new directory must be watched"
        );
    }

    #[tokio::test]
    async fn a_worktree_that_is_gone_stops_being_watched() {
        let (_dir, root) = repo("fc-fsw-gone").await;
        std::fs::write(root.join("a.txt"), "one").unwrap();

        let watcher = TreeWatcher::start();
        let id = Uuid::now_v7();
        watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
        assert!(watcher.watched_directories() >= 1);

        watcher.sync_worktrees(&[], &HashSet::new()).await;
        assert!(!watcher.covers_worktree(id), "a removed worktree keeps no registration");
        assert_eq!(watcher.watched_directories(), 0, "and gives its budget back");
    }

    /// The ceiling, reached on purpose.
    ///
    /// What matters is not that registration failed — it is that the daemon
    /// SAYS it is not covered, because that is what keeps the caller's own
    /// gates live rather than leaving a worktree nobody is watching and
    /// nobody knows it.
    #[tokio::test]
    async fn a_worktree_over_the_budget_reports_itself_uncovered() {
        let (_dir, root) = repo("fc-fsw-budget").await;
        for i in 0..8 {
            let d = root.join(format!("d{i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("f.txt"), "x").unwrap();
        }

        let mut watcher = TreeWatcher::start();
        // Smaller than the nine directories this worktree needs.
        watcher.budget = 4;
        let id = Uuid::now_v7();
        watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;

        assert!(
            !watcher.covers_worktree(id),
            "over budget must report as not covered so the fallback gates stay live"
        );
    }

    /// A backend that would not build, which is what `LogWatcher` warns about
    /// and carries on from. Here the carrying on has to be visible to the
    /// caller, so it can keep stat-polling.
    #[tokio::test]
    async fn a_dead_backend_covers_nothing() {
        let (_dir, root) = repo("fc-fsw-dead").await;
        std::fs::write(root.join("a.txt"), "one").unwrap();

        let watcher = TreeWatcher::start();
        // Exactly what `start` leaves behind when `recommended_watcher` fails.
        *watcher.watcher.lock().unwrap() = None;

        let id = Uuid::now_v7();
        watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
        assert!(!watcher.covers_worktree(id), "no backend is no coverage");
        let repo_id = Uuid::now_v7();
        watcher.sync_repositories(&[(repo_id, root.join(".git"))], &HashSet::new());
        assert!(!watcher.covers_repository(repo_id));
    }

    /// The one event this waits on is `worktrees` appearing in the common
    /// directory: a second `worktree add` makes nothing the repository's
    /// stream reports, since everything under `worktrees` is excluded from
    /// it. And FSEvents loses whole writes in spells (ov-666). So each
    /// attempt is a repository of its own, judged by a witness watch on its
    /// common directory: one the witness saw and the watcher never surfaced
    /// fails; one neither saw is made again (ov-668).
    #[tokio::test]
    async fn a_linked_worktree_appearing_surfaces_its_repository() {
        use notify::Watcher as _;
        let watcher = TreeWatcher::start();
        let start = Instant::now();
        let surfaces = |repo_id: Uuid, limit: Duration| {
            let at = Instant::now();
            while at.elapsed() <= limit {
                if watcher.drain_repositories().contains(&repo_id) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            false
        };
        for attempt in 1.. {
            let (dir, root) = repo("fc-fsw-linked").await;
            crate::git::git(&root, &["commit", "-q", "--allow-empty", "-m", "base"]).await.unwrap();
            let repo_id = Uuid::now_v7();
            let common = root.join(".git");
            watcher.sync_repositories(&[(repo_id, common.clone())], &HashSet::new());
            assert!(watcher.covers_repository(repo_id));
            let (tx, witnessed) = std::sync::mpsc::channel::<Vec<PathBuf>>();
            let mut witness = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let _ = tx.send(res.map(|e| e.paths).unwrap_or_default());
            })
            .unwrap();
            witness.watch(&common, notify::RecursiveMode::NonRecursive).unwrap();
            // As exclusion_tests' witness: its stream starts after `watch`.
            std::thread::sleep(Duration::from_millis(300));
            let _ = watcher.drain_repositories();

            let linked = dir.path().join("linked");
            crate::git::git(&root, &["worktree", "add", "-q", "-b", "side", linked.to_str().unwrap()])
                .await
                .unwrap();
            if surfaces(repo_id, Duration::from_secs(10)) {
                return;
            }
            let until = Instant::now() + Duration::from_secs(10);
            let saw = std::iter::from_fn(|| witnessed.recv_timeout(until.saturating_duration_since(Instant::now())).ok())
                .flatten()
                .any(|p| p.ends_with(".git/worktrees"));
            if saw {
                let why = "the witness saw `worktrees` appear, and the watcher never surfaced its repository";
                assert!(surfaces(repo_id, Duration::from_secs(60)), "a worktree appearing is what retires the thirty-second reconcile poll: {why}");
                return;
            }
            eprintln!("attempt {attempt}: FSEvents delivered `worktrees` to no stream; trying again");
            assert!(start.elapsed() < Duration::from_secs(240), "no attempt tested it in 240 s: {attempt} lost");
        }
    }

    /// Run by hand, not in CI. Prints the number the Linux watch ceiling is
    /// spent against, for whatever repositories are named:
    ///
    /// ```text
    /// FARCOOLER_WATCH_COUNT_ROOTS=/path/one:/path/two \
    ///   cargo test -p farcooler-daemon watch_count -- --ignored --nocapture
    /// ```
    ///
    /// The number this reports IS the inotify watch count — notify's inotify
    /// backend adds one watch per directory, whether they are registered one
    /// at a time or as one recursive walk — so it is measurable on a machine
    /// that has no inotify at all, which is exactly the point: the ceiling is
    /// on Linux and the person writing this is usually not.
    ///
    /// An environment variable rather than trailing arguments, because libtest
    /// reads a bare argument as a name filter and would run nothing.
    #[tokio::test]
    #[ignore]
    async fn watch_count_on_real_repositories() {
        let ceiling = std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok());
        println!("max_user_watches here: {ceiling:?}, budget {}", fleet_budget());

        let roots = std::env::var("FARCOOLER_WATCH_COUNT_ROOTS").unwrap_or_default();
        let mut total = 0;
        for root in roots.split(':').filter(|r| !r.is_empty()) {
            let path = PathBuf::from(root);
            let on_disk = walkdir(&path);
            match tracked_directories(&path).await {
                Ok((dirs, _)) => {
                    total += dirs.len();
                    println!(
                        "{:>7} watched  {on_disk:>7} on disk   {}",
                        dirs.len(),
                        path.display()
                    );
                }
                Err(why) => println!("{:>7}  {}  ({why})", "-", path.display()),
            }
        }
        println!("{total:>7} watched in total, against a budget of {}", fleet_budget());
    }

    /// Every directory actually on disk, which is what would be registered by
    /// a watcher that was not gitignore-aware. The comparison is the whole
    /// argument for the walk.
    fn walkdir(root: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(root) else { return 0 };
        let mut n = 0;
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                n += 1 + walkdir(&entry.path());
            }
        }
        n
    }
}
