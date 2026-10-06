//! A held permission, tied to the one tool call it holds (ov-366).
//!
//! `PermissionRequest` names no `tool_use_id` (claude 2.1.290's hook input is
//! `tool_name`, `tool_input` and `permission_suggestions` on the common
//! fields, read in the binary). It used to be settled by the first result of
//! any tool with the same name, so a Bash call that needed no permission
//! answered the one waiting on another Bash call, and its row stayed
//! provisional for good.
//!
//! Now it is tied to its call by name and summary (the one input field a
//! person reads first, from the same `summarize` either way), among the calls
//! that have not come back and are not tied already, newest first. The call
//! may come before the hook (claude writes the `tool_use` record, then asks)
//! or after it (a hook beats the file); either order ties them. A tied ask
//! is confirmed when its call's record is, and answered when that call, and
//! only that call, comes back. One that is never tied is settled with its
//! turn, or with its subagent.

use serde_json::Value;

use super::fold::{summarize, Projection};
use super::rows::*;

/// A held permission not yet tied to its tool call.
#[derive(Debug, Clone)]
pub(crate) struct PermWait {
    pub(super) ask: usize,
    /// The subagent whose call it holds; `None` for the main thread.
    pub(super) agent: Option<String>,
    pub(super) tool: String,
    pub(super) summary: String,
}

impl Projection {
    /// `PermissionRequest`: an `Ask` row, tied to its call if the call is in.
    pub(super) fn permission_request(&mut self, turn: usize, payload: &Value, input: &super::record::Input<'_>, now: i64) {
        let tool = payload.get("tool_name").and_then(Value::as_str).unwrap_or("Tool").to_string();
        let agent = payload.get("agent_id").and_then(Value::as_str).map(str::to_string);
        let summary = summarize(input);
        let ask = Ask {
            kind: AskKind::Permission,
            text: if summary.is_empty() { tool.clone() } else { format!("{tool} {summary}") },
            tool: Some(tool.clone()),
            asked_ms: Some(now),
            answered_ms: None,
            answered: false,
        };
        let id = format!("perm:{}", self.next_seq());
        let ask = self.push(id, Some(turn), true, RowKind::Ask(ask));
        match self.open_call(agent.as_deref(), &tool, &summary) {
            Some((call, confirmed)) => self.tie(ask, call, confirmed),
            None => self.perm_waiting.push(PermWait { ask, agent, tool, summary }),
        }
    }

    /// The newest call of `agent` (the main thread for `None`) by this name
    /// and summary that has not come back and holds no ask yet, and whether
    /// its record is in.
    fn open_call(&self, agent: Option<&str>, tool: &str, summary: &str) -> Option<(String, bool)> {
        match agent {
            None => self.rows.iter().rev().find_map(|row| match &row.kind {
                RowKind::Tool(t) if t.status == ToolStatus::Running && t.name == tool && t.summary == summary => {
                    let call = row.id.strip_prefix("tool:")?;
                    (!self.tool_asks.contains_key(call)).then(|| (call.to_string(), !row.provisional))
                }
                _ => None,
            }),
            Some(agent) => self.sub_tools.get(agent)?.iter().rev().find_map(|(call, name, said)| {
                (name == tool && said == summary && !self.tool_asks.contains_key(call)).then(|| (call.clone(), true))
            }),
        }
    }

    fn tie(&mut self, ask: usize, call: String, confirmed: bool) {
        self.tool_asks.insert(call, ask);
        if confirmed && self.rows[ask].provisional {
            self.rows[ask].provisional = false;
            self.touch(ask);
        }
    }

    /// A tool call is in, from a hook or a record: tie the oldest waiting ask
    /// that holds a call like it.
    pub(super) fn link_tool(&mut self, agent: Option<&str>, call: &str, name: &str, summary: &str, confirmed: bool) {
        if self.tool_asks.contains_key(call) {
            return;
        }
        let waiting = self.perm_waiting.iter().position(|w| w.agent.as_deref() == agent && w.tool == name && w.summary == summary);
        if let Some(n) = waiting {
            let wait = self.perm_waiting.remove(n);
            self.tie(wait.ask, call.to_string(), confirmed);
        }
    }

    /// The record of a call a hook announced is in: so is its ask's.
    pub(super) fn tool_confirmed(&mut self, call: &str) {
        if let Some(&ask) = self.tool_asks.get(call) {
            self.tie(ask, call.to_string(), true);
        }
    }

    /// A call came back: the ask that held it, if any, was answered.
    pub(super) fn tool_done(&mut self, call: &str, at: Option<i64>) {
        let Some(ask) = self.tool_asks.remove(call) else { return };
        self.answer(ask, at);
    }

    fn answer(&mut self, ask: usize, at: Option<i64>) {
        if let RowKind::Ask(a) = &mut self.rows[ask].kind {
            if !a.answered {
                a.answered = true;
                a.answered_ms = at;
                self.touch(ask);
            }
        }
    }

    /// A subagent ended: whatever it was asking is over.
    pub(super) fn subagent_asks_over(&mut self, agent: &str, at: Option<i64>) {
        let calls: Vec<String> = self.sub_tools.remove(agent).unwrap_or_default().into_iter().map(|(call, ..)| call).collect();
        for call in calls {
            self.tool_done(&call, at);
        }
        let (over, waiting): (Vec<PermWait>, Vec<PermWait>) =
            std::mem::take(&mut self.perm_waiting).into_iter().partition(|w| w.agent.as_deref() == Some(agent));
        self.perm_waiting = waiting;
        for wait in over {
            self.answer(wait.ask, at);
            self.rows[wait.ask].provisional = false;
            self.touch(wait.ask);
        }
    }
}
