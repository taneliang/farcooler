//! The collecting landing (ov-520, ov-523, ov-872): one landing per board that
//! is always collecting the tickets ready to land.
//!
//! **One collecting landing, by flag.** A board always has exactly one landing
//! that is `open` and has `collecting` set. A ticket that passes review (enters
//! the landing stage) joins it in the same write, and when none exists it is
//! made, numbered after the board's last. A landing started by hand, given a
//! lander or moved on stops collecting at once, and a new collecting landing
//! opens behind it.
//!
//! **A ticket that leaves landing comes off.** A ticket that goes back to
//! building, or is dropped, comes off a landing whose SHA is not out yet (open,
//! gating) in the same write. On a pushed, green or red landing it stays, since
//! the push carries its commits, and its timeline says so. The landing goes on
//! with the tickets still ready: a landing is never held for a ticket.
//!
//! **A ticket taken off by hand stays off.** `held_off` marks a ticket that was
//! taken off a landing by hand. The scan and the join skip it until it leaves
//! landing and comes back through review.
//!
//! **Departure retries.** Whenever a ticket joins, the runner's scan runs,
//! main's CI is recorded, or a landing settles, the collecting landing departs
//! (`gating`) if a ticket on it is ready, no landing is ahead of it, and main's
//! CI isn't red (running or unread doesn't hold it). A new collecting landing
//! opens behind it, and the orchestrator is told once (`departures_untold`). A
//! landing that isn't collecting never departs on its own.
//!
//! What happened is in `line_events`: a departure is a `state` event with
//! `departed`, and telling the orchestrator of it a `told` event.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::board_ci::{CiStatus, MAIN_SUBJECT, ci_of};
use crate::error::map_err;
use crate::landings::{Landing, LandingState, Route, add_tickets, landing_in, landing_of_in, landing_title, landings_where, move_landing};
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;
use crate::ticket_lines::{LineSubject, Stage, line_event};

/// Tickets in the landing stage on no live landing and not taken off by hand.
/// Both the gather and the runner's scan read it.
const UNCOLLECTED: &str = "l.stage = 'landing' AND l.held_off = 0 AND NOT EXISTS (
        SELECT 1 FROM landing_tickets lt JOIN landings g ON g.id = lt.landing_id
         WHERE lt.task_id = l.task_id AND g.state NOT IN ('landed', 'dropped'))";

/// The board's collecting landing.
pub(crate) fn collecting(conn: &Connection, workspace: Uuid) -> Result<Option<Landing>> {
    let id: Option<Uuid> = conn
        .query_row(
            "SELECT id FROM landings WHERE workspace_id = ?1 AND state = 'open' AND collecting = 1
              ORDER BY created_at, id LIMIT 1",
            params![uuid_blob(workspace)],
            |r| get_uuid(r, 0),
        )
        .optional()
        .map_err(map_err)?;
    id.map(|id| landing_in(conn, id)).transpose()
}

/// The board's collecting landing, made when it has none.
pub(crate) fn ensure_collecting(tx: &Connection, workspace: Uuid, actor: Actor) -> Result<Landing> {
    if let Some(open) = collecting(tx, workspace)? {
        return Ok(open);
    }
    let number: i64 = tx
        .query_row(
            "SELECT coalesce(max(number), 0) + 1 FROM landings WHERE workspace_id = ?1",
            params![uuid_blob(workspace)],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    let id = Uuid::now_v7();
    let now = now_millis();
    tx.execute(
        "INSERT INTO landings (id, workspace_id, number, route, base, state, state_since, title, collecting, actor,
                               created_at, resource_version)
         VALUES (?1, ?2, ?3, ?4, '', 'open', ?5, '', 1, ?6, ?5, 1)",
        params![uuid_blob(id), uuid_blob(workspace), number, Route::Push.as_str(), now, actor.to_string()],
    )
    .map_err(map_err)?;
    let extra = serde_json::json!({ "from": null, "to": "open", "collecting": true });
    line_event(tx, LineSubject::Landing(id), "state", actor, "Open.", extra)?;
    landing_in(tx, id)
}

/// A ticket's stage moved by a write to the line: it joins the collecting
/// landing when it reaches landing, and when it leaves landing it comes off an
/// open or gating landing, or is noted as kept on one whose push carries it.
pub(crate) fn stage_moved(tx: &Connection, task: Uuid, from: Stage, to: Stage, actor: Actor) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let on = landing_of_in(tx, task)?;
    if to == Stage::Landing {
        if on.is_none() {
            let workspace: Uuid = tx
                .query_row("SELECT workspace_id FROM tasks WHERE id = ?1", params![uuid_blob(task)], |r| get_uuid(r, 0))
                .map_err(map_err)?;
            join(tx, workspace, &[task], actor)?;
        }
        return Ok(());
    }
    if from != Stage::Landing || to == Stage::Landed {
        return Ok(());
    }
    let Some(landing) = on else { return Ok(()) };
    match landing.state {
        LandingState::Open | LandingState::Gating => {
            crate::landings::take_off(tx, &landing, task, false, actor)?;
            let body = format!("Taken off {}: it left landing.", landing_title(&landing));
            let extra = serde_json::json!({ "left_landing": true, "to_stage": to.as_str() });
            line_event(tx, LineSubject::TicketOnLanding { task, landing: landing.id }, "landing", actor, &body, extra)?;
            if drop_if_emptied(tx, landing.id, actor)? {
                ensure_collecting(tx, landing.workspace_id, actor)?;
                depart(tx, landing.workspace_id, actor)?;
            }
        }
        _ => {
            let body = format!("Stays on {}: its commits are in the landing's push.", landing_title(&landing));
            let extra = serde_json::json!({ "left_landing": true, "kept": true });
            line_event(tx, LineSubject::TicketOnLanding { task, landing: landing.id }, "landing", actor, &body, extra)?;
        }
    }
    Ok(())
}

/// Put `tickets` on the collecting landing, making it if need be, and let it
/// depart if it can. Each goes on with the rest of its same-branch group, and
/// one that can't go on yet (its stack parent hasn't landed and isn't coming,
/// its group isn't all ready, or a landing past open carries it) is left for
/// a later scan rather than refusing the write that moved it. The tickets put
/// on.
pub(crate) fn join(tx: &Connection, workspace: Uuid, tickets: &[Uuid], actor: Actor) -> Result<Vec<Uuid>> {
    let mut joined = Vec::new();
    for task in tickets {
        if joined.contains(task) {
            continue;
        }
        let mut unit = vec![*task];
        if let Some(group) = crate::ticket_lines::line_in(tx, *task)?.same_branch {
            let mut stmt = tx
                .prepare(&format!(
                    "SELECT l.task_id FROM ticket_lines l WHERE l.same_branch = ?1 AND l.task_id <> ?2 AND {UNCOLLECTED}"
                ))
                .map_err(map_err)?;
            let rest: Vec<Uuid> = stmt
                .query_map(params![uuid_blob(group), uuid_blob(*task)], |r| get_uuid(r, 0))
                .map_err(map_err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(map_err)?;
            unit.extend(rest);
        }
        let open = ensure_collecting(tx, workspace, actor)?;
        match add_tickets(tx, &open, &unit, actor) {
            Ok(()) => joined.extend(unit),
            Err(farcooler_core::DomainError::InvalidArgument { what: "parent_not_landed" | "same_branch" | "landing_moving" }) => {}
            Err(e) => return Err(e),
        }
    }
    depart(tx, workspace, actor)?;
    Ok(joined)
}

/// A landing the runner sent off on its own and that has lost every ticket it
/// carried has nothing to check: it is dropped, so it isn't ahead of the
/// collecting landing for good. Only a landing whose last state event is the
/// runner's departure, with no lander and no SHA: one the orchestrator gated by
/// hand is theirs to finish. True when it dropped.
pub(crate) fn drop_if_emptied(tx: &Connection, landing: Uuid, actor: Actor) -> Result<bool> {
    let l = landing_in(tx, landing)?;
    if l.state != LandingState::Gating || l.lander_id.is_some() || l.pushed_sha.is_some() {
        return Ok(false);
    }
    let last: Option<String> = tx
        .query_row(
            "SELECT extra FROM line_events WHERE landing_id = ?1 AND task_id IS NULL AND kind = 'state'
              ORDER BY at DESC, rowid DESC LIMIT 1",
            params![uuid_blob(landing)],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_err)?;
    let by_runner = last
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .is_some_and(|v| v["departed"] == true);
    if !by_runner {
        return Ok(false);
    }
    let carried: i64 = tx
        .query_row(
            "SELECT count(*) FROM landing_tickets lt JOIN ticket_lines l ON l.task_id = lt.task_id
              WHERE lt.landing_id = ?1 AND l.stage NOT IN ('landed', 'dropped')",
            params![uuid_blob(landing)],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    if carried > 0 {
        return Ok(false);
    }
    move_landing(tx, &l, LandingState::Dropped, actor, serde_json::json!({ "empty": true }))?;
    Ok(true)
}

/// Whether a landing on the board is ahead of the collecting one: one with its
/// checks started or out, or one started by hand and still assembling.
pub(crate) fn landing_ahead(conn: &Connection, workspace: Uuid) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM landings WHERE workspace_id = ?1
            AND (state IN ('gating', 'pushed', 'green', 'red') OR (state = 'open' AND collecting = 0))",
        params![uuid_blob(workspace)],
        |_| Ok(()),
    )
    .optional()
    .map_err(map_err)
    .map(|found| found.is_some())
}

/// The collecting landing departs for its checks when it can: a ticket on it
/// is ready, no landing is ahead and main's CI isn't red. A new collecting
/// landing opens behind it. Safe to ask again: it does nothing when it can't.
/// The landing that left, if one did.
pub(crate) fn depart(tx: &Connection, workspace: Uuid, actor: Actor) -> Result<Option<Landing>> {
    let Some(open) = collecting(tx, workspace)? else { return Ok(None) };
    if landing_ahead(tx, workspace)? {
        return Ok(None);
    }
    if ci_of(tx, workspace)?.iter().any(|c| c.subject == MAIN_SUBJECT && c.status == CiStatus::Failed) {
        return Ok(None);
    }
    let ready: i64 = tx
        .query_row(
            "SELECT count(*) FROM landing_tickets lt JOIN ticket_lines l ON l.task_id = lt.task_id
              WHERE lt.landing_id = ?1 AND l.stage = 'landing'",
            params![uuid_blob(open.id)],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    if ready == 0 {
        return Ok(None);
    }
    move_landing(tx, &open, LandingState::Gating, actor, serde_json::json!({ "departed": true }))?;
    ensure_collecting(tx, workspace, actor)?;
    Ok(Some(landing_in(tx, open.id)?))
}

/// A landing that left for its checks and whose orchestrator hasn't been told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departed {
    pub landing: Landing,
    /// When it left: what tells one departure from the next.
    pub at: i64,
    /// The keys of the tickets on it still open, in the order they joined.
    pub keys: Vec<String>,
    /// The tickets a notice can be filed on.
    pub cards: Vec<Uuid>,
}

/// What one scan of a board did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    /// The tickets put on the collecting landing.
    pub joined: Vec<Uuid>,
    /// The landing that left for its checks, if one did.
    pub departed: Option<Uuid>,
}

impl Store {
    /// Put every ticket of `workspace` that is ready to land, on no live
    /// landing and not taken off by hand on the collecting landing, and let it
    /// depart if it can. A departure that couldn't happen when it was asked
    /// for is retried here.
    pub fn collect_ready_tickets(&self, workspace: Uuid, actor: Actor) -> Result<Collected> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let ready = ready_uncollected(&tx, workspace)?;
        let open = match ready.is_empty() {
            true => collecting(&tx, workspace)?.map(|l| l.id),
            false => Some(ensure_collecting(&tx, workspace, actor)?.id),
        };
        let joined = if ready.is_empty() { Vec::new() } else { join(&tx, workspace, &ready, actor)? };
        depart(&tx, workspace, actor)?;
        let departed = match open {
            Some(id) if landing_in(&tx, id)?.state == LandingState::Gating => Some(id),
            _ => None,
        };
        tx.commit().map_err(map_err)?;
        Ok(Collected { joined, departed })
    }

    /// The boards the runner's scan has work on: a ticket ready to land that is
    /// on no live landing, or a collecting landing with a ready ticket on it.
    pub fn workspaces_to_collect(&self) -> Result<Vec<Uuid>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT t.workspace_id FROM ticket_lines l JOIN tasks t ON t.id = l.task_id WHERE {UNCOLLECTED}
                 UNION
                 SELECT g.workspace_id FROM landings g
                  WHERE g.state = 'open' AND g.collecting = 1
                    AND EXISTS (SELECT 1 FROM landing_tickets lt JOIN ticket_lines l ON l.task_id = lt.task_id
                                 WHERE lt.landing_id = g.id AND l.stage = 'landing')"
            ))
            .map_err(map_err)?;
        stmt.query_map([], |r| get_uuid(r, 0))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)
    }

    /// `workspace`'s landings that left for their checks by themselves and
    /// whose orchestrator hasn't been told.
    pub fn departures_untold(&self, workspace: Uuid) -> Result<Vec<Departed>> {
        departures_in(&self.conn(), workspace)
    }

    /// Record that the orchestrator is being told of `departed`. Written before
    /// the message is, so a crash between the two loses a notice and never
    /// repeats one. The check and the write are one transaction, so callers
    /// that race get one `true` between them. False when it was already
    /// recorded.
    pub fn record_departure_told(&self, departed: &Departed) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let untold = departures_in(&tx, departed.landing.workspace_id)?;
        if !untold.iter().any(|d| d.landing.id == departed.landing.id && d.at == departed.at) {
            return Ok(false);
        }
        let extra = serde_json::json!({ "departed_at": departed.at });
        line_event(&tx, LineSubject::Landing(departed.landing.id), "told", Actor::Runner, "", extra)?;
        tx.commit().map_err(map_err)?;
        Ok(true)
    }

    /// The boards with a landing that left for its checks.
    pub fn workspaces_with_gating_landings(&self) -> Result<Vec<Uuid>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT workspace_id FROM landings WHERE state = 'gating'").map_err(map_err)?;
        stmt.query_map([], |r| get_uuid(r, 0))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)
    }
}

/// The tickets the gather would put on the collecting landing, oldest move
/// first.
fn ready_uncollected(tx: &Transaction, workspace: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt = tx
        .prepare(&format!(
            "SELECT l.task_id FROM ticket_lines l JOIN tasks t ON t.id = l.task_id
              WHERE t.workspace_id = ?1 AND {UNCOLLECTED} ORDER BY l.stage_since, l.task_id"
        ))
        .map_err(map_err)?;
    stmt.query_map(params![uuid_blob(workspace)], |r| get_uuid(r, 0))
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)
}

/// The departures of `workspace` not yet told, read on `conn`.
fn departures_in(conn: &Connection, workspace: Uuid) -> Result<Vec<Departed>> {
    let landings = landings_where(conn, "workspace_id = ?1 AND state = 'gating'", workspace)?;
    let mut out = Vec::new();
    for landing in landings {
        let (mut left, mut told) = (None, Vec::new());
        let mut ev = conn
            .prepare(
                "SELECT at, kind, extra FROM line_events WHERE landing_id = ?1 AND task_id IS NULL
                    AND kind IN ('state', 'told') ORDER BY at, rowid",
            )
            .map_err(map_err)?;
        let rows = ev
            .query_map(params![uuid_blob(landing.id)], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        for (at, kind, extra) in rows {
            let extra: serde_json::Value = serde_json::from_str(&extra).unwrap_or_default();
            match kind.as_str() {
                "state" if extra["departed"] == true => left = Some(at),
                "told" => told.push(extra["departed_at"].as_i64().unwrap_or(0)),
                _ => {}
            }
        }
        let Some(at) = left.filter(|at| !told.contains(at)) else { continue };
        let mut open = Vec::new();
        for task in &landing.tickets {
            let (key, stage): (String, String) = conn
                .query_row(
                    "SELECT t.key, l.stage FROM tasks t JOIN ticket_lines l ON l.task_id = t.id WHERE t.id = ?1",
                    params![uuid_blob(*task)],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(map_err)?;
            if !matches!(stage.as_str(), "landed" | "dropped") {
                open.push((*task, key));
            }
        }
        // A landing is only "ready for its checks" while it has work to check.
        // One with a SHA out, or whose tickets have all landed or been dropped,
        // is past its checks however its own state reads, and an untold
        // departure from before that must not be told after a restart (ov-845).
        if landing.pushed_sha.is_some() || open.is_empty() {
            continue;
        }
        out.push(Departed {
            at,
            keys: open.iter().map(|(_, k)| k.clone()).collect(),
            cards: open.iter().map(|(t, _)| *t).collect(),
            landing,
        });
    }
    Ok(out)
}

#[cfg(test)]
#[path = "collecting_tests.rs"]
mod tests;
