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
        let (tx, rx) = std::sync::mpsc::channel::<Vec<PathBuf>>();
        let made = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            // Never more than a send: a callback that waits holds up the
            // stream, and stopping the stream waits for the callback.
            if let Ok(event) = event {
                let _ = tx.send(event.paths);
            }
        });
        let Ok(watcher) = made else {
            locked(&self.retry).insert(key, Instant::now() + RETRY_EVERY);
            return None;
        };
        let routes = self.routes.clone();
        let inner = inner.weak();
        // Ends when the watcher (and with it the sender) is dropped.
        let reader = std::thread::Builder::new().name("fc-projector-watch".into()).spawn(move || {
            while let Ok(first) = rx.recv() {
                // Everything already waiting, read once.
                let mut paths = first;
                while let Ok(more) = rx.try_recv() {
                    paths.extend(more);
                }
                paths.sort();
                paths.dedup();
                let terminals: Vec<Uuid> = {
                    let routes = locked(&routes);
                    let mut found: Vec<Uuid> =
                        paths.iter().filter_map(|p| p.parent().and_then(|d| routes.get(d))).flatten().copied().collect();
                    found.sort();
                    found.dedup();
                    found
                };
                if terminals.is_empty() {
                    continue;
                }
                let Some(inner) = inner.upgrade() else { break };
                inner.read_events(&terminals, &paths);
            }
        });
        if reader.is_err() {
            locked(&self.retry).insert(key, Instant::now() + RETRY_EVERY);
            return None;
        }
        Some(watcher)
    }
}

impl Inner {
    /// What the watch's thread holds: it must not keep the projectors alive.
    fn weak(&self) -> std::sync::Weak<Inner> {
        self.me.clone()
    }
}
