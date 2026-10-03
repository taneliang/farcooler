//! When a task will start (ov-212).
//!
//! Four things are stored, on the task's own row: a place in one of a
//! board's two lines, a hold until a time, a hold until an event, and
//! parked. Everything else a board says about starting is derived on read
//! and so can't go stale: the position in a line, who is ahead, and which
//! unfinished tasks it is blocked on (`waiting_on`).
//!
//! **Each wait fits some statuses and no others** (`Wait::fits`). A status
//! move clears a wait that doesn't fit both sides of the move, in the move's
//! own transaction (`after_move`); a read shows only a wait that fits
//! (`read_wait`); and opening the store clears any an older build left
//! behind by moving a task without knowing about waits (`sweep_unfitting`).
//!
//! **A line is replaced whole** (`Store::set_line`), never edited one task
//! at a time: the order is what goes stale, and the orchestrator already
//! rewrites the whole of it on every heartbeat. Only a change of kind writes
//! a `wait` note; a reorder writes nothing, or the record would be a list of
//! reorders.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde_json::json;
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, NoteKind, Task, TaskNote, TaskStatus, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::{insert_note, now_millis};

/// One of a board's two lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskLine {
    /// Tasks not yet started, in the order they get a free agent.
    Agent,
    /// Tasks started, waiting for the one build slot.
    Build,
}

impl TaskLine {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskLine::Agent => "agent",
            TaskLine::Build => "build",
        }
    }

    pub fn parse(raw: &str) -> Option<TaskLine> {
        match raw {
            "agent" => Some(TaskLine::Agent),
            "build" => Some(TaskLine::Build),
            _ => None,
        }
    }

    /// The statuses a task in this line may have.
    pub fn takes(self, status: TaskStatus) -> bool {
        match self {
            TaskLine::Agent => matches!(status, TaskStatus::Backlog | TaskStatus::Todo),
            TaskLine::Build => matches!(status, TaskStatus::InProgress | TaskStatus::InReview),
        }
    }
}

/// What a held task waits for. The details are the intent's to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitEvent {
    Release,
    Recurrence,
    ClearBoard,
}

impl WaitEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            WaitEvent::Release => "release",
            WaitEvent::Recurrence => "recurrence",
            WaitEvent::ClearBoard => "clear_board",
        }
    }

    pub fn parse(raw: &str) -> Option<WaitEvent> {
        match raw {
            "release" => Some(WaitEvent::Release),
            "recurrence" => Some(WaitEvent::Recurrence),
            "clear_board" => Some(WaitEvent::ClearBoard),
            _ => None,
        }
    }
}

/// What was said about when a task starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    InLine(TaskLine),
    /// Unix milliseconds.
    Until(i64),
    After(WaitEvent),
    Parked,
}

impl Wait {
    pub fn kind_str(self) -> &'static str {
        match self {
            Wait::InLine(_) => "in_line",
            Wait::Until(_) => "until",
            Wait::After(_) => "after",
            Wait::Parked => "parked",
        }
    }

    /// Whether this wait means anything for a task in `status`. A line has
    /// its own statuses (`TaskLine::takes`); a hold and parked are for a
    /// task that hasn't started and isn't ready to, which is backlog.
    ///
    /// `FITS_SQL` is the same rule for a query, and
    /// `the_two_copies_of_the_fit_rule_agree` holds them together.
    pub fn fits(self, status: TaskStatus) -> bool {
        match self {
            Wait::InLine(line) => line.takes(status),
            Wait::Until(_) | Wait::After(_) | Wait::Parked => status == TaskStatus::Backlog,
        }
    }

    /// The `wait` note's sentence for setting this.
    fn set_sentence(self) -> String {
        match self {
            Wait::InLine(TaskLine::Agent) => "In line to start.".into(),
            Wait::InLine(TaskLine::Build) => "In line to build.".into(),
            Wait::Until(at) => format!("Held until {}.", farcooler_core::local_time::moment(at)),
            Wait::After(WaitEvent::Release) => "Held until the next release.".into(),
            Wait::After(WaitEvent::Recurrence) => "Held until it happens again.".into(),
            Wait::After(WaitEvent::ClearBoard) => "Held until nothing else is waiting.".into(),
            Wait::Parked => "Parked: nobody plans to start it.".into(),
        }
    }

    /// The `wait` note's sentence for clearing this.
    fn cleared_sentence(self) -> &'static str {
        match self {
            Wait::InLine(TaskLine::Agent) => "Out of the line to start.",
            Wait::InLine(TaskLine::Build) => "Out of the line to build.",
            Wait::Until(_) | Wait::After(_) => "No longer held.",
            Wait::Parked => "No longer parked.",
        }
    }

    /// The `wait` note's structure: what the wait is now.
    fn extra(self) -> serde_json::Value {
        match self {
            Wait::InLine(line) => json!({ "wait": "in_line", "line": line.as_str() }),
            Wait::Until(at) => json!({ "wait": "until", "until": at }),
            Wait::After(event) => json!({ "wait": "after", "event": event.as_str() }),
            Wait::Parked => json!({ "wait": "parked" }),
        }
    }
}

/// A wait and when it was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskWait {
    pub wait: Wait,
    /// Unix milliseconds.
    pub since: i64,
}

/// What a board read derives about a task, beside its row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskFacts {
    /// In a line: 1 is next. Zero otherwise.
    pub position: u32,
    /// In a line: the first three tasks ahead, in line order.
    pub ahead: Vec<String>,
    /// The keys of the tasks it's blocked on that are neither done nor
    /// cancelled, in the order the blocks were written.
    pub waiting_on: Vec<String>,
    /// Its subagents: every open one, then the most recently closed one.
    pub workers: Vec<crate::workers::TaskWorker>,
}

/// `Wait::fits` as a condition on a `tasks` row.
const FITS_SQL: &str = "((wait_kind = 'in_line' AND wait_line = 'agent' AND status IN ('backlog', 'todo'))
      OR (wait_kind = 'in_line' AND wait_line = 'build' AND status IN ('in_progress', 'in_review'))
      OR (wait_kind IN ('until', 'after', 'parked') AND status = 'backlog'))";

/// Every wait column at once, for a clear.
const CLEARED: &str =
    "wait_kind = NULL, wait_line = NULL, wait_rank = NULL, wait_until = NULL, wait_event = NULL, wait_since = NULL";

/// The columns for ov-212's waits and ov-213's workers, in one migration
/// because the two ship together.
///
/// The waits are nullable columns on `tasks`: a task has at most one, and a
/// board read must not join for it. `wait_rank` is written only by
/// `set_line`; the position a client sees is derived from it.
///
/// `task_workers` is history: a row per (task, harness, agent), open while
/// `ended_at` is NULL, never deleted while its task lives. See `workers`.
///
/// `answer_wakes.kind` lets the wake queue carry more than answers: a held
/// task whose time came (`hold_ended`) queues a wake for its orchestrator in
/// the same transaction that clears the hold. The answer pump reads only
/// `answer` rows; telling a hold is ov-212's lane B.
///
/// `Older::Refused`, though nothing here is a constraint an old write could
/// trip: what this build WRITES isn't readable by an older one. A `wait` or
/// `worker` note fails an older build's note decoder (`NoteKind::parse`),
/// which fails its whole `task.get` and `task.search`; and an older answer
/// pump would type a `hold_ended` row to an agent as if it were an answer.
pub(crate) fn migration_0021_waits_and_workers(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE tasks ADD COLUMN wait_kind TEXT;
        ALTER TABLE tasks ADD COLUMN wait_line TEXT;
        ALTER TABLE tasks ADD COLUMN wait_rank INTEGER;
        ALTER TABLE tasks ADD COLUMN wait_until INTEGER;
        ALTER TABLE tasks ADD COLUMN wait_event TEXT;
        ALTER TABLE tasks ADD COLUMN wait_since INTEGER;
        CREATE INDEX tasks_in_line ON tasks (workspace_id, wait_line, wait_rank) WHERE wait_rank IS NOT NULL;
        CREATE INDEX tasks_held_until ON tasks (wait_until) WHERE wait_until IS NOT NULL;

        CREATE TABLE task_workers (
            id BLOB PRIMARY KEY NOT NULL,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            harness TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            session_id TEXT,
            session_cwd TEXT,
            orchestrator_terminal BLOB REFERENCES terminals(id) ON DELETE SET NULL,
            label TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            end_reason TEXT,
            linked_by TEXT NOT NULL,
            UNIQUE (task_id, harness, agent_id)
        );
        CREATE INDEX task_workers_open ON task_workers (task_id) WHERE ended_at IS NULL;
        CREATE INDEX task_workers_by_session ON task_workers (session_id) WHERE session_id IS NOT NULL;

        ALTER TABLE answer_wakes ADD COLUMN kind TEXT NOT NULL DEFAULT 'answer';
        "#,
    )
}

/// A task's wait from its row, at `idx` (`wait_kind`) and the four columns
/// after it, or `None` where nothing was said or what was said doesn't fit
/// `status`. A word this build doesn't know, from a newer one, is also
/// `None`: a wait is advice, and a board that can't read it should say
/// nothing rather than fail.
pub(crate) fn read_wait(row: &Row, idx: usize, status: TaskStatus) -> rusqlite::Result<Option<TaskWait>> {
    let kind: Option<String> = row.get(idx)?;
    let line: Option<String> = row.get(idx + 1)?;
    let until: Option<i64> = row.get(idx + 2)?;
    let event: Option<String> = row.get(idx + 3)?;
    let since: Option<i64> = row.get(idx + 4)?;
    let wait = match kind.as_deref() {
        Some("in_line") => line.as_deref().and_then(TaskLine::parse).map(Wait::InLine),
        Some("until") => until.map(Wait::Until),
        Some("after") => event.as_deref().and_then(WaitEvent::parse).map(Wait::After),
        Some("parked") => Some(Wait::Parked),
        _ => None,
    };
    Ok(wait.filter(|w| w.fits(status)).map(|wait| TaskWait { wait, since: since.unwrap_or(0) }))
}

/// What a status move did to the task's wait and workers, for its
/// `status_change` note.
pub(crate) struct Moved {
    /// The kind of wait it cleared, if it cleared one.
    pub wait_cleared: Option<&'static str>,
    /// How many open subagents it closed.
    pub workers_closed: usize,
}

impl Moved {
    /// Add what happened to a `status_change` note's structure. Nothing when
    /// nothing did, so most moves read as they always have.
    pub(crate) fn annotate(&self, extra: &mut serde_json::Value) {
        if let Some(kind) = self.wait_cleared {
            extra["wait_cleared"] = json!(kind);
        }
        if self.workers_closed > 0 {
            extra["workers_closed"] = json!(self.workers_closed);
        }
    }
}

/// Inside `set_task_status`'s transaction, after the row moved from `from`
/// to `to`: clear a wait that doesn't fit both, and close open subagents on
/// a task that finished.
///
/// Both sides, not just the new one: a wait that didn't fit the old status
/// was hidden there (`read_wait`), left by an older build, and moving into a
/// status it happens to fit must not bring it back.
pub(crate) fn after_move(tx: &Connection, task: Uuid, from: TaskStatus, to: TaskStatus) -> Result<Moved> {
    let row: Option<(Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT wait_kind, wait_line FROM tasks WHERE id = ?1",
            params![uuid_blob(task)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_err)?;
    let (kind, line) = row.ok_or(DomainError::NotFound)?;
    let wait = match kind.as_deref() {
        Some("in_line") => Some(line.as_deref().and_then(TaskLine::parse).map(Wait::InLine)),
        Some("until") => Some(Some(Wait::Until(0))),
        Some("after") => Some(Some(Wait::After(WaitEvent::Release))),
        Some("parked") => Some(Some(Wait::Parked)),
        // A word from a newer build: it fits nothing here.
        Some(_) => Some(None),
        None => None,
    };
    let wait_cleared = match wait {
        Some(Some(w)) if w.fits(from) && w.fits(to) => None,
        Some(w) => {
            tx.execute(&format!("UPDATE tasks SET {CLEARED} WHERE id = ?1"), params![uuid_blob(task)])
                .map_err(map_err)?;
            Some(w.map_or("unknown", Wait::kind_str))
        }
        None => None,
    };
    let workers_closed = if matches!(to, TaskStatus::Done | TaskStatus::Cancelled) {
        crate::workers::close_open(tx, task, crate::workers::EndReason::TaskClosed)?
    } else {
        0
    };
    Ok(Moved { wait_cleared, workers_closed })
}

/// On open: clear every wait that doesn't fit its task's status, and close
/// every open subagent on a finished task. Only an older build that moved a
/// task leaves either behind; it never knew to clear them.
///
/// Read first, so the ordinary open, which every `--stdio` and `--stream`
/// process does, takes no write lock.
pub(crate) fn sweep_unfitting(conn: &Connection) -> Result<()> {
    let stale: bool = conn
        .query_row(
            &format!(
                "SELECT EXISTS (SELECT 1 FROM tasks WHERE wait_kind IS NOT NULL AND NOT {FITS_SQL})
                     OR EXISTS (SELECT 1 FROM task_workers w JOIN tasks t ON t.id = w.task_id
                                 WHERE w.ended_at IS NULL AND t.status IN ('done', 'cancelled'))"
            ),
            [],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    if !stale {
        return Ok(());
    }
    conn.execute(&format!("UPDATE tasks SET {CLEARED} WHERE wait_kind IS NOT NULL AND NOT {FITS_SQL}"), [])
        .map_err(map_err)?;
    conn.execute(
        "UPDATE task_workers SET ended_at = ?1, end_reason = 'task_closed'
          WHERE ended_at IS NULL
            AND task_id IN (SELECT id FROM tasks WHERE status IN ('done', 'cancelled'))",
        params![now_millis()],
    )
    .map_err(map_err)?;
    Ok(())
}

/// The task's raw wait, whatever its status: what a write compares against
/// to decide whether the kind changed.
fn stored_wait(conn: &Connection, task: Uuid) -> Result<(TaskStatus, Uuid, Option<Wait>, Option<i64>)> {
    conn.query_row(
        "SELECT status, workspace_id, wait_kind, wait_line, wait_until, wait_event, wait_since
           FROM tasks WHERE id = ?1",
        params![uuid_blob(task)],
        |r| {
            let raw: String = r.get(0)?;
            let status = TaskStatus::parse(&raw).unwrap_or(TaskStatus::Backlog);
            let kind: Option<String> = r.get(2)?;
            let line: Option<String> = r.get(3)?;
            let until: Option<i64> = r.get(4)?;
            let event: Option<String> = r.get(5)?;
            let wait = match kind.as_deref() {
                Some("in_line") => line.as_deref().and_then(TaskLine::parse).map(Wait::InLine),
                Some("until") => until.map(Wait::Until),
                Some("after") => event.as_deref().and_then(WaitEvent::parse).map(Wait::After),
                Some("parked") => Some(Wait::Parked),
                _ => None,
            };
            Ok((status, get_uuid(r, 1)?, wait, r.get(6)?))
        },
    )
    .optional()
    .map_err(map_err)?
    .ok_or(DomainError::NotFound)
}

/// Write `wait` onto a task's row, `rank` for a line. `None` clears it.
fn write_wait(tx: &Connection, task: Uuid, wait: Option<Wait>, rank: Option<i64>, since: i64) -> Result<()> {
    let (kind, line, until, event) = match wait {
        None => (None, None, None, None),
        Some(Wait::InLine(l)) => (Some("in_line"), Some(l.as_str()), None, None),
        Some(Wait::Until(at)) => (Some("until"), None, Some(at), None),
        Some(Wait::After(e)) => (Some("after"), None, None, Some(e.as_str())),
        Some(Wait::Parked) => (Some("parked"), None, None, None),
    };
    tx.execute(
        "UPDATE tasks SET wait_kind = ?1, wait_line = ?2, wait_rank = ?3, wait_until = ?4, wait_event = ?5,
                          wait_since = ?6
          WHERE id = ?7",
        params![kind, line, rank, until, event, wait.map(|_| since), uuid_blob(task)],
    )
    .map_err(map_err)?;
    Ok(())
}

/// The `wait` note for going from `before` to `after`, or nothing when the
/// two are the same wait.
fn note_change(tx: &Connection, task: Uuid, before: Option<Wait>, after: Option<Wait>, actor: Actor) -> Result<()> {
    if before == after {
        return Ok(());
    }
    let (body, extra) = match (before, after) {
        (_, Some(now)) => (now.set_sentence(), now.extra()),
        (Some(was), None) => (was.cleared_sentence().to_string(), json!({ "wait": "none", "was": was.kind_str() })),
        (None, None) => return Ok(()),
    };
    insert_note(tx, task, NoteKind::Wait, actor, &body, &extra, None)?;
    Ok(())
}

impl Store {
    /// Say when a task that hasn't started will: hold it until a time or an
    /// event, park it, or (`None`) clear what was said, a place in a line
    /// included.
    ///
    /// A todo task held or parked moves to backlog in the same transaction,
    /// with its `status_change` note: a held task isn't ready. A task that
    /// has started, or finished, is refused (`wait_status`); a started task
    /// waits in the build line instead. A line is set whole, by `set_line`,
    /// so `Wait::InLine` is refused here (`kind`), and so is a time that
    /// isn't in the future (`until`).
    pub fn set_wait(&self, task: Uuid, wait: Option<Wait>, actor: Actor) -> Result<Task> {
        let now = now_millis();
        match wait {
            Some(Wait::InLine(_)) => return Err(DomainError::InvalidArgument { what: "kind" }),
            Some(Wait::Until(at)) if at <= now => return Err(DomainError::InvalidArgument { what: "until" }),
            _ => {}
        }
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let (status, _, before, since) = stored_wait(&tx, task)?;
            if wait.is_some() && !matches!(status, TaskStatus::Backlog | TaskStatus::Todo) {
                return Err(DomainError::InvalidArgument { what: "wait_status" });
            }
            if wait.is_some() && status == TaskStatus::Todo {
                crate::tasks::move_in(&tx, task, TaskStatus::Todo, TaskStatus::Backlog, actor, now)?;
            }
            let since = if before == wait { since.unwrap_or(now) } else { now };
            write_wait(&tx, task, wait, None, since)?;
            note_change(&tx, task, before, wait, actor)?;
            tx.commit().map_err(map_err)?;
        }
        self.get_task(task)
    }

    /// Replace one of a board's lines, whole, in order: the first task is
    /// next. Answers with the line's tasks, in order.
    ///
    /// A task in that line on that board and not named loses its place. A
    /// task named loses whatever other wait it had. Each task must be on the
    /// board (`other_board`), named once (`task_twice`), and in a status the
    /// line takes (`line_status`); anything refused writes nothing.
    pub fn set_line(&self, workspace: Uuid, line: TaskLine, tasks: &[Uuid], actor: Actor) -> Result<Vec<Task>> {
        let now = now_millis();
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let named: std::collections::HashSet<Uuid> = tasks.iter().copied().collect();
            if named.len() != tasks.len() {
                return Err(DomainError::InvalidArgument { what: "task_twice" });
            }
            let mut before = HashMap::new();
            for &id in tasks {
                let (status, board, wait, since) = stored_wait(&tx, id)?;
                if board != workspace {
                    return Err(DomainError::InvalidArgument { what: "other_board" });
                }
                if !line.takes(status) {
                    return Err(DomainError::InvalidArgument { what: "line_status" });
                }
                before.insert(id, (wait, since));
            }
            let members: Vec<Uuid> = {
                let mut stmt = tx
                    .prepare("SELECT id FROM tasks WHERE workspace_id = ?1 AND wait_kind = 'in_line' AND wait_line = ?2")
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map(params![uuid_blob(workspace), line.as_str()], |r| get_uuid(r, 0))
                    .map_err(map_err)?;
                rows.collect::<rusqlite::Result<_>>().map_err(map_err)?
            };
            for id in members.into_iter().filter(|id| !named.contains(id)) {
                write_wait(&tx, id, None, None, now)?;
                note_change(&tx, id, Some(Wait::InLine(line)), None, actor)?;
            }
            for (rank, &id) in tasks.iter().enumerate() {
                let (was, since) = before[&id];
                let wait = Some(Wait::InLine(line));
                let since = if was == wait { since.unwrap_or(now) } else { now };
                write_wait(&tx, id, wait, Some(rank as i64 + 1), since)?;
                note_change(&tx, id, was, wait, actor)?;
            }
            tx.commit().map_err(map_err)?;
        }
        tasks.iter().map(|&id| self.get_task(id)).collect()
    }

    /// The earliest time a held task is due, if any task is held until one.
    pub fn next_hold_due(&self) -> Result<Option<i64>> {
        self.conn()
            .query_row(
                "SELECT min(wait_until) FROM tasks WHERE wait_kind = 'until' AND status = 'backlog'",
                [],
                |r| r.get(0),
            )
            .map_err(map_err)
    }

    /// Let go of every task held until a time at or before `now`, each with a
    /// `wait` note from the runner saying the time came, and a `hold_ended`
    /// wake queued for its orchestrator on that note, in one transaction.
    /// Answers with each task and its note.
    pub fn release_due_holds(&self, now: i64) -> Result<Vec<(Task, TaskNote)>> {
        let released = {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let due: Vec<(Uuid, i64)> = {
                let mut stmt = tx
                    .prepare(
                        "SELECT id, wait_until FROM tasks
                          WHERE wait_kind = 'until' AND status = 'backlog' AND wait_until <= ?1
                          ORDER BY wait_until, rowid",
                    )
                    .map_err(map_err)?;
                let rows = stmt.query_map(params![now], |r| Ok((get_uuid(r, 0)?, r.get(1)?))).map_err(map_err)?;
                rows.collect::<rusqlite::Result<_>>().map_err(map_err)?
            };
            let mut released = Vec::with_capacity(due.len());
            for (id, until) in due {
                write_wait(&tx, id, None, None, now)?;
                let body =
                    format!("Held until {}. That time has come.", farcooler_core::local_time::moment(until));
                let extra = json!({ "wait": "none", "was": "until", "until": until });
                let note = insert_note(&tx, id, NoteKind::Wait, Actor::Runner, &body, &extra, None)?;
                tx.execute(
                    "INSERT OR IGNORE INTO answer_wakes (note_id, task_id, enqueued_at, kind)
                     VALUES (?1, ?2, ?3, 'hold_ended')",
                    params![uuid_blob(note.id), uuid_blob(id), now],
                )
                .map_err(map_err)?;
                released.push((id, note));
            }
            tx.commit().map_err(map_err)?;
            released
        };
        released.into_iter().map(|(id, note)| Ok((self.get_task(id)?, note))).collect()
    }

    /// What a board read derives for each of `tasks`, in their order: a
    /// line's position and who is ahead, the unfinished tasks each is
    /// blocked on, and its subagents.
    pub fn task_facts(&self, tasks: &[Task]) -> Result<Vec<TaskFacts>> {
        let conn = self.conn();
        let mut lines: HashMap<(Uuid, TaskLine), Vec<(Uuid, String)>> = HashMap::new();
        let mut blocked = conn
            .prepare(
                "SELECT t.key FROM task_blocks b JOIN tasks t ON t.id = b.blocked_by
                  WHERE b.task_id = ?1 AND t.status NOT IN ('done', 'cancelled')
                  ORDER BY b.rowid",
            )
            .map_err(map_err)?;
        let mut out = Vec::with_capacity(tasks.len());
        for task in tasks {
            let mut facts = TaskFacts::default();
            if let Some(TaskWait { wait: Wait::InLine(line), .. }) = task.wait {
                let members = match lines.entry((task.workspace_id, line)) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => e.insert(line_members(&conn, task.workspace_id, line)?),
                };
                if let Some(i) = members.iter().position(|(id, _)| *id == task.id) {
                    facts.position = i as u32 + 1;
                    facts.ahead = members[..i].iter().take(3).map(|(_, key)| key.clone()).collect();
                }
            }
            facts.waiting_on = blocked
                .query_map(params![uuid_blob(task.id)], |r| r.get(0))
                .map_err(map_err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(map_err)?;
            facts.workers = crate::workers::shown_for(&conn, task.id)?;
            out.push(facts);
        }
        Ok(out)
    }

    /// The tasks blocked on `task`: whose `waiting_on` changes when it
    /// finishes, or comes back.
    pub fn dependents_of(&self, task: Uuid) -> Result<Vec<Task>> {
        let ids: Vec<Uuid> = {
            let conn = self.conn();
            let mut stmt =
                conn.prepare("SELECT task_id FROM task_blocks WHERE blocked_by = ?1 ORDER BY rowid").map_err(map_err)?;
            let rows = stmt.query_map(params![uuid_blob(task)], |r| get_uuid(r, 0)).map_err(map_err)?;
            rows.collect::<rusqlite::Result<_>>().map_err(map_err)?
        };
        ids.into_iter().map(|id| self.get_task(id)).collect()
    }
}

/// A line's tasks whose status fits it, in order.
fn line_members(conn: &Connection, workspace: Uuid, line: TaskLine) -> Result<Vec<(Uuid, String)>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, key FROM tasks
              WHERE workspace_id = ?1 AND wait_kind = 'in_line' AND wait_line = ?2 AND {FITS_SQL}
              ORDER BY wait_rank, rowid"
        ))
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(workspace), line.as_str()], |r| Ok((get_uuid(r, 0)?, r.get(1)?)))
        .map_err(map_err)?;
    rows.collect::<rusqlite::Result<_>>().map_err(map_err)
}

#[cfg(test)]
#[path = "waits_tests.rs"]
mod tests;
