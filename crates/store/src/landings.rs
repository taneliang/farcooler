//! Landings (ov-872): putting finished tickets into the product.
//!
//! A landing takes one or more tickets in the landing stage along one
//! **route**: push (fold onto the base, gate, push, watch CI), pull request
//! (one per ticket, which a human merges) or ask (it waits for you to tap
//! Land). **A landing of one ticket is the normal case**; several tickets ready
//! at once may share one gate. It replaces the train (`board_trains`), and
//! `landing_tickets` replaces a lane's train label.
//!
//! Not to be confused with `landing.rs`, which keeps how each workspace lands
//! (`workspace_landing`).
//!
//! The tables are made by `line_migration.rs`.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::agents::{AgentRole, agent_in, board_exists, clean};
use crate::error::map_err;
use crate::models::{Actor, get_optional_uuid, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;
use crate::ticket_lines::{LineSubject, Stage, check_tickets, line_event, line_in, move_stage, task_key};

const BASE_MAX: usize = 200;
const TITLE_MAX: usize = 80;
const SHA_MAX: usize = 64;

/// How a ticket gets into the product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Fold onto the base, gate, push, and land on green CI.
    Push,
    /// A pull request per ticket, which a human merges.
    PullRequest,
    /// Needs you: it pushes once you tap Land.
    Ask,
}

impl Route {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            Route::Push => "push",
            Route::PullRequest => "pull_request",
            Route::Ask => "ask",
        }
    }

    /// The route a stored word names.
    pub fn parse(raw: &str) -> Option<Route> {
        Some(match raw {
            "push" => Route::Push,
            "pull_request" => Route::PullRequest,
            "ask" => Route::Ask,
            _ => return None,
        })
    }
}

/// Where a landing stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandingState {
    /// Taking tickets.
    Open,
    /// Its gate is running.
    Gating,
    /// A SHA is out and CI hasn't finished on it.
    Pushed,
    /// CI passed on the pushed SHA.
    Green,
    /// CI failed on the pushed SHA.
    Red,
    /// In the product.
    Landed,
    /// Given up; its tickets wait to land another way.
    Dropped,
}

impl LandingState {
    /// Every state's stored word, in order.
    pub const WORDS: [&'static str; 7] = ["open", "gating", "pushed", "green", "red", "landed", "dropped"];

    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            LandingState::Open => "open",
            LandingState::Gating => "gating",
            LandingState::Pushed => "pushed",
            LandingState::Green => "green",
            LandingState::Red => "red",
            LandingState::Landed => "landed",
            LandingState::Dropped => "dropped",
        }
    }

    /// The state a stored word names.
    pub fn parse(raw: &str) -> Option<LandingState> {
        Some(match raw {
            "open" => LandingState::Open,
            "gating" => LandingState::Gating,
            "pushed" => LandingState::Pushed,
            "green" => LandingState::Green,
            "red" => LandingState::Red,
            "landed" => LandingState::Landed,
            "dropped" => LandingState::Dropped,
            _ => return None,
        })
    }

    /// Landed or dropped: nothing moves it again.
    pub fn is_settled(self) -> bool {
        matches!(self, LandingState::Landed | LandingState::Dropped)
    }
}

/// One landing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landing {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// What a person calls it: Landing 12. Counts up per board.
    pub number: u32,
    pub route: Route,
    /// What it was cut from: `origin/main`, or a SHA.
    pub base: String,
    pub pushed_sha: Option<String>,
    pub state: LandingState,
    pub state_since: i64,
    /// The agent that lands it, when one was started.
    pub lander_id: Option<Uuid>,
    pub title: String,
    /// The board's collecting landing, which ready tickets join. At most one
    /// open per board.
    pub collecting: bool,
    /// Who started it, in `task_notes.actor`'s words.
    pub actor: String,
    pub created_at: i64,
    pub landed_at: Option<i64>,
    pub resource_version: u64,
    /// Its tickets, in the order they joined.
    pub tickets: Vec<Uuid>,
}

/// A landing to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLanding {
    pub route: Route,
    pub base: String,
    pub title: String,
    pub lander: Option<Uuid>,
    pub collecting: bool,
}

/// What `update_landing` changes; each `None` or empty list leaves it alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LandingUpdate {
    pub state: Option<LandingState>,
    pub base: Option<String>,
    pub title: Option<String>,
    pub lander: Option<Uuid>,
    /// A new pushed SHA. Without a `state`, an open, gating, green or red
    /// landing moves to `pushed`: CI on it hasn't run yet.
    pub sha: Option<String>,
    pub add_tickets: Vec<Uuid>,
    pub remove_tickets: Vec<Uuid>,
    /// Make the write only if the landing is still in this state with this
    /// pushed SHA, else refuse it as `landing_changed`.
    pub expect: Option<(LandingState, String)>,
}

fn invalid(what: &'static str) -> DomainError {
    DomainError::InvalidArgument { what }
}

const COLS: &str = "id, workspace_id, number, route, base, pushed_sha, state, state_since, lander_id, title, collecting, \
     actor, created_at, landed_at, resource_version";

fn row_to_landing(r: &Row) -> rusqlite::Result<Landing> {
    let route: String = r.get(3)?;
    let state: String = r.get(6)?;
    Ok(Landing {
        id: get_uuid(r, 0)?,
        workspace_id: get_uuid(r, 1)?,
        number: r.get::<_, i64>(2)?.clamp(0, u32::MAX as i64) as u32,
        route: Route::parse(&route).unwrap_or(Route::Ask),
        base: r.get(4)?,
        pushed_sha: r.get(5)?,
        // A word from a newer build reads as open: unsettled, so nothing
        // treats it as done.
        state: LandingState::parse(&state).unwrap_or(LandingState::Open),
        state_since: r.get(7)?,
        lander_id: get_optional_uuid(r, 8)?,
        title: r.get(9)?,
        collecting: r.get::<_, i64>(10)? != 0,
        actor: r.get(11)?,
        created_at: r.get(12)?,
        landed_at: r.get(13)?,
        resource_version: r.get::<_, i64>(14)?.max(0) as u64,
        tickets: Vec::new(),
    })
}

fn landing_in(conn: &Connection, id: Uuid) -> Result<Landing> {
    let mut landing = conn
        .query_row(&format!("SELECT {COLS} FROM landings WHERE id = ?1"), params![uuid_blob(id)], row_to_landing)
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)?;
    let mut stmt = conn
        .prepare("SELECT task_id FROM landing_tickets WHERE landing_id = ?1 ORDER BY added_at, task_id")
        .map_err(map_err)?;
    landing.tickets = stmt
        .query_map(params![uuid_blob(id)], |r| get_uuid(r, 0))
        .map_err(map_err)?
        .collect::<rusqlite::Result<_>>()
        .map_err(map_err)?;
    Ok(landing)
}

fn landings_where(conn: &Connection, filter: &str, id: Uuid) -> Result<Vec<Landing>> {
    let ids: Vec<Uuid> = {
        let mut stmt = conn
            .prepare(&format!("SELECT id FROM landings WHERE {filter} ORDER BY number, id"))
            .map_err(map_err)?;
        stmt.query_map(params![uuid_blob(id)], |r| get_uuid(r, 0))
            .map_err(map_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(map_err)?
    };
    ids.into_iter().map(|id| landing_in(conn, id)).collect()
}

/// The lander must be an open agent on the board that may land.
fn check_lander(conn: &Connection, workspace: Uuid, lander: Uuid) -> Result<()> {
    let agent = agent_in(conn, lander)?;
    if agent.workspace_id != workspace
        || agent.ended_at.is_some()
        || !matches!(agent.role, AgentRole::Lander | AgentRole::Orchestrator)
    {
        return Err(invalid("lander"));
    }
    Ok(())
}

/// Put `tickets` on `landing`. Each must be in the landing stage, or waiting
/// on you when the route is ask, and on no other open landing. A ticket that
/// shares a branch comes with its whole group, and one stacked on a ticket
/// that hasn't landed comes with that ticket.
fn add_tickets(conn: &Connection, landing: &Landing, tickets: &[Uuid], actor: Actor) -> Result<()> {
    check_tickets(conn, landing.workspace_id, tickets)?;
    let joining: HashSet<Uuid> = tickets.iter().chain(landing.tickets.iter()).copied().collect();
    for task in tickets {
        let line = line_in(conn, *task).map_err(|e| if matches!(e, DomainError::NotFound) { invalid("not_on_the_line") } else { e })?;
        let ready = line.stage == Stage::Landing || (line.stage == Stage::NeedsYou && landing.route == Route::Ask);
        if !ready {
            return Err(invalid("not_ready"));
        }
        let elsewhere: bool = conn
            .query_row(
                "SELECT 1 FROM landing_tickets lt JOIN landings l ON l.id = lt.landing_id
                  WHERE lt.task_id = ?1 AND l.id <> ?2 AND l.state NOT IN ('landed', 'dropped')",
                params![uuid_blob(*task), uuid_blob(landing.id)],
                |_| Ok(()),
            )
            .optional()
            .map_err(map_err)?
            .is_some();
        if elsewhere {
            return Err(invalid("on_a_landing"));
        }
        if let Some(parent) = line.stack_parent
            && !joining.contains(&parent)
            && line_in(conn, parent)?.stage != Stage::Landed
        {
            return Err(invalid("parent_not_landed"));
        }
        if let Some(group) = line.same_branch {
            let mut stmt = conn
                .prepare("SELECT task_id FROM ticket_lines WHERE same_branch = ?1 AND stage NOT IN ('landed', 'dropped')")
                .map_err(map_err)?;
            let members: Vec<Uuid> = stmt
                .query_map(params![uuid_blob(group)], |r| get_uuid(r, 0))
                .map_err(map_err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(map_err)?;
            if members.iter().any(|m| !joining.contains(m)) {
                return Err(invalid("same_branch"));
            }
        }
    }
    let now = now_millis();
    for task in tickets {
        let added = conn
            .execute(
                "INSERT OR IGNORE INTO landing_tickets (landing_id, task_id, added_at) VALUES (?1, ?2, ?3)",
                params![uuid_blob(landing.id), uuid_blob(*task), now],
            )
            .map_err(map_err)?;
        if line_in(conn, *task)?.stage == Stage::NeedsYou {
            move_stage(conn, *task, Stage::Landing, None, Some(landing.id), actor)?;
        }
        if added > 0 {
            let body = format!("{} went on Landing {}.", task_key(conn, *task), landing.number);
            let subject = LineSubject::TicketOnLanding { task: *task, landing: landing.id };
            line_event(conn, subject, "ticket_on", actor, &body, serde_json::json!({}))?;
        }
    }
    Ok(())
}

/// Land every ticket of `landing` at `sha`, each stacked parent before its
/// child.
fn land_tickets(conn: &Connection, landing: &Landing, sha: &str, actor: Actor) -> Result<()> {
    let mut left: Vec<Uuid> = landing.tickets.clone();
    while !left.is_empty() {
        let before = left.len();
        let mut next = Vec::new();
        for task in &left {
            let line = line_in(conn, *task)?;
            if line.stage.is_closed() {
                continue;
            }
            if line.stack_parent.is_some_and(|p| left.contains(&p) && line_in(conn, p).is_ok_and(|l| !l.stage.is_closed())) {
                next.push(*task);
                continue;
            }
            move_stage(conn, *task, Stage::Landed, Some(sha), Some(landing.id), actor)?;
        }
        if next.len() == before {
            return Err(invalid("stack_cycle"));
        }
        left = next;
    }
    Ok(())
}

impl Store {
    /// Start a landing on `workspace`'s board with `tickets`: one, normally.
    /// It is numbered after the board's last.
    pub fn start_landing(&self, workspace: Uuid, new: &NewLanding, tickets: &[Uuid], actor: Actor) -> Result<Landing> {
        let base = clean(&new.base, BASE_MAX, false, "base")?;
        let title = clean(&new.title, TITLE_MAX, false, "title")?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_exists(&tx, workspace)?;
        if let Some(lander) = new.lander {
            check_lander(&tx, workspace, lander)?;
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
            "INSERT INTO landings (id, workspace_id, number, route, base, state, state_since, lander_id, title, collecting,
                                   actor, created_at, resource_version)
             VALUES (?1, ?2, ?3, ?4, ?5, 'open', ?6, ?7, ?8, ?9, ?10, ?6, 1)",
            params![
                uuid_blob(id),
                uuid_blob(workspace),
                number,
                new.route.as_str(),
                base,
                now,
                new.lander.map(uuid_blob),
                title,
                new.collecting,
                actor.to_string(),
            ],
        )
        .map_err(map_err)?;
        line_event(&tx, LineSubject::Landing(id), "state", actor, "Started.", serde_json::json!({ "to": "open" }))?;
        let landing = landing_in(&tx, id)?;
        add_tickets(&tx, &landing, tickets, actor)?;
        let landing = landing_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(landing)
    }

    /// Change a landing: move it, record a pushed SHA, name its lander, and put
    /// tickets on or take them off, in one write. Landing it lands its tickets
    /// at its pushed SHA; dropping it leaves them in the landing stage, to land
    /// another way.
    pub fn update_landing(&self, id: Uuid, update: &LandingUpdate, actor: Actor) -> Result<Landing> {
        let base = update.base.as_deref().map(|v| clean(v, BASE_MAX, false, "base")).transpose()?;
        let title = update.title.as_deref().map(|v| clean(v, TITLE_MAX, false, "title")).transpose()?;
        let sha = update.sha.as_deref().map(|v| clean(v, SHA_MAX, true, "sha")).transpose()?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before = landing_in(&tx, id)?;
        if let Some((state, sha)) = &update.expect
            && (before.state != *state || before.pushed_sha.as_deref() != Some(sha.as_str()))
        {
            return Err(invalid("landing_changed"));
        }
        if before.state.is_settled() {
            return Err(invalid("landing_settled"));
        }
        if let Some(lander) = update.lander {
            check_lander(&tx, before.workspace_id, lander)?;
        }
        let state = match (update.state, &sha) {
            (Some(state), _) => state,
            (None, Some(_)) if before.state != LandingState::Pushed => LandingState::Pushed,
            (None, _) => before.state,
        };
        let pushed = sha.clone().or_else(|| before.pushed_sha.clone());
        if state == LandingState::Landed && pushed.is_none() {
            return Err(invalid("no_sha"));
        }
        let now = now_millis();
        tx.execute(
            "UPDATE landings SET state = ?2, state_since = CASE WHEN state = ?2 THEN state_since ELSE ?3 END,
                    pushed_sha = coalesce(?4, pushed_sha), base = coalesce(?5, base), title = coalesce(?6, title),
                    lander_id = coalesce(?7, lander_id),
                    landed_at = CASE WHEN ?2 = 'landed' THEN ?3 ELSE landed_at END,
                    collecting = CASE WHEN ?2 IN ('landed', 'dropped') THEN 0 ELSE collecting END,
                    resource_version = resource_version + 1
              WHERE id = ?1",
            params![uuid_blob(id), state.as_str(), now, sha, base, title, update.lander.map(uuid_blob)],
        )
        .map_err(map_err)?;
        if state != before.state {
            let extra = serde_json::json!({ "from": before.state.as_str(), "to": state.as_str() });
            line_event(&tx, LineSubject::Landing(id), "state", actor, &format!("Moved to {}.", state.as_str()), extra)?;
        }
        for task in &update.remove_tickets {
            let gone = tx
                .execute(
                    "DELETE FROM landing_tickets WHERE landing_id = ?1 AND task_id = ?2",
                    params![uuid_blob(id), uuid_blob(*task)],
                )
                .map_err(map_err)?;
            if gone > 0 {
                let body = format!("{} came off Landing {}.", task_key(&tx, *task), before.number);
                let subject = LineSubject::TicketOnLanding { task: *task, landing: id };
                line_event(&tx, subject, "ticket_off", actor, &body, serde_json::json!({}))?;
            }
        }
        if !update.add_tickets.is_empty() {
            let landing = landing_in(&tx, id)?;
            add_tickets(&tx, &landing, &update.add_tickets, actor)?;
        }
        let landing = landing_in(&tx, id)?;
        if state == LandingState::Landed {
            land_tickets(&tx, &landing, pushed.as_deref().unwrap_or_default(), actor)?;
        }
        tx.commit().map_err(map_err)?;
        Ok(landing)
    }

    /// A landing by id.
    pub fn landing(&self, id: Uuid) -> Result<Landing> {
        landing_in(&self.conn(), id)
    }

    /// Every landing on `workspace`'s board, by number.
    pub fn landings(&self, workspace: Uuid) -> Result<Vec<Landing>> {
        landings_where(&self.conn(), "workspace_id = ?1", workspace)
    }

    /// The open landing `task` is on, if any.
    pub fn landing_of(&self, task: Uuid) -> Result<Option<Landing>> {
        let conn = self.conn();
        Ok(landings_where(
            &conn,
            "state NOT IN ('landed', 'dropped') AND id IN (SELECT landing_id FROM landing_tickets WHERE task_id = ?1)",
            task,
        )?
        .into_iter()
        .next())
    }
}

#[cfg(test)]
#[path = "landings_tests.rs"]
mod tests;
