//! The platform half of `TreeWatcher`: something that takes a path and calls
//! back with paths.
//!
//! macOS gets FSEvents directly, one stream per watched path, so a worktree's
//! stream can leave its build output out — see `fsevents`. Everywhere else is
//! `notify`, exactly as before: inotify is registered one tracked directory at
//! a time and never sees an ignored one, so it has nothing to exclude, and
//! Windows keeps the one recursive `notify` watch it had.

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What every backend calls with a batch of changed paths.
pub(super) type Sink = Arc<dyn Fn(&[PathBuf]) + Send + Sync>;

/// Why a path could not be watched.
#[derive(Debug)]
pub(super) enum WatchError {
    /// The path vanished first; not worth a warning, still not covered.
    NotFound,
    /// The kernel's per-user watch table is full (inotify). FSEvents has no
    /// such table, so the macOS backend never says it.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    OutOfWatches,
    /// Anything else, for the log.
    Other(String),
}

#[cfg(target_os = "macos")]
pub(super) use mac::Backend;
#[cfg(all(test, target_os = "macos"))]
pub(super) use mac::STARTS;
#[cfg(not(target_os = "macos"))]
pub(super) use portable::Backend;

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::HashMap;

    use super::super::fsevents::{Queue, Stream, StreamError};
    use super::*;

    /// One stream per path, counted.
    ///
    /// Counted because two subjects can hold the same path: a main
    /// worktree's `.git` is also its repository's common directory. With one
    /// stream per path, the first subject to let go would otherwise stop the
    /// other's.
    struct Held {
        stream: Stream,
        recursive: bool,
        exclude: Vec<PathBuf>,
        holders: usize,
    }

    /// How long a replaced stream keeps running before it is stopped.
    ///
    /// The `handoff` replay does not recover an event FSEvents has registered
    /// but not yet put in its history, so a write made just before a
    /// replacement can be in neither the replay nor the new stream's live
    /// feed. Measured: 15 of 150 replacements lost one under load. The old
    /// stream still holds such a write in its latency window and delivers it
    /// when the window closes, so it outlives the swap by several windows.
    /// Both streams delivering the same path is absorbed by the routes' sets.
    const RETIRE_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

    /// Stop `old` after `RETIRE_AFTER`, off the caller's thread: dropping a
    /// stream blocks on its dispatch queue.
    fn retire(old: Stream, exclude: &[PathBuf]) {
        old.retire(exclude);
        let spawned = std::thread::Builder::new().name("fsevents-retire".into()).spawn(move || {
            std::thread::sleep(RETIRE_AFTER);
            drop(old);
        });
        if let Err(error) = spawned {
            // The closure, and the stream in it, are dropped here: stopped now,
            // as before this existed.
            tracing::debug!(?error, "could not retire a replaced watch later; stopped it now");
        }
    }

    /// Every stream this backend started, and the `since` it was given, so a
    /// test can check where a replacement's replay begins.
    #[cfg(test)]
    pub(in super::super) static STARTS: std::sync::Mutex<Vec<(PathBuf, Option<u64>)>> =
        std::sync::Mutex::new(Vec::new());

    pub(in super::super) struct Backend {
        queue: Arc<Queue>,
        sink: Sink,
        streams: HashMap<PathBuf, Held>,
    }

    impl Backend {
        pub(in super::super) fn new(sink: Sink) -> Result<Backend, String> {
            let queue = Queue::new().ok_or("could not create a dispatch queue")?;
            Ok(Backend { queue: Arc::new(queue), sink, streams: HashMap::new() })
        }

        /// Hold `path`, starting a stream for it if nothing holds it yet.
        pub(in super::super) fn watch(
            &mut self,
            path: &Path,
            recursive: bool,
            exclude: &[PathBuf],
        ) -> Result<(), WatchError> {
            // A second holder asking for different settings replaces the
            // stream for both, last caller winning. Today the only shared path
            // is a `.git` that is also a repository's common directory, and
            // both callers pass `git_dir_exclusions` of it — keep it that way.
            if let Some(held) = self.streams.get_mut(path) {
                if held.recursive == recursive && held.exclude == exclude {
                    held.holders += 1;
                    return Ok(());
                }
            }
            let since = self.streams.get(path).map(|h| h.stream.handoff());
            let stream = self.start(path, recursive, exclude, since)?;
            let holders = self.streams.get(path).map_or(0, |h| h.holders) + 1;
            // The old stream, if any, is retired only now that its
            // replacement is running, so nothing goes unwatched in between.
            let old = self.streams.insert(
                path.to_path_buf(),
                Held { stream, recursive, exclude: exclude.to_vec(), holders },
            );
            if let Some(old) = old {
                retire(old.stream, exclude);
            }
            Ok(())
        }

        /// Change what an already-held path leaves out, keeping its holders.
        ///
        /// A worktree's exclusions change when a re-walk finds new build
        /// output — the first `cargo build` in a fresh worktree makes
        /// `target/`. FSEvents fixes a stream's exclusions when it starts, so
        /// this starts a replacement from the old one's `handoff` and then
        /// retires the old one, which keeps running for `RETIRE_AFTER`.
        pub(in super::super) fn reconfigure(&mut self, path: &Path, exclude: &[PathBuf]) {
            let Some(held) = self.streams.get(path) else { return };
            if held.exclude == exclude {
                return;
            }
            let (recursive, since) = (held.recursive, held.stream.handoff());
            match self.start(path, recursive, exclude, Some(since)) {
                Ok(stream) => {
                    if let Some(held) = self.streams.get_mut(path) {
                        retire(std::mem::replace(&mut held.stream, stream), exclude);
                        held.exclude = exclude.to_vec();
                    }
                }
                // The old stream is still running and still delivers; the
                // routes drop what it should have left out, as they always did.
                Err(error) => tracing::debug!(
                    path = %path.display(),
                    ?error,
                    "could not restart a watch with new exclusions; keeping the old one"
                ),
            }
        }

        /// Let go of `path`; its stream stops when nothing holds it.
        pub(in super::super) fn unwatch(&mut self, path: &Path) {
            let Some(held) = self.streams.get_mut(path) else { return };
            held.holders = held.holders.saturating_sub(1);
            if held.holders == 0 {
                self.streams.remove(path);
            }
        }

        fn start(
            &self,
            path: &Path,
            recursive: bool,
            exclude: &[PathBuf],
            since: Option<u64>,
        ) -> Result<Stream, WatchError> {
            #[cfg(test)]
            STARTS.lock().unwrap_or_else(|e| e.into_inner()).push((path.to_path_buf(), since));
            Stream::start(&self.queue, path, exclude, recursive, Arc::clone(&self.sink), since).map_err(
                |error| match error {
                    StreamError::NotFound => WatchError::NotFound,
                    StreamError::Refused => {
                        WatchError::Other("FSEvents would not start a stream".to_string())
                    }
                },
            )
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod portable {
    use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

    use super::*;

    /// `notify`, unchanged: exclusions are accepted and ignored.
    pub(in super::super) struct Backend(RecommendedWatcher);

    impl Backend {
        pub(in super::super) fn new(sink: Sink) -> Result<Backend, String> {
            // No filtering by `EventKind`, for the reason `LogWatcher` gives:
            // the backends disagree about which kind an append surfaces as.
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    sink(&event.paths);
                }
            })
            .map(Backend)
            .map_err(|error| error.to_string())
        }

        pub(in super::super) fn watch(
            &mut self,
            path: &Path,
            recursive: bool,
            _exclude: &[PathBuf],
        ) -> Result<(), WatchError> {
            let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            self.0.watch(path, mode).map_err(|error| match error.kind {
                notify::ErrorKind::MaxFilesWatch => WatchError::OutOfWatches,
                notify::ErrorKind::PathNotFound => WatchError::NotFound,
                _ => WatchError::Other(error.to_string()),
            })
        }

        pub(in super::super) fn reconfigure(&mut self, _path: &Path, _exclude: &[PathBuf]) {}

        pub(in super::super) fn unwatch(&mut self, path: &Path) {
            // A directory that was removed took its watch with it, and the
            // backend has already forgotten it. Nothing to report.
            let _ = self.0.unwatch(path);
        }
    }
}
