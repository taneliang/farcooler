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
//! ceiling, and registering per directory there is actively harmful:
//! `FsEventWatcher::watch` tears the event stream down and rebuilds it on
//! *every* call, so N directories is N stream restarts, each of them a thread
//! stop and join. Windows `ReadDirectoryChangesW` is per-tree in the same way.
//! So on those platforms one recursive registration covers the worktree.
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

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use uuid::Uuid;

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
/// Repositories and workspaces are separate subjects because they answer
/// different questions on different clocks: a repository's `worktrees`
/// directory drives `reconcile_worktrees`, and a workspace's tree and `.git`
/// drive `probe_change_sets`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Subject {
    /// A worktree's working tree, or the `.git` directory that worktree reads
    /// `HEAD` and `index` out of.
    Workspace(Uuid),
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
/// Workspaces keep their paths and repositories do not, because the two
/// callers ask different things of them: `probe_change_sets` needs the paths
/// to notice a directory git cares about that did not exist when the
/// registration walk ran, and `reconcile_worktrees` needs only the fact that
/// something under `worktrees` moved.
#[derive(Default)]
struct Changed {
    workspaces: HashMap<Uuid, HashSet<PathBuf>>,
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
    /// Read by the notify callback on its own thread, so nothing may hold it
    /// across a `watch` or `unwatch` call: those block on the backend's event
    /// thread, and the backend's event thread is what is inside the callback.
    routes: Arc<Mutex<Vec<Route>>>,
    /// Written by the callback, drained by the two passes.
    changed: Arc<Mutex<Changed>>,
    /// Held, not read — a `notify::Watcher` stops the moment it is dropped.
    /// Behind a mutex because unlike `LogWatcher`'s this one is registered
    /// against for as long as the daemon runs. `None` when the platform
    /// backend itself would not build, which is not a per-subject failure and
    /// has nothing left to retry: every subject is then uncovered and every
    /// caller falls back.
    watcher: Mutex<Option<RecommendedWatcher>>,
    book: Mutex<Book>,
    budget: usize,
}

#[derive(Default)]
struct Book {
    workspaces: HashMap<Uuid, Registration>,
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
        // No filtering by `EventKind`, for the reason `LogWatcher` gives: the
        // backends disagree about which kind an append surfaces as, and the
        // cost of being permissive is a redundant set entry while the cost of
        // being strict is an edit nobody notices.
        let built = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            let table = table.lock().unwrap_or_else(|e| e.into_inner());
            if table.is_empty() {
                return;
            }
            let mut sink = sink.lock().unwrap_or_else(|e| e.into_inner());
            for path in &event.paths {
                for route in table.iter() {
                    if !route.covers(path) {
                        continue;
                    }
                    match route.subject {
                        Subject::Workspace(id) => {
                            sink.workspaces.entry(id).or_default().insert(path.clone());
                        }
                        Subject::Repository(id) => {
                            sink.repositories.insert(id);
                        }
                    }
                }
            }
        });

        let watcher = match built {
            Ok(w) => Some(w),
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "could not start a worktree watcher; every workspace falls back to \
                     stat-polling its .git and to the activity gate"
                );
                None
            }
        };

        TreeWatcher {
            routes,
            changed,
            watcher: Mutex::new(watcher),
            book: Mutex::new(Book::default()),
            budget: fleet_budget(),
        }
    }

    /// Whether this workspace's tree and `.git` are actually being watched.
    ///
    /// False keeps the caller's own gates live for it — see `Registration::covered`.
    pub fn covers_workspace(&self, id: Uuid) -> bool {
        self.book
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .workspaces
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

    /// The workspaces something moved under, and which paths moved.
    ///
    /// Idempotent in the way `LogWatcher::drain` is: draining empties the map,
    /// so a second call with nothing written in between reports nothing.
    pub fn drain_workspaces(&self) -> HashMap<Uuid, HashSet<PathBuf>> {
        std::mem::take(&mut self.changed.lock().unwrap_or_else(|e| e.into_inner()).workspaces)
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
        let Some(reg) = book.workspaces.get(&id) else { return false };
        paths.iter().any(|path| {
            let Some(rel) = reg.relative(path) else { return false };
            !reg.dirs.contains(rel) && path.is_dir()
        })
    }

    /// Bring the registered workspaces in line with the fleet.
    ///
    /// `rewalk` names workspaces whose directory set is known to be out of
    /// date — see `needs_rewalk`. Everything else is a set comparison and
    /// spends nothing: this is called from a pass that runs every three
    /// seconds, and an idle fleet must cost neither git nor syscalls here.
    ///
    /// When it DOES have work it does two blocking things — a `git ls-files`
    /// per worktree, and a `watch` call per directory — so the caller runs it
    /// somewhere those are affordable. See `Watcher::probe_change_sets`, which
    /// is a detached pass for exactly this class of reason.
    pub async fn sync_workspaces(&self, fleet: &[(Uuid, PathBuf)], rewalk: &HashSet<Uuid>) {
        let live: HashSet<Uuid> = fleet.iter().map(|(id, _)| *id).collect();
        let gone: Vec<Uuid> = {
            let book = self.book.lock().unwrap_or_else(|e| e.into_inner());
            book.workspaces.keys().copied().filter(|id| !live.contains(id)).collect()
        };
        for id in gone {
            self.forget(Subject::Workspace(id));
        }

        for (id, worktree) in fleet {
            let stale = {
                let book = self.book.lock().unwrap_or_else(|e| e.into_inner());
                match book.workspaces.get(id) {
                    // A worktree that moved on disk is a different tree under
                    // the same row, and the old registration answers for a
                    // path nothing writes to any more.
                    Some(reg) => rewalk.contains(id) || reg.roots.first() != Some(worktree),
                    None => true,
                }
            };
            if stale {
                self.register_workspace(*id, worktree).await;
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
    /// Blocking, and worth knowing where from: `notify::Watcher::watch` is a
    /// synchronous call into the platform backend, and on the per-tree backends
    /// it tears the event stream down and rebuilds it. The caller runs this off
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
        // `<common>/HEAD` belongs to a workspace, not to the set of worktrees.
        self.add_routes(
            both_names(&worktrees)
                .into_iter()
                .map(|at| Route { at, scope: Scope::Shallow, subject: Subject::Repository(id) })
                .collect(),
        );

        let held: HashSet<PathBuf> =
            previous.as_ref().map(|p| p.watched.iter().cloned().collect()).unwrap_or_default();
        let (watched, covered) =
            self.watch_all(&paths, RecursiveMode::NonRecursive, "repository", &held);
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
    async fn register_workspace(&self, id: Uuid, worktree: &Path) {
        let previous = self.take(Subject::Workspace(id));
        let held: HashSet<PathBuf> =
            previous.as_ref().map(|p| p.watched.iter().cloned().collect()).unwrap_or_default();

        // The `.git` half first and unconditionally: it is one watch, it is
        // what sees a commit land, and it is worth having even for a worktree
        // whose working tree turns out to be too large to watch.
        let git_dir = crate::review::git_dir(worktree);
        let (mut paths, mut covered) = self.watch_all(
            std::slice::from_ref(&git_dir),
            RecursiveMode::NonRecursive,
            "git directory",
            &held,
        );

        let roots = both_names(worktree);
        let dirs = match tracked_directories(worktree).await {
            Ok(dirs) => dirs,
            Err(why) => {
                tracing::warn!(
                    workspace = %id,
                    worktree = %worktree.display(),
                    reason = why,
                    "could not enumerate a worktree's directories; its edits will be found by \
                     the activity gate and the cheap gate instead"
                );
                self.retire(previous, &paths);
                self.finish_workspace(id, paths, Arc::new(HashSet::new()), roots, false);
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
                workspace = %id,
                worktree = %worktree.display(),
                directories = dirs.len(),
                already_watching = spent,
                budget = self.budget,
                per_worktree_limit = MAX_DIRS_PER_WORKTREE,
                "a worktree needs more watches than are left; its working tree is NOT being \
                 watched and its edits will be found by the activity gate and the cheap gate \
                 instead"
            );
            self.retire(previous, &paths);
            self.finish_workspace(id, paths, Arc::new(HashSet::new()), roots, false);
            return;
        }

        let dirs = Arc::new(dirs);
        let mut routes: Vec<Route> = roots
            .iter()
            .map(|root| Route {
                at: root.clone(),
                scope: Scope::Filtered(dirs.clone()),
                subject: Subject::Workspace(id),
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
            subject: Subject::Workspace(id),
        }));
        self.add_routes(routes);

        let (mut tree, all) = if PER_DIRECTORY {
            let want: Vec<PathBuf> = dirs.iter().map(|rel| worktree.join(rel)).collect();
            self.watch_all(&want, RecursiveMode::NonRecursive, "worktree directory", &held)
        } else {
            let want = [worktree.to_path_buf()];
            self.watch_all(&want, RecursiveMode::Recursive, "worktree", &held)
        };
        covered &= all;
        paths.append(&mut tree);

        let registered = dirs.len();
        self.retire(previous, &paths);
        self.finish_workspace(id, paths, dirs, roots, covered);

        // Logged AFTER the registration is booked, so `fleet_total` includes
        // this worktree. That total is what a runner running out of inotify
        // watches would see climbing.
        if covered {
            tracing::debug!(
                workspace = %id,
                worktree = %worktree.display(),
                directories = registered,
                fleet_total = self.watched_directories(),
                budget = self.budget,
                "watching a worktree"
            );
        } else {
            tracing::warn!(
                workspace = %id,
                worktree = %worktree.display(),
                directories = registered,
                "a worktree could not be fully watched; its edits will be found by the activity \
                 gate and the cheap gate instead"
            );
        }
    }

    fn finish_workspace(
        &self,
        id: Uuid,
        watched: Vec<PathBuf>,
        dirs: Arc<HashSet<PathBuf>>,
        roots: Vec<PathBuf>,
        covered: bool,
    ) {
        let mut book = self.book.lock().unwrap_or_else(|e| e.into_inner());
        book.spent += dirs.len();
        book.workspaces.insert(id, Registration { watched, dirs, roots, covered });
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
            Subject::Workspace(id) => book.workspaces.remove(&id),
            Subject::Repository(id) => book.repositories.remove(&id),
        };
        if let Some(reg) = &gone {
            book.spent = book.spent.saturating_sub(reg.dirs.len());
        }
        gone
    }

    /// Hand back the watches a previous registration held and the new one does not.
    fn retire(&self, previous: Option<Registration>, kept: &[PathBuf]) {
        let Some(previous) = previous else { return };
        let kept: HashSet<&PathBuf> = kept.iter().collect();
        let stale: Vec<PathBuf> =
            previous.watched.into_iter().filter(|p| !kept.contains(p)).collect();
        if stale.is_empty() {
            return;
        }
        let mut watcher = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        let Some(watcher) = watcher.as_mut() else { return };
        for path in stale {
            // A directory that was removed took its watch with it, and the
            // backend has already forgotten it. Nothing to report.
            let _ = watcher.unwatch(&path);
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

    /// Register every path not already held, and report what is held now.
    ///
    /// The returned list is the paths this registration actually holds — one
    /// that failed is left out, so the next walk retries it rather than
    /// inheriting a watch that was never taken, and `retire` never hands back
    /// something the kernel does not have. The bool is whether ALL of them
    /// took, which is what `covered` is made of.
    ///
    /// Never called with the route or change locks held: `watch` blocks on the
    /// backend's own event thread, and that thread is what runs the callback
    /// those locks are for.
    fn watch_all(
        &self,
        paths: &[PathBuf],
        mode: RecursiveMode,
        what: &str,
        held: &HashSet<PathBuf>,
    ) -> (Vec<PathBuf>, bool) {
        let mut watcher = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        let Some(watcher) = watcher.as_mut() else { return (Vec::new(), false) };
        let mut all = true;
        let mut taken = Vec::with_capacity(paths.len());
        for path in paths {
            if held.contains(path) {
                taken.push(path.clone());
                continue;
            }
            let Err(error) = watcher.watch(path, mode) else {
                taken.push(path.clone());
                continue;
            };
            all = false;
            match error.kind {
                // The ceiling this module is shaped around, reached anyway —
                // something else on the runner is holding the table down, or
                // `max_user_watches` was lowered under us. Named explicitly
                // because "No space left on device" from a watcher is the
                // least helpful true sentence the kernel produces.
                notify::ErrorKind::MaxFilesWatch => tracing::warn!(
                    path = %path.display(),
                    "the kernel is out of file watches (see \
                     /proc/sys/fs/inotify/max_user_watches); not watching this {what}"
                ),
                // Registration races a worktree being removed, and a path that
                // is already gone is not a failure worth a warning — but it
                // still means this subject is not covered.
                notify::ErrorKind::PathNotFound => tracing::debug!(
                    path = %path.display(),
                    "{what} vanished before it could be watched"
                ),
                _ => tracing::warn!(path = %path.display(), ?error, "could not watch a {what}"),
            }
        }
        (taken, all)
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
async fn tracked_directories(worktree: &Path) -> Result<HashSet<PathBuf>, &'static str> {
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
    // The root itself, which no listed path names: `README.md` has no parent
    // component, and a write directly into the worktree root has to land
    // somewhere.
    dirs.insert(PathBuf::new());
    for record in out.stdout.split(|b| *b == 0) {
        if record.is_empty() {
            continue;
        }
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
    Ok(dirs)
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
    use super::*;
    use std::time::{Duration, Instant};

    async fn repo(name: &str) -> (tempfile::TempDir, PathBuf) {
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

    /// Polls rather than sleeping a fixed amount, for the reason
    /// `log_watch`'s tests give: every backend has its own latency before an
    /// event surfaces, and a fixed sleep either wastes time on a fast machine
    /// or flakes on a slow one.
    fn wait_for(watcher: &TreeWatcher, id: Uuid, deadline: Duration) -> bool {
        let start = Instant::now();
        loop {
            if watcher.drain_workspaces().contains_key(&id) {
                return true;
            }
            if start.elapsed() > deadline {
                return false;
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

        let dirs = tracked_directories(&root).await.unwrap();
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
    async fn an_edit_with_nothing_running_surfaces_its_workspace() {
        let (_dir, root) = repo("fc-fsw-edit").await;
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        crate::git::git(&root, &["add", "-A"]).await.unwrap();
        crate::git::git(&root, &["commit", "-q", "-m", "base"]).await.unwrap();

        let watcher = TreeWatcher::start();
        let id = Uuid::now_v7();
        watcher.sync_workspaces(&[(id, root.clone())], &HashSet::new()).await;
        assert!(watcher.covers_workspace(id), "a plain repository must be watchable");
        let _ = watcher.drain_workspaces();

        // No pane, no agent, nothing this daemon served — the gap the working
        // tree half exists to close.
        std::fs::write(root.join("src/main.rs"), "fn main() { let x = 1; }").unwrap();
        assert!(wait_for(&watcher, id, Duration::from_secs(10)), "an in-place edit must surface");
    }

    #[tokio::test]
    async fn a_commit_surfaces_its_workspace_through_the_git_directory() {
        let (_dir, root) = repo("fc-fsw-commit").await;
        std::fs::write(root.join("a.txt"), "one").unwrap();

        let watcher = TreeWatcher::start();
        let id = Uuid::now_v7();
        watcher.sync_workspaces(&[(id, root.clone())], &HashSet::new()).await;
        let _ = watcher.drain_workspaces();

        crate::git::git(&root, &["add", "-A"]).await.unwrap();
        crate::git::git(&root, &["commit", "-q", "-m", "one"]).await.unwrap();
        assert!(wait_for(&watcher, id, Duration::from_secs(10)), "a commit must surface");
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
        watcher.sync_workspaces(&[(id, root.clone())], &HashSet::new()).await;
        assert!(watcher.covers_workspace(id));
        let _ = watcher.drain_workspaces();

        // Exactly what git does around an index it does not end up changing.
        let git_dir = crate::review::git_dir(&root);
        let lock = git_dir.join("index.lock");
        std::fs::write(&lock, "").unwrap();
        std::fs::remove_file(&lock).unwrap();
        assert!(
            !wait_for(&watcher, id, Duration::from_secs(3)),
            "a lock file the daemon's own probe took must not schedule the next probe"
        );

        // And the index itself still does, which is the half that must survive
        // the narrowing: git writes the lock and RENAMES it over `index`.
        std::fs::write(&lock, "").unwrap();
        std::fs::rename(&lock, git_dir.join("index")).unwrap();
        assert!(
            wait_for(&watcher, id, Duration::from_secs(10)),
            "the index actually changing is what the .git half is for"
        );
    }

    #[tokio::test]
    async fn a_new_directory_asks_for_a_rewalk() {
        let (_dir, root) = repo("fc-fsw-rewalk").await;
        std::fs::write(root.join("a.txt"), "one").unwrap();

        let watcher = TreeWatcher::start();
        let id = Uuid::now_v7();
        watcher.sync_workspaces(&[(id, root.clone())], &HashSet::new()).await;
        let _ = watcher.drain_workspaces();

        let fresh = root.join("added");
        std::fs::create_dir_all(&fresh).unwrap();
        assert!(wait_for(&watcher, id, Duration::from_secs(10)), "a new directory must surface");

        let mut paths = HashSet::new();
        paths.insert(fresh.clone());
        assert!(
            watcher.needs_rewalk(id, &paths),
            "a directory the walk never saw is what a re-walk is for"
        );

        // And after the re-walk it is watched, so what lands inside it is seen.
        std::fs::write(fresh.join("b.txt"), "two").unwrap();
        let one = HashSet::from([id]);
        watcher.sync_workspaces(&[(id, root.clone())], &one).await;
        let _ = watcher.drain_workspaces();
        std::fs::write(fresh.join("b.txt"), "three").unwrap();
        assert!(wait_for(&watcher, id, Duration::from_secs(10)), "the new directory must be watched");
    }

    #[tokio::test]
    async fn a_workspace_that_is_gone_stops_being_watched() {
        let (_dir, root) = repo("fc-fsw-gone").await;
        std::fs::write(root.join("a.txt"), "one").unwrap();

        let watcher = TreeWatcher::start();
        let id = Uuid::now_v7();
        watcher.sync_workspaces(&[(id, root.clone())], &HashSet::new()).await;
        assert!(watcher.watched_directories() >= 1);

        watcher.sync_workspaces(&[], &HashSet::new()).await;
        assert!(!watcher.covers_workspace(id), "a removed workspace keeps no registration");
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
        watcher.sync_workspaces(&[(id, root.clone())], &HashSet::new()).await;

        assert!(
            !watcher.covers_workspace(id),
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
        watcher.sync_workspaces(&[(id, root.clone())], &HashSet::new()).await;
        assert!(!watcher.covers_workspace(id), "no backend is no coverage");
        let repo_id = Uuid::now_v7();
        watcher.sync_repositories(&[(repo_id, root.join(".git"))], &HashSet::new());
        assert!(!watcher.covers_repository(repo_id));
    }

    #[tokio::test]
    async fn a_linked_worktree_appearing_surfaces_its_repository() {
        let (dir, root) = repo("fc-fsw-linked").await;
        crate::git::git(&root, &["commit", "-q", "--allow-empty", "-m", "base"]).await.unwrap();

        let watcher = TreeWatcher::start();
        let repo_id = Uuid::now_v7();
        let common = root.join(".git");
        watcher.sync_repositories(&[(repo_id, common.clone())], &HashSet::new());
        assert!(watcher.covers_repository(repo_id));
        let _ = watcher.drain_repositories();

        let linked = dir.path().join("linked");
        crate::git::git(&root, &["worktree", "add", "-q", "-b", "side", linked.to_str().unwrap()])
            .await
            .unwrap();

        let start = Instant::now();
        let mut seen = false;
        while start.elapsed() < Duration::from_secs(10) {
            if watcher.drain_repositories().contains(&repo_id) {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(seen, "a worktree appearing is what retires the thirty-second reconcile poll");
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
                Ok(dirs) => {
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
