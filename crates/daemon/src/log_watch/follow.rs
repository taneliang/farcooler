//! Following the member directories' session logs (ov-593).
//!
//! A member's pane runs claude with `CLAUDE_CONFIG_DIR=<runtime>/claude/<login
//! id>` (`claude_logins`), so its transcripts land under that directory's
//! `projects/`, not under `~/.claude/projects`, which `roots` watches. Without
//! a watch there, a member pane's log join waits for `LOG_JOIN_BACKSTOP_MS`.
//!
//! Member directories come and go while the daemon runs, and `projects/` only
//! exists once claude has run there, so the roots cannot be fixed at start:
//! the caller hands `follow` the current member directories every tick
//! (`claude_registry` already lists them every few seconds; this does no
//! scan of its own) and each one is joined once, as soon as it has a
//! `projects/` that is a real directory.
//!
//! Who may be hostile: a link left where `projects/` should be, pointing the
//! watch at a tree outside the runtime. A link is never followed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use super::LogWatcher;

/// How long after a failed join the next try waits, doubling per failure.
const RETRY_FIRST: Duration = Duration::from_secs(5);
/// The longest wait between tries.
const RETRY_LONGEST: Duration = Duration::from_secs(300);

/// A directory whose join keeps failing.
struct Failing {
    /// Not tried again before this.
    retry_at: Instant,
    /// The wait that follows the next failure.
    next_wait: Duration,
}

/// What `follow` has joined, and the one extra watcher that holds them.
#[derive(Default)]
pub(super) struct Following {
    /// `projects/` directories joined, or being joined now.
    joined: Mutex<HashSet<PathBuf>>,
    /// Directories whose last join failed, with when to try again. An entry
    /// is also the memory that the failure was already logged.
    failing: Mutex<HashMap<PathBuf, Failing>>,
    /// Made by the first join. Locked for the whole of a registration, which
    /// can block on `fseventsd` (see `LogWatcher::start`), so only the
    /// threads `follow` spawns ever wait on it.
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// Set when the `LogWatcher` is dropped: a join that finishes after that
    /// lets its directory go.
    pub(super) closed: AtomicBool,
}

/// What time it is. Passed in so a test can move it.
type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// Puts `path` under a watcher. Passed in so a test can fail it.
type Register = Arc<dyn Fn(&mut RecommendedWatcher, &Path) -> notify::Result<()> + Send + Sync>;

fn watch_recursively(watcher: &mut RecommendedWatcher, path: &Path) -> notify::Result<()> {
    watcher.watch(path, RecursiveMode::Recursive)
}

impl LogWatcher {
    /// Join the `projects/` of each of `member_dirs` not joined yet.
    ///
    /// Returns at once: a registration can stall for minutes (see `start`),
    /// so each runs on a thread of its own. A directory with no `projects/`
    /// yet, or one that is a link, is passed over and looked at again on the
    /// next call. A join that lands reports its directory as changed, once,
    /// since anything written while it registered produced no event. One that
    /// fails is tried again after a wait that doubles from five seconds to
    /// five minutes, and logged only the first time.
    pub(crate) fn follow(&self, member_dirs: &[PathBuf]) {
        self.follow_with(member_dirs, Arc::new(Instant::now), Arc::new(watch_recursively));
    }

    /// `follow`, with the clock and the registration passed in, so a test can
    /// fail a registration and move time. The clock is read when a failure is
    /// recorded, not before the registration starts: one that stalled for
    /// minutes must not leave a retry time that is already past.
    fn follow_with(&self, member_dirs: &[PathBuf], clock: Clock, register: Register) {
        let now = clock();
        // A directory that has gone is let go of, so one made again is joined,
        // and its failure record goes with it (an old retry time and warning
        // must not hold a new directory back). The watcher may be busy in a
        // registration; then the joined ones are looked at again on the next
        // call, not waited for here. Nothing is held across `unwatch`.
        self.following.failing.lock().unwrap_or_else(|e| e.into_inner()).retain(|p, _| p.exists());
        let gone: Vec<PathBuf> =
            self.following.joined.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|p| !p.exists()).cloned().collect();
        if !gone.is_empty()
            && let Ok(mut held) = self.following.watcher.try_lock()
        {
            for path in &gone {
                if let Some(watcher) = held.as_mut() {
                    let _ = watcher.unwatch(path);
                }
            }
            drop(held);
            let mut joined = self.following.joined.lock().unwrap_or_else(|e| e.into_inner());
            for path in &gone {
                joined.remove(path);
            }
        }
        let mut joined = self.following.joined.lock().unwrap_or_else(|e| e.into_inner());
        for dir in member_dirs {
            let projects = dir.join("projects");
            if joined.contains(&projects) || !is_real_dir(&projects) || !is_real_dir(dir) {
                continue;
            }
            let waiting = self.following.failing.lock().unwrap_or_else(|e| e.into_inner()).get(&projects).is_some_and(|f| f.retry_at > now);
            if waiting {
                continue;
            }
            joined.insert(projects.clone());
            let (following, sink, register, clock) = (self.following.clone(), self.changed.clone(), register.clone(), clock.clone());
            let spawned = std::thread::Builder::new().name("log-watch-follow".into()).spawn({
                let projects = projects.clone();
                move || join(&following, &sink, &projects, clock.as_ref(), register.as_ref())
            });
            if let Err(error) = spawned {
                tracing::warn!(?error, "could not start the thread that joins a member's log watch");
                joined.remove(&projects);
            }
        }
    }
}

fn join(
    following: &Following,
    sink: &Arc<Mutex<HashSet<PathBuf>>>,
    projects: &Path,
    clock: &(dyn Fn() -> Instant + Send + Sync),
    register: &(dyn Fn(&mut RecommendedWatcher, &Path) -> notify::Result<()> + Send + Sync),
) {
    let mut held = following.watcher.lock().unwrap_or_else(|e| e.into_inner());
    if following.closed.load(Ordering::SeqCst) {
        return;
    }
    if held.is_none() {
        let into = sink.clone();
        *held = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            into.lock().unwrap_or_else(|e| e.into_inner()).extend(event.paths);
        })
        .inspect_err(|error| tracing::warn!(?error, "could not start a watcher for the member log directories"))
        .ok();
    }
    let result = match held.as_mut() {
        Some(w) => register(w, projects),
        None => Err(notify::Error::generic("no watcher")),
    };
    drop(held);
    let mut failing = following.failing.lock().unwrap_or_else(|e| e.into_inner());
    match result {
        Ok(()) => {
            failing.remove(projects);
            if !following.closed.load(Ordering::SeqCst) {
                sink.lock().unwrap_or_else(|e| e.into_inner()).insert(projects.to_path_buf());
            }
        }
        Err(error) => {
            let wait = failing.get(projects).map_or(RETRY_FIRST, |f| f.next_wait);
            if !failing.contains_key(projects) {
                tracing::warn!(root = %projects.display(), ?error, "could not watch a member's log directory");
            }
            failing.insert(projects.to_path_buf(), Failing { retry_at: clock() + wait, next_wait: (wait * 2).min(RETRY_LONGEST) });
            drop(failing);
            following.joined.lock().unwrap_or_else(|e| e.into_inner()).remove(projects);
        }
    }
}

/// A real directory, never a link.
fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn scratch(name: &str) -> PathBuf {
        farcooler_core::scratch::dir(&format!("log-follow-{name}"))
    }

    /// An empty watcher that is registered, so only `follow` adds to it.
    fn bare() -> LogWatcher {
        let watcher = LogWatcher::start(Vec::new());
        assert!(watcher.wait_until_registered(Duration::from_secs(60)));
        watcher.drain();
        watcher
    }

    /// Drain, accumulating, until `file` has surfaced (compared canonical:
    /// `/var` is `/private/var` on macOS) or `deadline` passes. Everything
    /// drained along the way is returned, so a path that should NOT surface
    /// can be checked against the same list.
    fn collect(watcher: &LogWatcher, file: &Path, deadline: Duration) -> Vec<PathBuf> {
        let want = canon(file);
        let start = Instant::now();
        let mut seen = Vec::new();
        loop {
            seen.extend(watcher.drain());
            if seen.iter().any(|p| canon(p) == want) || start.elapsed() > deadline {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn canon(p: &Path) -> PathBuf {
        p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
    }

    fn surfaced(watcher: &LogWatcher, file: &Path, deadline: Duration) -> bool {
        collect(watcher, file, deadline).iter().any(|p| canon(p) == canon(file))
    }

    /// Write `log`, and write it again every few seconds until it surfaces or
    /// `ceiling` passes. A busy fseventsd can drop a single event outright
    /// (ov-656), and waiting longer for a lost one waits for nothing; the
    /// first delivery ends the wait, so a pass costs what it always did.
    fn written_and_surfaced(watcher: &LogWatcher, log: &Path, ceiling: Duration) -> bool {
        let start = Instant::now();
        loop {
            std::fs::write(log, "{}").unwrap();
            if surfaced(watcher, log, Duration::from_secs(4)) {
                return true;
            }
            if start.elapsed() > ceiling {
                return false;
            }
        }
    }

    /// Calls `follow` until `projects` has been joined (it reports itself).
    fn follow_until_joined(watcher: &LogWatcher, dirs: &[PathBuf], projects: &Path) {
        let start = Instant::now();
        loop {
            watcher.follow(dirs);
            if surfaced(watcher, projects, Duration::from_millis(200)) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(60), "never joined {projects:?}");
        }
    }

    /// A member directory that already has `projects/` is watched: a log
    /// written there surfaces without the backstop.
    #[test]
    fn a_member_projects_dir_is_watched() {
        let member = scratch("watched");
        let projects = member.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        let watcher = bare();
        follow_until_joined(&watcher, std::slice::from_ref(&member), &projects);
        let log = projects.join("s.jsonl");
        assert!(
            written_and_surfaced(&watcher, &log, Duration::from_secs(60)),
            "a member's log write was not noticed"
        );
    }

    /// A member whose `projects/` is made after the first call (claude has
    /// not run there yet) is picked up by a later one.
    #[test]
    fn a_projects_dir_made_later_is_picked_up() {
        let member = scratch("later");
        let watcher = bare();
        watcher.follow(std::slice::from_ref(&member));
        assert!(watcher.following.joined.lock().unwrap().is_empty(), "joined a directory that is not there");
        let projects = member.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        follow_until_joined(&watcher, std::slice::from_ref(&member), &projects);
        let log = projects.join("late.jsonl");
        assert!(
            written_and_surfaced(&watcher, &log, Duration::from_secs(60)),
            "a later member's log write was not noticed"
        );
    }

    /// A link where `projects/` should be is not followed: a write to what
    /// it points at never surfaces, while a real member beside it does.
    #[test]
    fn a_linked_projects_dir_is_not_followed() {
        let outside = scratch("outside");
        let linked = scratch("linked");
        std::os::unix::fs::symlink(&outside, linked.join("projects")).unwrap();
        let real = scratch("real");
        let real_projects = real.join("projects");
        std::fs::create_dir_all(&real_projects).unwrap();
        let watcher = bare();
        let dirs = [linked, real];
        follow_until_joined(&watcher, &dirs, &real_projects);
        let (stray, first, second) = (outside.join("stray.jsonl"), real_projects.join("a.jsonl"), real_projects.join("b.jsonl"));
        std::fs::write(&first, "{}").unwrap();
        std::fs::write(&stray, "{}").unwrap();
        std::fs::write(&second, "{}").unwrap();
        // The write after the stray is the marker: once it has surfaced, the
        // stray's event, had there been one, is in too. Everything drained
        // is kept, so the stray is checked against all of it.
        let seen = collect(&watcher, &second, Duration::from_secs(30));
        assert!(seen.iter().any(|p| canon(p) == canon(&second)), "the real member's write was not noticed");
        assert!(!seen.iter().any(|p| canon(p) == canon(&stray)), "followed a link out of the member directory: {seen:?}");
    }

    /// A registration that fails with a count of its tries, and succeeds
    /// from try `ok_from` on (never, for `usize::MAX`).
    fn flaky(tries: &Arc<AtomicUsize>, ok_from: usize) -> Register {
        let tries = tries.clone();
        Arc::new(move |watcher: &mut RecommendedWatcher, path: &Path| {
            let n = tries.fetch_add(1, Ordering::SeqCst) + 1;
            if n >= ok_from { watch_recursively(watcher, path) } else { Err(notify::Error::generic("injected")) }
        })
    }

    /// Wait until try number `n` of a failing join has run and been
    /// recorded (a failure takes the path out of `joined` last).
    fn settled(watcher: &LogWatcher, tries: &AtomicUsize, n: usize) {
        let start = Instant::now();
        while tries.load(Ordering::SeqCst) < n || tries_so_far(watcher) != 0 {
            assert!(start.elapsed() < Duration::from_secs(60), "try {n} never finished");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A clock a test sets by hand.
    #[derive(Clone)]
    struct Hand(Arc<Mutex<Instant>>);

    impl Hand {
        fn new() -> Hand {
            Hand(Arc::new(Mutex::new(Instant::now())))
        }
        fn clock(&self) -> Clock {
            let cell = self.0.clone();
            Arc::new(move || *cell.lock().unwrap())
        }
        fn set(&self, to: Instant) {
            *self.0.lock().unwrap() = to;
        }
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
        /// `follow_with` at `at`.
        fn follow(&self, watcher: &LogWatcher, dirs: &[PathBuf], at: Instant, register: &Register) {
            self.set(at);
            watcher.follow_with(dirs, self.clock(), register.clone());
        }
    }

    fn tries_so_far(watcher: &LogWatcher) -> usize {
        watcher.following.joined.lock().unwrap().len()
    }

    /// A join that keeps failing is not tried again every call: the wait
    /// doubles from `RETRY_FIRST`, and the clock here is a made-up one.
    #[test]
    fn a_join_that_keeps_failing_backs_off() {
        let member = scratch("backoff");
        std::fs::create_dir_all(member.join("projects")).unwrap();
        let dirs = [member];
        let watcher = bare();
        let tries = Arc::new(AtomicUsize::new(0));
        let register = flaky(&tries, usize::MAX);
        let hand = Hand::new();
        let t0 = hand.now();
        hand.follow(&watcher, &dirs, t0, &register);
        settled(&watcher, &tries, 1);
        assert_eq!(tries_so_far(&watcher), 0, "a failed join is not kept as joined");
        // Inside the first wait: nothing is started (a start marks it joined).
        hand.follow(&watcher, &dirs, t0 + RETRY_FIRST - Duration::from_secs(1), &register);
        assert_eq!(tries_so_far(&watcher), 0, "tried again inside the wait");
        assert_eq!(tries.load(Ordering::SeqCst), 1);
        // Past it: tried, and failed again, with a doubled wait.
        let t1 = t0 + RETRY_FIRST;
        hand.follow(&watcher, &dirs, t1, &register);
        settled(&watcher, &tries, 2);
        hand.follow(&watcher, &dirs, t1 + RETRY_FIRST * 2 - Duration::from_secs(1), &register);
        assert_eq!(tries_so_far(&watcher), 0, "the wait did not double");
        hand.follow(&watcher, &dirs, t1 + RETRY_FIRST * 2, &register);
        settled(&watcher, &tries, 3);
        // The wait stops doubling at the longest.
        let mut at = t1 + RETRY_FIRST * 2;
        for n in 4..=12 {
            at += RETRY_LONGEST;
            hand.follow(&watcher, &dirs, at, &register);
            settled(&watcher, &tries, n);
        }
        let last = watcher.following.failing.lock().unwrap().values().map(|f| f.next_wait).next().unwrap();
        assert_eq!(last, RETRY_LONGEST);
    }

    /// A failed join is retried once its wait is up, and a success clears
    /// the failure (so a later failure would be logged again).
    #[test]
    fn a_failed_join_is_retried_and_then_holds() {
        let member = scratch("retried");
        let projects = member.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        let dirs = [member];
        let watcher = bare();
        let tries = Arc::new(AtomicUsize::new(0));
        let register = flaky(&tries, 2);
        let hand = Hand::new();
        let t0 = hand.now();
        hand.follow(&watcher, &dirs, t0, &register);
        settled(&watcher, &tries, 1);
        assert_eq!(tries_so_far(&watcher), 0);
        hand.follow(&watcher, &dirs, t0 + RETRY_FIRST, &register);
        let start = Instant::now();
        while !watcher.following.failing.lock().unwrap().is_empty() {
            assert!(start.elapsed() < Duration::from_secs(60), "the retry never succeeded");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(tries_so_far(&watcher), 1, "joined");
        let log = projects.join("r.jsonl");
        assert!(
            written_and_surfaced(&watcher, &log, Duration::from_secs(60)),
            "the retried join does not watch"
        );
        // Joined: more calls start nothing.
        hand.follow(&watcher, &dirs, t0 + RETRY_LONGEST, &register);
        assert_eq!(tries.load(Ordering::SeqCst), 2);
    }

    /// A `projects/` deleted and made again is joined again: the first join
    /// is let go of when the directory goes.
    #[test]
    fn a_deleted_projects_dir_made_again_is_rejoined() {
        let member = scratch("recreated");
        let projects = member.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        let dirs = [member];
        let watcher = bare();
        let tries = Arc::new(AtomicUsize::new(0));
        let register = flaky(&tries, 1);
        let hand = Hand::new();
        hand.follow(&watcher, &dirs, Instant::now(), &register);
        let start = Instant::now();
        while !surfaced(&watcher, &projects, Duration::from_millis(100)) {
            assert!(start.elapsed() < Duration::from_secs(60), "never joined");
        }
        std::fs::remove_dir_all(&projects).unwrap();
        hand.follow(&watcher, &dirs, Instant::now(), &register);
        assert_eq!(tries_so_far(&watcher), 0, "a deleted directory is still held as joined");
        std::fs::create_dir_all(&projects).unwrap();
        hand.follow(&watcher, &dirs, Instant::now(), &register);
        let start = Instant::now();
        while tries.load(Ordering::SeqCst) < 2 {
            assert!(start.elapsed() < Duration::from_secs(60), "the recreated directory was not joined again");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A registration that stalls (the clock moves while it runs) still
    /// leaves a wait counted from when it failed, not from when it began.
    #[test]
    fn a_stalled_registration_still_waits_after_it_fails() {
        let member = scratch("stalled-backoff");
        std::fs::create_dir_all(member.join("projects")).unwrap();
        let dirs = [member];
        let watcher = bare();
        let hand = Hand::new();
        let t0 = hand.now();
        let tries = Arc::new(AtomicUsize::new(0));
        let (moved, count) = (hand.clone(), tries.clone());
        let register: Register = Arc::new(move |_: &mut RecommendedWatcher, _: &Path| {
            count.fetch_add(1, Ordering::SeqCst);
            moved.set(moved.now() + Duration::from_secs(100));
            Err(notify::Error::generic("injected"))
        });
        hand.follow(&watcher, &dirs, t0, &register);
        settled(&watcher, &tries, 1);
        // 50 s in is inside the wait counted from the failure at 100 s, and
        // past the one counted from the start at 0 s.
        hand.follow(&watcher, &dirs, t0 + Duration::from_secs(50), &register);
        assert_eq!(tries_so_far(&watcher), 0, "tried again inside the wait");
        assert_eq!(tries.load(Ordering::SeqCst), 1);
    }

    /// A failure record for a directory that has gone is dropped, so one made
    /// again is tried at once, not held to the old wait.
    #[test]
    fn a_failure_record_goes_with_its_directory() {
        let member = scratch("pruned");
        let projects = member.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        let dirs = [member];
        let watcher = bare();
        let hand = Hand::new();
        let t0 = hand.now();
        let tries = Arc::new(AtomicUsize::new(0));
        let register = flaky(&tries, usize::MAX);
        hand.follow(&watcher, &dirs, t0, &register);
        settled(&watcher, &tries, 1);
        assert_eq!(watcher.following.failing.lock().unwrap().len(), 1);
        std::fs::remove_dir_all(&projects).unwrap();
        hand.follow(&watcher, &dirs, t0 + Duration::from_secs(1), &register);
        assert!(watcher.following.failing.lock().unwrap().is_empty(), "a removed directory leaks its record");
        std::fs::create_dir_all(&projects).unwrap();
        hand.follow(&watcher, &dirs, t0 + Duration::from_secs(2), &register);
        let start = Instant::now();
        while tries.load(Ordering::SeqCst) < 2 {
            assert!(start.elapsed() < Duration::from_secs(60), "a recreated directory was held to the old wait");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
