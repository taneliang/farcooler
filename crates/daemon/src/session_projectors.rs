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
//! **Opened for a viewer, not for every pane** (ov-830). There is no
//! setting: `agent_rows` is always offered, and a pane's projector opens
//! when a device asks for its rows (`agent.rows`, `agent.rows_follow`,
//! `agent.image`, each marking it `viewed`), and is let go on the pane's
//! next tick once nobody has asked for `LINGER`. A pane nobody views has no
//! projector, no watch and no rebuild, so the idle cost is the one check of
//! the time it was last asked for. `FARCOOLER_PROJECTOR=1` (or `set_eager`,
//! in a test) opens one for every claude pane, as the projector did before
//! a viewer had to ask. watch.rs's own turn, question and subagent state
//! still come from `claude::parse_line` either way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use farcooler_core::session_log::projector::held::ASK_ENDED;
use farcooler_core::session_log::projector::{Activity, Change, HookEffect, Row, SessionProjector};
use serde_json::Value;
use uuid::Uuid;

#[path = "session_watch.rs"]
mod session_watch;
#[path = "session_projectors_holds.rs"]
mod holds;
use session_watch::Watches;

/// How often a watched projector still reads every file, for a change its
/// watch missed. An unwatched one reads them every tick.
const FULL_READ_EVERY: Duration = Duration::from_secs(30);

/// How often a waiting follow reads the main file itself (ov-674). The watch
/// wakes it within milliseconds when FSEvents delivers, and the pane's tick
/// reads the file too, but under a full parallel run FSEvents delivered none
/// of 40 appends and the watcher's sampling loop went up to 19 s between
/// ticks, so a follower's live rows stalled that long. This bounds it.
const CATCH_UP: Duration = Duration::from_secs(1);

/// The most changes a follow sends. A follower further behind pages again.
pub const MAX_CHANGES: usize = 500;

/// The most hooks held for one terminal while its first projector is read.
/// A build reads at a few microseconds a line, so this is never reached
/// unless something has gone wrong; past it, hooks are dropped, not kept.
pub const MAX_HELD: usize = 256;

/// Ticks in a row a pane's foreground must hold no agent before a claude
/// the registry has no file for is taken to have exited (`judge_agent`).
const BARE_TICKS: u32 = 3;

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
    /// The agent's process as the registry last named it in this pane
    /// (`watch_agent`).
    agent: Mutex<Option<crate::claude_registry::Entry>>,
    /// Whether the agent's fate was decided: watched from a registry entry
    /// seen (`watch_agent`), or judged without one (`judge_agent`).
    judged: AtomicBool,
    /// How many of the pane's last ticks in a row found no agent in its
    /// foreground processes (`note_foreground`).
    bare_ticks: AtomicU32,
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
            agent: Mutex::new(None),
            judged: AtomicBool::new(false),
            bare_ticks: AtomicU32::new(0),
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
pub(crate) fn next_epoch() -> u64 {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| AtomicU64::new((now_ms().max(1) as u64).saturating_mul(1000))).fetch_add(1, Ordering::Relaxed)
}

/// A hook that arrived while its terminal's projector was being built, or
/// one kept to replay into the next (`holds`).
#[derive(Clone)]
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
    /// Watch events waiting for a projector's lock, for a test to see one
    /// is (`events_waiting`).
    reading: AtomicUsize,
    next_build: AtomicU64,
    /// A waiting follow's own read, `CATCH_UP` but for a test that must see
    /// the watch alone wake a follower.
    catch_up: Duration,
    watches: Watches,
    /// What each pane's runner holds that its transcript can't say, replayed
    /// into every projector a build makes (ov-830).
    holds: holds::Holds,
    /// Itself, for the watch's thread, which must not keep it alive.
    me: std::sync::Weak<Inner>,
}

impl Default for SessionProjectors {
    fn default() -> Self {
        SessionProjectors::catching_up_every(CATCH_UP)
    }
}

impl SessionProjectors {
    /// Projectors whose waiting follows read for themselves every
    /// `catch_up`: `CATCH_UP`, but for a test of the watch alone.
    fn catching_up_every(catch_up: Duration) -> Self {
        SessionProjectors {
            inner: Arc::new_cyclic(|me| Inner {
                open: Mutex::new(HashMap::new()),
                building: Mutex::new(HashMap::new()),
                built: Condvar::new(),
                forgets: Mutex::new(HashMap::new()),
                waiters: AtomicUsize::new(0),
                reading: AtomicUsize::new(0),
                next_build: AtomicU64::new(1),
                catch_up,
                watches: Watches::default(),
                holds: holds::Holds::default(),
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

/// Whether every claude pane gets a projector, viewed or not: the
/// `FARCOOLER_PROJECTOR=1` environment, or a test's `set_eager`.
pub fn eager() -> bool {
    by_environment() || EAGER.load(Ordering::Relaxed)
}

/// A test's half of `eager`.
static EAGER: AtomicBool = AtomicBool::new(false);

fn by_environment() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FARCOOLER_PROJECTOR").is_some_and(|v| v == "1"))
}

/// Open a projector for every claude pane from now on, viewed or not, as
/// `FARCOOLER_PROJECTOR=1` does. For tests of the tick's own feeding.
pub fn set_eager(on: bool) {
    EAGER.store(on, Ordering::Relaxed);
}

/// How long a pane keeps its projector after a device last asked for its
/// rows. Longer than a follow is held (`rpc_rows::MAX_WAIT`, 25 s) with room
/// for a reconnect, so a view that is following never sees its projector let
/// go between two follows. Not longer: coming back later costs a rebuild,
/// measured in release at 7 ms for a 5 MB transcript and 66 ms for 25 MB of
/// dense rows (55,000 lines, 6,800 rows; core's `rebuilding_a_long_transcript`),
/// so keeping a watch and a read per tick for minutes would buy almost
/// nothing.
pub const LINGER: Duration = Duration::from_secs(90);

/// When each terminal's rows were last asked for.
static VIEWED: std::sync::LazyLock<Mutex<HashMap<Uuid, Instant>>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// A device asked for `terminal`'s rows just now: its projector opens on
/// the pane's next tick if it has none, and stays until `LINGER` after the
/// last such ask.
pub fn viewed(terminal: Uuid) {
    VIEWED.lock().unwrap_or_else(|e| e.into_inner()).insert(terminal, Instant::now());
}

/// Whether `terminal` should have a projector: every pane while `eager`,
/// else one a device asked for within `LINGER`. A pane past it is
/// forgotten here, so the map holds only the panes being viewed.
pub fn wanted(terminal: Uuid) -> bool {
    if eager() {
        return true;
    }
    let mut viewed = VIEWED.lock().unwrap_or_else(|e| e.into_inner());
    match viewed.get(&terminal) {
        Some(at) if at.elapsed() < LINGER => true,
        Some(_) => {
            viewed.remove(&terminal);
            false
        }
        None => false,
    }
}

/// Move `terminal`'s last ask back by `by`, as if that long had passed.
#[cfg(test)]
pub(crate) fn age_view(terminal: Uuid, by: Duration) {
    if let Some(at) = VIEWED.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&terminal) {
        *at = at.checked_sub(by).unwrap_or(*at);
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
            self.reading.fetch_add(1, Ordering::SeqCst);
            let mut session = open.lock();
            self.reading.fetch_sub(1, Ordering::SeqCst);
            session.poll_paths(paths);
            open.publish(&session);
        }
    }

    /// Every file of each of `terminals`' projectors, read again: the watch
    /// asked for a rescan of their directories, so any change in them may
    /// have gone unnamed (ov-681). On the watch's own thread.
    fn read_all(&self, terminals: &[Uuid]) {
        for &terminal in terminals {
            let Some(open) = self.get(terminal) else { continue };
            self.reading.fetch_add(1, Ordering::SeqCst);
            let mut session = open.lock();
            self.reading.fetch_sub(1, Ordering::SeqCst);
            session.poll();
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
        for hook in &held {
            apply(&mut session, hook.codex, &hook.event, &hook.payload, hook.at);
        }
        // What the transcript can't say: a live held ask, held messages and
        // why one wasn't sent, from while no projector was open (ov-830).
        self.inner.holds.replay(terminal, &mut session, &held);
        open.publish(&session);
        drop(session);
        self.inner.ensure_watch(terminal, &open, dirs);
    }

    /// How many watch events are waiting on a projector's lock right now.
    /// For a test that must know one is before it acts (ov-547).
    #[doc(hidden)]
    pub fn events_waiting(&self) -> usize {
        self.inner.reading.load(Ordering::SeqCst)
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
    ///
    /// Never the runner's own note (`ask_ended`), whatever a hook frame
    /// calls itself (ov-743): a process in the pane can send the hook socket
    /// any event, and that note says who answered an ask.
    pub fn hook(&self, terminal: Uuid, event: &str, payload: &Value) {
        if !runners_own(event) {
            self.hook_from(terminal, false, event, payload);
        }
    }

    /// A codex hook routed to `terminal` (ov-378), as `hook` is claude's.
    pub fn codex_hook(&self, terminal: Uuid, event: &str, payload: &Value) {
        if !runners_own(event) {
            self.hook_from(terminal, true, event, payload);
        }
    }

    /// The runner's note that the hold of an ask on `terminal` ended
    /// (`held::ASK_ENDED`): who answered it, from the device that did.
    pub fn ask_ended(&self, terminal: Uuid, note: &Value) {
        self.hook_from(terminal, false, ASK_ENDED, note);
    }

    fn hook_from(&self, terminal: Uuid, codex: bool, event: &str, payload: &Value) {
        // Kept whether or not a projector is open, for the next one (ov-830).
        if event == ASK_ENDED {
            if let Some(id) = payload.get("ask_id").and_then(Value::as_str) {
                self.inner.holds.ask_gone(terminal, id);
            }
        } else {
            self.inner.holds.hook(terminal, &Held { codex, event: event.to_string(), payload: payload.clone(), at: now_ms() });
        }
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
        if dirs.is_some() {
            self.rewatch_elsewhere(terminal, open);
        }
    }

    /// Point the watch at the files a hook moved `terminal` to, on a thread
    /// of its own (ov-674): a hook arrives on an async worker
    /// (`HookIngress::serve`), and routing a directory restarts the FSEvents
    /// stream, which blocks for seconds under a busy fseventsd. The
    /// directories are read there, not here, so a later move that routes
    /// first is never undone by this one; the pane's tick routes them too.
    fn rewatch_elsewhere(&self, terminal: Uuid, open: Arc<Open>) {
        let inner = self.inner.clone();
        let spawned = std::thread::Builder::new().name("fc-projector-rewatch".into()).spawn(move || {
            let dirs = open.lock().watched_dirs();
            inner.ensure_watch(terminal, &open, dirs);
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "could not move a projector's watch; its tick will");
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

    /// Follow the agent's process in `terminal`'s pane (ov-612): `seen` is
    /// the registry's entry for it this tick, if it named one. Once the
    /// process last seen is gone (`running` says no), no subagent it launched
    /// is still running, and none will say it ended; nor when another process
    /// has taken its place between two ticks. Decided by the process
    /// alone: a screen that misreads for a tick, or an editor in front, isn't
    /// an exit, and a shell still showing claude's last frame isn't a claude.
    pub fn watch_agent(&self, terminal: Uuid, seen: Option<crate::claude_registry::Entry>, running: impl Fn(&crate::claude_registry::Entry) -> bool) {
        let Some(open) = self.get(terminal) else { return };
        {
            let mut agent = open.agent.lock().unwrap_or_else(|e| e.into_inner());
            // Another process in the last one's place (a respawn, an exit and
            // a quick relaunch, both between two ticks) is the last one's
            // exit, once it's gone. One still running beside it (a claude
            // started inside it) isn't.
            let replaced = |last: &crate::claude_registry::Entry, seen: &crate::claude_registry::Entry| {
                (last.pid, last.started) != (seen.pid, seen.started) && !running(last)
            };
            match (agent.take(), seen) {
                (Some(last), Some(seen)) if replaced(&last, &seen) => *agent = Some(seen),
                (_, Some(seen)) => {
                    *agent = Some(seen);
                    return;
                }
                (Some(last), None) if !running(&last) => {}
                (last, None) => {
                    *agent = last;
                    return;
                }
            }
        }
        let mut session = open.lock();
        if session.projection_mut().stop_running_subagents(Some(now_ms())) {
            open.publish(&session);
        }
    }

    /// What the pane's foreground processes held this tick: `bare` when no
    /// agent ran among them (`running_agent::of`), read from the process
    /// table, not the screen. Counted in a row, as `judge_agent` needs them.
    pub fn note_foreground(&self, terminal: Uuid, bare: bool) {
        let Some(open) = self.get(terminal) else { return };
        if bare {
            // Counted only up to the threshold `judge_agent` reads, so it can't wrap.
            if open.bare_ticks.load(Ordering::Relaxed) < BARE_TICKS {
                open.bare_ticks.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            open.bare_ticks.store(0, Ordering::Relaxed);
        }
    }

    /// Decide the fate of a claude this projector never saw run, once: a
    /// daemon that restarted while claude was gone has nothing watched, so
    /// `watch_agent` would never see it end (ov-623). `seen` is the live
    /// process the registry names in this pane now, which `watch_agent` takes
    /// over; `pane_gone` is a pane that has exited or been lost. Otherwise, in the order of evidence:
    ///
    /// - the registry still holds files naming the session (`ended`): all of
    ///   a process now gone is claude's end, and any other is not;
    /// - it holds none (a clean exit removes it): claude's end only when the
    ///   pane's foreground has held no agent for `BARE_TICKS` ticks read
    ///   (a tick unread, or one that found an agent, starts the count over or
    ///   adds nothing), or its pane is gone. A claude outside the registry
    ///   that runs under a wrapper not named `claude` reads as no agent.
    ///
    /// An agent running with the registry unable to say (an older claude, a
    /// pane whose foreground is not claude's) is never stopped by this.
    pub fn judge_agent(
        &self,
        terminal: Uuid,
        seen: Option<&crate::claude_registry::Entry>,
        pane_gone: bool,
        ended: impl FnOnce(&str) -> Option<bool>,
    ) {
        let Some(open) = self.get(terminal) else { return };
        if open.judged.load(Ordering::Relaxed) {
            return;
        }
        if let Some(seen) = seen {
            open.judged.store(true, Ordering::Relaxed);
            // A resume into the same session before the daemon was back hides
            // the old process's end: whatever was launched before this one
            // started is not its (a second of slack for `procStart`).
            if let Some(started) = seen.started {
                let mut session = open.lock();
                if session.projection_mut().stop_subagents_launched_before((started - 1) * 1000, Some(now_ms())) {
                    open.publish(&session);
                }
            }
            return;
        }
        let session = open.lock().transcript().file_stem().map(|s| s.to_string_lossy().into_owned());
        let gone = match session.as_deref().and_then(ended) {
            Some(gone) => gone,
            None => pane_gone || open.bare_ticks.load(Ordering::Relaxed) >= BARE_TICKS,
        };
        if !gone {
            return;
        }
        open.judged.store(true, Ordering::Relaxed);
        let mut session = open.lock();
        if session.projection_mut().stop_running_subagents(Some(now_ms())) {
            open.publish(&session);
        }
    }

    /// What claude's box shows on `terminal`'s screen now (ov-409): the
    /// prompt it suggests, for its newest turn row, and its generic `Try`
    /// example, for the hint row. Nothing when the terminal has no projector
    /// open.
    pub fn suggest(&self, terminal: Uuid, suggestion: Option<String>, hint: Option<String>) {
        let Some(open) = self.get(terminal) else { return };
        let mut session = open.lock();
        session.projection_mut().set_suggestion(suggestion);
        session.projection_mut().set_hint(hint);
        open.publish(&session);
    }

    /// `f` on `terminal`'s projection, published, if one is open.
    fn on_projection(&self, terminal: Uuid, f: impl FnOnce(&mut farcooler_core::session_log::projector::Projection)) {
        let Some(open) = self.get(terminal) else { return };
        let mut session = open.lock();
        f(session.projection_mut());
        open.publish(&session);
    }

    /// The runner holds `text` for `terminal` as `id` (ov-468): Queued on its
    /// rows, now if a projector is open and in every one opened while it's
    /// held (ov-830).
    pub fn runner_held(&self, terminal: Uuid, id: &str, text: &str, at_ms: i64) {
        self.inner.holds.held(terminal, id, text, at_ms);
        self.on_projection(terminal, |p| p.runner_held(id, text, at_ms));
    }

    /// The runner pasted `text` as `id` into the agent's box, its Enter
    /// waiting for the turn's end (ov-827): Waiting to send, now and in a
    /// projector opened later.
    pub fn runner_in_box(&self, terminal: Uuid, id: &str, text: &str, at_ms: i64) {
        self.inner.holds.in_box(terminal, id, text, at_ms);
        self.on_projection(terminal, |p| p.runner_in_box(id, text, at_ms));
    }

    /// The runner typed `id`: its Queued row comes off, and claude's own
    /// record shows it from now on.
    pub fn runner_told(&self, terminal: Uuid, id: &str) {
        self.inner.holds.told(terminal, id);
        self.on_projection(terminal, |p| p.runner_told(id));
    }

    /// The runner gave `id` up, for `why`: Withdrawn, with a notice, now and
    /// in a projector opened later (ov-830).
    pub fn runner_dropped(&self, terminal: Uuid, id: &str, why: &str, at_ms: i64) {
        self.inner.holds.dropped(terminal, id, why, at_ms);
        self.on_projection(terminal, |p| p.runner_dropped(id, why, at_ms));
    }

    /// The held ask `id` on `terminal` ended, however it did, a device's
    /// answer or a withdrawal that tells no row: a projector opened from
    /// now on doesn't show it held (ov-830).
    pub fn ask_gone(&self, terminal: Uuid, id: &str) {
        self.inner.holds.ask_gone(terminal, id);
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
    ///
    /// Woken by a publish: the watch's read of an append, a hook, a tick.
    /// While none comes it reads the main file itself every `CATCH_UP`, on a
    /// blocking thread, so an append whose event was lost or late still
    /// reaches the follower within about that.
    pub async fn follow(&self, terminal: Uuid, epoch: u64, after: u64, deadline: tokio::time::Instant, max: usize) -> Option<Follow> {
        let open = self.get(terminal)?;
        let mut changed = open.revision.subscribe();
        let mut catch_up = tokio::time::Instant::now() + self.inner.catch_up;
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
            let wake = catch_up.min(deadline);
            if tokio::time::timeout_at(wake, changed.changed()).await.is_ok() {
                continue;
            }
            if wake == deadline {
                // Nothing in the wait: no changes, and the follower stays at
                // its revision (one that lands just now, the next follow sees).
                return Some(Follow::Changes { epoch: open.epoch, rev: after, changes: Vec::new() });
            }
            let read = open.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let mut session = read.lock();
                let main = session.transcript().to_path_buf();
                session.poll_paths(&[main]);
                read.publish(&session);
            })
            .await;
            catch_up = tokio::time::Instant::now() + self.inner.catch_up;
        }
    }

    /// `terminal` is gone: its projector, what the runner held for it, and
    /// when it was last viewed.
    pub fn forget_terminal(&self, terminal: Uuid) {
        self.forget(terminal);
        self.inner.holds.forget(terminal);
        VIEWED.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
    }

    /// The projector goes: the terminal is gone, or nobody views it. Its
    /// watch is let go on a thread of its own: never the watch's (where
    /// FSEvents would wait on itself forever), and not the caller's, since
    /// taking a directory off an FSEvents stream restarts the stream, about
    /// half a second measured. What the runner holds for the pane stays,
    /// for the next projector (`forget_terminal` takes that too).
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

pub(crate) fn owned(changes: Vec<Change<'_>>) -> Vec<RowChange> {
    changes
        .into_iter()
        .map(|c| match c {
            Change::Insert(row) => RowChange::Insert(row.clone()),
            Change::Update(row) => RowChange::Update(row.clone()),
            Change::Remove { id, rev } => RowChange::Remove { id: id.to_string(), rev },
        })
        .collect()
}

/// Whether `event` is one only the runner writes, never a hook: dropped
/// when it comes in on the hook socket (`SessionProjectors::hook`).
fn runners_own(event: &str) -> bool {
    if event == ASK_ENDED {
        tracing::debug!(event, "a hook frame named for the runner's own note; dropped");
        return true;
    }
    false
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
