//! Landings (ov-872): how finished tickets went into the product before the
//! workflow engine landed them.
//!
//! The old landing code is gone (ov-960). The engine lands now (its groups
//! draw as landings, `workflow_overlay.rs`), and the rows here are history:
//! nothing writes them, and every read takes each as where it ended
//! (`landing_phase::history`). `landing show` still prints one with its
//! timeline.
//!
//! Not to be confused with `landing.rs`, which keeps how each workspace lands
//! (`workspace_landing`).
//!
//! The tables are made by `line_migration.rs`.

use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{get_optional_uuid, get_uuid, uuid_blob};
use crate::store::Store;

/// Whether any landing was recorded for this workspace, so the board can refuse a delete that
/// would cascade into landing history without naming the landings table itself (ov-268).
pub(crate) fn has_history(conn: &Connection, workspace: Uuid) -> Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM landings WHERE workspace_id = ?1)", params![uuid_blob(workspace)], |r| r.get(0))
        .map_err(map_err)
}

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
    /// It was the board's collecting landing, which ready tickets joined.
    pub collecting: bool,
    /// Who started it, in `task_notes.actor`'s words.
    pub actor: String,
    pub created_at: i64,
    pub landed_at: Option<i64>,
    pub resource_version: u64,
    /// Its tickets, in the order they joined.
    pub tickets: Vec<Uuid>,
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

pub(crate) fn landing_in(conn: &Connection, id: Uuid) -> Result<Landing> {
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

/// "Landing 12", or the title the orchestrator wrote.
pub fn landing_title(landing: &Landing) -> String {
    if landing.title.is_empty() { format!("Landing {}", landing.number) } else { landing.title.clone() }
}

impl Store {
    /// A landing by id.
    pub fn landing(&self, id: Uuid) -> Result<Landing> {
        landing_in(&self.conn(), id)
    }

    /// A board's landing by its number.
    pub fn landing_numbered(&self, workspace: Uuid, number: u32) -> Result<Landing> {
        let conn = self.conn();
        let id: Uuid = conn
            .query_row(
                "SELECT id FROM landings WHERE workspace_id = ?1 AND number = ?2",
                params![uuid_blob(workspace), number],
                |r| get_uuid(r, 0),
            )
            .optional()
            .map_err(map_err)?
            .ok_or(DomainError::NotFound)?;
        landing_in(&conn, id)
    }

    /// Every landing on `workspace`'s board, by number.
    pub fn landings(&self, workspace: Uuid) -> Result<Vec<Landing>> {
        landings_where(&self.conn(), "workspace_id = ?1", workspace)
    }
}

impl Landing {
    /// The CI subject its pushed SHA is read under, once it has one.
    pub fn ci_subject(&self) -> Option<String> {
        self.pushed_sha.as_deref().map(crate::board_ci::sha_subject)
    }
}

/// A board's landings for the plan read, most recent move first: those that
/// moved at or after `since_ms`. Every one is history (ov-960), so an old
/// landing left pushed or open drops out of the read with the settled ones.
pub(crate) fn landings_for_plan(conn: &Connection, workspace: Uuid, since_ms: i64) -> Result<Vec<Landing>> {
    let ids: Vec<Uuid> = {
        let mut stmt = conn
            .prepare("SELECT id FROM landings WHERE workspace_id = ?1 AND state_since >= ?2 ORDER BY state_since DESC, id")
            .map_err(map_err)?;
        stmt.query_map(params![uuid_blob(workspace), since_ms], |r| get_uuid(r, 0))
            .map_err(map_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(map_err)?
    };
    ids.into_iter().map(|id| landing_in(conn, id)).collect()
}

#[cfg(test)]
#[path = "landings_tests.rs"]
mod tests;
