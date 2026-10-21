//! One filesystem watch for every session projector (ov-366 review 1).
//!
//! A watcher per projector cost a thread each (and on Linux an inotify
//! instance, three descriptors and a thread, against a per-user ceiling often
//! of 128), kept until the terminal was deleted. Worse, its callback held the
//! projector: forgotten while an event was in flight, the callback dropped
//! the last reference, the watcher went with it on its own run-loop thread,
//! and FSEvents' `stop()` spun forever waiting for that thread to go idle
//! (measured at 1.96 s of CPU every 2 s).
//!
//! So there is one watcher, its directories routed to the terminals that read
//! them, and its callback only hands each event's paths to a thread of its
//! own, which reads them into the projectors. The callback never waits on
//! anything, so stopping the stream never waits on it; and the watcher is
//! only ever changed or dropped on the thread that asks.
//!
//! A directory that could not be watched (a new worktree's project directory
//! before its first prompt) is tried again at most every `RETRY_EVERY`, not
//! on every tick: on Linux each try was a thread made and torn down.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::Watcher as _;
use uuid::Uuid;

use super::Inner;

/// How long a directory, or the watcher itself, that failed waits before it
/// is tried again.
const RETRY_EVERY: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(crate) struct Watches {
    /// Only locked to change the watch, never by its callback or its thread.
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// Each watched directory, and the terminals that read it.
    routes: Arc<Mutex<HashMap<PathBuf, HashSet<Uuid>>>>,
    /// When a directory that failed may be tried again. The watcher itself
    /// is keyed by the empty path.
    retry: Mutex<HashMap<PathBuf, Instant>>,
    #[cfg(test)]
    pub(super) seam: Arc<Seam>,
}

/// What a test sees of the watch, and does to it (ov-674).
#[cfg(test)]
#[derive(Default)]
pub(super) struct Seam {
    /// Every event is thrown away, as a busy fseventsd loses them.
    pub drop_events: std::sync::atomic::AtomicBool,
    /// The thread each directory was handed to the watcher on.
    pub routed_on: Mutex<Vec<std::thread::ThreadId>>,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Watches {
    /// Have `terminal` read what lands in `dir`. Whether `dir` is watched.
    pub(super) fn route(&self, inner: &Inner, terminal: Uuid, dir: &Path) -> bool {
        let mut watcher = locked(&self.watcher);
        if let Some(readers) = locked(&self.routes).get_mut(dir) {
            readers.insert(terminal);
            return true;
        }
        if locked(&self.retry).get(dir).is_some_and(|at| Instant::now() < *at) {
            return false;
        }
        if watcher.is_none() {
            *watcher = self.start(inner);
        }
        let Some(w) = watcher.as_mut() else { return false };
        #[cfg(test)]
        locked(&self.seam.routed_on).push(std::thread::current().id());
        match w.watch(dir, notify::RecursiveMode::NonRecursive) {
            Ok(()) => {
                locked(&self.routes).entry(dir.to_path_buf()).or_default().insert(terminal);
                locked(&self.retry).remove(dir);
                true
            }
            Err(_) => {
                locked(&self.retry).insert(dir.to_path_buf(), Instant::now() + RETRY_EVERY);
                false
            }
        }
    }

    /// `terminal` no longer reads `dir`; the last reader takes the watch off.
    pub(super) fn unroute(&self, terminal: Uuid, dir: &Path) {
        let mut watcher = locked(&self.watcher);
        let last = {
            let mut routes = locked(&self.routes);
            let Some(readers) = routes.get_mut(dir) else { return };
            readers.remove(&terminal);
            readers.is_empty() && routes.remove(dir).is_some()
        };
        if let (true, Some(w)) = (last, watcher.as_mut()) {
            let _ = w.unwatch(dir);
        }
    }

    /// Drop the watcher here, on the owner's thread.
    pub(super) fn close(&self) {
        let watcher = locked(&self.watcher).take();
        drop(watcher);
        locked(&self.routes).clear();
    }

    /// The watcher, and the thread that reads its events. `None` (and a
    /// retry later) when the platform refuses one.
    fn start(&self, inner: &Inner) -> Option<notify::RecommendedWatcher> {
        let key = PathBuf::new();
        if locked(&self.retry).get(&key).is_some_and(|at| Instant::now() < *at) {
            return None;
        }
        let (tx, rx) = std::sync::mpsc::channel::<Batch>();
        let made = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            // Never more than a send: a callback that waits holds up the
            // stream, and stopping the stream waits for the callback.
            if let Ok(event) = event {
                let _ = tx.send(Batch::of(event));
            }
        });
        let Ok(watcher) = made else {
            locked(&self.retry).insert(key, Instant::now() + RETRY_EVERY);
            return None;
        };
        let routes = self.routes.clone();
        let inner = inner.weak();
        #[cfg(test)]
        let seam = self.seam.clone();
        // Ends when the watcher (and with it the sender) is dropped.
        let reader = std::thread::Builder::new().name("fc-projector-watch".into()).spawn(move || {
            while let Ok(mut batch) = rx.recv() {
                // Everything already waiting, read once.
                while let Ok(more) = rx.try_recv() {
                    batch.join(more);
                }
                #[cfg(test)]
                if seam.drop_events.load(std::sync::atomic::Ordering::SeqCst) {
                    continue;
                }
                if !deliver(&routes, &inner, batch) {
                    break;
                }
            }
        });
        if reader.is_err() {
            locked(&self.retry).insert(key, Instant::now() + RETRY_EVERY);
            return None;
        }
        Some(watcher)
    }
}

/// What the watch said, as its thread reads it: the files named, and the
/// directories it asked to have read again.
#[derive(Default)]
pub(super) struct Batch {
    pub paths: Vec<PathBuf>,
    /// The directories a rescan named (ov-681): FSEvents names the watched
    /// directory itself when fseventsd drops events and asks for one, so it
    /// routes by no file's parent.
    pub rescan: Vec<PathBuf>,
    /// A rescan that named nothing (inotify's queue overflowed): every
    /// directory may have missed something.
    pub rescan_all: bool,
}

impl Batch {
    fn of(event: notify::Event) -> Batch {
        if !event.need_rescan() {
            return Batch { paths: event.paths, ..Batch::default() };
        }
        Batch { rescan_all: event.paths.is_empty(), rescan: event.paths, ..Batch::default() }
    }

    fn join(&mut self, more: Batch) {
        self.paths.extend(more.paths);
        self.rescan.extend(more.rescan);
        self.rescan_all |= more.rescan_all;
    }
}

/// Read what `batch` names into the projectors whose directories hold it:
/// a file, by the directory it is in; a rescan, by every watched directory
/// under the one it names, read in full. `false` once the projectors are
/// gone.
pub(super) fn deliver(routes: &Mutex<HashMap<PathBuf, HashSet<Uuid>>>, inner: &std::sync::Weak<Inner>, batch: Batch) -> bool {
    let Batch { mut paths, rescan, rescan_all } = batch;
    paths.sort();
    paths.dedup();
    let (rescanned, terminals) = {
        let routes = locked(routes);
        let rescanned: HashSet<Uuid> = routes
            .iter()
            .filter(|(dir, _)| rescan_all || rescan.iter().any(|r| dir.starts_with(r)))
            .flat_map(|(_, readers)| readers.iter().copied())
            .collect();
        let mut found: Vec<Uuid> = paths
            .iter()
            .filter_map(|p| p.parent().and_then(|d| routes.get(d)))
            .flatten()
            .copied()
            .filter(|t| !rescanned.contains(t))
            .collect();
        found.sort();
        found.dedup();
        (rescanned.into_iter().collect::<Vec<Uuid>>(), found)
    };
    if terminals.is_empty() && rescanned.is_empty() {
        return true;
    }
    let Some(inner) = inner.upgrade() else { return false };
    inner.read_events(&terminals, &paths);
    inner.read_all(&rescanned);
    true
}

impl Inner {
    /// What the watch's thread holds: it must not keep the projectors alive.
    fn weak(&self) -> std::sync::Weak<Inner> {
        self.me.clone()
    }
}
