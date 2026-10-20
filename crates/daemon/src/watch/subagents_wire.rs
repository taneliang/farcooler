//! A pane's running subagents, as the record says them (ov-619).
//!
//! Three fields go together and are set together, from one `Signals`: the
//! names (`subagents`), the `tool_use` id behind each name (`subagent_ids`,
//! in the same order) and how many run (`subagents_running`). The count is
//! not the length of the names. The names are the spawns this turn has seen,
//! and `forget_the_turn` clears them at every turn boundary; the agents a
//! turn started in the background run on past it, and only claude's own count
//! of them survives. A client that counted names would see an orchestrator's
//! background agents vanish the moment its turn ended, while they still ran.

use farcooler_protocol::v1::Terminal;

use super::Signals;

impl Signals {
    /// The `tool_use` ids behind `subagents()`, one each, in its order.
    fn subagent_ids(&self) -> Vec<String> {
        self.spawns
            .iter()
            .filter(|spawn| spawn.running && !spawn.description.is_empty())
            .map(|spawn| spawn.id.clone())
            .collect()
    }

    /// Set the names, their ids and the count on `message`, all three from
    /// this one moment of the fold.
    pub(super) fn stamp_subagents(&self, message: &mut Terminal) {
        message.subagents = self.subagents();
        message.subagent_ids = self.subagent_ids();
        message.subagents_running = Some(self.running() as u32);
    }
}

#[cfg(test)]
#[path = "subagents_wire_tests.rs"]
mod tests;
