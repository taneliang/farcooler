//! What a subagent's own transcript says beyond its tool count: the agents
//! it launches, and the calls a permission of its can hold (ov-366).
//!
//! A subagent can launch subagents. Claude writes the nested one's meta with
//! `parentAgentId` and `spawnDepth`, and its `toolUseId` names the `Agent`
//! call in the PARENT's transcript, not the main one (78 such metas in the
//! real corpus, read Oct 6). So that call, its launch result and the
//! `<task-notification>` that ends it are all in the parent's file, which the
//! fold used only to count tools: the nested agent's row never existed, its
//! meta never joined, and its transcript stayed an orphan for good.
//!
//! Here the parent's file is read for the same records the main transcript
//! is read for: an `Agent` call becomes a `Subagent` row in the turn its
//! parent belongs to, its result launches or ends it, and a notification
//! (as a prompt, a queued attachment or an enqueue) ends it.

use super::fold::{summarize, Projection};
use super::record::{Content, Record};

impl Projection {
    /// One record of subagent `agent`'s transcript, for what it launches,
    /// what its calls ask, and what ends the agents it launched.
    pub(super) fn subagent_record(&mut self, agent: &str, record: &Record<'_>, at: Option<i64>) {
        match record.kind.get() {
            Some("assistant") => self.subagent_calls(agent, record, at),
            Some("user") => self.subagent_results(agent, record, at),
            Some("attachment") => {
                let Some(attachment) = record.attachment.0.as_ref() else { return };
                if attachment.kind.get() == Some("queued_command") {
                    if let Some(body) = attachment.prompt.get().unwrap_or_default().trim_start().strip_prefix("<task-notification>") {
                        self.task_notification(body, at);
                    }
                }
            }
            Some("queue-operation") if record.operation.get() == Some("enqueue") => {
                if let Some(body) = record.content.get().unwrap_or_default().trim_start().strip_prefix("<task-notification>") {
                    self.task_notification(body, at);
                }
            }
            _ => {}
        }
    }

    /// The turn a row of `agent`'s belongs in: its own row's, else the
    /// transcript's.
    fn subagent_turn(&self, agent: &str) -> Option<usize> {
        let own = self.agents.get(agent).and_then(|&i| self.rows[i].turn.as_ref()).and_then(|t| self.index.get(t).copied());
        own.or(self.turn)
    }

    fn subagent_calls(&mut self, agent: &str, record: &Record<'_>, at: Option<i64>) {
        let Some(Content::Blocks(blocks)) = record.message.0.as_ref().map(|m| &m.content) else { return };
        for block in blocks.iter().filter_map(|b| b.0.as_ref()) {
            if block.kind.get() != Some("tool_use") {
                continue;
            }
            let Some(call) = block.id.get() else { continue };
            let name = block.name.get().unwrap_or_default();
            if matches!(name, "Agent" | "Task") {
                if let Some(turn) = self.subagent_turn(agent) {
                    self.tool_use(turn, block, at, false);
                }
                continue;
            }
            let summary = block.input.0.as_ref().map(summarize).unwrap_or_default();
            self.sub_tools.entry(agent.to_string()).or_default().push((call.to_string(), name.to_string(), summary.clone()));
            self.link_tool(Some(agent), call, name, &summary, true);
        }
    }

    fn subagent_results(&mut self, agent: &str, record: &Record<'_>, at: Option<i64>) {
        let Some(content) = record.message.0.as_ref().map(|m| &m.content) else { return };
        let blocks = match content {
            Content::Blocks(blocks) => blocks,
            Content::Text(text) => {
                if let Some(body) = text.trim_start().strip_prefix("<task-notification>") {
                    self.task_notification(body, at);
                }
                return;
            }
            Content::None => return,
        };
        for block in blocks.iter().filter_map(|b| b.0.as_ref()) {
            match block.kind.get() {
                Some("tool_result") => {
                    let Some(call) = block.tool_use_id.get() else { continue };
                    if let Some(&i) = self.index.get(&format!("sub:{call}")) {
                        self.subagent_result(i, block.is_error.yes(), record.tool_use_result.0.as_ref(), at);
                        continue;
                    }
                    if let Some(open) = self.sub_tools.get_mut(agent) {
                        open.retain(|(id, ..)| id != call);
                    }
                    self.tool_done(call, at);
                }
                Some("text") => {
                    if let Some(body) = block.text.get().unwrap_or_default().trim_start().strip_prefix("<task-notification>") {
                        self.task_notification(body, at);
                    }
                }
                _ => {}
            }
        }
    }
}
