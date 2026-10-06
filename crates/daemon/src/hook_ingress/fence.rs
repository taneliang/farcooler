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
    if let Some(fence) = session.and_then(|session| asks.mark_call(session, call, agent))
        && fence.try_lock().is_err()
    {
        write_reply(write, &Reply::hold(FENCE_HOLD)).await?;
        let _ = tokio::time::timeout(FENCE_HOLD, fence.lock()).await;
    }
    write_reply(write, &Reply::verdict(None)).await
}

/// A call's end clears that call; a subagent's end, its calls; a turn's
/// beginning or end (a failed one's too), the main thread's.
pub(super) fn ended(asks: &HookAsks, session: &str, event: &str, payload: &serde_json::Value) {
    match event {
        "PostToolUse" | "PostToolUseFailure" => asks.tool_ended(session, payload["tool_use_id"].as_str()),
        "SubagentStop" => {
            if let Some(agent) = payload["agent_id"].as_str() {
                asks.subagent_ended(session, agent);
            }
        }
        "Stop" | "StopFailure" | "UserPromptSubmit" => asks.turn_bounded(session),
        _ => {}
    }
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
