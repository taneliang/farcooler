//! When a task will start (ov-212) and who works it (ov-213): the three
//! routes, and the derived half of every task the board sends.
//!
//! Beside `task_ops` rather than in it, for its reason: the wire shapes stay
//! out of the store, and the routes stay out of the dispatch table. Every
//! write here announces, as every write there does.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::models::{Actor, Task};
use farcooler_store::waits::{TaskFacts, TaskLine, TaskWait, Wait, WaitEvent};
use farcooler_store::workers::{EndReason, LinkedBy, TaskWorker, WorkerRecord};
use uuid::Uuid;

use crate::service::Service;
use crate::task_ops::{actor_from_wire, pb_task, required_id};
use crate::watch::Watcher;
use crate::worker_seen::WorkerSeen;
use crate::wire::id_bytes;

// ---------------------------------------------------------------------------
// the derived half of a task
// ---------------------------------------------------------------------------

/// Tasks as the wire carries them, each with its wait's position, the
/// unfinished tasks it's blocked on, and its subagents.
pub(crate) fn pb_tasks(svc: &Service, tasks: &[Task]) -> Result<Vec<pb::Task>> {
    let facts = svc.store.task_facts(tasks)?;
    Ok(tasks.iter().zip(facts).map(|(task, facts)| with_facts(task, facts, &svc.worker_seen)).collect())
}

/// One task, as `pb_tasks` sends it.
pub(crate) fn pb_one(svc: &Service, task: &Task) -> Result<pb::Task> {
    Ok(pb_tasks(svc, std::slice::from_ref(task))?.remove(0))
}

fn with_facts(task: &Task, facts: TaskFacts, seen: &WorkerSeen) -> pb::Task {
    pb::Task {
        wait: task.wait.map(|w| pb_wait(w, &facts)),
        waiting_on: facts.waiting_on,
        workers: facts.workers.iter().map(|w| pb_worker(w, seen)).collect(),
        ..pb_task(task)
    }
}

fn pb_line(line: TaskLine) -> i32 {
    (match line {
        TaskLine::Agent => pb::TaskLine::Agent,
        TaskLine::Build => pb::TaskLine::Build,
    }) as i32
}

fn pb_event(event: WaitEvent) -> i32 {
    (match event {
        WaitEvent::Release => pb::TaskWaitEvent::Release,
        WaitEvent::Recurrence => pb::TaskWaitEvent::Recurrence,
        WaitEvent::ClearBoard => pb::TaskWaitEvent::ClearBoard,
    }) as i32
}

fn pb_wait(wait: TaskWait, facts: &TaskFacts) -> pb::TaskWait {
    let mut out = pb::TaskWait { since: wait.since, ..Default::default() };
    match wait.wait {
        Wait::InLine(line) => {
            out.kind = pb::TaskWaitKind::InLine as i32;
            out.line = pb_line(line);
            out.position = facts.position;
            out.ahead = facts.ahead.clone();
        }
        Wait::Until(at) => {
            out.kind = pb::TaskWaitKind::Until as i32;
            out.until = at;
        }
        Wait::After(event) => {
            out.kind = pb::TaskWaitKind::After as i32;
            out.event = pb_event(event);
        }
        Wait::Parked => out.kind = pb::TaskWaitKind::Parked as i32,
    }
    out
}

/// A subagent as recorded, and as the runner sees it. An open one is
/// `RUNNING` while the runner reads the session it's in, whose transcript
/// says when it last moved and what it's doing; where it can't (codex, a
/// session it can't find) it's `UNOBSERVED`. A closed one says how it closed.
fn pb_worker(worker: &TaskWorker, seen: &WorkerSeen) -> pb::TaskWorker {
    let observed = worker.harness == "claude" && seen.is_observed(&worker.agent_id);
    let seen = if observed { seen.get(&worker.agent_id).unwrap_or_default() } else { Default::default() };
    let state = match worker.end_reason {
        None if observed => pb::TaskWorkerState::Running,
        None => pb::TaskWorkerState::Unobserved,
        Some(EndReason::Finished | EndReason::HandedBack) => pb::TaskWorkerState::Finished,
        Some(EndReason::Failed | EndReason::Stopped | EndReason::TaskClosed | EndReason::Relinked) => {
            pb::TaskWorkerState::Stopped
        }
    };
    pb::TaskWorker {
        id: id_bytes(worker.id),
        harness: worker.harness.clone(),
        agent_id: worker.agent_id.clone(),
        label: worker.label.clone(),
        model: if worker.model.is_empty() { seen.model.clone() } else { worker.model.clone() },
        started_at: worker.started_at,
        ended_at: worker.ended_at.unwrap_or(0),
        state: state as i32,
        last_activity_at: seen.last_activity_at,
        doing: seen.doing,
        linked_by_description: worker.linked_by == LinkedBy::Description,
        orchestrator_terminal: worker.orchestrator_terminal.map(id_bytes),
    }
}

// ---------------------------------------------------------------------------
// the routes
// ---------------------------------------------------------------------------

fn line_from_wire(raw: i32) -> Option<TaskLine> {
    match pb::TaskLine::try_from(raw).ok()? {
        pb::TaskLine::Agent => Some(TaskLine::Agent),
        pb::TaskLine::Build => Some(TaskLine::Build),
        pb::TaskLine::Unspecified => None,
    }
}

/// The wait a `task.set_wait` names: `None` to clear, and refused where it
/// can't be one (see `TaskSetWait` in the proto).
fn wait_from_wire(req: &pb::TaskSetWait) -> Result<Option<Wait>> {
    Ok(match pb::TaskWaitKind::try_from(req.kind) {
        Ok(pb::TaskWaitKind::Unspecified) => None,
        Ok(pb::TaskWaitKind::Until) => Some(Wait::Until(req.until)),
        Ok(pb::TaskWaitKind::After) => Some(Wait::After(match pb::TaskWaitEvent::try_from(req.event) {
            Ok(pb::TaskWaitEvent::Release) => WaitEvent::Release,
            Ok(pb::TaskWaitEvent::Recurrence) => WaitEvent::Recurrence,
            Ok(pb::TaskWaitEvent::ClearBoard) => WaitEvent::ClearBoard,
            Ok(pb::TaskWaitEvent::Unspecified) | Err(_) => {
                return Err(DomainError::InvalidArgument { what: "event" });
            }
        })),
        Ok(pb::TaskWaitKind::Parked) => Some(Wait::Parked),
        Ok(pb::TaskWaitKind::InLine) | Err(_) => return Err(DomainError::InvalidArgument { what: "kind" }),
    })
}

/// `task.set_wait`: hold a task, park it, or clear what was said.
pub fn set_wait(svc: &Service, watcher: &Watcher, req: &pb::TaskSetWait) -> Result<pb::Task> {
    let id = required_id(&req.task_id)?;
    let actor = actor_from_wire(&req.actor)?;
    let wait = wait_from_wire(req)?;
    let before = svc.store.get_task(id)?.status;
    let task = svc.store.set_wait(id, wait, actor)?;
    announce(watcher, &task, actor);
    if before != task.status {
        watcher.task_event(&task, crate::watch::task_notice::TaskEvent::Moved { to: task.status }, actor);
    }
    pb_one(svc, &task)
}

/// `task.set_line`: replace one of a board's lines, whole.
///
/// Announces every task whose place changed: the ones named, and the ones
/// that were in the line and aren't any more.
pub fn set_line(svc: &Service, watcher: &Watcher, req: &pb::TaskSetLine) -> Result<pb::TaskList> {
    let workspace = svc.store.get_workspace(required_id(&req.workspace_id)?)?.id;
    let line = line_from_wire(req.line).ok_or(DomainError::InvalidArgument { what: "line" })?;
    let actor = actor_from_wire(&req.actor)?;
    let ids = req
        .task_ids
        .iter()
        .map(|raw| crate::wire::parse_id(raw).ok_or(DomainError::InvalidArgument { what: "task_ids" }))
        .collect::<Result<Vec<Uuid>>>()?;
    let was: Vec<Task> = svc
        .store
        .list_tasks(farcooler_store::TaskScope::Workspace(workspace), None)?
        .into_iter()
        .filter(|t| t.wait.is_some_and(|w| w.wait == Wait::InLine(line)) && !ids.contains(&t.id))
        .collect();
    let in_line = svc.store.set_line(workspace, line, &ids, actor)?;
    for task in in_line.iter().chain(&was) {
        let task = if ids.contains(&task.id) { task.clone() } else { svc.store.get_task(task.id)? };
        announce(watcher, &task, actor);
    }
    Ok(pb::TaskList { items: pb_tasks(svc, &in_line)?, reads: None })
}

/// The pane a `task.worker` was run from, when the socket it names is this
/// runner's own tmux: a pane id means nothing on another server.
pub(crate) async fn orchestrator_pane(svc: &Service, req: &pb::TaskWorkerSet) -> Option<Uuid> {
    let pane = req.tmux_pane.as_deref().filter(|p| !p.is_empty())?;
    let socket = req.tmux_socket.as_deref()?.split(',').next()?;
    let ours = std::path::Path::new(socket).file_name()?.to_str()? == svc.tmux.socket();
    if !ours {
        return None;
    }
    let panes = svc.tmux.list_tagged_panes().await.ok()?;
    panes.into_iter().find(|p| p.pane_id == pane).map(|p| p.terminal_id)
}

/// `task.worker`: record a subagent working a task, or that it ended.
/// `pane` is `orchestrator_pane`'s answer, looked up before this runs.
pub fn worker(svc: &Service, watcher: &Watcher, req: &pb::TaskWorkerSet, pane: Option<Uuid>) -> Result<pb::Task> {
    let id = required_id(&req.task_id)?;
    let actor = actor_from_wire(&req.actor)?;
    let before = svc.store.get_task(id)?.status;
    let task = if req.end {
        let reason = match req.end_reason.as_str() {
            "" | "finished" => EndReason::Finished,
            "failed" => EndReason::Failed,
            "stopped" => EndReason::Stopped,
            _ => return Err(DomainError::InvalidArgument { what: "end_reason" }),
        };
        // No id ends every subagent open on the task (`task worker KEY --done`).
        let agent = Some(req.agent_id.trim()).filter(|a| !a.is_empty());
        svc.store.end_worker(id, &req.harness, agent, reason, actor)?
    } else {
        let given = |s: &Option<String>| s.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let record = WorkerRecord {
            harness: req.harness.clone(),
            agent_id: req.agent_id.trim().to_string(),
            session_id: given(&req.session_id),
            session_cwd: given(&req.session_cwd),
            orchestrator_terminal: pane,
            label: given(&req.label),
            model: given(&req.model),
            linked_by: LinkedBy::Orchestrator,
        };
        svc.store.record_worker(id, &record, actor)?
    };
    announce(watcher, &task, actor);
    if before != task.status {
        watcher.task_event(&task, crate::watch::task_notice::TaskEvent::Moved { to: task.status }, actor);
    }
    pb_one(svc, &task)
}

/// When a task moves into or out of done or cancelled, the tasks blocked on
/// it read differently (`Task.waiting_on`), so each is announced too.
pub(crate) fn announce_dependents(svc: &Service, watcher: &Watcher, task: &Task, actor: Actor) -> Result<()> {
    for dependent in svc.store.dependents_of(task.id)? {
        announce(watcher, &dependent, actor);
    }
    Ok(())
}

fn announce(watcher: &Watcher, task: &Task, actor: Actor) {
    watcher.announce_task_changed(task, None, actor);
}

#[cfg(test)]
#[path = "task_starts_tests.rs"]
mod tests;
