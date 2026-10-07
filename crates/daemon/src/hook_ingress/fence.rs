//! Answering claude's `PreToolUse`, the fence (ov-360, `hook_asks`).
//!
//! The call is marked in flight at once. The answer, "no decision", is
//! written only once the session's fence is free: claude draws no permission
//! dialog before it, so none can be drawn while a mid-turn Enter holds the
//! fence. A hook kept waiting is told so first, with a hold of `FENCE_HOLD`,
//! which `farcooler hook` honors past its usual 400 ms; the Enter's own
//! deadline lets go well before that (`answer_wake::mid_turn::LONGEST_FENCE`).

use farcooler_agent_hooks::Agent;
use farcooler_agent_hooks::ask::is_gate;
use farcooler_agent_hooks::wire::Reply;
use tokio::net::unix::OwnedWriteHalf;

use super::write_reply;
use crate::hook_asks::{FENCE_HOLD, HookAsks};

/// Mark `session`'s call in flight, wait out any Enter holding its fence,
/// and answer the hook.
pub(super) async fn answer(
    asks: &HookAsks,
    session: Option<&str>,
    payload: &serde_json::Value,
    write: &mut OwnedWriteHalf,
) -> std::io::Result<()> {
    let call = payload["tool_use_id"].as_str();
    let agent = payload["agent_id"].as_str();
    if let (Some(session), Some(turn)) = (session, payload["prompt_id"].as_str()) {
        asks.saw_turn(session, turn);
    }
    if let Some(fence) = session.and_then(|session| asks.mark_call(session, call, agent))
        && fence.try_lock().is_err()
    {
        write_reply(write, &Reply::hold(FENCE_HOLD)).await?;
        let _ = tokio::time::timeout(FENCE_HOLD, fence.lock()).await;
    }
    write_reply(write, &Reply::verdict(None)).await
}

/// A call's end clears that call; a subagent's end, its calls; a turn's
/// beginning or end (a failed one's too), the main thread's. A `Stop` or
/// `StopFailure` that names an `agent_id` is a subagent's end, not the
/// turn's. A turn's beginning is kept with its prompt, which is what confirms
/// a prompt typed in went in (`answer_wake::compose`).
///
/// A `UserPromptSubmit` for a turn a hook already named is a message added to
/// claude's queue while that turn runs (`HookAsks::prompted`), not a turn's
/// beginning: the turn's calls stay in flight. Returns whether this hook
/// bounded the turn. Held asks are withdrawn on every main-thread
/// `UserPromptSubmit` regardless (`bounds_turn`): a queued message was typed
/// into the box, so no dialog was up.
pub(super) fn ended(asks: &HookAsks, session: &str, event: &str, payload: &serde_json::Value) -> bool {
    let turn = payload["prompt_id"].as_str();
    let queued = match (event, payload["agent_id"].as_str(), payload["prompt"].as_str()) {
        ("UserPromptSubmit", None, Some(prompt)) => asks.prompted(session, prompt, turn),
        _ => {
            if let Some(turn) = turn {
                asks.saw_turn(session, turn);
            }
            false
        }
    };
    match (event, payload["agent_id"].as_str()) {
        ("PostToolUse" | "PostToolUseFailure", _) => asks.tool_ended(session, payload["tool_use_id"].as_str()),
        ("SubagentStop" | "Stop" | "StopFailure", Some(agent)) => asks.subagent_ended(session, agent),
        ("UserPromptSubmit", None) if queued => return false,
        ("Stop" | "StopFailure" | "UserPromptSubmit", None) => asks.turn_bounded(session),
        _ => {}
    }
    true
}

/// Whether this hook is the main thread's turn beginning or ending: claude's
/// `UserPromptSubmit`, `Stop` or `StopFailure`, and not a subagent's, which
/// carries its `agent_id`.
pub(super) fn bounds_turn(agent: Agent, event: &str, payload: &serde_json::Value) -> bool {
    agent == Agent::Claude
        && matches!(event, "Stop" | "StopFailure" | "UserPromptSubmit")
        && payload.get("agent_id").is_none()
}

/// Whether this hook says claude is putting up a dialog: a gate
/// (`wire::GATES`), or a `Notification` for a permission or an MCP
/// elicitation. Either keeps a mid-turn Enter off the session for a while
/// (`HookAsks::heard`).
///
/// The notice is late: on 2.1.290 the permission one came 6.0 s after the
/// dialog was drawn (twice, against a stand-in API), so it adds nothing for
/// a dialog's first seconds, which the fence and `PermissionRequest` cover.
/// It covers a dialog that's still up after that, as a second signal.
pub(super) fn raises_dialog(agent: Agent, event: &str, payload: &serde_json::Value) -> bool {
    is_gate(agent, event)
        || (agent == Agent::Claude
            && event == "Notification"
            && matches!(payload["notification_type"].as_str(), Some("permission_prompt" | "elicitation_dialog")))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// A main-thread `UserPromptSubmit` keeps its prompt for the session; a
    /// subagent's, or another session's, doesn't.
    #[test]
    fn a_turns_prompt_is_kept_for_its_session() {
        let asks = HookAsks::new(Default::default());
        asks.heard("s1", false);
        let since = Instant::now();
        let prompt = serde_json::json!({ "prompt": "fix\nthe  tests", "prompt_id": "p1" });
        ended(&asks, "s1", "UserPromptSubmit", &serde_json::json!({ "prompt": "not this", "agent_id": "a1" }));
        assert_eq!(asks.prompted_since("s1", since, "not this"), None, "a subagent's");
        assert!(ended(&asks, "s1", "UserPromptSubmit", &prompt), "a turn's beginning");
        assert_eq!(asks.prompted_since("s1", since, "fix the tests"), Some(false), "whitespace aside");
        assert_eq!(asks.prompted_since("s1", since, "fix the test"), None);
        assert_eq!(asks.prompted_since("s2", since, "fix the tests"), None, "another session");
        let later = Instant::now() + std::time::Duration::from_secs(1);
        assert_eq!(asks.prompted_since("s1", later, "fix the tests"), None, "before since");
        ended(&asks, "s1", "Stop", &serde_json::json!({ "prompt_id": "p1" }));
        assert_eq!(asks.prompted_since("s1", since, "fix the tests"), Some(false), "kept past the turn's end");
    }

    /// A message submitted while a turn runs fires `UserPromptSubmit` with
    /// that turn's `prompt_id` (claude 2.1.290): it's queued, and the turn's
    /// call stays in flight. One for a turn no hook named begins a turn.
    #[test]
    fn a_prompt_for_a_running_turn_is_queued_and_bounds_nothing() {
        let asks = HookAsks::new(Default::default());
        asks.heard("s1", false);
        let since = Instant::now();
        ended(&asks, "s1", "UserPromptSubmit", &serde_json::json!({ "prompt": "go", "prompt_id": "p1" }));
        asks.mark_call("s1", Some("t1"), None);
        asks.saw_turn("s1", "p1");
        let queued = serde_json::json!({ "prompt": "and then this", "prompt_id": "p1" });
        assert!(!ended(&asks, "s1", "UserPromptSubmit", &queued), "no turn's beginning");
        assert_eq!(asks.prompted_since("s1", since, "and then this"), Some(true));
        assert!(asks.tool_in_flight("s1"), "the turn's call is still in flight");
        // A turn claude ran from its queue, named first by a tool's hook.
        ended(&asks, "s1", "PostToolUse", &serde_json::json!({ "tool_use_id": "t1", "prompt_id": "p2" }));
        let into_p2 = serde_json::json!({ "prompt": "more", "prompt_id": "p2" });
        assert!(!ended(&asks, "s1", "UserPromptSubmit", &into_p2));
        assert_eq!(asks.prompted_since("s1", since, "more"), Some(true));
        let fresh = serde_json::json!({ "prompt": "new turn", "prompt_id": "p3" });
        assert!(ended(&asks, "s1", "UserPromptSubmit", &fresh));
        assert_eq!(asks.prompted_since("s1", since, "new turn"), Some(false));
    }
}
