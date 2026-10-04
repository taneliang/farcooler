//! Showing a failed chat turn on its row (ov-140), moved out of `watch.rs`
//! to keep it under its size ceiling.

use super::AgentActivity;

/// A chat pane's observation, when a turn has failed that its row has not yet
/// shown.
///
/// The watcher sees a chat pane only at its ticks, and needs `CONFIRMATIONS`
/// of them to believe a Working. A refused key ends the turn in milliseconds,
/// with nothing before it, so the row went Idle to Idle and never reached
/// Done — never in front of anybody, failure or not. So a failure the row has
/// not shown is observed as `Done` itself, which `advance(Idle, Done)` takes,
/// until the row is on that `Done`; `failures_shown` then stops it, so a row
/// somebody has looked at is not lit up again. Not while the agent is busy:
/// a new turn already under way is the newer news.
pub(super) fn agent_failure_observation(
    observed: AgentActivity,
    failed_turns: u64,
    failures_shown: u64,
) -> AgentActivity {
    if failed_turns > failures_shown && matches!(observed, AgentActivity::Idle | AgentActivity::Done) {
        AgentActivity::Done
    } else {
        observed
    }
}
