//! Stopping a Claude subagent (ov-872): the hook side of `agent.stop`.
//!
//! No screen has a Stop button. A person asks the orchestrator, which runs
//! `farcooler agent stop`, and the runner records the ask on the agent
//! (`Store::ask_agent_stop`). Far Cooler can't kill a subagent: it runs
//! inside its parent's claude process. What it can do is refuse the
//! subagent's next tool call, since claude waits on the `PreToolUse` fence
//! for every one, and the refusal reaches the subagent verbatim, telling it
//! to stop and hand back. A tool call already running isn't touched, so it
//! finishes first; the agent reads "Stopping" until then.
//!
//! When that subagent's `SubagentStop` arrives, its agent ends as stopped, and
//! the watcher is told so the plan stops showing it.
//!
//! Only Claude's hooks carry `agent_id` and wait on the runner's answer, so
//! another harness's subagent is refused `stop_unsupported` before it gets
//! here (`rpc_agent_stop`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use farcooler_agent_hooks::Agent;
use farcooler_agent_hooks::wire::Decision;
use farcooler_store::agents::Agent as Recorded;
use farcooler_store::ticket_work::Verdict;
use uuid::Uuid;

use super::HookIngress;

/// Told the board of a stopped subagent that has now ended.
pub type OnEnded = Arc<dyn Fn(Uuid) + Send + Sync>;

/// What the ingress keeps for stopping.
#[derive(Default)]
pub(crate) struct Stopping {
    /// Why each agent was asked to stop, when the ask said. Kept in memory:
    /// after a restart the refusal still tells it to stop, without the why.
    reasons: Mutex<HashMap<Uuid, String>>,
    on_ended: Mutex<Option<OnEnded>>,
}

/// The words a stopped subagent's refused tool call carries to it.
pub(crate) fn told(reason: Option<&str>) -> String {
    let why = reason.map(|r| format!(": {r}")).unwrap_or_default();
    format!(
        "Far Cooler: you were asked to stop{why}. Make no more tool calls. \
         Stop now and hand back what you have, saying what's done and what isn't."
    )
}

impl HookIngress {
    /// Keep why `agent` was asked to stop, for its refusal.
    pub fn stop_reason(&self, agent: Uuid, reason: Option<String>) {
        if let Some(reason) = reason {
            self.stopping.reasons.lock().unwrap_or_else(|e| e.into_inner()).insert(agent, reason);
        }
    }

    /// Where to say a stopped subagent has ended. Set once, by the watcher.
    pub fn on_stopped_ended(&self, on_ended: OnEnded) {
        *self.stopping.on_ended.lock().unwrap_or_else(|e| e.into_inner()) = Some(on_ended);
    }

    /// The recorded Claude subagent a hook is from, if it was asked to stop
    /// and hasn't ended.
    fn asked_to_stop(&self, agent: Agent, payload: &serde_json::Value) -> Option<Recorded> {
        let id = payload["agent_id"].as_str().filter(|_| agent == Agent::Claude)?;
        let recorded = self.store.subagent("claude", id).ok().flatten()?;
        (recorded.stop_asked_at.is_some() && recorded.ended_at.is_none()).then_some(recorded)
    }

    /// The answer to a `PreToolUse` from a subagent asked to stop: a refusal
    /// telling it to stop. `None` for every other call, which goes on.
    pub(super) fn stop_verdict(&self, agent: Agent, payload: &serde_json::Value) -> Option<Decision> {
        let recorded = self.asked_to_stop(agent, payload)?;
        let reasons = self.stopping.reasons.lock().unwrap_or_else(|e| e.into_inner());
        Some(Decision::Deny { message: told(reasons.get(&recorded.id).map(String::as_str)) })
    }

    /// A stopped subagent's end (`SubagentStop`, or a `Stop` naming it): its
    /// agent ends as stopped, and the watcher hears of it.
    pub(super) fn stopped_ended(&self, agent: Agent, event: &str, payload: &serde_json::Value) {
        if !matches!(event, "SubagentStop" | "Stop" | "StopFailure") {
            return;
        }
        let Some(recorded) = self.asked_to_stop(agent, payload) else { return };
        if let Err(e) = self.store.end_agent(recorded.id, Verdict::Stopped) {
            tracing::warn!(error = %e, agent = %recorded.id, "couldn't end a subagent that stopped");
            return;
        }
        self.stopping.reasons.lock().unwrap_or_else(|e| e.into_inner()).remove(&recorded.id);
        let on_ended = self.stopping.on_ended.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(on_ended) = on_ended {
            on_ended(recorded.workspace_id);
        }
    }
}
