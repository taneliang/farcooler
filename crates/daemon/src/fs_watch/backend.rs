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

/// What a test sees of a backend, and how it slows one down. See `offload_tests`.
#[cfg(test)]
#[derive(Default)]
pub(super) struct Seam {
    /// How long every new watch sleeps before it is made: a busy fseventsd,
    /// made to order.
    pub(super) stall: Option<std::time::Duration>,
    /// Every `watch`, `reconfigure` and `unwatch`, and the thread it ran on.
    pub(super) calls: Vec<(&'static str, std::thread::ThreadId)>,
    /// Every call panics: a backend bug, made to order.
    pub(super) panics: bool,
    /// While set, a replaced stream waits as if fseventsd had not yet got
    /// past the replacement's start: a busy fseventsd, made to order for
    /// `retire`.
    #[cfg(target_os = "macos")]
    pub(super) behind: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl Seam {
    fn touched(&mut self, call: &'static str) {
        self.calls.push((call, std::thread::current().id()));
        assert!(!self.panics, "the test seam panicked in {call}");
    }

    fn started(&self) {
        if let Some(stall) = self.stall {
            std::thread::sleep(stall);
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) use mac::Backend;
#[cfg(all(test, target_os = "macos"))]
pub(super) use mac::{BEACONED, RETIRED, STARTS};
#[cfg(not(target_os = "macos"))]
pub(super) use portable::Backend;

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use std::sync::atomic::AtomicBool;

    use super::super::fsevents::{Queue, Stream, StreamError, current_event_id, volume};
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

    /// How a replaced stream is stopped without losing what only it holds.
    ///
    /// A replacement misses every event fseventsd registers while it is
    /// starting: the `handoff` replay reads fseventsd's history, which lags
    /// behind it, and the live feed begins only once `FSEventStreamStart`
    /// returns. Measured under load (ov-671): twice in 49 replacements a
    /// write whose id fell between `FSEventsGetCurrentEventId` before and
    /// after the start reached the old stream, 1.6 s and 3.6 s after it was
    /// made, and never the replacement; every later id reached both. And
    /// fseventsd hands a write to its streams as late as it gets to it, 2 to
    /// 12 s late in the same runs, so a fixed 2 s retirement could stop the
    /// old stream first, and the write was lost from both.
    ///
    /// So the old stream runs on until fseventsd has handed some stream on
    /// this queue, on the same volume, an event registered after the
    /// replacement started (`Queue::reached`), then for `WINDOW` more while
    /// its latency window delivers, and only then stops. On a quiet machine
    /// another worktree's event, or the beacon's, gets there within a second.
    /// A tree on another volume than the beacon may wait out `RETIRE_CAP`.
    struct Retiring {
        queue: Arc<Queue>,
        /// The id the replacement's live feed starts after.
        started: u64,
        /// The replaced path's volume; none, and only the cap ends the wait.
        volume: Option<u64>,
        /// The beacon, when it is on that volume.
        beacon: Option<PathBuf>,
        #[cfg(test)]
        behind: Arc<AtomicBool>,
    }

    /// After fseventsd has got past the replacement's start, how long the old
    /// stream keeps running: its quarter-second latency window, twice over.
    const WINDOW: Duration = Duration::from_millis(500);

    /// How often the beacon is written while fseventsd has not got there: a
    /// busy fseventsd loses whole writes, so one is not enough.
    const BEACON_EVERY: Duration = Duration::from_secs(2);

    /// The longest a replaced stream runs. Past this fseventsd is losing
    /// writes rather than running late — ov-666 measured spells in which most
    /// writes reached no stream at all — and the old stream would be kept
    /// for nothing while it is handed the build output its replacement
    /// leaves out.
    const RETIRE_CAP: Duration = Duration::from_secs(60);

    impl Retiring {
        fn passed(&self) -> bool {
            #[cfg(test)]
            if self.behind.load(std::sync::atomic::Ordering::Acquire) {
                return false;
            }
            self.volume.is_some_and(|v| self.queue.reached(v) >= self.started)
        }

        /// Block until the old stream can stop without losing anything, or
        /// until `RETIRE_CAP`: whether fseventsd got there in time.
        fn wait(&self) -> bool {
            let begun = Instant::now();
            let mut beaconed: Option<Instant> = None;
            let mut writes = 0u64;
            while !self.passed() {
                if begun.elapsed() > RETIRE_CAP {
                    tracing::debug!(
                        started = self.started,
                        reached = self.volume.map(|v| self.queue.reached(v)),
                        "fseventsd did not get past a replaced watch's handover; stopped it anyway"
                    );
                    return false;
                }
                if let Some(beacon) = &self.beacon {
                    if beaconed.is_none_or(|at| at.elapsed() >= BEACON_EVERY) {
                        // New content every time: rewriting an empty file
                        // as empty may register nothing at all. Gone with
                        // its backend: nothing left to wait for, but the cap
                        // still ends this.
                        writes += 1;
                        let _ = std::fs::write(beacon, format!("{}.{writes}", std::process::id()));
                        #[cfg(test)]
                        BEACONED.lock().unwrap_or_else(|e| e.into_inner()).push(beacon.clone());
                        beaconed = Some(Instant::now());
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            std::thread::sleep(WINDOW);
            true
        }
    }

    /// Stop `old` once nothing is held only by it (`Retiring`), off the
    /// caller's thread: dropping a stream blocks on its dispatch queue.
    ///
    /// While it runs on, its `left_out` is the exclusions of the replacement
    /// at the time, and stale if that changes again: harmless, the routes
    /// drop what it should have left out.
    fn retire(old: Stream, exclude: &[PathBuf], until: Retiring, path: &Path) {
        old.retire(exclude);
        #[cfg(test)]
        let path = path.to_path_buf();
        #[cfg(not(test))]
        let _ = path;
        let spawned = std::thread::Builder::new().name("fsevents-retire".into()).spawn(move || {
            let _in_time = until.wait();
            drop(old);
            #[cfg(test)]
            RETIRED.lock().unwrap_or_else(|e| e.into_inner()).push((path, _in_time));
        });
        if let Err(error) = spawned {
            // The closure, and the stream in it, are dropped here: stopped now,
            // as before this existed.
            tracing::debug!(?error, "could not retire a replaced watch later; stopped it now");
        }
    }

    /// Every path whose replaced stream has stopped, so a test can see when,
    /// and whether fseventsd had got past its replacement's start or the cap
    /// stopped it.
    #[cfg(test)]
    pub(in super::super) static RETIRED: std::sync::Mutex<Vec<(PathBuf, bool)>> =
        std::sync::Mutex::new(Vec::new());

    /// Every beacon write, so a test can see one was made.
    #[cfg(test)]
    pub(in super::super) static BEACONED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

    /// A directory of this backend's own, watched on its queue, that a
    /// retirement writes to when the watched trees are quiet: an event
    /// fseventsd must get to, so `Queue::reached` moves.
    struct Beacon {
        file: PathBuf,
        dir: PathBuf,
        volume: Option<u64>,
        /// Dropped before the directory is removed.
        stream: Option<Stream>,
    }

    impl Beacon {
        fn new(queue: &Arc<Queue>) -> Option<Beacon> {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("farcooler-fsevents-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).ok()?;
            let ignore: Sink = Arc::new(|_: &[PathBuf]| {});
            match Stream::start(queue, &dir, &[], true, ignore, None) {
                Ok(stream) => Some(Beacon {
                    file: dir.join("beacon"),
                    volume: volume(&dir),
                    dir,
                    stream: Some(stream),
                }),
                Err(error) => {
                    tracing::debug!(?error, "could not watch a beacon; a replaced watch waits out its cap");
                    let _ = std::fs::remove_dir_all(&dir);
                    None
                }
            }
        }
    }

    impl Drop for Beacon {
        fn drop(&mut self) {
            drop(self.stream.take());
            let _ = std::fs::remove_dir_all(&self.dir);
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
        /// Cleared when this backend is dropped, so a replaced stream still
        /// retiring delivers nothing more.
        live: Arc<AtomicBool>,
        streams: HashMap<PathBuf, Held>,
        /// Made by the first replacement: most backends never replace a
        /// stream, and need not start one more.
        beacon: Option<Beacon>,
        #[cfg(test)]
        pub(in super::super) seam: Seam,
    }

    impl Backend {
        pub(in super::super) fn new(sink: Sink) -> Result<Backend, String> {
            let queue = Arc::new(Queue::new().ok_or("could not create a dispatch queue")?);
            let live = Arc::new(AtomicBool::new(true));
            let gate = Arc::clone(&live);
            let sink: Sink = Arc::new(move |paths: &[PathBuf]| {
                if gate.load(std::sync::atomic::Ordering::Acquire) {
                    sink(paths);
                }
            });
            Ok(Backend {
                beacon: None,
                queue,
                sink,
                live,
                streams: HashMap::new(),
                #[cfg(test)]
                seam: Seam::default(),
            })
        }

        /// Hold `path`, starting a stream for it if nothing holds it yet.
        pub(in super::super) fn watch(
            &mut self,
            path: &Path,
            recursive: bool,
            exclude: &[PathBuf],
        ) -> Result<(), WatchError> {
            #[cfg(test)]
            self.seam.touched("watch");
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
            // Only a replacement needs one: the id costs a call to fseventsd.
            let until = self.streams.contains_key(path).then(|| self.retiring(path));
            let holders = self.streams.get(path).map_or(0, |h| h.holders) + 1;
            // The old stream, if any, is retired only now that its
            // replacement is running, so nothing goes unwatched in between.
            let old = self.streams.insert(
                path.to_path_buf(),
                Held { stream, recursive, exclude: exclude.to_vec(), holders },
            );
            if let (Some(old), Some(until)) = (old, until) {
                retire(old.stream, exclude, until, path);
            }
            Ok(())
        }

        /// Change what an already-held path leaves out, keeping its holders.
        ///
        /// A worktree's exclusions change when a re-walk finds new build
        /// output — the first `cargo build` in a fresh worktree makes
        /// `target/`. FSEvents fixes a stream's exclusions when it starts, so
        /// this starts a replacement from the old one's `handoff` and then
        /// retires the old one, which keeps running until fseventsd has got
        /// past the replacement's start (`Retiring`).
        pub(in super::super) fn reconfigure(&mut self, path: &Path, exclude: &[PathBuf]) {
            #[cfg(test)]
            self.seam.touched("reconfigure");
            let Some(held) = self.streams.get(path) else { return };
            if held.exclude == exclude {
                return;
            }
            let (recursive, since) = (held.recursive, held.stream.handoff());
            match self.start(path, recursive, exclude, Some(since)) {
                Ok(stream) => {
                    let until = self.retiring(path);
                    if let Some(held) = self.streams.get_mut(path) {
                        retire(std::mem::replace(&mut held.stream, stream), exclude, until, path);
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
            #[cfg(test)]
            self.seam.touched("unwatch");
            let Some(held) = self.streams.get_mut(path) else { return };
            held.holders = held.holders.saturating_sub(1);
            if held.holders == 0 {
                self.streams.remove(path);
            }
        }

        /// What a stream replaced just now waits for: see `Retiring`. Taken
        /// after the replacement has started.
        fn retiring(&mut self, path: &Path) -> Retiring {
            if self.beacon.is_none() {
                self.beacon = Beacon::new(&self.queue);
            }
            let volume = volume(path);
            Retiring {
                queue: Arc::clone(&self.queue),
                started: current_event_id(),
                volume,
                beacon: self.beacon.as_ref().filter(|b| volume.is_some() && b.volume == volume).map(|b| b.file.clone()),
                #[cfg(test)]
                behind: Arc::clone(&self.seam.behind),
            }
        }

        /// The beacon's directory, once a replacement has made it.
        #[cfg(test)]
        pub(in super::super) fn beacon_dir(&self) -> Option<PathBuf> {
            self.beacon.as_ref().map(|b| b.dir.clone())
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
            #[cfg(test)]
            self.seam.started();
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

    impl Drop for Backend {
        fn drop(&mut self) {
            self.live.store(false, std::sync::atomic::Ordering::Release);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod portable {
    use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

    use super::*;

    /// `notify`, unchanged: exclusions are accepted and ignored.
    pub(in super::super) struct Backend {
        inner: RecommendedWatcher,
        #[cfg(test)]
        pub(in super::super) seam: Seam,
    }

    impl Backend {
        pub(in super::super) fn new(sink: Sink) -> Result<Backend, String> {
            // No filtering by `EventKind`, for the reason `LogWatcher` gives:
            // the backends disagree about which kind an append surfaces as.
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    sink(&event.paths);
                }
            })
            .map(|inner| Backend {
                inner,
                #[cfg(test)]
                seam: Seam::default(),
            })
            .map_err(|error| error.to_string())
        }

        pub(in super::super) fn watch(
            &mut self,
            path: &Path,
            recursive: bool,
            _exclude: &[PathBuf],
        ) -> Result<(), WatchError> {
            #[cfg(test)]
            self.seam.touched("watch");
            #[cfg(test)]
            self.seam.started();
            let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            self.inner.watch(path, mode).map_err(|error| match error.kind {
                notify::ErrorKind::MaxFilesWatch => WatchError::OutOfWatches,
                notify::ErrorKind::PathNotFound => WatchError::NotFound,
                _ => WatchError::Other(error.to_string()),
            })
        }

        pub(in super::super) fn reconfigure(&mut self, _path: &Path, _exclude: &[PathBuf]) {
            #[cfg(test)]
            self.seam.touched("reconfigure");
        }

        pub(in super::super) fn unwatch(&mut self, path: &Path) {
            #[cfg(test)]
            self.seam.touched("unwatch");
            // A directory that was removed took its watch with it, and the
            // backend has already forgotten it. Nothing to report.
            let _ = self.inner.unwatch(path);
        }
    }
}
