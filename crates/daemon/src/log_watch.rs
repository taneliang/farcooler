//! Noticing that a session log changed, without reading it.
//!
//! The daemon used to learn an agent's state by round-tripping tmux once a
//! second, per pane, forever — work spent even while the agent sits idle.
//! Watching the three log roots (see `docs/agent-session-logs.md`) replaces
//! that with the filesystem telling us when something moved. This module's
//! whole job is "did a path under one of these roots change", and stops
//! there: reading and parsing what changed is `farcooler-core`'s, built by a
//! different task running in parallel on the same tree.
//!
//! Deliberately outside `farcooler-core`: it has no filesystem-watching
//! dependency today, and giving it one for a concern that is purely about
//! *when* the daemon wakes up — not what any of the three formats mean —
//! would tie a library crate to a notification backend it does not need for
//! anything else it does.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Watches the agent-log roots and remembers which paths under them changed.
///
/// Coalescing lives in the data structure, not in a timer: every event just
/// inserts its paths into a `HashSet`, so ten appends to one file during a
/// burst are ten insertions of the same key and one entry to show for it.
/// There is nothing a debounce window would add here that the set does not
/// already give for free — the "short window" the brief describes is
/// whatever gap sits between two calls to `drain`, which is the caller's
/// sampling cadence, not something this type needs to invent a clock for.
pub struct LogWatcher {
    changed: Arc<Mutex<HashSet<PathBuf>>>,
    /// Where the registering thread leaves the watcher once it exists, and
    /// what this `LogWatcher` takes back when it is dropped. See `Slot`.
    slot: Arc<Mutex<Slot>>,
}

/// The watcher's life, shared between a `LogWatcher` and the thread that
/// registers for it.
///
/// Kept alive for as long as the `LogWatcher` is: a `notify::Watcher` stops
/// watching the moment it is dropped. `Watching` holds `None` only when the platform
/// backend itself could not be built, which is not a per-root failure and has
/// nothing left to retry.
enum Slot {
    Registering,
    Watching { _watcher: Option<RecommendedWatcher> },
    /// The `LogWatcher` went away first. A registration that finishes after
    /// this drops its watcher on the spot rather than keeping one nobody reads.
    Closed,
}

impl Drop for LogWatcher {
    fn drop(&mut self) {
        let old = std::mem::replace(&mut *self.slot.lock().unwrap_or_else(|e| e.into_inner()), Slot::Closed);
        drop(old);
    }
}

/// The three directories the agents write their sessions under.
///
/// Named here rather than at the call site because "which roots" is this
/// module's own subject, and a second list somewhere else is a list that can
/// disagree with this one about which agents exist. Empty when `$HOME` is
/// unset, which `start` already treats as "nothing to watch" rather than an
/// error.
pub fn roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    vec![
        home.join(".claude/projects"),
        home.join(".codex/sessions"),
        home.join(".cursor/projects"),
    ]
}

impl LogWatcher {
    /// Start watching every root that exists, recursively.
    ///
    /// A root that does not exist is not an error and not even logged above
    /// debug: `~/.cursor/projects` genuinely does not exist on a machine that
    /// has never run cursor-agent, and most developers have never run all
    /// three agents this watches for. Refusing to start over that would break
    /// the daemon for most users on day one.
    ///
    /// Recursive: sessions live several directories deep
    /// (`~/.codex/sessions/YYYY/MM/DD/`,
    /// `~/.cursor/projects/<slug>/agent-transcripts/<uuid>/`) and new date
    /// directories appear while the daemon keeps running. `RecursiveMode::Recursive`
    /// covers both — the platform backends watch the whole subtree, including
    /// directories created after `watch` was called, so there is no separate
    /// path to re-register when codex rolls over to a new day.
    ///
    /// **Returns at once; the watch is registered on a thread of its own.**
    /// `Watcher::watch` on macOS is `FSEventStreamStart`, a round trip to
    /// `fseventsd`, and with `fseventsd` backed up (an Xcode build, an
    /// XProtect scan) it has been sampled blocking for MINUTES inside
    /// `register_with_server`. This runs from `watch::Watcher::new`, on the
    /// daemon's startup path, so registering inline held the whole daemon --
    /// and every `rpc_over_socket` harness start, 1530 s for one run of that
    /// file -- behind `fseventsd`. Nothing reads this watcher as a promise:
    /// its one caller uses `drain` as a gate with a backstop on its own clock
    /// (`watch.rs`, `LOG_JOIN_BACKSTOP_MS`), so a watch that is not live yet
    /// costs a join that waits for the backstop, never one that is missed.
    ///
    /// And to cut even that short: once registration finishes, every root is
    /// reported as changed, once. Whatever was written while the registration
    /// stalled produced no event, and this is what tells the caller to look.
    pub fn start(roots: Vec<PathBuf>) -> LogWatcher {
        Self::start_registering(roots, watch_roots)
    }

    /// `start`, with the registration passed in, so a test can stand in one
    /// that stalls.
    fn start_registering<R>(roots: Vec<PathBuf>, register: R) -> LogWatcher
    where
        R: FnOnce(&[PathBuf], Arc<Mutex<HashSet<PathBuf>>>) -> Option<RecommendedWatcher> + Send + 'static,
    {
        let changed: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
        let slot = Arc::new(Mutex::new(Slot::Registering));
        let (sink, landing) = (changed.clone(), slot.clone());
        let spawned = std::thread::Builder::new().name("log-watch-register".into()).spawn(move || {
            // A panic in `notify` is a registration that failed, the same as
            // `None`: left to unwind, it would leave this slot `Registering`
            // for good with nothing to say why.
            let watcher = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| register(&roots, sink.clone())))
                .unwrap_or_else(|_| {
                    tracing::warn!("registering the log watch panicked; falling back to no watching");
                    None
                });
            let mut slot = landing.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(*slot, Slot::Closed) {
                return;
            }
            // The report goes in BEFORE the slot leaves `Registering`, under
            // its lock: anyone who sees the registration finished must also
            // see the report, or a drain taken in between misses it.
            sink.lock().unwrap_or_else(|e| e.into_inner()).extend(roots.into_iter().filter(|r| r.exists()));
            *slot = Slot::Watching { _watcher: watcher };
        });
        if let Err(error) = spawned {
            tracing::warn!(?error, "could not start the thread that registers the log watch; falling back to no watching");
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Slot::Watching { _watcher: None };
        }
        LogWatcher { changed, slot }
    }

    /// Whether registration has finished, successfully or not. For tests,
    /// which cannot write a file and expect an event until it has.
    #[cfg(test)]
    fn wait_until_registered(&self, deadline: std::time::Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < deadline {
            if !matches!(*self.slot.lock().unwrap_or_else(|e| e.into_inner()), Slot::Registering) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    /// The paths that changed since the last call to `drain`.
    ///
    /// Idempotent: draining empties the set, so calling this twice with no
    /// writes in between returns nothing the second time. Order is not
    /// meaningful — callers wanting a stable order should sort.
    pub fn drain(&self) -> Vec<PathBuf> {
        let mut set = self.changed.lock().unwrap_or_else(|e| e.into_inner());
        set.drain().collect()
    }
}

/// Build the watcher and register every root that exists, recursively --
/// the blocking half of `LogWatcher::start`, run on its own thread.
///
/// A root that does not exist is not an error and not even logged above
/// debug: see `LogWatcher::start`. Every path in every event is kept, with no
/// filtering by `EventKind`. The three backends (FSEvents, inotify,
/// ReadDirectoryChangesW) do not agree on which kind an append surfaces as,
/// and the cost of being wrong in the permissive direction is a redundant
/// entry in a `HashSet` — the cost of being wrong the other way is a session
/// log that grows and nobody notices.
fn watch_roots(roots: &[PathBuf], sink: Arc<Mutex<HashSet<PathBuf>>>) -> Option<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        let mut set = sink.lock().unwrap_or_else(|e| e.into_inner());
        set.extend(event.paths);
    });
    let mut watcher = match watcher {
        Ok(w) => w,
        Err(error) => {
            // Not a per-root problem — the backend itself failed to start,
            // which no amount of retrying a root fixes. Logged and left inert
            // rather than panicking the daemon over a feature that trades
            // work for none, never none for none.
            tracing::warn!(?error, "could not start a log watcher; falling back to no watching");
            return None;
        }
    };
    for root in roots {
        if !root.exists() {
            tracing::debug!(root = %root.display(), "log root does not exist, skipping");
            continue;
        }
        if let Err(error) = watcher.watch(root, RecursiveMode::Recursive) {
            tracing::warn!(root = %root.display(), ?error, "could not watch log root");
        }
    }
    Some(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// `LogWatcher::start`, then wait for its registration to finish and
    /// drain the one report of every root that comes with it. A test that
    /// writes a file and expects the event can only do so once the watch is
    /// live, which `start` no longer waits for (see its doc).
    fn registered(roots: Vec<PathBuf>) -> LogWatcher {
        let watcher = LogWatcher::start(roots);
        assert!(watcher.wait_until_registered(Duration::from_secs(60)), "the log watch never registered");
        watcher.drain();
        watcher
    }

    /// The daemon's startup path is not held behind a registration that
    /// stalls -- `FSEventStreamStart` has been sampled blocking for minutes
    /// when `fseventsd` is backed up. The stand-in registration here blocks
    /// until the test lets it go.
    #[test]
    fn a_stalled_registration_does_not_hold_up_start() {
        let root = scratch("stalled");
        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let begun = Instant::now();
        let watcher = LogWatcher::start_registering(vec![root.clone()], move |_, _| {
            entered_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(10));
            None
        });
        let took = begun.elapsed();
        entered_rx.recv_timeout(Duration::from_secs(10)).expect("the registration ran");
        assert!(!watcher.wait_until_registered(Duration::from_millis(100)), "still registering");
        release_tx.send(()).unwrap();
        assert!(took < Duration::from_secs(2), "start waited {took:?} for a registration that stalled");
        assert!(watcher.wait_until_registered(Duration::from_secs(10)), "registration finished once released");
    }

    /// Anything written while registration stalled produced no event, so a
    /// finished registration reports every root that exists as changed,
    /// once: the caller's next tick looks, instead of waiting for its
    /// backstop.
    #[test]
    fn a_finished_registration_reports_every_root_once() {
        let root = scratch("reported");
        let missing = scratch("reported-parent").join("absent");
        let watcher = LogWatcher::start_registering(vec![root.clone(), missing], |_, _| None);
        assert!(watcher.wait_until_registered(Duration::from_secs(10)));
        assert_eq!(watcher.drain(), vec![root], "each root that exists, and no other");
        assert!(watcher.drain().is_empty(), "once");
    }

    /// A registration that panics finishes like one that failed: the slot
    /// leaves `Registering`, and the roots are still reported.
    #[test]
    fn a_registration_that_panics_finishes_as_no_watcher() {
        let root = scratch("panics");
        let watcher = LogWatcher::start_registering(vec![root.clone()], |_, _| panic!("notify's runloop thread died"));
        assert!(watcher.wait_until_registered(Duration::from_secs(10)), "stuck registering after a panic");
        assert_eq!(watcher.drain(), vec![root]);
    }

    /// A `LogWatcher` dropped while its registration is still running does
    /// not have a watcher land in it afterwards: the finished registration
    /// finds the slot closed and lets its watcher go.
    #[test]
    fn a_registration_that_finishes_after_the_drop_keeps_nothing() {
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let watcher = LogWatcher::start_registering(Vec::new(), move |_, _| {
            let _ = release_rx.recv_timeout(Duration::from_secs(10));
            let built = notify::recommended_watcher(|_: notify::Result<notify::Event>| {}).ok();
            done_tx.send(()).unwrap();
            built
        });
        let slot = watcher.slot.clone();
        drop(watcher);
        release_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(matches!(*slot.lock().unwrap(), Slot::Closed), "a watcher landed in a dropped LogWatcher");
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("farcooler-log-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Polls `drain` instead of sleeping a fixed amount: the platform backend
    /// (FSEvents on macOS) has its own latency before an event surfaces, and a
    /// fixed sleep either wastes time on a fast machine or flakes on a slow
    /// one. Panics past the deadline, which is the failure this exists to
    /// catch — a change that never surfaces at all.
    fn wait_for_drain(watcher: &LogWatcher, deadline: Duration) -> Vec<PathBuf> {
        let start = Instant::now();
        loop {
            let found = watcher.drain();
            if !found.is_empty() || start.elapsed() > deadline {
                return found;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn a_write_under_a_watched_root_surfaces_its_path() {
        let root = scratch("basic");
        let watcher = registered(vec![root.clone()]);

        let file = root.join("session.jsonl");
        std::fs::write(&file, "{}").unwrap();

        let found = wait_for_drain(&watcher, Duration::from_secs(5));
        let canonical_file = file.canonicalize().unwrap_or_else(|_| file.clone());
        assert!(
            found.iter().any(|p| p.canonicalize().unwrap_or_else(|_| p.clone()) == canonical_file),
            "expected {file:?} among {found:?}"
        );
    }

    /// Drain until the backend has stopped producing, or the deadline passes.
    ///
    /// **One write is not one event.** Linux inotify reports a create AND a
    /// modify for a single `fs::write`, and each reaches the watcher thread
    /// separately; macOS coalesces differently again. `drain` is a set, so it
    /// collapses duplicates INSIDE one batch and cannot collapse across two —
    /// which means the first non-empty drain can return while the rest of that
    /// same write is still in flight, and the next drain hands back the same
    /// path having invented nothing.
    ///
    /// Half a second of continuous quiet, not one empty drain: a single empty
    /// drain only says nothing arrived in the last 50ms, which is also true in
    /// the gap between a create and its modify. The gap is what has to be
    /// outlasted, and on a loaded CI runner it is not small — this test began
    /// failing on ubuntu EVERY run once enough sibling tests in this binary
    /// started spawning `git`, having passed for months before that.
    fn wait_until_quiet(watcher: &LogWatcher, deadline: Duration) {
        let start = Instant::now();
        let mut empties = 0;
        while empties < 10 && start.elapsed() < deadline {
            if watcher.drain().is_empty() {
                empties += 1;
            } else {
                empties = 0;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn drain_is_idempotent() {
        let root = scratch("idempotent");
        let watcher = registered(vec![root.clone()]);

        std::fs::write(root.join("session.jsonl"), "{}").unwrap();
        let first = wait_for_drain(&watcher, Duration::from_secs(5));
        assert!(!first.is_empty(), "the write should have surfaced first");

        // Asserted against a QUIET watcher rather than against the tick after
        // the first drain. What this test is about is that `drain` clears what
        // it hands back; a straggling inotify event for the write above is the
        // OS speaking a second time, which is a different thing entirely and
        // was failing this test on ubuntu perhaps one run in several.
        wait_until_quiet(&watcher, Duration::from_secs(10));

        // Nothing written since. Drain must not produce one.
        let second = watcher.drain();
        assert!(second.is_empty(), "a second drain with nothing new must return nothing: {second:?}");
    }

    #[test]
    fn a_missing_root_is_not_an_error() {
        // `~/.cursor/projects` on a machine that has never run cursor-agent,
        // reproduced without touching a real home directory.
        let missing = scratch("parent").join("does-not-exist-yet");
        let real = scratch("sibling");

        // Must not panic, and the root that DOES exist must still work —
        // one bad root must not take the good ones down with it.
        let watcher = registered(vec![missing, real.clone()]);
        std::fs::write(real.join("session.jsonl"), "{}").unwrap();

        let file = real.join("session.jsonl");
        let found = wait_for_drain(&watcher, Duration::from_secs(5));
        let canonical_file = file.canonicalize().unwrap_or_else(|_| file.clone());
        assert!(
            found.iter().any(|p| p.canonicalize().unwrap_or_else(|_| p.clone()) == canonical_file),
            "the root that exists must still be watched: expected {file:?} among {found:?}"
        );
    }

    #[test]
    fn ten_writes_to_one_file_coalesce_to_one_path() {
        let root = scratch("coalesce");
        let file = root.join("session.jsonl");
        std::fs::write(&file, "").unwrap();

        let watcher = registered(vec![root.clone()]);

        for i in 0..10 {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
            writeln!(f, "line {i}").unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }

        // One settling wait rather than draining after every write: the point
        // under test is that a burst collapses to one entry, which a drain
        // taken mid-burst could not show either way.
        std::thread::sleep(Duration::from_millis(500));
        let found = watcher.drain();

        let canonical_file = file.canonicalize().unwrap_or(file.clone());
        let hits = found
            .iter()
            .filter(|p| p.canonicalize().unwrap_or_else(|_| (*p).clone()) == canonical_file)
            .count();
        assert_eq!(hits, 1, "ten writes to one file must surface it once, not {hits} times: {found:?}");
    }

    /// Run by hand, not in CI: it watches the real `~/.claude/projects`
    /// directory, which on a working machine holds a large amount of real
    /// session history, and this exists to confirm a watcher does not choke
    /// on it. `cargo test -p farcooler-daemon log_watch -- --ignored --nocapture`.
    ///
    /// Does NOT assert an empty `drain` — that assumption turned out to be
    /// false on the machine this was verified against, which runs several
    /// agents at once (this very task is one of a batch). A first run
    /// surfaced two paths, and their mtimes matched the test's own clock to
    /// the second: a real, concurrently running session in another worktree
    /// was actually writing to them. That is the watcher working, not a
    /// spurious event, so the test only checks that whatever it reports is
    /// real (an mtime this watcher itself could plausibly have caused) and
    /// that the whole thing completes promptly rather than hanging — the
    /// actual question this exists to answer for a directory this large.
    #[test]
    #[ignore]
    fn watching_the_real_claude_projects_directory_does_not_choke_on_a_large_tree() {
        let Some(home) = dirs_home() else { return };
        let root = home.join(".claude/projects");
        if !root.exists() {
            return;
        }
        let started = std::time::SystemTime::now();
        let watcher = registered(vec![root]);
        std::thread::sleep(Duration::from_secs(3));
        let found = watcher.drain();

        println!("{} path(s) changed while watching: {found:?}", found.len());
        for path in &found {
            // Real activity, not noise: whatever it reports must actually
            // have been touched during the window this watched, not a path
            // dredged up from unrelated history.
            let modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
            match modified {
                Some(m) => assert!(
                    m >= started,
                    "{path:?} was reported but last modified before the watch even started"
                ),
                None => panic!("{path:?} was reported but no longer exists"),
            }
        }
    }

    #[cfg(test)]
    fn dirs_home() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}
