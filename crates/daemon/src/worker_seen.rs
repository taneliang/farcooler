//! What the runner has seen subagents do (ov-213), which the store doesn't
//! keep.
//!
//! The store keeps what only the orchestrator knows: which task, which
//! agent, and when it started and stopped. Whether a subagent is working,
//! what it is doing and when it last moved are read from its transcript, and
//! go stale the moment they are written, so they live here, in memory,
//! and are re-read within a second of a restart. `task_starts::pb_worker`
//! puts the two together on every task the board sends.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// One subagent, as its transcript last left it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeenWorker {
    /// The time on the last line it wrote, in unix milliseconds. The
    /// transcript's, not the runner's clock.
    pub last_activity_at: i64,
    /// What it's doing, as a short phrase: "Running cargo test".
    pub doing: String,
    /// The model its last call says, for a subagent recorded without one.
    pub model: String,
}

#[derive(Default)]
struct Inner {
    /// Subagents whose session file the runner is reading now.
    observed: HashSet<String>,
    seen: HashMap<String, SeenWorker>,
}

/// Every subagent the runner follows, by claude `agentId`.
#[derive(Default)]
pub struct WorkerSeen {
    inner: Mutex<Inner>,
}

impl WorkerSeen {
    /// Say which subagents' sessions are being read, and forget the rest.
    pub fn observing(&self, agents: impl IntoIterator<Item = String>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.observed = agents.into_iter().collect();
        let observed = inner.observed.clone();
        inner.seen.retain(|agent, _| observed.contains(agent));
    }

    /// Whether the runner is reading the session this subagent is in.
    pub fn is_observed(&self, agent: &str) -> bool {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).observed.contains(agent)
    }

    /// Fold in what new lines of its transcript said. Only ever moves the
    /// time forward, and keeps what it was doing when the new lines named
    /// nothing.
    pub fn note(&self, agent: &str, at_ms: Option<i64>, doing: Option<String>, model: Option<String>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let seen = inner.seen.entry(agent.to_string()).or_default();
        if let Some(at) = at_ms {
            seen.last_activity_at = seen.last_activity_at.max(at);
        }
        if let Some(doing) = doing {
            seen.doing = doing;
        }
        if let Some(model) = model {
            seen.model = model;
        }
    }

    pub fn get(&self, agent: &str) -> Option<SeenWorker> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).seen.get(agent).cloned()
    }
}
