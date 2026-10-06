//! The daemon's session projectors: one per claude pane that has one open
//! (ov-363), read a page at a time and followed by revision (ov-366).
//!
//! A projector is opened for a terminal and rebuilt from the transcript on
//! disk (design decision D8: rebuild, no checkpoint), so after a daemon
//! restart the rows come back from the files and nothing else. From then on
//! it is fed from four places:
//!
//! - a filesystem watch on its files (FSEvents on macOS, inotify on Linux),
//!   which reads a record within milliseconds of claude writing it;
//! - every claude hook routed to the terminal (`HookIngress::accept`), as
//!   provisional rows;
//! - the watcher's tick for the pane (`watch::registry_join`): claude's
//!   registry status, and a full read now and then for a change a watch
//!   missed;
//! - `SessionStart` for another session, which moves it to that transcript.
//!
//! **Rebuild first, then hooks.** A hook that arrives while a terminal's
//! first projector is being read is held, and applied once the read is
//! whole, in the order the hooks came. Applied to a half-read projection it
//! would be a row with an `ord` ahead of turns older than it.
//!
//! **Behind `FARCOOLER_PROJECTOR=1` until a client reads it** (ov-372). The
//! flag gates all of it: with it unset no projector opens for a pane, and
//! `agent.rows` and `agent.rows_follow` are refused as unsupported, so the
//! daemon does what it did before. watch.rs's own turn, question and subagent
//! state still come from `claude::parse_line` either way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use farcooler_core::session_log::projector::{Activity, Change, HookEffect, Row, SessionProjector};
use serde_json::Value;
use uuid::Uuid;

/// How often a watched projector still reads every file, for a change its
/// watch missed. An unwatched one reads them every tick.
const FULL_READ_EVERY: Duration = Duration::from_secs(30);

/// The most changes a follow sends. A follower further behind pages again.
pub const MAX_CHANGES: usize = 500;

/// A projector, and what its followers wait on.
pub struct Open {
    session: Mutex<SessionProjector>,
    /// Which projection this is. A follower holding another's revisions
    /// pages again.
    pub epoch: u64,
    /// The projection's revision, sent after every change.
    revision: tokio::sync::watch::Sender<u64>,
    watch: Mutex<Option<Watch>>,
    last_full: Mutex<Instant>,
}

/// A watch on one session's files.
struct Watch {
    _watcher: notify::RecommendedWatcher,
    dirs: (PathBuf, PathBuf),
    /// The subagents directory may not exist yet; it is watched once it does.
    subagents: bool,
}

impl Open {
    fn new(session: SessionProjector) -> Open {
        let (revision, _) = tokio::sync::watch::channel(session.projection().revision());
        Open { session: Mutex::new(session), epoch: next_epoch(), revision, watch: Mutex::new(None), last_full: Mutex::new(Instant::now()) }
    }

    pub fn lock(&self) -> MutexGuard<'_, SessionProjector> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn publish(&self, session: &SessionProjector) {
        self.revision.send_if_modified(|rev| {
            let now = session.projection().revision();
            let moved = *rev != now;
            *rev = now;
            moved
        });
    }

    /// Watch the session's files, or move the watch to where they are now.
    /// Never with the projector locked: the watch's callback takes that lock,
    /// and replacing a watcher waits for its callback thread.
    fn ensure_watch(self: &Arc<Self>, dirs: (PathBuf, PathBuf)) -> bool {
        let mut watch = self.watch.lock().unwrap_or_else(|e| e.into_inner());
        let subagents = dirs.1.is_dir();
        if watch.as_ref().is_some_and(|w| w.dirs == dirs && w.subagents == subagents) {
            return true;
        }
        let weak = Arc::downgrade(self);
        let made = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let (Ok(event), Some(open)) = (event, weak.upgrade()) else { return };
            let mut session = open.lock();
            session.poll_paths(&event.paths);
            open.publish(&session);
        });
        let Ok(mut watcher) = made else {
            *watch = None;
            return false;
        };
        use notify::Watcher as _;
        if watcher.watch(&dirs.0, notify::RecursiveMode::NonRecursive).is_err() {
            *watch = None;
            return false;
        }
        let subagents = subagents && watcher.watch(&dirs.1, notify::RecursiveMode::NonRecursive).is_ok();
        *watch = Some(Watch { _watcher: watcher, dirs, subagents });
        true
    }
}

/// A projection's epoch: the daemon's start in milliseconds, times a
/// thousand, plus a count, so no two projectors share one, before a restart
/// or after it.
fn next_epoch() -> u64 {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| AtomicU64::new((now_ms().max(1) as u64).saturating_mul(1000))).fetch_add(1, Ordering::Relaxed)
}

/// A hook that arrived while its terminal's projector was being built.
struct Held {
    event: String,
    payload: Value,
    at: i64,
}

/// Every open projector, by terminal.
///
/// One lock per projector, and the map's own lock only long enough to find
/// it: a rebuild reads a whole transcript, and every claude hook on the runner
/// (a held PermissionRequest among them) passes through `hook` here.
#[derive(Default)]
pub struct SessionProjectors {
    open: Mutex<HashMap<Uuid, Arc<Open>>>,
    /// Terminals whose first projector is being built outside the lock, with
    /// the hooks that arrived meanwhile. A `forget` meanwhile takes the
    /// terminal out of here, and the finished build is then dropped rather
    /// than kept for a terminal that is gone.
    building: Mutex<HashMap<Uuid, Vec<Held>>>,
}

/// Whether claude panes get a projector, and `agent.rows` is served.
pub fn shadowing() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FARCOOLER_PROJECTOR").is_some_and(|v| v == "1"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// The path as the filesystem names it, so a watch's events (FSEvents
/// reports `/private/var/…` for `/var/…`) compare equal to it. A file not
/// written yet is named through its directory.
pub fn canonical(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    match (path.parent().and_then(|p| std::fs::canonicalize(p).ok()), path.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => path.to_path_buf(),
    }
}

/// One hook, on a projector whose file is read up to now first, so a hook
/// that arrives after its own record is checked against it rather than put
/// up as news.
fn apply(session: &mut SessionProjector, event: &str, payload: &Value, at: i64) {
    let main = session.transcript().to_path_buf();
    session.poll_paths(&[main]);
    if let HookEffect::Rebind { transcript_path: Some(path), .. } = session.projection_mut().hook(event, payload, at) {
        session.rebind(canonical(&path));
        session.poll();
    }
}

/// A page of rows, as `agent.rows` answers it.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub epoch: u64,
    pub rev: u64,
    pub rows: Vec<Row>,
    pub more_before: bool,
}

/// One change, owned, for the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum RowChange {
    Insert(Row),
    Update(Row),
    Remove { id: String, rev: u64 },
}

/// What a follow answers.
#[derive(Debug, Clone, PartialEq)]
pub enum Follow {
    /// Changes after the follower's revision (none, when the wait ran out).
    Changes { epoch: u64, rev: u64, changes: Vec<RowChange> },
    /// The follower's rows are another projection's, or too far behind.
    Reset { epoch: u64, rev: u64 },
}

impl SessionProjectors {
    fn get(&self, terminal: Uuid) -> Option<Arc<Open>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).get(&terminal).cloned()
    }

    /// Open a projector for `terminal` on `transcript`, read what is on disk
    /// so far, and keep it. A terminal already open on that transcript is
    /// left as it is; on another, it is moved there with its rows kept.
    ///
    /// A new one is built and read with no lock held, and put in the map only
    /// once it is whole; hooks that arrive meanwhile are applied after.
    pub fn open(&self, terminal: Uuid, transcript: PathBuf) {
        let transcript = canonical(&transcript);
        if let Some(open) = self.get(terminal) {
            let dirs = {
                let mut session = open.lock();
                session.rebind(transcript);
                session.poll();
                open.publish(&session);
                session.watched_dirs()
            };
            open.ensure_watch(dirs);
            return;
        }
        self.building.lock().unwrap_or_else(|e| e.into_inner()).entry(terminal).or_default();
        let mut session = SessionProjector::open(transcript);
        session.poll();
        self.finish(terminal, session);
    }

    /// Keep a projector built for `terminal`, unless `forget` came first, and
    /// apply the hooks held while it was built.
    fn finish(&self, terminal: Uuid, session: SessionProjector) {
        let dirs = session.watched_dirs();
        let mut building = self.building.lock().unwrap_or_else(|e| e.into_inner());
        let Some(held) = building.remove(&terminal) else { return };
        let built = Arc::new(Open::new(session));
        let open = self.open.lock().unwrap_or_else(|e| e.into_inner()).entry(terminal).or_insert(built).clone();
        // Locked before the build is no longer marked as one, so a hook that
        // misses the mark waits here for the held ones to go first.
        let mut session = open.lock();
        drop(building);
        for hook in held {
            apply(&mut session, &hook.event, &hook.payload, hook.at);
        }
        open.publish(&session);
        drop(session);
        open.ensure_watch(dirs);
    }

    pub fn is_open(&self, terminal: Uuid) -> bool {
        self.get(terminal).is_some()
    }

    /// The transcript `terminal`'s projector reads, if one is open.
    pub fn transcript(&self, terminal: Uuid) -> Option<PathBuf> {
        let open = self.get(terminal)?;
        let path = open.lock().transcript().to_path_buf();
        Some(path)
    }

    /// A claude hook routed to `terminal`. Held while its projector is being
    /// built; nothing when none is open.
    pub fn hook(&self, terminal: Uuid, event: &str, payload: &Value) {
        {
            let mut building = self.building.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(held) = building.get_mut(&terminal) {
                held.push(Held { event: event.to_string(), payload: payload.clone(), at: now_ms() });
                return;
            }
        }
        let Some(open) = self.get(terminal) else { return };
        let dirs = {
            let mut session = open.lock();
            let before = session.transcript().to_path_buf();
            apply(&mut session, event, payload, now_ms());
            open.publish(&session);
            (session.transcript() != before).then(|| session.watched_dirs())
        };
        if let Some(dirs) = dirs {
            open.ensure_watch(dirs);
        }
    }

    /// The watcher's tick: claude's registry status, a read of the main
    /// file, a read of every file when there is no watch or the last was a
    /// while ago, and a watch on a subagents directory that has appeared
    /// since. The main file is one read whatever the session's size, so it is
    /// read every tick, watch or no.
    pub fn tick(&self, terminal: Uuid, activity: Option<Activity>) {
        let Some(open) = self.get(terminal) else { return };
        let dirs = open.lock().watched_dirs();
        let watched = open.ensure_watch(dirs);
        let mut last_full = open.last_full.lock().unwrap_or_else(|e| e.into_inner());
        let mut session = open.lock();
        if !watched || last_full.elapsed() >= FULL_READ_EVERY {
            session.poll();
            *last_full = Instant::now();
        } else {
            let main = session.transcript().to_path_buf();
            session.poll_paths(&[main]);
        }
        if let Some(activity) = activity {
            session.projection_mut().set_activity(activity);
        }
        open.publish(&session);
    }

    /// A page of `terminal`'s rows, oldest first: up to `limit` before `ord`.
    pub fn page(&self, terminal: Uuid, before: Option<u64>, limit: usize) -> Option<Vec<Row>> {
        self.read_page(terminal, before, limit).map(|p| p.rows)
    }

    /// A page with what a client needs to follow on from it.
    pub fn read_page(&self, terminal: Uuid, before: Option<u64>, limit: usize) -> Option<Page> {
        let open = self.get(terminal)?;
        let session = open.lock();
        let p = session.projection();
        let rows: Vec<Row> = p.page(before, limit).into_iter().cloned().collect();
        let more_before = rows.first().is_some_and(|r| p.any_before(r.ord));
        Some(Page { epoch: open.epoch, rev: p.revision(), rows, more_before })
    }

    /// `terminal`'s rows changed after revision `rev`, and the revision now.
    pub fn changed_since(&self, terminal: Uuid, rev: u64) -> Option<(Vec<Row>, u64)> {
        let open = self.get(terminal)?;
        let session = open.lock();
        let p = session.projection();
        Some((p.changed_since(rev).into_iter().cloned().collect(), p.revision()))
    }

    /// What changed for a follower at `after` in projection `epoch`, waiting
    /// up to `wait` for something to. `None` when no projector is open.
    pub async fn follow(&self, terminal: Uuid, epoch: u64, after: u64, wait: Duration) -> Option<Follow> {
        let open = self.get(terminal)?;
        let mut changed = open.revision.subscribe();
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let answer = {
                let session = open.lock();
                let p = session.projection();
                let rev = p.revision();
                if epoch != open.epoch || after > rev {
                    return Some(Follow::Reset { epoch: open.epoch, rev });
                }
                match p.changes_since(after, MAX_CHANGES) {
                    None => return Some(Follow::Reset { epoch: open.epoch, rev }),
                    Some(changes) if !changes.is_empty() => Some(Follow::Changes { epoch: open.epoch, rev, changes: owned(changes) }),
                    Some(_) => None,
                }
            };
            if let Some(answer) = answer {
                return Some(answer);
            }
            if tokio::time::timeout_at(deadline, changed.changed()).await.is_err() {
                // Nothing in the wait: no changes, and the follower stays at
                // its revision (one that lands just now, the next follow sees).
                return Some(Follow::Changes { epoch: open.epoch, rev: after, changes: Vec::new() });
            }
        }
    }

    /// The terminal is gone.
    pub fn forget(&self, terminal: Uuid) {
        let mut building = self.building.lock().unwrap_or_else(|e| e.into_inner());
        building.remove(&terminal);
        self.open.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
    }
}

fn owned(changes: Vec<Change<'_>>) -> Vec<RowChange> {
    changes
        .into_iter()
        .map(|c| match c {
            Change::Insert(row) => RowChange::Insert(row.clone()),
            Change::Update(row) => RowChange::Update(row.clone()),
            Change::Remove { id, rev } => RowChange::Remove { id: id.to_string(), rev },
        })
        .collect()
}

/// The daemon's projectors.
pub fn global() -> &'static SessionProjectors {
    static PROJECTORS: OnceLock<SessionProjectors> = OnceLock::new();
    PROJECTORS.get_or_init(SessionProjectors::default)
}

#[cfg(test)]
#[path = "session_projectors_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "session_projectors_follow_tests.rs"]
mod follow_tests;
