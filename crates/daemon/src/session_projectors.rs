//! The daemon's session projectors: one per claude pane that has one open
//! (ov-363).
//!
//! A projector is opened for a terminal and rebuilt from the transcript on
//! disk (design decision D8: rebuild, no checkpoint). From then on it is fed
//! from three places, which are the inputs the design names:
//!
//! - every claude hook routed to the terminal (`HookIngress::accept`), as
//!   provisional rows;
//! - the watcher's tick for the pane (`watch::registry_join`), which reads
//!   what the files gained and hands over claude's registry status;
//! - `SessionStart` for another session, which moves it to that transcript.
//!
//! **Beside the old readers, not yet instead of them.** Nothing reads rows
//! out of here yet: the paged `agent.rows` RPC is ov-366. Until it lands a
//! projector is opened only on request (`open`), or for every claude pane
//! whose log the registry names when `FARCOOLER_PROJECTOR=1` is set, so the
//! daemon can be run with it shadowing the old readers. watch.rs's own
//! turn, question and subagent state still come from `claude::parse_line`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use farcooler_core::session_log::projector::{Activity, HookEffect, Row, SessionProjector};
use uuid::Uuid;

/// Every open projector, by terminal.
///
/// One lock per projector, and the map's own lock only long enough to find
/// it: a rebuild reads a whole transcript, and every claude hook on the runner
/// (a held PermissionRequest among them) passes through `hook` here.
#[derive(Default)]
pub struct SessionProjectors {
    open: Mutex<HashMap<Uuid, Arc<Mutex<SessionProjector>>>>,
    /// Terminals whose first projector is being built outside the lock. A
    /// `forget` meanwhile takes the terminal out of here, and the finished
    /// build is then dropped rather than kept for a terminal that is gone.
    building: Mutex<std::collections::HashSet<Uuid>>,
}

/// Whether claude panes get a projector without anyone asking for one.
pub fn shadowing() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FARCOOLER_PROJECTOR").is_some_and(|v| v == "1"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn lock(session: &Mutex<SessionProjector>) -> std::sync::MutexGuard<'_, SessionProjector> {
    session.lock().unwrap_or_else(|e| e.into_inner())
}

impl SessionProjectors {
    fn get(&self, terminal: Uuid) -> Option<Arc<Mutex<SessionProjector>>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).get(&terminal).cloned()
    }

    /// Open a projector for `terminal` on `transcript`, read what is on disk
    /// so far, and keep it. A terminal already open on that transcript is
    /// left as it is; on another, it is moved there with its rows kept.
    ///
    /// A new one is built and read with no lock held, and put in the map only
    /// once it is whole.
    pub fn open(&self, terminal: Uuid, transcript: PathBuf) {
        if let Some(session) = self.get(terminal) {
            let mut session = lock(&session);
            session.rebind(transcript);
            session.poll();
            return;
        }
        self.building.lock().unwrap_or_else(|e| e.into_inner()).insert(terminal);
        let mut session = SessionProjector::open(transcript);
        session.poll();
        self.finish(terminal, session);
    }

    /// Keep a projector built for `terminal`, unless `forget` came first.
    fn finish(&self, terminal: Uuid, session: SessionProjector) {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if self.building.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal) {
            open.entry(terminal).or_insert_with(|| Arc::new(Mutex::new(session)));
        }
    }

    pub fn is_open(&self, terminal: Uuid) -> bool {
        self.get(terminal).is_some()
    }

    /// The transcript `terminal`'s projector reads, if one is open.
    pub fn transcript(&self, terminal: Uuid) -> Option<PathBuf> {
        let session = self.get(terminal)?;
        let path = lock(&session).transcript().to_path_buf();
        Some(path)
    }

    /// A claude hook routed to `terminal`. Nothing when no projector is open.
    pub fn hook(&self, terminal: Uuid, event: &str, payload: &serde_json::Value) {
        let Some(session) = self.get(terminal) else { return };
        let mut session = lock(&session);
        // The file first, so a hook that arrives after its own record is
        // checked against it rather than put up as news.
        session.poll();
        if let HookEffect::Rebind { transcript_path: Some(path), .. } = session.projection_mut().hook(event, payload, now_ms()) {
            session.rebind(path);
            session.poll();
        }
    }

    /// The watcher's tick: whatever the files gained, and what claude's
    /// registry says the process is doing.
    pub fn tick(&self, terminal: Uuid, activity: Option<Activity>) {
        let Some(session) = self.get(terminal) else { return };
        let mut session = lock(&session);
        session.poll();
        if let Some(activity) = activity {
            session.projection_mut().set_activity(activity);
        }
    }

    /// A page of `terminal`'s rows, oldest first: up to `limit` before `ord`.
    pub fn page(&self, terminal: Uuid, before: Option<u64>, limit: usize) -> Option<Vec<Row>> {
        let session = self.get(terminal)?;
        let page = lock(&session).projection().page(before, limit).to_vec();
        Some(page)
    }

    /// `terminal`'s rows changed after revision `rev`, and the revision now.
    pub fn changed_since(&self, terminal: Uuid, rev: u64) -> Option<(Vec<Row>, u64)> {
        let session = self.get(terminal)?;
        let session = lock(&session);
        let p = session.projection();
        Some((p.changed_since(rev).into_iter().cloned().collect(), p.revision()))
    }

    /// The terminal is gone.
    pub fn forget(&self, terminal: Uuid) {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        self.building.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
        open.remove(&terminal);
    }
}

/// The daemon's projectors.
pub fn global() -> &'static SessionProjectors {
    static PROJECTORS: OnceLock<SessionProjectors> = OnceLock::new();
    PROJECTORS.get_or_init(SessionProjectors::default)
}

#[cfg(test)]
#[path = "session_projectors_tests.rs"]
mod tests;
