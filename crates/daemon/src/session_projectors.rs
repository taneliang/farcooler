//! The daemon's session projectors: one per claude or codex pane that has
//! one open (ov-363, ov-378), read a page at a time and followed by revision
//! (ov-366). A codex pane's is its rollout, joined by the file its process
//! holds open, and its activity comes from the rollout itself.
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
//! **One watch for every projector** (`session_watch.rs`): a single notify
//! watcher, its directories routed to the terminals that read them, and its
//! events read on a thread of their own, so the watcher's callback never
//! waits on a projector.
//!
//! **Rebuild first, then hooks.** A hook that arrives while a terminal's
//! first projector is being read is held, and applied once the read is
//! whole, in the order the hooks came. Applied to a half-read projection it
//! would be a row with an `ord` ahead of turns older than it. A second open
//! meanwhile waits for that read rather than starting its own.
//!
//! **Behind a setting, off by default** (ov-372): `[agents] projector` in
//! config.toml, which a client's settings turn on (`settings.set_projector`),
//! or `FARCOOLER_PROJECTOR=1`. It gates all of it: off, no projector opens for a pane,
//! `agent.rows` and `agent.rows_follow` are refused as unsupported, and
//! `agent_rows` is left out of the hello, so the daemon does what it did
//! before. watch.rs's own turn, question and subagent state still come from
//! `claude::parse_line` either way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use farcooler_core::session_log::projector::{Activity, Change, HookEffect, Row, SessionProjector};
use serde_json::Value;
use uuid::Uuid;

#[path = "session_watch.rs"]
mod session_watch;
use session_watch::Watches;

/// How often a watched projector still reads every file, for a change its
/// watch missed. An unwatched one reads them every tick.
const FULL_READ_EVERY: Duration = Duration::from_secs(30);

/// The most changes a follow sends. A follower further behind pages again.
pub const MAX_CHANGES: usize = 500;

/// The most hooks held for one terminal while its first projector is read.
/// A build reads at a few microseconds a line, so this is never reached
/// unless something has gone wrong; past it, hooks are dropped, not kept.
pub const MAX_HELD: usize = 256;

/// How long a second open waits for a build already under way.
const BUILD_WAIT: Duration = Duration::from_secs(120);

/// A projector, and what its followers wait on.
pub struct Open {
    session: Mutex<SessionProjector>,
    /// Which projection this is. A follower holding another's revisions
    /// pages again.
    pub epoch: u64,
    /// The projection's revision, sent after every change.
    revision: tokio::sync::watch::Sender<u64>,
    /// The directories the shared watch reads for this projector.
    watched: Mutex<Vec<PathBuf>>,
    last_full: Mutex<Instant>,
}

impl Open {
    fn new(session: SessionProjector) -> Open {
        let (revision, _) = tokio::sync::watch::channel(session.projection().revision());
        Open {
            session: Mutex::new(session),
            epoch: next_epoch(),
            revision,
            watched: Mutex::new(Vec::new()),
            last_full: Mutex::new(Instant::now()),
        }
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
    codex: bool,
    event: String,
    payload: Value,
    at: i64,
}

/// Every open projector, by terminal.
///
/// One lock per projector, and the map's own lock only long enough to find
/// it: a rebuild reads a whole transcript, and every claude hook on the runner
/// (a held PermissionRequest among them) passes through `hook` here.
pub struct SessionProjectors {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    open: Mutex<HashMap<Uuid, Arc<Open>>>,
    /// Terminals whose first projector is being built outside the lock, with
    /// the hooks that arrived meanwhile. A `forget` meanwhile takes the
    /// terminal out of here, and the finished build is then dropped rather
    /// than kept for a terminal that is gone.
    building: Mutex<HashMap<Uuid, Build>>,
    /// Told whenever a build ends, kept or not, and on every forget.
    built: Condvar,
    /// How many times each terminal was forgotten: an open waiting on a build
    /// that a forget ended goes away rather than building one for a terminal
    /// that is gone.
    forgets: Mutex<HashMap<Uuid, u64>>,
    /// Opens waiting on another's build, for a test to see one is waiting.
    waiters: AtomicUsize,
    next_build: AtomicU64,
    watches: Watches,
    /// Itself, for the watch's thread, which must not keep it alive.
    me: std::sync::Weak<Inner>,
}

impl Default for SessionProjectors {
    fn default() -> Self {
        SessionProjectors {
            inner: Arc::new_cyclic(|me| Inner {
                open: Mutex::new(HashMap::new()),
                building: Mutex::new(HashMap::new()),
                built: Condvar::new(),
                forgets: Mutex::new(HashMap::new()),
                waiters: AtomicUsize::new(0),
                next_build: AtomicU64::new(1),
                watches: Watches::default(),
                me: me.clone(),
            }),
        }
    }
}

/// The watch goes with its owner, on the owner's thread: dropped on its own
/// event thread, an FSEvents watcher waits forever for itself to go idle.
impl Drop for SessionProjectors {
    fn drop(&mut self) {
        self.inner.watches.close();
    }
}

/// A build under way. Dropped without `finish` (a panic in the read), it
/// takes the terminal out of `building`, so hooks stop being held for it and
/// a waiting open stops waiting.
struct Building {
    inner: Arc<Inner>,
    terminal: Uuid,
    /// Which build: a forget and a new open can put a later one in the map
    /// under the same terminal, and that one is not this one's to clear.
    id: u64,
}

/// A terminal's build in the map, and the hooks held for it.
struct Build {
    id: u64,
    held: Vec<Held>,
}

impl Drop for Building {
    fn drop(&mut self) {
        let mut building = self.inner.building.lock().unwrap_or_else(|e| e.into_inner());
        if building.get(&self.terminal).is_some_and(|b| b.id == self.id) {
            building.remove(&self.terminal);
        }
        self.inner.built.notify_all();
    }
}

impl Inner {
    fn forgotten(&self, terminal: Uuid) -> u64 {
        self.forgets.lock().unwrap_or_else(|e| e.into_inner()).get(&terminal).copied().unwrap_or(0)
    }

    /// Mark `terminal` as building, if nothing is: the guard of the build.
    fn claim(self: &Arc<Self>, building: &mut HashMap<Uuid, Build>, terminal: Uuid) -> Option<Building> {
        let std::collections::hash_map::Entry::Vacant(free) = building.entry(terminal) else { return None };
        let id = self.next_build.fetch_add(1, Ordering::Relaxed);
        free.insert(Build { id, held: Vec::new() });
        Some(Building { inner: self.clone(), terminal, id })
    }
}

/// Whether claude panes get a projector, and `agent.rows` is served: the
/// `FARCOOLER_PROJECTOR=1` environment, or `[agents] projector` in
/// config.toml as the daemon read it at start or a client last set it
/// (`set_shadowing`, ov-372).
pub fn shadowing() -> bool {
    by_environment() || SETTING.load(Ordering::Relaxed)
}

/// The config's half of `shadowing`, which `settings.set_projector` changes
/// while the daemon runs.
static SETTING: AtomicBool = AtomicBool::new(false);

fn by_environment() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FARCOOLER_PROJECTOR").is_some_and(|v| v == "1"))
}

/// Turn the projector on or off, and offer `agent_rows` in every hello from
/// now on only while it's on. A connection open before keeps the hello it
/// had, so a client reconnects to see the change. Turned off, a projector
/// already open keeps following its pane until the daemon restarts; no new
/// one opens, and `agent.rows` is refused.
pub fn set_shadowing(on: bool) {
    SETTING.store(on, Ordering::Relaxed);
    let rows = farcooler_protocol::capability::AGENT_ROWS;
    match shadowing() {
        true => farcooler_protocol::capability::offer(rows),
        false => farcooler_protocol::capability::withhold(rows),
    }
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
fn apply(session: &mut SessionProjector, codex: bool, event: &str, payload: &Value, at: i64) {
    // A hook is folded only by its own agent's projection: codex's hooks
    // carry no `prompt_id`, claude's no `turn_id`.
    if session.is_codex() != codex {
        return;
    }
    let main = session.transcript().to_path_buf();
    session.poll_paths(&[main]);
    if let HookEffect::Rebind { transcript_path: Some(path), .. } = session.hook(event, payload, at) {
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

impl Inner {
    fn get(&self, terminal: Uuid) -> Option<Arc<Open>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).get(&terminal).cloned()
    }

    /// What a watch event names, read by the projectors whose directories
    /// hold it. On the watch's own thread, never the watcher's callback.
    fn read_events(&self, terminals: &[Uuid], paths: &[PathBuf]) {
        for &terminal in terminals {
            let Some(open) = self.get(terminal) else { continue };
            let mut session = open.lock();
            session.poll_paths(paths);
            open.publish(&session);
        }
    }

    /// Point the shared watch at `dirs` for `terminal`, and away from any it
    /// read before and no longer does. Whether the main file's directory is
    /// watched. Never with a projector locked.
    fn ensure_watch(&self, terminal: Uuid, open: &Open, dirs: (PathBuf, PathBuf)) -> bool {
        let mut want = vec![dirs.0.clone()];
        if dirs.1.is_dir() {
            want.push(dirs.1);
        }
        // Held throughout, so two at once (a tick and a hook's rebind) can't
        // each route a directory and leave one of them unrecorded, and so
        // `forget`, which takes the list, sees every route made.
        let mut watched = open.watched.lock().unwrap_or_else(|e| e.into_inner());
        if *watched == want {
            return true;
        }
        // Forgotten meanwhile: route nothing that no forget will let go.
        if !self.get(terminal).is_some_and(|now| std::ptr::eq(Arc::as_ptr(&now), open)) {
            return false;
        }
        for dir in watched.iter().filter(|d| !want.contains(d)) {
            self.watches.unroute(terminal, dir);
        }
        let routed: Vec<PathBuf> = want.into_iter().filter(|dir| watched.contains(dir) || self.watches.route(self, terminal, dir)).collect();
        let main = routed.contains(&dirs.0);
        *watched = routed;
        main
    }
}

impl SessionProjectors {
    fn get(&self, terminal: Uuid) -> Option<Arc<Open>> {
        self.inner.get(terminal)
    }

    /// Open a projector for `terminal` on `transcript`, read what is on disk
    /// so far, and keep it. A terminal already open on that transcript is
    /// left as it is; on another, it is moved there with its rows kept.
    ///
    /// A new one is built and read with no lock held, and put in the map only
    /// once it is whole; hooks that arrive meanwhile are applied after. An
    /// open while another is building waits for that build.
    pub fn open(&self, terminal: Uuid, transcript: PathBuf) {
        let transcript = canonical(&transcript);
        let started = Instant::now();
        let forgets = self.inner.forgotten(terminal);
        let guard = loop {
            if let Some(open) = self.get(terminal) {
                let dirs = {
                    let mut session = open.lock();
                    session.rebind(transcript);
                    session.poll();
                    open.publish(&session);
                    session.watched_dirs()
                };
                self.inner.ensure_watch(terminal, &open, dirs);
                return;
            }
            let mut building = self.inner.building.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(guard) = self.inner.claim(&mut building, terminal) {
                break guard;
            }
            let Some(left) = BUILD_WAIT.checked_sub(started.elapsed()) else { return };
            self.inner.waiters.fetch_add(1, Ordering::SeqCst);
            let (building, waited) = self.inner.built.wait_timeout(building, left).unwrap_or_else(|e| e.into_inner());
            self.inner.waiters.fetch_sub(1, Ordering::SeqCst);
            drop(building);
            // Woken by a forget: the terminal is gone, so build nothing.
            if waited.timed_out() || self.inner.forgotten(terminal) != forgets {
                return;
            }
        };
        let mut session = SessionProjector::open(transcript);
        session.poll();
        self.finish(guard, session);
    }

    /// Keep a projector built for `terminal`, unless `forget` came first, and
    /// apply the hooks held while it was built.
    fn finish(&self, guard: Building, session: SessionProjector) {
        let terminal = guard.terminal;
        let dirs = session.watched_dirs();
        let mut building = self.inner.building.lock().unwrap_or_else(|e| e.into_inner());
        // Not this build's entry (a forget took it, or a later open's is
        // there): this build is dropped.
        let Some(Build { held, .. }) = building.remove_entry(&terminal).and_then(|(t, b)| {
            if b.id == guard.id {
                Some(b)
            } else {
                building.insert(t, b);
                None
            }
        }) else {
            drop(building);
            drop(guard);
            return;
        };
        let built = Arc::new(Open::new(session));
        let open = self.inner.open.lock().unwrap_or_else(|e| e.into_inner()).entry(terminal).or_insert(built).clone();
        // Locked before the build is no longer marked as one, so a hook that
        // misses the mark waits here for the held ones to go first.
        let mut session = open.lock();
        drop(building);
        drop(guard);
        for hook in held {
            apply(&mut session, hook.codex, &hook.event, &hook.payload, hook.at);
        }
        open.publish(&session);
        drop(session);
        self.inner.ensure_watch(terminal, &open, dirs);
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

    /// Run `f` on `terminal`'s projector, locked. For a caller that reads
    /// more than one thing at one revision, and for tests that hold the lock.
    pub fn with_session<R>(&self, terminal: Uuid, f: impl FnOnce(&mut SessionProjector) -> R) -> Option<R> {
        let open = self.get(terminal)?;
        let mut session = open.lock();
        Some(f(&mut session))
    }

    /// A claude hook routed to `terminal`. Held while its projector is being
    /// built; nothing when none is open.
    pub fn hook(&self, terminal: Uuid, event: &str, payload: &Value) {
        self.hook_from(terminal, false, event, payload);
    }

    /// A codex hook routed to `terminal` (ov-378), as `hook` is claude's.
    pub fn codex_hook(&self, terminal: Uuid, event: &str, payload: &Value) {
        self.hook_from(terminal, true, event, payload);
    }

    fn hook_from(&self, terminal: Uuid, codex: bool, event: &str, payload: &Value) {
        {
            let mut building = self.inner.building.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(Build { held, .. }) = building.get_mut(&terminal) {
                if held.len() < MAX_HELD {
                    held.push(Held { codex, event: event.to_string(), payload: payload.clone(), at: now_ms() });
                } else if held.len() == MAX_HELD {
                    tracing::warn!(terminal = %terminal, "a projector's build is holding too many hooks; dropping the rest");
                }
                return;
            }
        }
        let Some(open) = self.get(terminal) else { return };
        let dirs = {
            let mut session = open.lock();
            let before = session.transcript().to_path_buf();
            apply(&mut session, codex, event, payload, now_ms());
            open.publish(&session);
            (session.transcript() != before).then(|| session.watched_dirs())
        };
        if let Some(dirs) = dirs {
            self.inner.ensure_watch(terminal, &open, dirs);
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
        let watched = self.inner.ensure_watch(terminal, &open, dirs);
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

    /// What claude's box suggests on `terminal`'s screen now (ov-409), for its
    /// newest turn row. Nothing when the terminal has no projector open.
    pub fn suggest(&self, terminal: Uuid, suggestion: Option<String>) {
        let Some(open) = self.get(terminal) else { return };
        let mut session = open.lock();
        session.projection_mut().set_suggestion(suggestion);
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
    /// until `deadline` for something to, and sending at most `max` changes.
    /// `None` when no projector is open.
    pub async fn follow(&self, terminal: Uuid, epoch: u64, after: u64, deadline: tokio::time::Instant, max: usize) -> Option<Follow> {
        let open = self.get(terminal)?;
        let mut changed = open.revision.subscribe();
        loop {
            let answer = {
                let session = open.lock();
                let p = session.projection();
                let rev = p.revision();
                if epoch != open.epoch || after > rev {
                    return Some(Follow::Reset { epoch: open.epoch, rev });
                }
                match p.changes_since(after, max) {
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

    /// The terminal is gone. Its watch is let go on a thread of its own:
    /// never the watch's (where FSEvents would wait on itself forever), and
    /// not the caller's, since taking a directory off an FSEvents stream
    /// restarts the stream, about half a second measured.
    pub fn forget(&self, terminal: Uuid) {
        let open = {
            let mut building = self.inner.building.lock().unwrap_or_else(|e| e.into_inner());
            building.remove(&terminal);
            *self.inner.forgets.lock().unwrap_or_else(|e| e.into_inner()).entry(terminal).or_default() += 1;
            self.inner.built.notify_all();
            self.inner.open.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal)
        };
        let Some(open) = open else { return };
        let dirs = std::mem::take(&mut *open.watched.lock().unwrap_or_else(|e| e.into_inner()));
        if dirs.is_empty() {
            return;
        }
        let inner = self.inner.clone();
        let unrouted = std::thread::Builder::new().name("fc-projector-unwatch".into()).spawn(move || {
            for dir in &dirs {
                inner.watches.unroute(terminal, dir);
            }
        });
        if let Err(e) = unrouted {
            tracing::warn!(error = %e, "could not let a forgotten projector's watch go");
        }
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
impl SessionProjectors {
    /// `open`'s first half: `terminal` marked as building.
    fn begin(&self, terminal: Uuid) -> Building {
        let mut building = self.inner.building.lock().unwrap();
        self.inner.claim(&mut building, terminal).expect("nothing building")
    }

    async fn follow_for(&self, terminal: Uuid, epoch: u64, after: u64, wait: Duration) -> Option<Follow> {
        self.follow(terminal, epoch, after, tokio::time::Instant::now() + wait, MAX_CHANGES).await
    }
}

#[cfg(test)]
#[path = "session_projectors_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "session_projectors_follow_tests.rs"]
mod follow_tests;
