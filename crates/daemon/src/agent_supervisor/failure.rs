//! What the supervisor holds about failed turns (ov-140). Its own file to keep
//! `agent_supervisor.rs` inside its size budget.

use super::*;

impl AgentSupervisor {
    /// Whether this pane's last finished turn failed.
    ///
    /// What `watch` puts in `Terminal.turn_failed` for a chat pane, the same
    /// field a terminal pane's session log fills.
    pub fn turn_failed(&self, terminal: Uuid) -> bool {
        self.sessions.lock().ok().and_then(|s| s.get(&terminal).map(|st| st.turn_failed)).unwrap_or(false)
    }

    /// How many of this pane's turns have ended `Failed`. See `failed_turns`.
    pub fn failed_turns(&self, terminal: Uuid) -> u64 {
        self.sessions.lock().ok().and_then(|s| s.get(&terminal).map(|st| st.failed_turns)).unwrap_or(0)
    }
}
