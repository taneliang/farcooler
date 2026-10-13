//! What a subagent's parent called it (ov-509).
//!
//! The one-line summary an orchestrator gives a subagent ("Review ov-113
//! phones slices") is on the parent's `Agent` tool call and in the sibling
//! `agent-<id>.meta.json`, and the subagent's own transcript carries none.
//! A `task_workers` record keeps it as `label`, but only for an agent somebody
//! recorded on a card; 27 of 36 subagents in a day were recorded on nothing,
//! so the Plan listed them with no name at all.
//!
//! This keeps it by agent id, whatever else is or isn't recorded. One row per
//! agent, the latest description winning, in a table of its own so nothing a
//! build from before it reads has changed. The plan's agents list falls back
//! to it when no worker record names the agent (`plan_agents`).

use rusqlite::{OptionalExtension, params};

use crate::error::map_err;
use crate::store::Store;
use crate::tasks::now_millis;
use farcooler_core::Result;

/// Longest description kept, in characters. Parents write three to five
/// words; this only bounds a pathological one.
const MAX_CHARS: usize = 200;

/// One new table, so a build from before it reads every table it knows
/// exactly as it did. One that runs meanwhile never names it.
pub(crate) fn migration_0049_subagent_descriptions(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE subagent_descriptions (
             agent_id TEXT PRIMARY KEY NOT NULL,
             description TEXT NOT NULL,
             recorded_at INTEGER NOT NULL
         );",
    )
}

impl Store {
    /// Keep `description` as what subagent `agent_id` (Claude's id) is called.
    /// Blank text is not a name and changes nothing.
    pub fn record_subagent_description(&self, agent_id: &str, description: &str) -> Result<()> {
        let description: String = description.split_whitespace().collect::<Vec<_>>().join(" ");
        let description: String = description.chars().take(MAX_CHARS).collect();
        if agent_id.is_empty() || description.is_empty() {
            return Ok(());
        }
        self.conn()
            .execute(
                "INSERT INTO subagent_descriptions (agent_id, description, recorded_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(agent_id) DO UPDATE SET description = excluded.description,
                                                     recorded_at = excluded.recorded_at",
                params![agent_id, description, now_millis()],
            )
            .map_err(map_err)?;
        Ok(())
    }

    /// What subagent `agent_id` was called, when anything kept it.
    pub fn subagent_description(&self, agent_id: &str) -> Result<Option<String>> {
        self.conn()
            .query_row(
                "SELECT description FROM subagent_descriptions WHERE agent_id = ?1",
                params![agent_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)
    }
}

#[cfg(test)]
#[path = "subagent_notes_tests.rs"]
mod tests;
