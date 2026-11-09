//! Whether a workflow still names a member's handle as a decider (ov-1002 review F1).
//!
//! A gate names `member:<handle>`, and admission gives that name to whoever holds the handle now. A handle a
//! removed member had could be issued to someone new, who would then decide what waits on the one removed. So a
//! handle isn't issued again while a workflow in force, or a run not yet closed, names it. Once the owner
//! publishes a workflow that doesn't and those runs end, it's free.
use rusqlite::Connection;

use farcooler_core::Result;
use farcooler_workflow::{Definition, Phase, State};

use crate::error::map_err;

/// Whether a workspace's selected workflow, or a run of any workspace that isn't done or cancelled, names
/// `member:<handle>` at any step. Read on `conn`, so a caller's transaction sees one moment. A record that
/// won't read counts as naming it: unread, it can't be said not to.
pub(crate) fn named_by_a_workflow(conn: &Connection, handle: &str) -> Result<bool> {
    let name = format!("member:{handle}");
    let names = |definition: &Definition| definition.deciders().contains(&name.as_str());
    let mut stmt = conn
        .prepare(
            "SELECT d.definition FROM workflow_bundle_selections s JOIN workflow_bundles b ON b.pin = s.pin
             JOIN workflow_definitions d ON d.pin = b.definition_pin",
        )
        .map_err(map_err)?;
    let definitions = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(map_err)?;
    for raw in definitions {
        let Ok(definition) = serde_json::from_str::<Definition>(&raw.map_err(map_err)?) else { return Ok(true) };
        if names(&definition) {
            return Ok(true);
        }
    }
    let mut stmt = conn.prepare("SELECT state FROM workflow_states").map_err(map_err)?;
    let states = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(map_err)?;
    for raw in states {
        let Ok(state) = serde_json::from_str::<State>(&raw.map_err(map_err)?) else { return Ok(true) };
        if state.items.values().any(|i| !matches!(i.phase, Phase::Done | Phase::Cancelled) && names(&i.definition)) {
            return Ok(true);
        }
    }
    Ok(false)
}
