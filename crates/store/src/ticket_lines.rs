//! The line (ov-872): one row per ticket being worked, keyed by its task.
//!
//! A ticket's **stage** lives here and not in the task's status (the owner's
//! call, Oct 31): waiting, building, reviewing, needs you, landing, landed or
//! dropped. So does the ticket's own branch and worktree, the ticket it is
//! stacked on, its pull request, how it landed, and its place in the plan.
//!
//! **The ticket is the landing unit.** A builder with several tickets gives
//! each one its own branch, stacked on the previous one when they depend on
//! each other, and each lands alone once its parent has. Only tickets carried
//! over from a lane of several cards share a branch: they keep that lane's id
//! in `same_branch` and land together.
//!
//! Its history, and that of themes and landings, is kept in `line_events`,
//! which replaces `plan_events` and `train_events`.
//!
//! The tables are made by `line_migration.rs`.

use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::agents::{board_exists, clean};
use crate::error::map_err;
use crate::models::{Actor, get_optional_uuid, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;

pub(crate) const LINE_MAX: usize = 300;
const BRANCH_MAX: usize = 240;
const PATH_MAX: usize = 512;
const SHA_MAX: usize = 64;

/// Where a ticket is on the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// In the plan, or waiting for a place in it; nobody is building it.
    Waiting,
    /// A builder has it, for the first time or for a fix round.
    Building,
    /// A reviewer has it.
    Reviewing,
    /// It waits on a person: a ruling, a pull request a human merges, or a
    /// landing the charter says to ask about.
    NeedsYou,
    /// On its way into the product.
    Landing,
    /// In the product.
    Landed,
    /// Given up.
    Dropped,
}

impl Stage {
    /// Every stage's stored word, in order.
    pub const WORDS: [&'static str; 7] = ["waiting", "building", "reviewing", "needs_you", "landing", "landed", "dropped"];

    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Waiting => "waiting",
            Stage::Building => "building",
            Stage::Reviewing => "reviewing",
            Stage::NeedsYou => "needs_you",
            Stage::Landing => "landing",
            Stage::Landed => "landed",
            Stage::Dropped => "dropped",
        }
    }

    /// The stage a stored word names.
    pub fn parse(raw: &str) -> Option<Stage> {
        Some(match raw {
            "waiting" => Stage::Waiting,
            "building" => Stage::Building,
            "reviewing" => Stage::Reviewing,
            "needs_you" => Stage::NeedsYou,
            "landing" => Stage::Landing,
            "landed" => Stage::Landed,
            "dropped" => Stage::Dropped,
            _ => return None,
        })
    }

    /// Landed and dropped tickets are finished: they take no more moves.
    pub fn is_closed(self) -> bool {
        matches!(self, Stage::Landed | Stage::Dropped)
    }

    /// Whether a ticket may move from `self` to `to`. Staying put is always
    /// allowed; any open ticket may be dropped or handed to a person.
    pub fn can_move_to(self, to: Stage) -> bool {
        use Stage::*;
        if self == to {
            return true;
        }
        if self.is_closed() {
            return false;
        }
        matches!(to, Dropped | NeedsYou)
            || matches!(
                (self, to),
                (Waiting, Building)
                    | (Building, Waiting)
                    | (Building, Reviewing)
                    | (Reviewing, Building)
                    | (Reviewing, Landing)
                    | (Reviewing, Landed)
                    | (NeedsYou, Waiting)
                    | (NeedsYou, Building)
                    | (NeedsYou, Reviewing)
                    | (NeedsYou, Landing)
                    | (Landing, Building)
                    | (Landing, Landed)
            )
    }

    pub(crate) fn said(self) -> &'static str {
        match self {
            Stage::Waiting => "Waiting.",
            Stage::Building => "Building.",
            Stage::Reviewing => "Moved to review.",
            Stage::NeedsYou => "Needs you.",
            Stage::Landing => "Landing.",
            Stage::Landed => "Landed.",
            Stage::Dropped => "Dropped.",
        }
    }
}

/// One ticket on the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketLine {
    pub task_id: Uuid,
    pub stage: Stage,
    /// One line: why it is in the plan, or why it is where it is now.
    pub reason: String,
    /// The ticket's own branch; empty until one is cut.
    pub branch: String,
    pub worktree_id: Option<Uuid>,
    pub worktree_path: String,
    /// The ticket whose branch this one's is stacked on.
    pub stack_parent: Option<Uuid>,
    /// The lane a carried-over ticket shared a branch with others in: they land
    /// together. Only the migration writes it.
    pub same_branch: Option<Uuid>,
    pub pr_number: Option<u32>,
    pub landed_sha: Option<String>,
    /// 1 for next up; `None` outside the plan.
    pub plan_rank: Option<u32>,
    pub stage_since: i64,
    /// Taken off a landing by hand: the runner doesn't put it back on its own.
    pub held_off: bool,
    pub created_at: i64,
    pub resource_version: u64,
}

/// A ticket to put on the line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTicketLine {
    pub reason: String,
    pub branch: String,
    pub worktree_id: Option<Uuid>,
    pub worktree_path: String,
    pub stack_parent: Option<Uuid>,
}

/// What `update_line` changes; each `None` leaves a field alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineUpdate {
    pub stage: Option<Stage>,
    pub reason: Option<String>,
    pub branch: Option<String>,
    pub worktree_id: Option<Option<Uuid>>,
    pub worktree_path: Option<String>,
    pub stack_parent: Option<Option<Uuid>>,
    pub pr_number: Option<Option<u32>>,
    pub landed_sha: Option<String>,
    pub held_off: Option<bool>,
}

/// What a line event is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSubject {
    Theme(Uuid),
    Ticket(Uuid),
    Landing(Uuid),
}

/// One entry in a theme's, a ticket's or a landing's timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEvent {
    pub id: Uuid,
    pub at: i64,
    pub actor: String,
    pub theme_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub landing_id: Option<Uuid>,
    pub kind: String,
    pub body: String,
    pub extra: serde_json::Value,
}

fn invalid(what: &'static str) -> DomainError {
    DomainError::InvalidArgument { what }
}

/// Write one timeline entry.
pub(crate) fn line_event(
    conn: &Connection,
    subject: LineSubject,
    kind: &str,
    actor: Actor,
    body: &str,
    extra: serde_json::Value,
) -> Result<()> {
    let (theme, task, landing) = match subject {
        LineSubject::Theme(id) => (Some(id), None, None),
        LineSubject::Ticket(id) => (None, Some(id), None),
        LineSubject::Landing(id) => (None, None, Some(id)),
    };
    conn.execute(
        "INSERT INTO line_events (id, at, actor, theme_id, task_id, landing_id, kind, body, extra)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            uuid_blob(Uuid::now_v7()),
            now_millis(),
            actor.to_string(),
            theme.map(uuid_blob),
            task.map(uuid_blob),
            landing.map(uuid_blob),
            kind,
            body,
            extra.to_string()
        ],
    )
    .map_err(map_err)?;
    Ok(())
}

/// A ticket's key, as a timeline names it.
pub(crate) fn task_key(conn: &Connection, task: Uuid) -> String {
    conn.query_row("SELECT key FROM tasks WHERE id = ?1", params![uuid_blob(task)], |r| r.get(0))
        .unwrap_or_else(|_| "a ticket".to_string())
}

/// Each ticket must exist and be on `workspace`'s board.
pub(crate) fn check_tickets(conn: &Connection, workspace: Uuid, tasks: &[Uuid]) -> Result<()> {
    for task in tasks {
        let board: Option<Vec<u8>> = conn
            .query_row("SELECT workspace_id FROM tasks WHERE id = ?1", params![uuid_blob(*task)], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        match board {
            None => return Err(DomainError::NotFound),
            Some(b) if b != uuid_blob(workspace) => return Err(invalid("other_board")),
            Some(_) => {}
        }
    }
    Ok(())
}

fn board_of(conn: &Connection, task: Uuid) -> Result<Uuid> {
    let board: Vec<u8> = conn
        .query_row("SELECT workspace_id FROM tasks WHERE id = ?1", params![uuid_blob(task)], |r| r.get(0))
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)?;
    Uuid::from_slice(&board).map_err(|_| DomainError::OperationFailed)
}

const LINE_COLS: &str = "task_id, stage, reason, branch, worktree_id, worktree_path, stack_parent, same_branch, pr_number, \
     landed_sha, plan_rank, stage_since, held_off, created_at, resource_version";

fn row_to_line(r: &Row) -> rusqlite::Result<TicketLine> {
    let stage: String = r.get(1)?;
    Ok(TicketLine {
        task_id: get_uuid(r, 0)?,
        // A word from a newer build reads as waiting: open, and nobody on it.
        stage: Stage::parse(&stage).unwrap_or(Stage::Waiting),
        reason: r.get(2)?,
        branch: r.get(3)?,
        worktree_id: get_optional_uuid(r, 4)?,
        worktree_path: r.get(5)?,
        stack_parent: get_optional_uuid(r, 6)?,
        same_branch: get_optional_uuid(r, 7)?,
        pr_number: r.get::<_, Option<i64>>(8)?.map(|n| n.clamp(0, u32::MAX as i64) as u32),
        landed_sha: r.get(9)?,
        plan_rank: r.get::<_, Option<i64>>(10)?.map(|n| n.max(0) as u32),
        stage_since: r.get(11)?,
        held_off: r.get::<_, i64>(12)? != 0,
        created_at: r.get(13)?,
        resource_version: r.get::<_, i64>(14)?.max(0) as u64,
    })
}

pub(crate) fn line_in(conn: &Connection, task: Uuid) -> Result<TicketLine> {
    conn.query_row(&format!("SELECT {LINE_COLS} FROM ticket_lines WHERE task_id = ?1"), params![uuid_blob(task)], row_to_line)
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

/// `parent` may carry `task`'s branch: on the line, on the same board, open or
/// landed, and not `task` itself or stacked (however far up) on it.
fn check_parent(conn: &Connection, task: Uuid, parent: Uuid) -> Result<()> {
    if parent == task {
        return Err(invalid("stack_parent"));
    }
    if board_of(conn, parent)? != board_of(conn, task)? {
        return Err(invalid("other_board"));
    }
    let mut at = Some(parent);
    let mut steps = 0;
    while let Some(id) = at {
        let line = line_in(conn, id).map_err(|e| if matches!(e, DomainError::NotFound) { invalid("stack_parent") } else { e })?;
        if id == parent && line.stage == Stage::Dropped {
            return Err(invalid("stack_parent"));
        }
        at = line.stack_parent;
        steps += 1;
        if at == Some(task) || steps > 1000 {
            return Err(invalid("stack_cycle"));
        }
    }
    Ok(())
}

/// Move `task` to `to`, and write it down.
pub(crate) fn move_stage(conn: &Connection, task: Uuid, to: Stage, sha: Option<&str>, actor: Actor) -> Result<()> {
    let line = line_in(conn, task)?;
    if line.stage == to {
        return Ok(());
    }
    if !line.stage.can_move_to(to) {
        return Err(invalid(if line.stage.is_closed() { "ticket_closed" } else { "stage" }));
    }
    if to == Stage::Landed {
        if let Some(parent) = line.stack_parent
            && line_in(conn, parent)?.stage != Stage::Landed
        {
            return Err(invalid("parent_not_landed"));
        }
        if let Some(group) = line.same_branch {
            let open: i64 = conn
                .query_row(
                    "SELECT count(*) FROM ticket_lines WHERE same_branch = ?1 AND task_id <> ?2 AND stage <> 'landed'",
                    params![uuid_blob(group), uuid_blob(task)],
                    |r| r.get(0),
                )
                .map_err(map_err)?;
            if open > 0 {
                return Err(invalid("same_branch"));
            }
        }
    }
    let rank_kept = to == Stage::Waiting;
    conn.execute(
        "UPDATE ticket_lines SET stage = ?2, stage_since = ?3,
                plan_rank = CASE WHEN ?4 THEN plan_rank ELSE NULL END,
                landed_sha = coalesce(?5, landed_sha),
                held_off = CASE WHEN ?2 = 'landing' THEN held_off ELSE 0 END,
                resource_version = resource_version + 1
          WHERE task_id = ?1",
        params![uuid_blob(task), to.as_str(), now_millis(), rank_kept, sha],
    )
    .map_err(map_err)?;
    let subject = LineSubject::Ticket(task);
    // Landing straight from review is one write that passes through landing,
    // and the timeline says so, as it would for the two writes.
    let from = if line.stage == Stage::Reviewing && to == Stage::Landed {
        let via = serde_json::json!({ "from": "reviewing", "to": "landing" });
        line_event(conn, subject, "stage", actor, Stage::Landing.said(), via)?;
        Stage::Landing
    } else {
        line.stage
    };
    let extra = serde_json::json!({ "from": from.as_str(), "to": to.as_str() });
    line_event(conn, subject, "stage", actor, to.said(), extra)
}

impl Store {
    /// Put `task` on the line, waiting. A ticket already on it is refused.
    pub fn put_on_line(&self, task: Uuid, new: &NewTicketLine, actor: Actor) -> Result<TicketLine> {
        let reason = clean(&new.reason, LINE_MAX, false, "reason")?;
        let branch = clean(&new.branch, BRANCH_MAX, false, "branch")?;
        let path = clean(&new.worktree_path, PATH_MAX, false, "path")?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_of(&tx, task)?;
        crate::workflow_projection::refuse_unless_at_rest(&tx, task)?;
        if line_in(&tx, task).is_ok() {
            return Err(invalid("on_the_line"));
        }
        let now = now_millis();
        tx.execute(
            "INSERT INTO ticket_lines (task_id, stage, reason, branch, worktree_id, worktree_path, stage_since, created_at,
                                       resource_version)
             VALUES (?1, 'waiting', ?2, ?3, ?4, ?5, ?6, ?6, 1)",
            params![uuid_blob(task), reason, branch, new.worktree_id.map(uuid_blob), path, now],
        )
        .map_err(map_err)?;
        if let Some(parent) = new.stack_parent {
            check_parent(&tx, task, parent)?;
            tx.execute("UPDATE ticket_lines SET stack_parent = ?2 WHERE task_id = ?1", params![uuid_blob(task), uuid_blob(parent)])
                .map_err(map_err)?;
        }
        line_event(&tx, LineSubject::Ticket(task), "stage", actor, "Waiting.", serde_json::json!({ "to": "waiting" }))?;
        let line = line_in(&tx, task)?;
        tx.commit().map_err(map_err)?;
        Ok(line)
    }

    /// Change a ticket on the line: move it along the arrows
    /// `Stage::can_move_to` allows, and set its branch, worktree, stack parent,
    /// pull request and how it landed, in one write.
    ///
    /// A ticket that leaves waiting loses its place in the plan. One stacked on
    /// a ticket that hasn't landed can't land, and one sharing a branch lands
    /// with the rest of its group, through a landing.
    pub fn update_line(&self, task: Uuid, update: &LineUpdate, actor: Actor) -> Result<TicketLine> {
        let reason = update.reason.as_deref().map(|v| clean(v, LINE_MAX, false, "reason")).transpose()?;
        let branch = update.branch.as_deref().map(|v| clean(v, BRANCH_MAX, false, "branch")).transpose()?;
        let path = update.worktree_path.as_deref().map(|v| clean(v, PATH_MAX, false, "path")).transpose()?;
        let sha = update.landed_sha.as_deref().map(|v| clean(v, SHA_MAX, false, "sha")).transpose()?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        crate::workflow_projection::refuse_unless_at_rest(&tx, task)?;
        let before = line_in(&tx, task)?;
        if let Some(Some(parent)) = update.stack_parent {
            check_parent(&tx, task, parent)?;
        }
        tx.execute(
            "UPDATE ticket_lines SET
                reason = coalesce(?2, reason),
                branch = coalesce(?3, branch),
                worktree_id = CASE WHEN ?4 THEN ?5 ELSE worktree_id END,
                worktree_path = coalesce(?6, worktree_path),
                stack_parent = CASE WHEN ?7 THEN ?8 ELSE stack_parent END,
                pr_number = CASE WHEN ?9 THEN ?10 ELSE pr_number END,
                held_off = coalesce(?11, held_off),
                resource_version = resource_version + 1
              WHERE task_id = ?1",
            params![
                uuid_blob(task),
                reason,
                branch,
                update.worktree_id.is_some(),
                update.worktree_id.flatten().map(uuid_blob),
                path,
                update.stack_parent.is_some(),
                update.stack_parent.flatten().map(uuid_blob),
                update.pr_number.is_some(),
                update.pr_number.flatten(),
                update.held_off,
            ],
        )
        .map_err(map_err)?;
        let after = line_in(&tx, task)?;
        let changed: Vec<&str> = [
            ("reason", before.reason != after.reason),
            ("branch", before.branch != after.branch),
            ("worktree", before.worktree_id != after.worktree_id || before.worktree_path != after.worktree_path),
            ("stack", before.stack_parent != after.stack_parent),
            ("pull request", before.pr_number != after.pr_number),
        ]
        .into_iter()
        .filter_map(|(what, moved)| moved.then_some(what))
        .collect();
        if !changed.is_empty() {
            let body = match changed.as_slice() {
                [one] => format!("Changed its {one}."),
                [rest @ .., last] => format!("Changed its {} and {last}.", rest.join(", ")),
                [] => unreachable!(),
            };
            let extra = serde_json::json!({ "changed": changed, "reason": after.reason, "branch": after.branch });
            line_event(&tx, LineSubject::Ticket(task), "line", actor, &body, extra)?;
        }
        match update.stage {
            Some(to) => move_stage(&tx, task, to, sha.as_deref(), actor)?,
            None if sha.is_some() && before.stage == Stage::Landed => {
                tx.execute("UPDATE ticket_lines SET landed_sha = ?2 WHERE task_id = ?1", params![uuid_blob(task), sha])
                    .map_err(map_err)?;
            }
            None if sha.is_some() => return Err(invalid("not_landed")),
            None => {}
        }
        let line = line_in(&tx, task)?;
        tx.commit().map_err(map_err)?;
        Ok(line)
    }

    /// Set `workspace`'s plan: `tasks` in order, next up first. Each must be a
    /// waiting ticket on that board. Every other waiting ticket there leaves
    /// the plan.
    pub fn set_line_order(&self, workspace: Uuid, tasks: &[Uuid], actor: Actor) -> Result<Vec<TicketLine>> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_exists(&tx, workspace)?;
        check_tickets(&tx, workspace, tasks)?;
        let mut seen = std::collections::HashSet::new();
        for task in tasks {
            if !seen.insert(*task) {
                return Err(invalid("duplicate"));
            }
            if line_in(&tx, *task).map_err(|e| if matches!(e, DomainError::NotFound) { invalid("not_on_the_line") } else { e })?.stage
                != Stage::Waiting
            {
                return Err(invalid("not_waiting"));
            }
        }
        tx.execute(
            "UPDATE ticket_lines SET plan_rank = NULL, resource_version = resource_version + 1
              WHERE plan_rank IS NOT NULL AND task_id IN (SELECT id FROM tasks WHERE workspace_id = ?1)",
            params![uuid_blob(workspace)],
        )
        .map_err(map_err)?;
        let mut out = Vec::with_capacity(tasks.len());
        for (i, task) in tasks.iter().enumerate() {
            let rank = i as i64 + 1;
            tx.execute(
                "UPDATE ticket_lines SET plan_rank = ?2, resource_version = resource_version + 1 WHERE task_id = ?1",
                params![uuid_blob(*task), rank],
            )
            .map_err(map_err)?;
            let body = if rank == 1 { "Next up.".to_string() } else { format!("Number {rank} in the plan.") };
            line_event(&tx, LineSubject::Ticket(*task), "plan", actor, &body, serde_json::json!({ "rank": rank }))?;
            out.push(line_in(&tx, *task)?);
        }
        tx.commit().map_err(map_err)?;
        Ok(out)
    }

    /// Put `task` on the line if it isn't on it: what dispatching an agent to
    /// it does. A ticket already on the line is left as it is.
    pub fn ensure_on_line(&self, task: Uuid, actor: Actor) -> Result<TicketLine> {
        match self.ticket_line(task) {
            Ok(line) => Ok(line),
            Err(DomainError::NotFound) => match self.put_on_line(task, &NewTicketLine::default(), actor) {
                Err(DomainError::InvalidArgument { what: "on_the_line" }) => self.ticket_line(task),
                other => other,
            },
            Err(e) => Err(e),
        }
    }

    /// A ticket's place on the line.
    pub fn ticket_line(&self, task: Uuid) -> Result<TicketLine> {
        line_in(&self.conn(), task)
    }

    /// Every ticket on `workspace`'s line: the plan in order, then the rest by
    /// how long they have been where they are.
    pub fn ticket_lines(&self, workspace: Uuid) -> Result<Vec<TicketLine>> {
        lines_of(&self.conn(), workspace)
    }

    /// A theme's, a ticket's or a landing's timeline since `since_ms`, oldest
    /// first. A ticket's includes its moves on landings.
    pub fn line_events(&self, subject: LineSubject, since_ms: i64) -> Result<Vec<LineEvent>> {
        let (column, id) = match subject {
            LineSubject::Theme(id) => ("theme_id", id),
            LineSubject::Ticket(id) => ("task_id", id),
            LineSubject::Landing(id) => ("landing_id", id),
        };
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id, at, actor, theme_id, task_id, landing_id, kind, body, extra FROM line_events
                  WHERE {column} = ?1 AND at >= ?2 ORDER BY at, id"
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![uuid_blob(id), since_ms], |r| {
                let extra: String = r.get(8)?;
                Ok(LineEvent {
                    id: get_uuid(r, 0)?,
                    at: r.get(1)?,
                    actor: r.get(2)?,
                    theme_id: get_optional_uuid(r, 3)?,
                    task_id: get_optional_uuid(r, 4)?,
                    landing_id: get_optional_uuid(r, 5)?,
                    kind: r.get(6)?,
                    body: r.get(7)?,
                    extra: serde_json::from_str(&extra).unwrap_or_else(|_| serde_json::json!({})),
                })
            })
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(map_err)
    }
}

/// Every ticket on `workspace`'s line, read on `conn`: the plan in order, then
/// the rest by how long they have been where they are.
pub(crate) fn lines_of(conn: &Connection, workspace: Uuid) -> Result<Vec<TicketLine>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {} FROM ticket_lines l JOIN tasks t ON t.id = l.task_id WHERE t.workspace_id = ?1
              ORDER BY l.plan_rank IS NULL, l.plan_rank, l.stage_since, l.task_id",
            LINE_COLS.split(", ").map(|c| format!("l.{c}")).collect::<Vec<_>>().join(", ")
        ))
        .map_err(map_err)?;
    let rows = stmt.query_map(params![uuid_blob(workspace)], row_to_line).map_err(map_err)?;
    rows.collect::<rusqlite::Result<_>>().map_err(map_err)
}

#[cfg(test)]
#[path = "ticket_lines_tests.rs"]
mod tests;
