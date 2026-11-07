//! Registry 68 (ov-965): the engine owns only a ticket's stage. A card the engine holds as a ticket with no run
//! (a tracker-only definition at rest) is fenced on its column alone: its line, plan rank and hold, its wait, its
//! blocks and its subagent records stay the orchestrator's, at every cutover phase. A card with a run, and a
//! closed ticket, keep every fence they had.
//!
//! The guards can't read the engine's state, so `engine_stage_tickets` names the cards at rest. `workflow_apply`
//! rewrites it from the state it commits, in the same transaction, and nothing else writes it. A card missing
//! from it is fenced, so a sync that never ran fails closed.
use farcooler_workflow::{Phase, State};
use rusqlite::{Transaction, params};

/// The `workflow_excludes_*` guards whose rule changes. They are outside the engine's namespace, so the
/// cutover's conservation check names them as the one delta a source below 68 takes (`verify`).
pub(crate) const EXCLUDES: [(&str, &str, &str); 4] =
    [("ticket_lines", "insert", "INSERT"), ("ticket_lines", "update", "UPDATE"), ("ticket_work", "insert", "INSERT"), ("ticket_work", "update", "UPDATE")];

/// A `workflow_excludes_*` guard as `sqlite_master` keeps it: before 68 (`at_rest` false, the text
/// `workflow_ownership` writes) and from 68.
pub(crate) fn excludes(table: &str, suffix: &str, event: &str, at_rest: bool) -> String {
    let rest = if at_rest { "\n                AND NOT EXISTS(SELECT 1 FROM engine_stage_tickets WHERE task_id=NEW.task_id)" } else { "" };
    format!("CREATE TRIGGER workflow_excludes_{table}_{suffix} BEFORE {event} ON {table}
                WHEN NEW.task_id IS NOT NULL AND EXISTS(SELECT 1 FROM workflow_owned_items WHERE task_id=NEW.task_id){rest}
                BEGIN SELECT RAISE(ABORT,'task owned by workflow engine'); END")
}

/// The tracker-only tickets at rest in `state`: what `engine_stage_tickets` holds for its workspace.
fn at_rest(state: &State) -> impl Iterator<Item = &str> {
    state.items.iter().filter(|(_, i)| i.definition.is_ticket() && i.phase == Phase::WaitingApproval).map(|(name, _)| name.as_str())
}

/// Rewrite `workspace`'s cards at rest from the state just committed for it.
pub(crate) fn sync(tx: &Transaction<'_>, workspace: &str, state: &State) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM engine_stage_tickets WHERE task_id IN (SELECT task_id FROM workflow_owned_items WHERE workspace=?1)", [workspace])?;
    for item in at_rest(state) {
        tx.execute("INSERT INTO engine_stage_tickets(task_id) SELECT task_id FROM workflow_owned_items
            WHERE workspace=?1 AND item=?2 AND task_id IS NOT NULL", params![workspace, item])?;
    }
    Ok(())
}

pub(crate) fn migration(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE engine_stage_tickets(task_id BLOB PRIMARY KEY REFERENCES tasks(id));")?;
    // A released install already holds tickets: carry them forward from the state each workspace committed.
    let states: Vec<(String, String)> = tx.prepare("SELECT workspace, state FROM workflow_states ORDER BY workspace")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (workspace, raw) in states {
        // A state this build can't read holds no ticket at rest: its cards stay fenced.
        if let Ok(state) = serde_json::from_str::<State>(&raw) { sync(tx, &workspace, &state)?; }
    }
    for (table, suffix, event) in EXCLUDES {
        tx.execute_batch(&format!("DROP TRIGGER workflow_excludes_{table}_{suffix}; {};", excludes(table, suffix, event, true)))?;
    }
    let phase = "(SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'";
    let fenced = |task: &str| format!("(EXISTS(SELECT 1 FROM workflow_owned_items WHERE task_id={task})
        AND NOT EXISTS(SELECT 1 FROM engine_stage_tickets WHERE task_id={task}))");
    let (old, new, old_id) = (fenced("OLD.task_id"), fenced("NEW.task_id"), fenced("OLD.id"));
    for table in ["ticket_lines", "ticket_work", "task_blocks"] {
        tx.execute_batch(&format!("DROP TRIGGER engine_fence_{table}_d;
            CREATE TRIGGER engine_fence_{table}_d BEFORE DELETE ON {table}
            WHEN {old} AND {phase}
            BEGIN SELECT RAISE(ABORT,'task owned by workflow engine'); END;"))?;
    }
    // The column is the engine's on every card it holds; the wait is the card's own on a ticket at rest.
    tx.execute_batch(&format!("DROP TRIGGER engine_fence_task_blocks_i; DROP TRIGGER engine_fence_task_blocks_u; DROP TRIGGER engine_task_execution_fence;
        CREATE TRIGGER engine_fence_task_blocks_i BEFORE INSERT ON task_blocks
        WHEN {new} AND {phase}
        BEGIN SELECT RAISE(ABORT,'task owned by workflow engine'); END;
        CREATE TRIGGER engine_fence_task_blocks_u BEFORE UPDATE ON task_blocks
        WHEN ({old} OR {new}) AND {phase}
        BEGIN SELECT RAISE(ABORT,'task owned by workflow engine'); END;
        CREATE TRIGGER engine_task_execution_fence BEFORE UPDATE OF status,status_since ON tasks
        WHEN EXISTS(SELECT 1 FROM workflow_owned_items WHERE task_id=OLD.id) AND {phase}
        BEGIN SELECT RAISE(ABORT,'legacy task execution authority retired'); END;
        CREATE TRIGGER engine_task_wait_fence BEFORE UPDATE OF wait_kind,wait_line,wait_rank,wait_until,wait_event,wait_since ON tasks
        WHEN {old_id} AND {phase}
        BEGIN SELECT RAISE(ABORT,'legacy task execution authority retired'); END;"))?;
    Ok(())
}
