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

/// Whether the engine holds this card as a tracker-only ticket at rest: no run, only a column. What a
/// card's own notes (a wait) can change without touching the run's say over its column (ov-962).
pub(crate) fn is_ticket(conn: &Connection, task: uuid::Uuid) -> rusqlite::Result<bool> {
    let held: Option<(String, String)> = conn.query_row("SELECT workspace, item FROM workflow_owned_items WHERE task_id=?1",
        [task.as_bytes().as_slice()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((workspace, item)) = held else { return Ok(false) };
    let raw: Option<String> = conn.query_row("SELECT state FROM workflow_states WHERE workspace=?1", [&workspace], |r| r.get(0)).optional()?;
    Ok(raw.and_then(|s| serde_json::from_str::<State>(&s).ok())
        .is_some_and(|state| state.items.get(&item).is_some_and(|i| i.definition.is_ticket() && i.phase == Phase::WaitingApproval)))
}

/// Open cards no run owns: (workspace, key, column the card is in). The engine tracks these (ov-951).
/// A rep's desk card is a person's thread, not work, so it is never one (ov-962). Nor is a card the engine
/// would refuse (a live worker or open landing, or what a migrated card keeps): it stays out, and asking the
/// engine each start would write one permanent refusal receipt per card per start.
pub fn untracked(conn: &Connection, only: Option<uuid::Uuid>) -> rusqlite::Result<Vec<(uuid::Uuid, String, TaskStatus)>> {
    let mut statement = conn.prepare("SELECT t.workspace_id, t.key, t.status FROM tasks t
        WHERE t.status NOT IN ('done','cancelled') AND (?1 IS NULL OR t.id=?1)
        AND NOT EXISTS(SELECT 1 FROM workflow_owned_items o WHERE o.task_id=t.id)
        AND NOT EXISTS(SELECT 1 FROM rep_desks d WHERE d.task_id=t.id)
        AND NOT EXISTS(SELECT 1 FROM ticket_work w WHERE w.task_id=t.id AND w.ended_at IS NULL)
        AND NOT EXISTS(SELECT 1 FROM terminals x WHERE x.task_id=t.id AND x.exit_code IS NULL AND x.exit_signal IS NULL)
        AND NOT EXISTS(SELECT 1 FROM landing_tickets lt JOIN landings l ON l.id=lt.landing_id WHERE lt.task_id=t.id AND l.state NOT IN ('landed','dropped'))
        AND COALESCE((SELECT c.disposition FROM engine_cutover_items c WHERE c.task=t.id),'retained-intent')='retained-intent'
        AND (NOT EXISTS(SELECT 1 FROM engine_cutover_items c WHERE c.task=t.id) OR NOT (
            EXISTS(SELECT 1 FROM ticket_lines WHERE task_id=t.id AND stage NOT IN ('landed','dropped'))
            OR EXISTS(SELECT 1 FROM task_blocks WHERE task_id=t.id)
            OR t.wait_kind IS NOT NULL
            OR EXISTS(SELECT 1 FROM check_in_schedules WHERE task_id=t.id AND state='enabled')))
        ORDER BY t.created_at, t.rowid")?;
    let rows = statement.query_map([only.map(|u| u.as_bytes().to_vec())], |r| {
        let workspace: Vec<u8> = r.get(0)?; let status: String = r.get(2)?;
        Ok((uuid::Uuid::from_slice(&workspace).ok(), r.get::<_, String>(1)?, TaskStatus::parse(&status)))
    })?;
    Ok(rows.filter_map(|row| row.ok()).filter_map(|(w, k, s)| Some((w?, k, s?))).collect())
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
/// The `ticket_work` pair is here because closing a card closes its open subagents inside the move (ov-962).
const GUARDS: [&str; 5] = ["engine_task_execution_fence", "workflow_excludes_ticket_lines_insert", "workflow_excludes_ticket_lines_update",
    "workflow_excludes_ticket_work_insert", "workflow_excludes_ticket_work_update"];

/// What a tracked ticket's own notes touch: its wait (a column the status fence also covers) and its blocks.
pub(crate) const TICKET_GUARDS: [&str; 4] = ["engine_task_execution_fence", "engine_fence_task_blocks_i", "engine_fence_task_blocks_u", "engine_fence_task_blocks_d"];

/// Runs `write` with `names` dropped, and puts them back from their stored text, as `project` does: a ticket the
/// engine holds with no run keeps its say over its own wait and blocks after release, while a card with a run keeps
/// every fence. The catalog is byte-identical afterward, and a failure rolls the drops back with the rest.
pub(crate) fn with_guards_down<T>(tx: &Transaction<'_>, names: &[&str], write: impl FnOnce() -> DomainResult<T>) -> DomainResult<T> {
    let map = crate::error::map_err;
    let mut saved = Vec::new();
    for name in names {
        let sql: Option<String> = tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1", [name], |r| r.get(0)).optional().map_err(map)?;
        saved.extend(sql);
    }
    for name in names { tx.execute_batch(&format!("DROP TRIGGER IF EXISTS {name}")).map_err(map)?; }
    let written = write();
    for sql in &saved { tx.execute_batch(sql).map_err(map)?; }
    written
}

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
        // A ticket with no run holds only its column. Its line, rank and hold are the board's own history, and
        // tracking a card must not rewrite them (ov-962); the run that Start makes projects the line.
        let ticket = item.definition.is_ticket();
        let differs = have.as_deref() != Some(status.as_str()) || (!ticket && line.as_deref() != Some(stage.as_str()));
        if differs { moves.push((task, status, stage, have, line, ticket)); }
    }
    if moves.is_empty() { return Ok(Vec::new()); }
    let moved = moves.iter().filter_map(|(task, to, _, from, _, _)| Some(BoardMove {
        task: uuid::Uuid::from_slice(task).ok()?, from: from.as_deref().and_then(TaskStatus::parse), to: *to })).collect();
    let saved: Vec<String> = {
        let mut statement = tx.prepare("SELECT sql FROM sqlite_master WHERE type='trigger' AND name IN (?1,?2,?3,?4,?5) ORDER BY name")?;
        statement.query_map(params![GUARDS[0], GUARDS[1], GUARDS[2], GUARDS[3], GUARDS[4]], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
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
type Move = (Vec<u8>, TaskStatus, Stage, Option<String>, Option<String>, bool);
fn write(tx: &Transaction<'_>, moves: Vec<Move>) -> Result<()> {
    let now = crate::tasks::now_millis();
    let board = |e: DomainError| WorkflowError::Board(e.to_string());
    for (task, status, stage, have, line, ticket) in moves {
        let id = uuid::Uuid::from_slice(&task).map_err(|e| WorkflowError::Board(e.to_string()))?;
        if let Some(from) = have.as_deref().and_then(TaskStatus::parse).filter(|from| *from != status) {
            crate::tasks::move_in(tx, id, from, status, Actor::Runner, now).map_err(board)?;
        }
        if ticket || line.as_deref() == Some(stage.as_str()) { continue; }
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
