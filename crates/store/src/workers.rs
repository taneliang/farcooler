//! Subagents as task workers (ov-213), kept as agents and their work on
//! tickets since ov-872.
//!
//! A pane working a task is `terminals.task_id`, as it always was. This is
//! the other kind of worker: a subagent inside another agent's session (an
//! orchestrator's Agent-tool dispatch), which has no pane of its own.
//!
//! What only the orchestrator knows is stored: which task, which agent, when
//! it started, and, once it ended, when and how. The agent is a row of
//! `agents` (`agents.rs`) and its work on the task a row of `ticket_work`
//! (`ticket_work.rs`); a `TaskWorker` is the two read together. What the
//! runner can see for itself (whether it's running now, what it's doing) is
//! observed on read and never stored, so it can't go stale. Rows are history:
//! a task keeps every subagent it ever had, and one resumed for a fix round is
//! the same row, reopened.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::json;
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::agents::AgentRole;
use crate::error::map_err;
use crate::models::{Actor, NoteKind, Task, TaskStatus, get_optional_uuid, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::{insert_note, move_in, now_millis};
use crate::ticket_work::{Verdict, WorkRole};

/// Why a subagent stopped working a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Finished,
    Failed,
    /// Killed or stopped.
    Stopped,
    /// Its work went back to the orchestrator.
    HandedBack,
    /// Its task was done or cancelled while it was open.
    TaskClosed,
    /// The runner linked it from its description, and the orchestrator then
    /// recorded it on another task.
    Relinked,
}

impl EndReason {
    pub fn as_str(self) -> &'static str {
        match self {
            EndReason::Finished => "finished",
            EndReason::Failed => "failed",
            EndReason::Stopped => "stopped",
            EndReason::HandedBack => "handed_back",
            EndReason::TaskClosed => "task_closed",
            EndReason::Relinked => "relinked",
        }
    }

    pub fn parse(raw: &str) -> Option<EndReason> {
        Some(match raw {
            "finished" => EndReason::Finished,
            "failed" => EndReason::Failed,
            "stopped" => EndReason::Stopped,
            "handed_back" => EndReason::HandedBack,
            "task_closed" => EndReason::TaskClosed,
            "relinked" => EndReason::Relinked,
            _ => return None,
        })
    }

    /// The verdict its work on the ticket ends with.
    pub fn verdict(self) -> Verdict {
        match self {
            EndReason::Finished => Verdict::Finished,
            EndReason::Failed => Verdict::Failed,
            EndReason::Stopped => Verdict::Stopped,
            EndReason::HandedBack => Verdict::HandedBack,
            EndReason::TaskClosed => Verdict::TaskClosed,
            EndReason::Relinked => Verdict::Relinked,
        }
    }

    /// How a worker reads a verdict: a review's verdict is a finish.
    pub fn of_verdict(verdict: Verdict) -> EndReason {
        match verdict {
            Verdict::Finished | Verdict::Approved | Verdict::ChangesAsked => EndReason::Finished,
            Verdict::Failed => EndReason::Failed,
            Verdict::Stopped => EndReason::Stopped,
            Verdict::HandedBack => EndReason::HandedBack,
            Verdict::TaskClosed => EndReason::TaskClosed,
            Verdict::Relinked => EndReason::Relinked,
        }
    }

    /// The end of a `worker` note's sentence: "Claude subagent finished."
    fn said(self) -> &'static str {
        match self {
            EndReason::Finished => "finished",
            EndReason::Failed => "failed",
            EndReason::Stopped => "stopped",
            EndReason::HandedBack => "handed its work back",
            EndReason::TaskClosed => "closed with its task",
            EndReason::Relinked => "was recorded on another task",
        }
    }
}

/// Who linked a subagent to its task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedBy {
    /// `task worker`, from the orchestrator's own session.
    Orchestrator,
    /// The runner, from a key at the start of the subagent's description.
    Description,
}

impl LinkedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkedBy::Orchestrator => "orchestrator",
            LinkedBy::Description => "description",
        }
    }
}

/// The harnesses a subagent can run in: words the runner sends and each app
/// names in its own copy.
pub const HARNESSES: [&str; 2] = ["claude", "codex"];

/// What a caller records about a subagent. The `Option`s fill in what is
/// known and leave alone what was recorded before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerRecord {
    /// One of `HARNESSES`.
    pub harness: String,
    /// Claude's `agentId`, or codex's agent path.
    pub agent_id: String,
    /// The orchestrator's session, when known.
    pub session_id: Option<String>,
    pub session_cwd: Option<String>,
    /// The pane the orchestrator runs in, when known.
    pub orchestrator_terminal: Option<Uuid>,
    pub label: Option<String>,
    pub model: Option<String>,
    pub linked_by: LinkedBy,
}

/// One subagent on one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskWorker {
    /// The agent's id (`agents.id`).
    pub id: Uuid,
    pub task_id: Uuid,
    pub harness: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub session_cwd: Option<String>,
    pub orchestrator_terminal: Option<Uuid>,
    pub label: String,
    pub model: String,
    /// Unix milliseconds.
    pub started_at: i64,
    /// Unix milliseconds; `None` while open.
    pub ended_at: Option<i64>,
    pub end_reason: Option<EndReason>,
    pub linked_by: LinkedBy,
    /// What it does for the task.
    pub role: WorkRole,
}

/// A subagent's work on a task: its agent and its `ticket_work` row, read
/// together. `{filter}` goes after `WHERE`.
fn worker_query(filter: &str, order: &str) -> String {
    format!(
        "SELECT a.id, w.task_id, a.harness, a.subagent_id, a.session_id, a.session_cwd, a.parent_terminal_id, a.title,
                a.model, w.started_at, w.ended_at, w.verdict, w.linked_by, w.role
           FROM ticket_work w JOIN agents a ON a.id = w.agent_id
          WHERE a.kind = 'subagent' AND {filter} ORDER BY {order}"
    )
}

fn row_to_worker(row: &Row) -> rusqlite::Result<TaskWorker> {
    let verdict: Option<String> = row.get(11)?;
    let linked: String = row.get(12)?;
    let role: String = row.get(13)?;
    Ok(TaskWorker {
        id: get_uuid(row, 0)?,
        task_id: get_uuid(row, 1)?,
        harness: row.get(2)?,
        agent_id: row.get(3)?,
        session_id: row.get(4)?,
        session_cwd: row.get(5)?,
        orchestrator_terminal: get_optional_uuid(row, 6)?,
        label: row.get(7)?,
        model: row.get(8)?,
        started_at: row.get(9)?,
        ended_at: row.get(10)?,
        // A word from a newer build reads as stopped: it ended, somehow.
        end_reason: verdict.map(|v| Verdict::parse(&v).map_or(EndReason::Stopped, EndReason::of_verdict)),
        linked_by: if linked == "description" { LinkedBy::Description } else { LinkedBy::Orchestrator },
        role: WorkRole::parse(&role).unwrap_or(WorkRole::Build),
    })
}

fn workers_where(conn: &Connection, filter: &str, order: &str, args: impl rusqlite::Params) -> Result<Vec<TaskWorker>> {
    let mut stmt = conn.prepare_cached(&worker_query(filter, order)).map_err(map_err)?;
    let rows = stmt.query_map(args, row_to_worker).map_err(map_err)?;
    rows.collect::<rusqlite::Result<_>>().map_err(map_err)
}

/// "Claude subagent", the start of every `worker` note.
fn who(harness: &str) -> &'static str {
    match harness {
        "codex" => "Codex subagent",
        _ => "Claude subagent",
    }
}

/// The task key a subagent's description starts with, if it starts with
/// one: `ov-12: polish the sidebar`, `ov-12 polish`, or just `ov-12`. A key
/// anywhere else ("after ov-92 lands") is not a link.
pub fn leading_key(description: &str) -> Option<&str> {
    let text = description.trim_start();
    let end = text.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(text.len());
    let (key, rest) = text.split_at(end);
    if !(rest.is_empty() || rest.starts_with(':') || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    let (prefix, number) = key.split_once('-')?;
    let prefix_ok = (1..=8).contains(&prefix.len())
        && prefix.starts_with(|c: char| c.is_ascii_alphabetic())
        && prefix.chars().all(|c| c.is_ascii_alphanumeric());
    let number_ok = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
    (prefix_ok && number_ok).then_some(key)
}

/// The spend a subagent already made, filed to the task it now works.
///
/// Its transcript is read from when the orchestrator's session is first
/// followed, which is before the orchestrator says which task the subagent
/// works, so its `agent_turns` row (`claude-log:agent:<agentId>`, ov-194) is
/// there already, on no task or on the pane's. Rows written afterwards are
/// filed by the daemon the same way (`task_of_subagent`).
fn attach_usage(tx: &Connection, task: Uuid, record: &WorkerRecord) -> Result<()> {
    if record.harness != "claude" {
        return Ok(());
    }
    tx.execute(
        "UPDATE agent_turns SET task_id = ?1 WHERE turn_key = ?2 AND task_id IS NOT ?1",
        params![uuid_blob(task), format!("claude-log:agent:{}", record.agent_id)],
    )
    .map_err(map_err)?;
    Ok(())
}

/// End a subagent whose every piece of work has ended, with the same reason:
/// it was a worker, and has nothing left to work.
fn end_agent_if_idle(tx: &Connection, agent: Uuid, reason: EndReason) -> Result<()> {
    tx.execute(
        "UPDATE agents SET ended_at = ?2, end_reason = ?3
          WHERE id = ?1 AND ended_at IS NULL AND kind = 'subagent'
            AND NOT EXISTS (SELECT 1 FROM ticket_work WHERE agent_id = ?1 AND ended_at IS NULL)",
        params![uuid_blob(agent), now_millis(), reason.verdict().as_str()],
    )
    .map_err(map_err)?;
    Ok(())
}

/// End one open subagent, with its `worker` note on its own task.
fn end_one(tx: &Connection, worker: &TaskWorker, reason: EndReason, actor: Actor) -> Result<()> {
    tx.execute(
        "UPDATE ticket_work SET ended_at = ?3, verdict = ?4 WHERE agent_id = ?1 AND task_id = ?2 AND ended_at IS NULL",
        params![uuid_blob(worker.id), uuid_blob(worker.task_id), now_millis(), reason.verdict().as_str()],
    )
    .map_err(map_err)?;
    end_agent_if_idle(tx, worker.id, reason)?;
    let body = format!("{} {}.", who(&worker.harness), reason.said());
    let extra = json!({
        "event": "ended",
        "harness": worker.harness,
        "agent_id": worker.agent_id,
        "end_reason": reason.as_str(),
    });
    insert_note(tx, worker.task_id, NoteKind::Worker, actor, &body, &extra, None)?;
    Ok(())
}

/// The orchestrator's record wins over the runner's guess: a subagent the
/// runner linked from its description to another task, and still open
/// there, is closed there (`Relinked`) when the orchestrator records it on
/// `task`.
fn unlink_elsewhere(tx: &Connection, task: Uuid, record: &WorkerRecord, actor: Actor) -> Result<()> {
    let guessed = workers_where(
        tx,
        "a.harness = ?1 AND a.subagent_id = ?2 AND w.task_id <> ?3 AND w.ended_at IS NULL AND w.linked_by = 'description'",
        "w.started_at, w.task_id",
        params![record.harness, record.agent_id, uuid_blob(task)],
    )?;
    for worker in &guessed {
        end_one(tx, worker, EndReason::Relinked, actor)?;
    }
    Ok(())
}

/// Close every open subagent on `task` for `reason`, inside a caller's
/// transaction. Answers how many it closed.
pub(crate) fn close_open(tx: &Connection, task: Uuid, reason: EndReason) -> Result<usize> {
    let open = workers_where(tx, "w.task_id = ?1 AND w.ended_at IS NULL", "w.started_at", params![uuid_blob(task)])?;
    for worker in &open {
        tx.execute(
            "UPDATE ticket_work SET ended_at = ?3, verdict = ?4 WHERE agent_id = ?1 AND task_id = ?2 AND ended_at IS NULL",
            params![uuid_blob(worker.id), uuid_blob(task), now_millis(), reason.verdict().as_str()],
        )
        .map_err(map_err)?;
        end_agent_if_idle(tx, worker.id, reason)?;
    }
    Ok(open.len())
}

/// The subagents a board shows for `task`: every open one, oldest first,
/// then the one that closed most recently.
pub(crate) fn shown_for(conn: &Connection, task: Uuid) -> Result<Vec<TaskWorker>> {
    let mut shown = workers_where(conn, "w.task_id = ?1 AND w.ended_at IS NULL", "w.started_at, a.id", params![uuid_blob(task)])?;
    let last_closed = workers_where(
        conn,
        "w.task_id = ?1 AND w.ended_at IS NOT NULL",
        "w.ended_at DESC, a.id DESC LIMIT 1",
        params![uuid_blob(task)],
    )?;
    shown.extend(last_closed);
    Ok(shown)
}

fn find(conn: &Connection, task: Uuid, harness: &str, agent: &str) -> Result<Option<TaskWorker>> {
    Ok(workers_where(
        conn,
        "w.task_id = ?1 AND a.harness = ?2 AND a.subagent_id = ?3",
        "w.ended_at IS NOT NULL, w.started_at DESC LIMIT 1",
        params![uuid_blob(task), harness, agent],
    )?
    .into_iter()
    .next())
}

fn task_status(conn: &Connection, task: Uuid) -> Result<(TaskStatus, Uuid)> {
    let (raw, board): (String, Vec<u8>) = conn
        .query_row("SELECT status, workspace_id FROM tasks WHERE id = ?1", params![uuid_blob(task)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)?;
    let status = TaskStatus::parse(&raw).ok_or(DomainError::NotFound)?;
    Ok((status, Uuid::from_slice(&board).map_err(|_| DomainError::OperationFailed)?))
}

/// The subagent `record` names on `workspace`'s board, recorded or filled in:
/// its id. A new one is a builder; one that had ended is open again.
fn subagent_row(tx: &Connection, workspace: Uuid, record: &WorkerRecord) -> Result<(Uuid, AgentRole)> {
    let found: Option<(Uuid, String)> = tx
        .query_row(
            "SELECT id, role FROM agents WHERE workspace_id = ?1 AND harness = ?2 AND subagent_id = ?3",
            params![uuid_blob(workspace), record.harness, record.agent_id],
            |r| Ok((get_uuid(r, 0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_err)?;
    let terminal = record.orchestrator_terminal.map(uuid_blob);
    match found {
        Some((id, role)) => {
            tx.execute(
                "UPDATE agents SET session_id = coalesce(?2, session_id), session_cwd = coalesce(?3, session_cwd),
                        parent_terminal_id = coalesce(?4, parent_terminal_id),
                        title = coalesce(nullif(?5, ''), title), model = coalesce(nullif(?6, ''), model),
                        ended_at = NULL, end_reason = NULL
                  WHERE id = ?1",
                params![uuid_blob(id), record.session_id, record.session_cwd, terminal, record.label, record.model],
            )
            .map_err(map_err)?;
            Ok((id, AgentRole::parse(&role).unwrap_or(AgentRole::Builder)))
        }
        None => {
            let id = Uuid::now_v7();
            tx.execute(
                "INSERT INTO agents (id, workspace_id, role, kind, harness, subagent_id, parent_terminal_id, session_id,
                                     session_cwd, model, title, started_at)
                 VALUES (?1, ?2, 'builder', 'subagent', ?3, ?4, ?5, ?6, ?7, coalesce(?8, ''), coalesce(?9, ''), ?10)",
                params![
                    uuid_blob(id),
                    uuid_blob(workspace),
                    record.harness,
                    record.agent_id,
                    terminal,
                    record.session_id,
                    record.session_cwd,
                    record.model,
                    record.label,
                    now_millis(),
                ],
            )
            .map_err(map_err)?;
            Ok((id, AgentRole::Builder))
        }
    }
}

/// The work an agent in `role` does for a ticket it is recorded on as a worker.
fn work_role(role: AgentRole) -> WorkRole {
    match role {
        AgentRole::Reviewer => WorkRole::Review,
        AgentRole::Lander => WorkRole::Land,
        AgentRole::Orchestrator | AgentRole::Builder => WorkRole::Build,
    }
}

impl Store {
    /// Record a subagent working `task`, or fill in what's newly known about
    /// one already recorded. One that had ended is reopened: a resume.
    ///
    /// A backlog or todo task moves to in progress in the same transaction,
    /// with its `status_change` note, since a subagent at work means the task
    /// started. A done or cancelled task is refused (`task_closed`). A new
    /// subagent and a resumed one each write a `worker` note; filling in a
    /// session writes none.
    pub fn record_worker(&self, task: Uuid, record: &WorkerRecord, actor: Actor) -> Result<Task> {
        if !HARNESSES.contains(&record.harness.as_str()) {
            return Err(DomainError::InvalidArgument { what: "harness" });
        }
        if record.agent_id.trim().is_empty() {
            return Err(DomainError::InvalidArgument { what: "agent_id" });
        }
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let (status, workspace) = task_status(&tx, task)?;
            if matches!(status, TaskStatus::Done | TaskStatus::Cancelled) {
                return Err(DomainError::InvalidArgument { what: "task_closed" });
            }
            let now = now_millis();
            let existing = find(&tx, task, &record.harness, &record.agent_id)?;
            let (agent, role) = subagent_row(&tx, workspace, record)?;
            let said = match &existing {
                None => {
                    tx.execute(
                        "INSERT INTO ticket_work (agent_id, task_id, role, started_at, linked_by) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![uuid_blob(agent), uuid_blob(task), work_role(role).as_str(), now, record.linked_by.as_str()],
                    )
                    .map_err(map_err)?;
                    let how = match record.linked_by {
                        LinkedBy::Orchestrator => "started",
                        LinkedBy::Description => "linked from its description",
                    };
                    Some(("started", how))
                }
                Some(w) => {
                    tx.execute(
                        "UPDATE ticket_work SET ended_at = NULL, verdict = NULL,
                                linked_by = CASE WHEN ?4 = 'orchestrator' THEN ?4 ELSE linked_by END
                          WHERE agent_id = ?1 AND task_id = ?2 AND role = ?3",
                        params![uuid_blob(agent), uuid_blob(task), w.role.as_str(), record.linked_by.as_str()],
                    )
                    .map_err(map_err)?;
                    w.ended_at.map(|_| ("resumed", "resumed"))
                }
            };
            if let Some((event, how)) = said {
                let label = record.label.as_deref().or(existing.as_ref().map(|w| w.label.as_str())).unwrap_or("");
                let body = match label.trim() {
                    "" => format!("{} {how}.", who(&record.harness)),
                    label => format!("{} {how}: {label}", who(&record.harness)),
                };
                let extra = json!({
                    "event": event,
                    "harness": record.harness,
                    "agent_id": record.agent_id,
                    "linked_by": record.linked_by.as_str(),
                });
                insert_note(&tx, task, NoteKind::Worker, actor, &body, &extra, None)?;
            }
            if record.linked_by == LinkedBy::Orchestrator {
                unlink_elsewhere(&tx, task, record, actor)?;
            }
            attach_usage(&tx, task, record)?;
            if matches!(status, TaskStatus::Backlog | TaskStatus::Todo) {
                move_in(&tx, task, status, TaskStatus::InProgress, actor, now)?;
            }
            tx.commit().map_err(map_err)?;
        }
        self.get_task(task)
    }

    /// Record that a subagent stopped working `task`, and how, with a
    /// `worker` note. `agent` names one, by harness and id: one already
    /// ended is left as it is, and one never recorded on the task is
    /// `NotFound`. `None` ends every subagent open on the task, whatever its
    /// harness, and is `NotFound` when none is.
    pub fn end_worker(
        &self,
        task: Uuid,
        harness: &str,
        agent: Option<&str>,
        reason: EndReason,
        actor: Actor,
    ) -> Result<Task> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            task_status(&tx, task)?;
            let ending: Vec<TaskWorker> = match agent {
                Some(agent) => {
                    let worker = find(&tx, task, harness, agent)?.ok_or(DomainError::NotFound)?;
                    worker.ended_at.is_none().then_some(worker).into_iter().collect()
                }
                None => {
                    let open = shown_for(&tx, task)?.into_iter().filter(|w| w.ended_at.is_none()).collect::<Vec<_>>();
                    if open.is_empty() {
                        return Err(DomainError::NotFound);
                    }
                    open
                }
            };
            for worker in &ending {
                end_one(&tx, worker, reason, actor)?;
            }
            tx.commit().map_err(map_err)?;
        }
        self.get_task(task)
    }

    /// Link a subagent the runner saw start to the task its description
    /// starts with (`leading_key`), as the runner.
    ///
    /// Links only when the key names exactly one task on `workspace`'s board
    /// that is still open, and nothing recorded says otherwise: a subagent
    /// already working any task (by its harness and id) is left where the
    /// orchestrator put it. `None` when it linked nothing.
    pub fn link_worker_by_description(
        &self,
        workspace: Uuid,
        description: &str,
        record: &WorkerRecord,
    ) -> Result<Option<Task>> {
        let Some(key) = leading_key(description) else { return Ok(None) };
        let recorded: bool = self
            .conn()
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM ticket_work w JOIN agents a ON a.id = w.agent_id
                                 WHERE a.harness = ?1 AND a.subagent_id = ?2)",
                params![record.harness, record.agent_id],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        if recorded {
            return Ok(None);
        }
        let on_board: Vec<Task> =
            self.tasks_with_key(None, key)?.into_iter().filter(|t| t.workspace_id == workspace).collect();
        let [task] = on_board.as_slice() else { return Ok(None) };
        if matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled) {
            return Ok(None);
        }
        let record = WorkerRecord { linked_by: LinkedBy::Description, ..record.clone() };
        self.record_worker(task.id, &record, Actor::Runner).map(Some)
    }

    /// Every subagent `task` ever had, oldest first.
    pub fn workers_for(&self, task: Uuid) -> Result<Vec<TaskWorker>> {
        workers_where(&self.conn(), "w.task_id = ?1", "w.started_at, a.id", params![uuid_blob(task)])
    }

    /// The subagents the runner follows: those open, and those that ended
    /// at or after `since` (a resume is what ends that), each with the
    /// workspace of its task. Only ones recorded with a session.
    pub fn followed_workers(&self, since: i64) -> Result<Vec<(TaskWorker, Uuid)>> {
        let workers = workers_where(
            &self.conn(),
            "a.session_id IS NOT NULL AND a.harness = 'claude' AND (w.ended_at IS NULL OR w.ended_at >= ?1)",
            "w.started_at, a.id",
            params![since],
        )?;
        let conn = self.conn();
        workers
            .into_iter()
            .map(|w| {
                let board = conn
                    .query_row("SELECT workspace_id FROM tasks WHERE id = ?1", params![uuid_blob(w.task_id)], |r| get_uuid(r, 0))
                    .map_err(map_err)?;
                Ok((w, board))
            })
            .collect()
    }

    /// Every record of one subagent, on any task, newest first.
    pub fn workers_of_agent(&self, harness: &str, agent: &str) -> Result<Vec<TaskWorker>> {
        workers_where(
            &self.conn(),
            "a.harness = ?1 AND a.subagent_id = ?2",
            "w.started_at DESC, a.id DESC",
            params![harness, agent],
        )
    }

    /// The task a subagent is recorded on, the newest if it was recorded on
    /// more than one.
    pub fn task_of_subagent(&self, harness: &str, agent: &str) -> Result<Option<Uuid>> {
        Ok(self.workers_of_agent(harness, agent)?.first().map(|w| w.task_id))
    }

    /// Every open subagent on the runner, oldest first.
    pub fn open_workers(&self) -> Result<Vec<TaskWorker>> {
        workers_where(&self.conn(), "w.ended_at IS NULL", "w.started_at, a.id", [])
    }
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workers_observed_tests.rs"]
mod observed_tests;
