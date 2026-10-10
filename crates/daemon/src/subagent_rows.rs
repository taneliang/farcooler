//! A subagent's own rows (ov-453): `agent_id` on `agent.rows` and
//! `agent.rows_follow`, for a view that opens a running agent from its tray.
//!
//! The agent's transcript is claude's `<session>/subagents/agent-<id>.jsonl`
//! beside the pane's (`projector::subagent_transcript`), read by a projector
//! of its own as though it were a session: the task it was given is the
//! turn's prompt, then its calls and its words, so it draws in the view's
//! own style.
//!
//! Kept apart from `session_projectors` on purpose. Nobody follows a
//! subagent but a person looking at it, so there is no watch and no hook
//! here: a page reads what the file gained, and a follow reads again every
//! `POLL` until something changes or its wait runs out. At most `KEPT` are
//! held, the least recently read let go first, and a terminal's go with it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use farcooler_core::session_log::projector::SessionProjector;
use uuid::Uuid;

use crate::session_projectors::{Follow, Page, next_epoch, owned};

/// How often a follow reads the file again while nothing has changed.
pub const POLL: Duration = Duration::from_millis(250);
/// The most subagent projectors held at once.
pub const KEPT: usize = 32;

struct Open {
    session: Mutex<SessionProjector>,
    epoch: u64,
    used: Mutex<Instant>,
}

impl Open {
    fn lock(&self) -> MutexGuard<'_, SessionProjector> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Every subagent projector open, by terminal and agent.
#[derive(Default)]
pub struct SubagentProjectors {
    open: Mutex<HashMap<(Uuid, String), Arc<Open>>>,
}

impl SubagentProjectors {
    /// The projector for `agent` of `terminal`, on `path`: the one held, or a
    /// new one when none is or the pane's session moved (`/clear`).
    fn get(&self, terminal: Uuid, agent: &str, path: PathBuf) -> Arc<Open> {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        let key = (terminal, agent.to_string());
        if let Some(held) = open.get(&key).filter(|held| held.lock().transcript() == path) {
            *held.used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
            return held.clone();
        }
        if open.len() >= KEPT && !open.contains_key(&key) {
            let oldest = open.iter().min_by_key(|(_, o)| *o.used.lock().unwrap_or_else(|e| e.into_inner())).map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                open.remove(&oldest);
            }
        }
        let fresh = Arc::new(Open { session: Mutex::new(SessionProjector::open(path)), epoch: next_epoch(), used: Mutex::new(Instant::now()) });
        open.insert(key, fresh.clone());
        fresh
    }

    /// Up to `limit` of the agent's rows before `before`, oldest first, read
    /// up to now. Blocking: it reads the file.
    pub fn page(&self, terminal: Uuid, agent: &str, path: PathBuf, before: Option<u64>, limit: usize) -> Page {
        let open = self.get(terminal, agent, path);
        let mut session = open.lock();
        session.poll();
        let p = session.projection();
        let rows: Vec<_> = p.page(before, limit).into_iter().cloned().collect();
        let more_before = rows.first().is_some_and(|r| p.any_before(r.ord));
        Page { epoch: open.epoch, rev: p.revision(), rows, more_before }
    }

    /// What changed after `after` in projection `epoch`, read again every
    /// `POLL` until something has or `deadline` passes.
    pub async fn follow(
        &self, terminal: Uuid, agent: &str, path: PathBuf, epoch: u64, after: u64, deadline: tokio::time::Instant, max: usize,
    ) -> Follow {
        let open = self.get(terminal, agent, path);
        loop {
            let read = open.clone();
            let answer = tokio::task::spawn_blocking(move || {
                let mut session = read.lock();
                session.poll();
                let p = session.projection();
                let rev = p.revision();
                if epoch != read.epoch || after > rev {
                    return Some(Follow::Reset { epoch: read.epoch, rev });
                }
                match p.changes_since(after, max) {
                    None => Some(Follow::Reset { epoch: read.epoch, rev }),
                    Some(changes) if !changes.is_empty() => Some(Follow::Changes { epoch: read.epoch, rev, changes: owned(changes) }),
                    Some(_) => None,
                }
            })
            .await
            .unwrap_or(None);
            if let Some(answer) = answer {
                return answer;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Follow::Changes { epoch: open.epoch, rev: after, changes: Vec::new() };
            }
            tokio::time::sleep_until((now + POLL).min(deadline)).await;
        }
    }

    /// The terminal is gone: its agents' projectors go too.
    pub fn forget(&self, terminal: Uuid) {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).retain(|(t, _), _| *t != terminal);
    }

    #[cfg(test)]
    fn held(&self) -> usize {
        self.open.lock().unwrap().len()
    }
}

/// The daemon's subagent projectors.
pub fn global() -> &'static SubagentProjectors {
    static PROJECTORS: OnceLock<SubagentProjectors> = OnceLock::new();
    PROJECTORS.get_or_init(SubagentProjectors::default)
}

#[cfg(test)]
#[path = "subagent_rows_tests.rs"]
mod tests;
