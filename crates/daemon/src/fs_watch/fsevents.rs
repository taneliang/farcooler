//! One FSEvents stream per watched path, with the two settings `notify` keeps
//! to itself: paths the stream never delivers, and a latency above zero.
//!
//! `notify` 8's macOS backend folds every watched path into ONE stream, created
//! with zero latency, per-file events and `NoDefer`, and it rebuilds that
//! stream on every `watch` and `unwatch`. It has no way to name paths the
//! stream should leave out, so a worktree's `target/` was delivered to this
//! process file by file and dropped here. Measured on a scratch worktree with
//! a synthetic `target/` churn of about 13,000 files a second, a debug
//! `farcoolerd` spent 12–18% CPU on it, and 24–25% when the churn was in a
//! nested worktree, which both the checkout's watch and its own received.
//! With this binding it spends 0.1–0.3%.
//!
//! `FSEventStreamSetExclusionPaths` is what does it: fseventsd drops an
//! excluded path's descendants before they reach the client (the excluded
//! path's own creation and removal still arrive). It takes at most
//! `MAX_EXCLUSIONS` paths per stream, which is why there is one stream per
//! worktree rather than one for the fleet. What it does NOT do measurably is
//! lower fseventsd's own CPU: fseventsd reads every event on the volume
//! whoever is listening, and on a shared machine that cost moved by less than
//! its run-to-run noise.
//!
//! Every stream delivers on one serial dispatch queue, owned by `Queue`, so
//! callbacks never run concurrently with each other — the same guarantee the
//! single `notify` thread gave. Teardown runs ON that queue, which is what
//! makes "no callback is running or will run" true by the time `Drop` returns.

use std::ffi::{CStr, OsStr, c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::backend::Sink;
use super::exclusions::MAX_EXCLUSIONS;

/// How long fseventsd may hold events before delivering them, in seconds.
///
/// With `NoDefer`, the first event after a quiet spell is delivered at once and
/// anything that follows within this window comes as one batch — at most four
/// deliveries a second per stream. The passes that read what this collects
/// drain once a second (repositories) or once every three (worktrees), so a
/// quarter second is invisible to them, where zero meant one wakeup per file.
const LATENCY_SECONDS: f64 = 0.25;

type CFRef = *const c_void;
type FSEventStreamRef = *mut c_void;
type DispatchQueue = *mut c_void;

#[repr(C)]
struct FSEventStreamContext {
    version: isize,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> CFRef>,
}

type Callback = extern "C" fn(
    FSEventStreamRef,
    *mut c_void,
    usize,
    *mut c_void,
    *const u32,
    *const u64,
);

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_FS_EVENT_STREAM_EVENT_ID_SINCE_NOW: u64 = u64::MAX;
const K_FS_EVENT_STREAM_EVENT_FLAG_HISTORY_DONE: u32 = 0x0000_0010;

/// How far back, in event ids, a replacement stream may replay. A million ids
/// replays in about a tenth of a second and covers a quarter-second window at
/// any event rate below four million a second.
const REPLAY_FLOOR: u64 = 1_000_000;
const K_FS_EVENT_STREAM_CREATE_FLAG_NO_DEFER: u32 = 0x0000_0002;
const K_FS_EVENT_STREAM_CREATE_FLAG_FILE_EVENTS: u32 = 0x0000_0010;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeArrayCallBacks: c_void;
    fn CFStringCreateWithBytes(
        alloc: CFRef,
        bytes: *const u8,
        len: isize,
        encoding: u32,
        external: u8,
    ) -> CFRef;
    fn CFArrayCreate(alloc: CFRef, values: *const CFRef, count: isize, callbacks: *const c_void) -> CFRef;
    fn CFRelease(cf: CFRef);
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn FSEventStreamCreate(
        alloc: CFRef,
        callback: Callback,
        context: *const FSEventStreamContext,
        paths: CFRef,
        since_when: u64,
        latency: f64,
        flags: u32,
    ) -> FSEventStreamRef;
    fn FSEventStreamSetExclusionPaths(stream: FSEventStreamRef, paths: CFRef) -> u8;
    fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, queue: DispatchQueue);
    fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
    fn FSEventStreamFlushSync(stream: FSEventStreamRef);
    fn FSEventStreamGetLatestEventId(stream: FSEventStreamRef) -> u64;
    fn FSEventsGetCurrentEventId() -> u64;
    fn FSEventStreamStop(stream: FSEventStreamRef);
    fn FSEventStreamInvalidate(stream: FSEventStreamRef);
    fn FSEventStreamRelease(stream: FSEventStreamRef);
}

// libdispatch is part of libSystem, which every binary links.
unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> DispatchQueue;
    fn dispatch_sync_f(queue: DispatchQueue, context: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_release(object: DispatchQueue);
}

/// The serial queue every stream delivers on.
pub(super) struct Queue(DispatchQueue);

// SAFETY: a dispatch queue is a thread-safe object by design; the pointer is
// only ever handed to libdispatch and FSEvents, both of which accept it from
// any thread.
unsafe impl Send for Queue {}
unsafe impl Sync for Queue {}

impl Queue {
    pub(super) fn new() -> Option<Queue> {
        let label = c"com.farcooler.fs-watch";
        // SAFETY: a NUL-terminated label and a null attribute, which is a
        // serial queue.
        let queue = unsafe { dispatch_queue_create(label.as_ptr(), std::ptr::null()) };
        (!queue.is_null()).then_some(Queue(queue))
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        // SAFETY: created by `dispatch_queue_create` and released once. Every
        // `Stream` holds an `Arc<Queue>`, so none outlives it.
        unsafe { dispatch_release(self.0) }
    }
}

/// What the callback needs, owned by the stream for its whole life.
struct Context {
    sink: Sink,
    /// The registered path, in both its spellings: see `deliver`.
    roots: Vec<PathBuf>,
    recursive: bool,
    /// Set when a replacement takes over: what the replacement leaves out is
    /// dropped here too, while this stream finishes delivering. See `retire`.
    left_out: Arc<Mutex<Vec<PathBuf>>>,
}

/// One FSEvents stream. Flushed, stopped and released on drop.
///
/// The `Context` belongs to the stream: FSEvents frees it through the
/// context's `release` callback when the stream is released, which is the
/// documented way to tie `info` to the stream's real lifetime.
pub(super) struct Stream {
    stream: FSEventStreamRef,
    queue: Arc<Queue>,
    /// Whether `FSEventStreamStart` succeeded. A stream that never started is
    /// invalidated and released but not stopped, which Apple does not
    /// document as safe.
    started: bool,
    /// The volume's event id when this stream was made: where a replacement
    /// picks up if this one never delivered anything. See `handoff`.
    opened_at: u64,
    /// Shared with the callback's `Context`: see `retire`.
    left_out: Arc<Mutex<Vec<PathBuf>>>,
}

// SAFETY: the stream is only touched through FSEvents calls that Apple
// documents as callable from any thread, and teardown is serialized onto the
// stream's own queue.
unsafe impl Send for Stream {}

/// Why a stream could not be made.
#[derive(Debug)]
pub(super) enum StreamError {
    /// The path does not exist; FSEvents itself would watch it anyway, but no
    /// caller here wants a watch on nothing.
    NotFound,
    /// FSEvents refused to create or start the stream.
    Refused,
}

impl Stream {
    /// Start a stream on `path`, leaving out everything at or under `exclude`.
    ///
    /// `exclude` past `MAX_EXCLUSIONS` is cut, not refused: the remainder is
    /// still dropped by the routes after delivery, which is what happened to
    /// all of it before this existed. A non-recursive stream still receives
    /// the whole subtree from FSEvents, which has no such mode; `deliver`
    /// keeps only `path` and its direct children, as `notify` did.
    ///
    /// `since` is a `handoff` from the stream this one replaces, or `None` to
    /// start from now.
    pub(super) fn start(
        queue: &Arc<Queue>,
        path: &Path,
        exclude: &[PathBuf],
        recursive: bool,
        sink: Sink,
        since: Option<u64>,
    ) -> Result<Stream, StreamError> {
        if !path.exists() {
            return Err(StreamError::NotFound);
        }
        let mut roots = vec![path.to_path_buf()];
        if let Ok(real) = std::fs::canonicalize(path) {
            if real != path {
                roots.push(real);
            }
        }
        // SAFETY: no arguments; reads the current id of the event history.
        let opened_at = unsafe { FSEventsGetCurrentEventId() };
        let left_out: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let context =
            Box::into_raw(Box::new(Context { sink, roots, recursive, left_out: Arc::clone(&left_out) }));
        let stream_context = FSEventStreamContext {
            version: 0,
            info: context.cast(),
            retain: None,
            release: Some(release_context),
            copy_description: None,
        };

        let paths = cf_array(std::slice::from_ref(&path.to_path_buf()));
        let Some(paths) = paths else {
            // SAFETY: never handed to FSEvents.
            drop(unsafe { Box::from_raw(context) });
            return Err(StreamError::Refused);
        };
        // SAFETY: `paths` is a valid CFArray of CFStrings, the context is
        // owned by the stream from here (freed by `release_context`), and the
        // callback has the signature FSEvents calls.
        let stream = unsafe {
            FSEventStreamCreate(
                std::ptr::null(),
                callback,
                &stream_context,
                paths,
                since.unwrap_or(K_FS_EVENT_STREAM_EVENT_ID_SINCE_NOW),
                LATENCY_SECONDS,
                K_FS_EVENT_STREAM_CREATE_FLAG_FILE_EVENTS | K_FS_EVENT_STREAM_CREATE_FLAG_NO_DEFER,
            )
        };
        // SAFETY: created above; FSEventStreamCreate retained what it keeps.
        unsafe { CFRelease(paths) };
        if stream.is_null() {
            // Leaked, not freed: Apple does not say whether a failed create
            // has already called `release_context` on it, and a leak on a
            // path that effectively never runs beats a possible double free.
            let _ = context;
            return Err(StreamError::Refused);
        }

        let excluded: Vec<PathBuf> = exclude.iter().take(MAX_EXCLUSIONS).cloned().collect();
        if !excluded.is_empty() {
            if let Some(list) = cf_array(&excluded) {
                // SAFETY: a live, not-yet-started stream and a CFArray of
                // CFStrings, at most `MAX_EXCLUSIONS` long.
                let ok = unsafe { FSEventStreamSetExclusionPaths(stream, list) };
                unsafe { CFRelease(list) };
                if ok == 0 {
                    tracing::warn!(
                        path = %path.display(),
                        "FSEvents would not take this watch's exclusions; build output under it \
                         will be delivered and dropped here instead"
                    );
                }
            }
        }

        let mut built = Stream { stream, queue: Arc::clone(queue), started: false, opened_at, left_out };
        // SAFETY: a live stream, scheduled once, on a queue that outlives it.
        built.started = unsafe {
            FSEventStreamSetDispatchQueue(stream, queue.0);
            FSEventStreamStart(stream)
        } != 0;
        if !built.started {
            // `built`'s drop invalidates and releases it, without a stop.
            return Err(StreamError::Refused);
        }
        Ok(built)
    }
}

impl Stream {
    /// This stream is being replaced by one that leaves out `exclude`, and
    /// will run on briefly to deliver what its latency window holds. The
    /// replacement's exclusions apply to it from now on, so the build output
    /// they exist to keep out is not delivered twice in the meantime.
    pub(super) fn retire(&self, exclude: &[PathBuf]) {
        let mut spellings = Vec::with_capacity(exclude.len() * 2);
        for path in exclude {
            if let Ok(real) = std::fs::canonicalize(path) {
                if &real != path {
                    spellings.push(real);
                }
            }
            spellings.push(path.clone());
        }
        *self.left_out.lock().unwrap_or_else(|e| e.into_inner()) = spellings;
    }

    /// Where a replacement for this stream should start so nothing is lost.
    ///
    /// Stopping a stream discards what its latency window still holds, and
    /// `FSEventStreamFlushSync` measurably does not always deliver it first:
    /// it recovered a held write about half the time. So a replacement starts
    /// from the last event this stream delivered — or from when it was made,
    /// if it delivered none — and FSEvents replays the gap from its history.
    /// What both streams saw arrives twice, which the routes' sets absorb.
    ///
    /// The replay alone is not enough: it can miss a write that FSEvents has
    /// registered but not yet put in its history, which is why a replaced
    /// stream is retired later rather than stopped at once.
    pub(super) fn handoff(&self) -> u64 {
        // SAFETY: a live stream.
        let latest = unsafe { FSEventStreamGetLatestEventId(self.stream) };
        let from = if latest == K_FS_EVENT_STREAM_EVENT_ID_SINCE_NOW {
            self.opened_at
        } else {
            latest.max(self.opened_at)
        };
        // Never further back than `REPLAY_FLOOR` ids: live events wait behind
        // a replay, and a stream quiet for a day would replay a day. The gap
        // this covers is a quarter-second latency window.
        // SAFETY: no arguments.
        from.max(unsafe { FSEventsGetCurrentEventId() }.saturating_sub(REPLAY_FLOOR))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // Deliver what the latency window is still holding before stopping:
        // with `NoDefer`, events within a quarter second of a delivery wait in
        // the stream, `FSEventStreamStop` discards them, and a replacement
        // started at `SinceNow` never sees them. Called from this thread, not
        // from `teardown`: it blocks until the callbacks have run on the
        // queue. Dropped by `unwatch`, the thread holds only the backend's
        // lock; a retired stream is dropped on the `fsevents-retire` thread,
        // which holds nothing. The callback takes neither.
        if self.started {
            // SAFETY: a live, started stream.
            unsafe { FSEventStreamFlushSync(self.stream) };
        }
        // Run on the stream's own queue, so no callback is mid-flight when the
        // stream is invalidated and none runs after it; the context is freed
        // by `release_context` once FSEvents lets go, which may be a moment
        // later, but never before invalidation. Never called from
        // the callback itself — `dispatch_sync_f` onto the current queue would
        // deadlock — and nothing in the callback drops a stream.
        extern "C" fn teardown(raw: *mut c_void) {
            farcooler_ffi_guard::guarded((), || {
                // SAFETY: `raw` is the `Stream` being dropped, alive for the
                // whole synchronous call.
                let this = unsafe { &mut *raw.cast::<Stream>() };
                unsafe {
                    if this.started {
                        FSEventStreamStop(this.stream);
                    }
                    FSEventStreamInvalidate(this.stream);
                    // Frees the context through `release_context`.
                    FSEventStreamRelease(this.stream);
                }
            })
        }
        let queue = self.queue.0;
        // SAFETY: a synchronous call; `self` outlives it.
        unsafe { dispatch_sync_f(queue, (self as *mut Stream).cast(), teardown) };
    }
}

/// A CFArray of CFStrings, one per path, or `None` if any path is not UTF-8.
///
/// APFS and HFS+ store names as UTF-8, so a path that is not is a path that
/// does not exist on this platform's own filesystems.
fn cf_array(paths: &[PathBuf]) -> Option<CFRef> {
    let mut strings: Vec<CFRef> = Vec::with_capacity(paths.len());
    let mut ok = true;
    for path in paths {
        let bytes = path.as_os_str().as_bytes();
        if std::str::from_utf8(bytes).is_err() {
            ok = false;
            break;
        }
        // SAFETY: a byte slice and its length, copied by CoreFoundation.
        let s = unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                K_CF_STRING_ENCODING_UTF8,
                0,
            )
        };
        if s.is_null() {
            ok = false;
            break;
        }
        strings.push(s);
    }
    let array = if ok {
        // SAFETY: `strings` holds valid CFStrings; the array retains them.
        let a = unsafe {
            CFArrayCreate(
                std::ptr::null(),
                strings.as_ptr(),
                strings.len() as isize,
                &raw const kCFTypeArrayCallBacks,
            )
        };
        (!a.is_null()).then_some(a)
    } else {
        None
    };
    for s in strings {
        // SAFETY: each was created above and is released once; the array, if
        // made, holds its own retain.
        unsafe { CFRelease(s) };
    }
    array
}

/// FSEvents letting go of a stream's `Context`.
extern "C" fn release_context(info: *const c_void) {
    farcooler_ffi_guard::guarded((), || {
        // SAFETY: `info` is the `Box<Context>` handed to `FSEventStreamCreate`,
        // released exactly once, by the stream that owns it.
        drop(unsafe { Box::from_raw(info.cast_mut().cast::<Context>()) });
    })
}

extern "C" fn callback(
    _stream: FSEventStreamRef,
    info: *mut c_void,
    count: usize,
    paths: *mut c_void,
    flags: *const u32,
    _ids: *const u64,
) {
    // A panic must not unwind into FSEvents, which is C.
    farcooler_ffi_guard::guarded((), || {
        // SAFETY: `info` is the `Context` this stream was created with, freed
        // by `release_context` only after the stream is invalidated, which
        // happens on this same queue; without
        // `UseCFTypes`, `paths` is an array of `count` C strings and `flags`
        // an array of `count` flag words.
        let context = unsafe { &*info.cast::<Context>() };
        let raw = paths.cast::<*const c_char>();
        let mut batch = Vec::with_capacity(count);
        for i in 0..count {
            // The end of a replay (`handoff`) is a marker, not a change.
            if unsafe { *flags.add(i) } & K_FS_EVENT_STREAM_EVENT_FLAG_HISTORY_DONE != 0 {
                continue;
            }
            let bytes = unsafe { CStr::from_ptr(*raw.add(i)) }.to_bytes();
            #[cfg(test)]
            IDS.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((PathBuf::from(OsStr::from_bytes(bytes)), unsafe { *_ids.add(i) }));
            batch.push(PathBuf::from(OsStr::from_bytes(bytes)));
        }
        deliver(context, batch);
    })
}

/// Every path FSEvents handed any stream, before the non-recursive filter in
/// `deliver` — the only place a test can see what a git directory's
/// exclusions saved, since that filter would drop the same paths afterwards.
#[cfg(test)]
pub(super) static RAW: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Every path any stream was handed, with its event id: what a test
/// compares a replacement's `since` against.
#[cfg(test)]
pub(super) static IDS: std::sync::Mutex<Vec<(PathBuf, u64)>> = std::sync::Mutex::new(Vec::new());

/// Keep what this stream's registration asked for, and pass it on.
///
/// FSEvents reports under the real path — `/private/var/...` for a watch on
/// `/var/...` — so a non-recursive check compares against both spellings.
fn deliver(context: &Context, mut batch: Vec<PathBuf>) {
    #[cfg(test)]
    RAW.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&batch);
    if !context.recursive {
        batch.retain(|path| {
            context
                .roots
                .iter()
                .any(|root| path == root || path.parent() == Some(root.as_path()))
        });
    }
    let left_out = context.left_out.lock().unwrap_or_else(|e| e.into_inner());
    if !left_out.is_empty() {
        // Strictly under: an excluded directory's own creation still passes,
        // as it does from a stream that was started with the exclusion.
        batch.retain(|path| !left_out.iter().any(|dir| path.starts_with(dir) && path != dir));
    }
    drop(left_out);
    if !batch.is_empty() {
        (context.sink)(&batch);
    }
}
