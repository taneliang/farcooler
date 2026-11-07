//! ov-948: the engine's phase, shown on the board. A card the engine owns
//! (`workflow_owned_items`) has its column (`tasks.status`) and its line stage
//! (`ticket_lines.stage`) written from the run, inside the same transaction that
//! accepted the command, so a card can never disagree with the state that
//! moved it. Nobody else writes them: a hand move on such a card is refused in
//! plain words (`refuse_if_owned`). Cards outside a run stay hand-moved (R-98).
use crate::models::{Actor, TaskStatus};
use crate::ticket_lines::{LineSubject, Stage};
use crate::workflow::{Result, WorkflowError};
use farcooler_core::{DomainError, Result as DomainResult};
use farcooler_workflow::{Item, Phase, Reviewer, StageKind, State};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// Whether the engine owns this card.
pub(crate) fn is_owned(conn: &Connection, task: uuid::Uuid) -> rusqlite::Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM workflow_owned_items WHERE task_id=?1)",
        [task.as_bytes().as_slice()], |r| r.get(0))
}

/// A hand move (column, hold, line edit) on a card the engine owns: refused
/// in any phase, because the next step of the run would overwrite it.
pub(crate) fn refuse_if_owned(conn: &Connection, task: uuid::Uuid) -> DomainResult<()> {
    if !is_owned(conn, task).map_err(crate::error::map_err)? { return Ok(()); }
    // Ownership is permanent, so no word here may promise the card comes free.
    // A run that ended says so; the column it left is final.
    let status: Option<String> = conn.query_row("SELECT status FROM tasks WHERE id=?1", [task.as_bytes().as_slice()], |r| r.get(0))
        .optional().map_err(crate::error::map_err)?;
    let ended = status.as_deref().and_then(TaskStatus::parse).is_some_and(|s| matches!(s, TaskStatus::Done | TaskStatus::Cancelled));
    Err(DomainError::Conflict { what: if ended { "engine_owned_ended" } else { "engine_owned" } })
}

/// Where a run puts its card: the column and the line stage.
pub(crate) fn place(item: &Item) -> (TaskStatus, Stage) {
    if item.definition.is_ticket() && !matches!(item.phase, Phase::Done | Phase::Cancelled) {
        // A ticket with no run sits where the orchestrator put it; nobody is asked for anything by the engine.
        return match item.definition.stages.get(item.stage).map(|s| s.id.as_str()) {
            Some("todo") => (TaskStatus::Todo, Stage::Waiting),
            Some("doing") => (TaskStatus::InProgress, Stage::Building),
            Some("review") => (TaskStatus::InReview, Stage::Reviewing),
            Some("decision") => (TaskStatus::NeedsDecision, Stage::NeedsYou),
            _ => (TaskStatus::Backlog, Stage::Waiting),
        };
    }
    match item.phase {
        Phase::Done => (TaskStatus::Done, Stage::Landed),
        Phase::Cancelled => (TaskStatus::Cancelled, Stage::Dropped),
        // Nothing has started: it waits its turn or its dependencies.
        Phase::Blocked => (TaskStatus::Todo, Stage::Waiting),
        // Built and waiting to land, or queued into a batch: nobody here has a decision to make.
        Phase::WaitingJoin | Phase::Joined => (TaskStatus::InReview, Stage::Landing),
        // Only a person can move these on, or the controller can cancel a failed run.
        Phase::WaitingApproval | Phase::Uncertain | Phase::Failed => (TaskStatus::NeedsDecision, Stage::NeedsYou),
        Phase::WaitingReview if matches!(kind(item), Some(StageKind::Review(Reviewer::Person(_)))) => (TaskStatus::NeedsDecision, Stage::NeedsYou),
        Phase::WaitingReview => (TaskStatus::InReview, Stage::Reviewing),
        Phase::Running | Phase::Cancelling => match kind(item) {
            Some(StageKind::Review(_)) => (TaskStatus::InReview, Stage::Reviewing),
            Some(StageKind::Land | StageKind::Ci) => (TaskStatus::InReview, Stage::Landing),
            _ => (TaskStatus::InProgress, Stage::Building),
        },
    }
}
fn kind(item: &Item) -> Option<&StageKind> { item.definition.stages.get(item.stage).map(|s| &s.kind) }

/// Triggers that refuse a write to an owned card. The projection is the one
/// writer they must let through, and a trigger cannot tell it from a legacy
/// write, so for the length of this transaction they are dropped and put back
/// from their own stored text: the catalog is byte-identical afterward, and a
/// failure rolls the drops back with everything else.
const GUARDS: [&str; 3] = ["engine_task_execution_fence", "workflow_excludes_ticket_lines_insert", "workflow_excludes_ticket_lines_update"];

/// Project every owned card of `state` onto the board. Only a card whose
/// column or stage differs is touched, so a command that moves no card writes
/// nothing.
///
/// Returns the cards it moved, for the announce after the commit.
pub(crate) fn project(tx: &Transaction<'_>, workspace: &str, state: &State) -> Result<Vec<BoardMove>> {
    let mut moves = Vec::new();
    for (name, item) in &state.items {
        let task: Option<Vec<u8>> = tx.query_row("SELECT task_id FROM workflow_owned_items WHERE workspace=?1 AND item=?2",
            params![workspace, name], |r| r.get(0)).optional()?.flatten();
        let Some(task) = task else { continue };
        let (status, stage) = place(item);
        let have: Option<String> = tx.query_row("SELECT status FROM tasks WHERE id=?1", [&task], |r| r.get(0)).optional()?;
        let line: Option<String> = tx.query_row("SELECT stage FROM ticket_lines WHERE task_id=?1", [&task], |r| r.get(0)).optional()?;
        if have.as_deref() != Some(status.as_str()) || line.as_deref() != Some(stage.as_str()) { moves.push((task, status, stage, have, line)); }
    }
    if moves.is_empty() { return Ok(Vec::new()); }
    let moved = moves.iter().filter_map(|(task, to, _, from, _)| Some(BoardMove {
        task: uuid::Uuid::from_slice(task).ok()?, from: from.as_deref().and_then(TaskStatus::parse), to: *to })).collect();
    let saved: Vec<String> = {
        let mut statement = tx.prepare("SELECT sql FROM sqlite_master WHERE type='trigger' AND name IN (?1,?2,?3) ORDER BY name")?;
        statement.query_map(params![GUARDS[0], GUARDS[1], GUARDS[2]], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    for name in GUARDS { tx.execute_batch(&format!("DROP TRIGGER IF EXISTS {name}"))?; }
    let written = write(tx, moves);
    for sql in &saved { tx.execute_batch(sql)?; }
    written?;
    Ok(moved)
}
/// What the daemon registers to hear of engine moves.
pub type BoardListener = std::sync::Arc<dyn Fn(&[BoardMove]) + Send + Sync>;
/// A card the engine moved: what the board must tell its open clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardMove { pub task: uuid::Uuid, pub from: Option<TaskStatus>, pub to: TaskStatus }
/// A card to move: its id, where the run puts it, and what the board has now.
type Move = (Vec<u8>, TaskStatus, Stage, Option<String>, Option<String>);
fn write(tx: &Transaction<'_>, moves: Vec<Move>) -> Result<()> {
    let now = crate::tasks::now_millis();
    let board = |e: DomainError| WorkflowError::Board(e.to_string());
    for (task, status, stage, have, line) in moves {
        let id = uuid::Uuid::from_slice(&task).map_err(|e| WorkflowError::Board(e.to_string()))?;
        if let Some(from) = have.as_deref().and_then(TaskStatus::parse).filter(|from| *from != status) {
            crate::tasks::move_in(tx, id, from, status, Actor::Runner, now).map_err(board)?;
        }
        if line.as_deref() == Some(stage.as_str()) { continue; }
        if line.is_none() {
            tx.execute("INSERT INTO ticket_lines (task_id, stage, stage_since, created_at, resource_version) VALUES (?1, ?2, ?3, ?3, 1)",
                params![task, stage.as_str(), now])?;
        } else {
            tx.execute("UPDATE ticket_lines SET stage = ?2, stage_since = ?3, plan_rank = NULL,
                    held_off = 0, resource_version = resource_version + 1 WHERE task_id = ?1", params![task, stage.as_str(), now])?;
        }
        let said = stage.said();
        let extra = serde_json::json!({ "from": line, "to": stage.as_str() });
        crate::ticket_lines::line_event(tx, LineSubject::Ticket(id), "stage", Actor::Runner, said, extra).map_err(board)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
