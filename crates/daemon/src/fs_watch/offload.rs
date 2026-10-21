//! Every call into the platform backend, kept off the async workers (ov-670).
//!
//! On macOS a `watch` starts an FSEvents stream, which is two round trips to
//! fseventsd (`FSEventsGetCurrentEventId`, `FSEventStreamStart`), and an
//! `unwatch` stops one, which flushes it and waits on its delivery queue
//! (`FSEventStreamFlushSync`, then `dispatch_sync_f`). Under a busy fseventsd
//! a start took 3.5 to 11 seconds. `sync_worktrees` is async, because the
//! registration walk awaits `git ls-files`, and it used to make those calls
//! inline: the change-set pass that runs it is a detached task, but a detached
//! task is still on a worker, so the connections that worker was serving
//! waited with it.
//!
//! So the async half hands each backend call to `spawn_blocking`, whole: the
//! closure takes the backend's lock and makes the call there. What a call
//! does inside the backend is unchanged — a replacement stream still reads
//! its `handoff` from the old one, starts, and only then retires it, all
//! under one lock — and the calls of one registration still run in order,
//! because each is awaited before the next is made. The backend's lock is
//! now only ever waited on from a blocking thread, which matters as much as
//! the call itself: `sync_repositories` already ran on one, and a worker
//! queued behind it on the lock was stalled just the same.
//!
//! Not `block_in_place`: it panics on the current-thread runtime that
//! `#[tokio::test]` uses.
//! Not a dedicated thread that owns the backend: the lock already serializes
//! every caller, and a command channel would add a second ordering to keep
//! honest for no gain.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use super::backend::{Backend, WatchError};
use super::{Registration, Subject, TreeWatcher};

impl TreeWatcher {
    /// Run `op` against the backend on a blocking thread, or `None` when the
    /// backend never built (or the runtime is shutting down).
    async fn on_backend<R: Send + 'static>(
        &self,
        op: impl FnOnce(&mut Backend) -> R + Send + 'static,
    ) -> Option<R> {
        let backend = Arc::clone(&self.watcher);
        let ran = tokio::task::spawn_blocking(move || {
            backend.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(op)
        })
        .await;
        match ran {
            Ok(result) => result,
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(_) => None,
        }
    }

    /// `watch_all`, from an async caller.
    pub(super) async fn watch_all_offloaded(
        &self,
        paths: Vec<PathBuf>,
        recursive: bool,
        exclude: Vec<PathBuf>,
        what: &'static str,
        held: Arc<HashSet<PathBuf>>,
    ) -> (Vec<PathBuf>, bool) {
        self.on_backend(move |backend| watch_all(backend, &paths, recursive, &exclude, what, &held))
            .await
            .unwrap_or_default()
    }

    /// `Backend::reconfigure`, from an async caller.
    pub(super) async fn reconfigure_offloaded(&self, path: PathBuf, exclude: Vec<PathBuf>) {
        self.on_backend(move |backend| backend.reconfigure(&path, &exclude)).await;
    }

    /// `retire`, from an async caller. Spends no thread when nothing is stale,
    /// which is every re-walk that found the same directories.
    pub(super) async fn retire_offloaded(&self, previous: Option<Registration>, kept: &[PathBuf]) {
        let stale = stale(previous, kept);
        if stale.is_empty() {
            return;
        }
        self.on_backend(move |backend| unwatch_all(backend, &stale)).await;
    }

    /// `forget`, from an async caller.
    pub(super) async fn forget_offloaded(&self, subject: Subject) {
        let previous = self.take(subject);
        self.retire_offloaded(previous, &[]).await;
    }
}

/// The watches a previous registration held and the new one does not.
pub(super) fn stale(previous: Option<Registration>, kept: &[PathBuf]) -> Vec<PathBuf> {
    let Some(previous) = previous else { return Vec::new() };
    let kept: HashSet<&PathBuf> = kept.iter().collect();
    previous.watched.into_iter().filter(|p| !kept.contains(p)).collect()
}

pub(super) fn unwatch_all(backend: &mut Backend, paths: &[PathBuf]) {
    for path in paths {
        backend.unwatch(path);
    }
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
pub(super) fn watch_all(
    backend: &mut Backend,
    paths: &[PathBuf],
    recursive: bool,
    exclude: &[PathBuf],
    what: &str,
    held: &HashSet<PathBuf>,
) -> (Vec<PathBuf>, bool) {
    let mut all = true;
    let mut taken = Vec::with_capacity(paths.len());
    for path in paths {
        if held.contains(path) {
            taken.push(path.clone());
            continue;
        }
        let Err(error) = backend.watch(path, recursive, exclude) else {
            taken.push(path.clone());
            continue;
        };
        all = false;
        match error {
            // The ceiling this module is shaped around, reached anyway —
            // something else on the runner is holding the table down, or
            // `max_user_watches` was lowered under us. Named explicitly
            // because "No space left on device" from a watcher is the
            // least helpful true sentence the kernel produces.
            WatchError::OutOfWatches => tracing::warn!(
                path = %path.display(),
                "the kernel is out of file watches (see \
                 /proc/sys/fs/inotify/max_user_watches); not watching this {what}"
            ),
            // Registration races a worktree being removed, and a path that
            // is already gone is not a failure worth a warning — but it
            // still means this subject is not covered.
            WatchError::NotFound => tracing::debug!(
                path = %path.display(),
                "{what} vanished before it could be watched"
            ),
            WatchError::Other(error) => {
                tracing::warn!(path = %path.display(), %error, "could not watch a {what}")
            }
        }
    }
    (taken, all)
}
