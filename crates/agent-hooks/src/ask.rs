//! A gating hook's line, as the ask a phone can answer.
//!
//! Claude runs its `PermissionRequest` hook while it draws its own dialog, so
//! the same question is on the pane's screen and, through this, on the owner's
//! lock screen and watch. The event is the one ACP's `can_use_tool` becomes,
//! built by the same builder, so the surfaces that answer one answer both.

use farcooler_agent_core::event::AgentEvent;
use farcooler_agent_core::permission::permission_options;

use crate::Agent;
use crate::wire::GATES;

/// Whether `event` from `agent` is an ask: a hook the agent waits on.
pub fn is_gate(agent: Agent, event: &str) -> bool {
    GATES.iter().any(|&(a, e)| a == agent && e == event)
}

/// A claude `PermissionRequest` payload, as the ask held under `id`.
///
/// `tool_call` is empty because the hook payload carries no `tool_use_id`, and
/// empty is already what an agent that names no tool call sends. The payload
/// says `tool_input` where ACP says `input`.
pub fn permission_ask(id: &str, payload: &serde_json::Value) -> AgentEvent {
    let name = payload["tool_name"].as_str().unwrap_or("this tool");
    AgentEvent::Permission {
        id: id.to_string(),
        tool_call: String::new(),
        options: permission_options(name, &payload["tool_input"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent_core::event::PermissionOption;

    fn fixture(name: &str) -> serde_json::Value {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(&path).expect(&path)).expect("valid json")
    }

    #[test]
    fn a_claude_permission_request_becomes_a_permission_with_no_tool_call() {
        let ask = permission_ask("hook-ask-1", &fixture("claude-permission-request"));
        let AgentEvent::Permission { id, tool_call, options } = ask else {
            panic!("an ask is a Permission, got {ask:?}");
        };
        assert_eq!(id, "hook-ask-1");
        assert_eq!(tool_call, "", "the hook payload has no tool_use_id to give");
        let names: Vec<&str> = options.iter().map(|o: &PermissionOption| o.name.as_str()).collect();
        assert_eq!(names, ["Allow /tmp/probe/probe-test.txt", "Deny"]);
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["allow", "deny"], "the ids a phone echoes back, the same as ACP's");
    }

    #[test]
    fn only_claudes_permission_request_is_a_gate() {
        assert!(is_gate(Agent::Claude, "PermissionRequest"));
        assert!(!is_gate(Agent::Codex, "PermissionRequest"), "codex registers no gate");
        assert!(!is_gate(Agent::Cursor, "beforeShellExecution"), "cursor registers no gate");
        assert!(!is_gate(Agent::Claude, "Stop"));
    }
}
