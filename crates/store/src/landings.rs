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

/// Whether any landing was recorded for this workspace, so the board can refuse a delete that
/// would cascade into landing history without naming the landings table itself (ov-268).
pub(crate) fn has_history(conn: &Connection, workspace: Uuid) -> Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM landings WHERE workspace_id = ?1)", params![uuid_blob(workspace)], |r| r.get(0))
        .map_err(map_err)
}

const BASE_MAX: usize = 200;
const TITLE_MAX: usize = 80;

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

/// Refuse a landing write once the board has moved to the workflow engine
/// (`engine_lands`). The cutover freezes the landing tables as history until
/// the engine lands tickets itself (ov-947, ov-960), and the fence's own abort
/// would come back as a stale read, which no rereading fixes (ov-987).
fn check_legacy(conn: &Connection) -> Result<()> {
    if crate::engine_cutover::legacy_authority(conn) { Ok(()) } else { Err(invalid("engine_lands")) }
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

pub(crate) fn landings_where(conn: &Connection, filter: &str, id: Uuid) -> Result<Vec<Landing>> {
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

/// "Landing 12", or the title the orchestrator wrote.
pub fn landing_title(landing: &Landing) -> String {
    if landing.title.is_empty() { format!("Landing {}", landing.number) } else { landing.title.clone() }
}

/// The open landing other than `except` that `task` is on.
fn other_live_landing(conn: &Connection, task: Uuid, except: Uuid) -> Result<Option<Landing>> {
    let found: Option<Uuid> = conn
        .query_row(
            "SELECT l.id FROM landing_tickets lt JOIN landings l ON l.id = lt.landing_id
              WHERE lt.task_id = ?1 AND l.id <> ?2 AND l.state NOT IN ('landed', 'dropped')
              ORDER BY l.created_at DESC LIMIT 1",
            params![uuid_blob(task), uuid_blob(except)],
            |r| get_uuid(r, 0),
        )
        .optional()
        .map_err(map_err)?;
    found.map(|id| landing_in(conn, id)).transpose()
}

/// Take `task` off `landing`, with its timeline entries. `by_hand` marks it
/// held off, so the runner doesn't put it back on its own. True when it was on.
pub(crate) fn take_off(conn: &Connection, landing: &Landing, task: Uuid, by_hand: bool, actor: Actor) -> Result<bool> {
    let gone = conn
        .execute(
            "DELETE FROM landing_tickets WHERE landing_id = ?1 AND task_id = ?2",
            params![uuid_blob(landing.id), uuid_blob(task)],
        )
        .map_err(map_err)?;
    if gone == 0 {
        return Ok(false);
    }
    if by_hand {
        conn.execute(
            "UPDATE ticket_lines SET held_off = 1, resource_version = resource_version + 1 WHERE task_id = ?1",
            params![uuid_blob(task)],
        )
        .map_err(map_err)?;
    }
    let body = format!("{} came off {}.", task_key(conn, task), landing_title(landing));
    let subject = LineSubject::TicketOnLanding { task, landing: landing.id };
    line_event(conn, subject, "ticket_off", actor, &body, serde_json::json!({ "by_hand": by_hand }))?;
    Ok(true)
}

/// Every ticket in `tickets` must be ready to go on a landing: in the landing
/// stage, or waiting on you when the route is ask. One still in review is
/// `ticket_in_review` (the CLI's `--force` moves it on first), a finished one
/// `ticket_closed`, any other `not_ready`.
fn check_ready(conn: &Connection, landing: &Landing, tickets: &[Uuid]) -> Result<()> {
    for task in tickets {
        if landing.tickets.contains(task) {
            continue;
        }
        let line = line_in(conn, *task).map_err(|e| if matches!(e, DomainError::NotFound) { invalid("not_on_the_line") } else { e })?;
        match line.stage {
            Stage::Landing => {}
            Stage::NeedsYou if landing.route == Route::Ask => {}
            Stage::Reviewing => return Err(invalid("ticket_in_review")),
            s if s.is_closed() => return Err(invalid("ticket_closed")),
            _ => return Err(invalid("not_ready")),
        }
    }
    Ok(())
}

/// Put `tickets` on an open, unpushed `landing`. Each must be ready
/// (`check_ready`). One on another open, unpushed landing moves here; a gate
/// or pushed SHA fixes membership (`landing_moving`). A ticket that shares a
/// branch comes with its whole group, and one stacked on a ticket that hasn't
/// landed comes with that ticket.
pub(crate) fn add_tickets(conn: &Connection, landing: &Landing, tickets: &[Uuid], actor: Actor) -> Result<()> {
    if (landing.state != LandingState::Open || landing.pushed_sha.is_some())
        && tickets.iter().any(|t| !landing.tickets.contains(t))
    {
        return Err(invalid("landing_moving"));
    }
    check_tickets(conn, landing.workspace_id, tickets)?;
    check_ready(conn, landing, tickets)?;
    let joining: HashSet<Uuid> = tickets.iter().chain(landing.tickets.iter()).copied().collect();
    for task in tickets {
        let line = line_in(conn, *task)?;
        if let Some(other) = other_live_landing(conn, *task, landing.id)?
            && (other.state != LandingState::Open || other.pushed_sha.is_some())
        {
            return Err(invalid("landing_moving"));
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
        if let Some(other) = other_live_landing(conn, *task, landing.id)? {
            take_off(conn, &other, *task, false, actor)?;
            crate::collecting::drop_if_emptied(conn, other.id, actor)?;
        }
        let added = conn
            .execute(
                "INSERT OR IGNORE INTO landing_tickets (landing_id, task_id, added_at) VALUES (?1, ?2, ?3)",
                params![uuid_blob(landing.id), uuid_blob(*task), now],
            )
            .map_err(map_err)?;
        conn.execute(
            "UPDATE ticket_lines SET held_off = 0, resource_version = resource_version + 1 WHERE task_id = ?1",
            params![uuid_blob(*task)],
        )
        .map_err(map_err)?;
        if line_in(conn, *task)?.stage == Stage::NeedsYou {
            move_stage(conn, *task, Stage::Landing, None, Some(landing.id), actor)?;
        }
        if added > 0 {
            let body = format!("{} went on {}.", task_key(conn, *task), landing_title(landing));
            let subject = LineSubject::TicketOnLanding { task: *task, landing: landing.id };
            line_event(conn, subject, "ticket_on", actor, &body, serde_json::json!({}))?;
        }
    }
    Ok(())
}

/// A landing landed: land every ticket on it that was ready (landing, or in
/// review), each stacked parent before its child, at the landing's SHA, unless
/// the workspace lands by pull request, where the landing was a rehearsal never
/// pushed. A ticket on it that never was ready (a stray, or one sent back for a
/// fix) comes off and keeps its stage: a landing says nothing about work it
/// didn't carry.
fn land_tickets(conn: &Connection, landing: &Landing, actor: Actor) -> Result<()> {
    let rehearsal = crate::landing::read(conn, landing.workspace_id)?.mode == Some(crate::landing::LandingMode::PullRequests);
    let sha = if rehearsal { None } else { landing.pushed_sha.as_deref() };
    let mut left = Vec::new();
    for task in &landing.tickets {
        let line = line_in(conn, *task)?;
        match line.stage {
            Stage::Landing | Stage::Reviewing => left.push(*task),
            s if s.is_closed() => {}
            _ => {
                conn.execute(
                    "DELETE FROM landing_tickets WHERE landing_id = ?1 AND task_id = ?2",
                    params![uuid_blob(landing.id), uuid_blob(*task)],
                )
                .map_err(map_err)?;
                let body = format!("Taken off {}, which landed without it.", landing_title(landing));
                let subject = LineSubject::TicketOnLanding { task: *task, landing: landing.id };
                line_event(conn, subject, "ticket_off", actor, &body, serde_json::json!({ "landed_without": true }))?;
            }
        }
    }
    while !left.is_empty() {
        let before = left.len();
        let mut next = Vec::new();
        for task in &left {
            let line = line_in(conn, *task)?;
            if line.stack_parent.is_some_and(|p| left.contains(&p) && line_in(conn, p).is_ok_and(|l| !l.stage.is_closed())) {
                next.push(*task);
                continue;
            }
            move_stage(conn, *task, Stage::Landed, sha, Some(landing.id), actor)?;
        }
        if next.len() == before {
            return Err(invalid("stack_cycle"));
        }
        left = next;
    }
    Ok(())
}

/// Release every ticket of a dropped landing, so none still says it is on it.
fn release_tickets(conn: &Connection, landing: &Landing, actor: Actor) -> Result<()> {
    for task in &landing.tickets {
        take_off(conn, landing, *task, false, actor)?;
    }
    Ok(())
}

/// What a person reads for `state`, in a landing's timeline.
fn state_said(state: LandingState) -> &'static str {
    match state {
        LandingState::Open => "Open.",
        LandingState::Gating => "Gating.",
        LandingState::Pushed => "Pushed.",
        LandingState::Green => "CI passed.",
        LandingState::Red => "CI failed.",
        LandingState::Landed => "Landed.",
        LandingState::Dropped => "Dropped.",
    }
}

/// Move `landing` to `to` and write it down, with `extra` merged into the
/// event's `from` and `to`.
pub(crate) fn move_landing(conn: &Connection, landing: &Landing, to: LandingState, actor: Actor, extra: serde_json::Value) -> Result<()> {
    let now = now_millis();
    conn.execute(
        "UPDATE landings SET state = ?2, state_since = ?3,
                landed_at = CASE WHEN ?2 = 'landed' THEN coalesce(landed_at, ?3) ELSE landed_at END,
                collecting = CASE WHEN ?2 = 'open' THEN collecting ELSE 0 END,
                resource_version = resource_version + 1
          WHERE id = ?1",
        params![uuid_blob(landing.id), to.as_str(), now],
    )
    .map_err(map_err)?;
    let mut said = serde_json::json!({ "from": landing.state.as_str(), "to": to.as_str() });
    if let (Some(said), Some(more)) = (said.as_object_mut(), extra.as_object()) {
        said.extend(more.clone());
    }
    line_event(conn, LineSubject::Landing(landing.id), "state", actor, state_said(to), said)
}

impl Store {
    /// Start a landing on `workspace`'s board with `tickets`: one, normally.
    ///
    /// The board always has a collecting landing (`collecting.rs`). A landing
    /// started by hand adopts it: the same landing, with its number, takes the
    /// route, base, title and lander, carries exactly the tickets listed, and
    /// stops collecting. Tickets it held that weren't listed move to a fresh
    /// collecting landing behind it.
    pub fn start_landing(&self, workspace: Uuid, new: &NewLanding, tickets: &[Uuid], actor: Actor) -> Result<Landing> {
        let base = clean(&new.base, BASE_MAX, false, "base")?;
        let title = clean(&new.title, TITLE_MAX, false, "title")?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        check_legacy(&tx)?;
        board_exists(&tx, workspace)?;
        if let Some(lander) = new.lander {
            check_lander(&tx, workspace, lander)?;
        }
        let now = now_millis();
        let id = match crate::collecting::collecting(&tx, workspace)? {
            Some(open) => {
                tx.execute(
                    "UPDATE landings SET route = ?2, base = ?3, title = ?4, lander_id = ?5, collecting = ?6, actor = ?7,
                            resource_version = resource_version + 1
                      WHERE id = ?1",
                    params![
                        uuid_blob(open.id),
                        new.route.as_str(),
                        base,
                        title,
                        new.lander.map(uuid_blob),
                        new.collecting,
                        actor.to_string()
                    ],
                )
                .map_err(map_err)?;
                let unlisted: Vec<Uuid> = open.tickets.iter().filter(|t| !tickets.contains(t)).copied().collect();
                if !unlisted.is_empty() {
                    for task in &unlisted {
                        take_off(&tx, &open, *task, false, Actor::Runner)?;
                    }
                    let fresh = crate::collecting::ensure_collecting(&tx, workspace, Actor::Runner)?;
                    add_tickets(&tx, &fresh, &unlisted, Actor::Runner)?;
                }
                open.id
            }
            None => {
                let number: i64 = tx
                    .query_row(
                        "SELECT coalesce(max(number), 0) + 1 FROM landings WHERE workspace_id = ?1",
                        params![uuid_blob(workspace)],
                        |r| r.get(0),
                    )
                    .map_err(map_err)?;
                let id = Uuid::now_v7();
                tx.execute(
                    "INSERT INTO landings (id, workspace_id, number, route, base, state, state_since, lander_id, title,
                                           collecting, actor, created_at, resource_version)
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
                line_event(&tx, LineSubject::Landing(id), "state", actor, "Open.", serde_json::json!({ "to": "open" }))?;
                id
            }
        };
        let landing = landing_in(&tx, id)?;
        add_tickets(&tx, &landing, tickets, actor)?;
        if !new.collecting {
            crate::collecting::ensure_collecting(&tx, workspace, Actor::Runner)?;
        }
        let landing = landing_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(landing)
    }

    /// Change a landing: move it, rebase it, record what it pushed, name its
    /// lander, and put tickets on or take them off, in one write.
    ///
    /// A settled landing (landed or dropped) takes no more writes. A ticket
    /// joins or comes off only while the landing is open and has no pushed
    /// SHA: from gating on, its gate or push carries exactly those tickets
    /// (`landing_moving`). Landing it lands its tickets; dropping it lets
    /// them go, to land another way. Moving it on,
    /// or naming a lander, stops it collecting, and the board's collecting
    /// landing then departs if it can.
    pub fn update_landing(&self, id: Uuid, update: &LandingUpdate, actor: Actor) -> Result<Landing> {
        self.update_landing_with_membership(id, update, actor, None)
    }

    /// Record a job's pushed SHA only for the tickets it routed and gated.
    /// Membership can change if a caller reopens an unpushed landing during
    /// fetch or push. Check in the same transaction that exposes the SHA to
    /// the CI watch; after this write, even reopening cannot add tickets.
    pub fn record_landing_push(&self, id: Uuid, sha: &str, tickets: &[Uuid]) -> Result<Landing> {
        let update = LandingUpdate { sha: Some(sha.into()), ..Default::default() };
        self.update_landing_with_membership(id, &update, Actor::Runner, Some(tickets))
    }

    fn update_landing_with_membership(&self, id: Uuid, update: &LandingUpdate, actor: Actor, tickets: Option<&[Uuid]>) -> Result<Landing> {
        let base = update.base.as_deref().map(|v| clean(v, BASE_MAX, false, "base")).transpose()?;
        let title = update.title.as_deref().map(|v| clean(v, TITLE_MAX, false, "title")).transpose()?;
        let sha = update.sha.as_deref().map(clean_sha).transpose()?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        check_legacy(&tx)?;
        let before = landing_in(&tx, id)?;
        if tickets.is_some_and(|tickets| tickets != before.tickets.as_slice()) {
            return Err(invalid("landing_changed"));
        }
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
        check_tickets(&tx, before.workspace_id, &update.remove_tickets)?;
        if (before.state != LandingState::Open || before.pushed_sha.is_some())
            && update.remove_tickets.iter().any(|t| before.tickets.contains(t))
        {
            return Err(invalid("landing_moving"));
        }
        let state = match (update.state, &sha) {
            (Some(state), _) => state,
            (None, Some(_)) => LandingState::Pushed,
            (None, None) => before.state,
        };
        let pushed = sha.clone().or_else(|| before.pushed_sha.clone());
        if matches!(state, LandingState::Pushed | LandingState::Green | LandingState::Red) && pushed.is_none() {
            return Err(invalid("sha"));
        }
        if state == LandingState::Landed && pushed.is_none() {
            return Err(invalid("no_sha"));
        }
        tx.execute(
            "UPDATE landings SET pushed_sha = coalesce(?2, pushed_sha), base = coalesce(?3, base), title = coalesce(?4, title),
                    lander_id = coalesce(?5, lander_id),
                    collecting = CASE WHEN ?5 IS NOT NULL THEN 0 ELSE collecting END,
                    resource_version = resource_version + 1
              WHERE id = ?1",
            params![uuid_blob(id), sha, base, title, update.lander.map(uuid_blob)],
        )
        .map_err(map_err)?;
        for task in &update.remove_tickets {
            if take_off(&tx, &before, *task, !update.add_tickets.contains(task), actor)? {
                crate::collecting::drop_if_emptied(&tx, id, actor)?;
            }
        }
        if !update.add_tickets.is_empty() {
            let landing = landing_in(&tx, id)?;
            add_tickets(&tx, &landing, &update.add_tickets, actor)?;
        }
        if state != before.state {
            let now_on = landing_in(&tx, id)?;
            move_landing(&tx, &now_on, state, actor, serde_json::json!({}))?;
        }
        let landing = landing_in(&tx, id)?;
        match (state != before.state, state) {
            (true, LandingState::Landed) => land_tickets(&tx, &landing, actor)?,
            (true, LandingState::Dropped) => release_tickets(&tx, &landing, actor)?,
            _ => {}
        }
        crate::collecting::depart(&tx, before.workspace_id, Actor::Runner)?;
        crate::collecting::ensure_collecting(&tx, before.workspace_id, Actor::Runner)?;
        let landing = landing_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(landing)
    }

    /// Write one step of the runner's landing job on `landing`'s timeline
    /// (ov-872): what `landing show` reads back as its progress. `step` is a
    /// word a client can sort by (`fetched`, `gated`, `pushed`, `stopped`).
    pub fn note_landing(&self, id: Uuid, step: &str, body: &str, extra: serde_json::Value) -> Result<()> {
        let conn = self.conn();
        landing_in(&conn, id)?;
        let mut said = serde_json::json!({ "step": step });
        if let (Some(said), Some(more)) = (said.as_object_mut(), extra.as_object()) {
            said.extend(more.clone());
        }
        line_event(&conn, LineSubject::Landing(id), "job", Actor::Runner, body, said)
    }

    /// The charter asks before `landing` lands (the ask route): it takes the
    /// ask route, and each ticket on it ready to land waits on you, still on
    /// it, with `reason`. Only an open landing; a ticket isn't taken off.
    pub fn hold_for_you(&self, id: Uuid, reason: &str, actor: Actor) -> Result<Landing> {
        let reason = clean(reason, crate::ticket_lines::LINE_MAX, false, "reason")?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        check_legacy(&tx)?;
        let landing = landing_in(&tx, id)?;
        if landing.state != LandingState::Open {
            return Err(invalid(if landing.state.is_settled() { "landing_settled" } else { "landing_moving" }));
        }
        tx.execute(
            "UPDATE landings SET route = 'ask', collecting = 0, resource_version = resource_version + 1 WHERE id = ?1",
            params![uuid_blob(id)],
        )
        .map_err(map_err)?;
        for task in &landing.tickets {
            if line_in(&tx, *task)?.stage != Stage::Landing {
                continue;
            }
            tx.execute(
                "UPDATE ticket_lines SET reason = ?2, resource_version = resource_version + 1 WHERE task_id = ?1",
                params![uuid_blob(*task), reason],
            )
            .map_err(map_err)?;
            move_stage(&tx, *task, Stage::NeedsYou, None, Some(id), actor)?;
        }
        line_event(&tx, LineSubject::Landing(id), "job", actor, &reason, serde_json::json!({ "step": "asked" }))?;
        let landing = landing_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(landing)
    }

    /// You said to land `landing`, which the charter asked about: it takes the
    /// push route, and each ticket on it waiting on you is ready to land
    /// again. `not_asked` for a landing that isn't on the ask route.
    pub fn land_asked(&self, id: Uuid, actor: Actor) -> Result<Landing> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        check_legacy(&tx)?;
        let landing = landing_in(&tx, id)?;
        if landing.state.is_settled() {
            return Err(invalid("landing_settled"));
        }
        if landing.route != Route::Ask {
            return Err(invalid("not_asked"));
        }
        tx.execute(
            "UPDATE landings SET route = 'push', resource_version = resource_version + 1 WHERE id = ?1",
            params![uuid_blob(id)],
        )
        .map_err(map_err)?;
        for task in &landing.tickets {
            if line_in(&tx, *task)?.stage == Stage::NeedsYou {
                move_stage(&tx, *task, Stage::Landing, None, Some(id), actor)?;
            }
        }
        line_event(&tx, LineSubject::Landing(id), "job", actor, "Told to land.", serde_json::json!({ "step": "told_to_land" }))?;
        let landing = landing_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(landing)
    }

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

    /// The open landing `task` is on, if any.
    pub fn landing_of(&self, task: Uuid) -> Result<Option<Landing>> {
        landing_of_in(&self.conn(), task)
    }

    /// Every board's landings whose CI the runner reads: pushed, green or red,
    /// with a SHA. What the runner's CI watch walks (`daemon/src/ci_watch.rs`).
    pub fn landings_following_ci(&self) -> Result<Vec<Landing>> {
        let conn = self.conn();
        let ids: Vec<Uuid> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM landings WHERE state IN ('pushed', 'green', 'red') AND pushed_sha IS NOT NULL
                      ORDER BY created_at, id",
                )
                .map_err(map_err)?;
            stmt.query_map([], |r| get_uuid(r, 0))
                .map_err(map_err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(map_err)?
        };
        ids.into_iter().map(|id| landing_in(&conn, id)).collect()
    }
}

/// The open landing `task` is on, read on `conn`.
pub(crate) fn landing_of_in(conn: &Connection, task: Uuid) -> Result<Option<Landing>> {
    Ok(landings_where(
        conn,
        "state NOT IN ('landed', 'dropped') AND id IN (SELECT landing_id FROM landing_tickets WHERE task_id = ?1)",
        task,
    )?
    .into_iter()
    .next())
}

/// A SHA as given, lowercased: 7 to 40 hex digits.
pub fn clean_sha(value: &str) -> Result<String> {
    let sha = value.trim().to_ascii_lowercase();
    if !(7..=40).contains(&sha.len()) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("sha"));
    }
    Ok(sha)
}

impl Landing {
    /// The CI subject its pushed SHA is read under, once it has one.
    pub fn ci_subject(&self) -> Option<String> {
        self.pushed_sha.as_deref().map(crate::board_ci::sha_subject)
    }

    /// Whether the runner moves it with its CI: once a SHA is out and until it
    /// settles.
    pub fn follows_ci(&self) -> bool {
        matches!(self.state, LandingState::Pushed | LandingState::Green | LandingState::Red)
    }
}

/// Move `workspace`'s landings that follow CI on `subject` to what it now
/// says: green when it passed, red when it failed, pushed while it runs again.
/// The landings moved, by id.
pub(crate) fn follow_ci(conn: &Connection, workspace: Uuid, subject: &str, status: crate::board_ci::CiStatus) -> Result<Vec<Uuid>> {
    use crate::board_ci::CiStatus;
    let to = match status {
        CiStatus::Passed => LandingState::Green,
        CiStatus::Failed => LandingState::Red,
        CiStatus::Running | CiStatus::Queued => LandingState::Pushed,
        // Nothing ran yet, GitHub couldn't be asked, or a newer push canceled
        // its run: the landing stays put.
        CiStatus::None | CiStatus::Unknown | CiStatus::Superseded => return Ok(Vec::new()),
    };
    let landings = landings_where(
        conn,
        "workspace_id = ?1 AND state IN ('pushed', 'green', 'red') AND pushed_sha IS NOT NULL",
        workspace,
    )?;
    let mut moved = Vec::new();
    for landing in landings.into_iter().filter(|l| l.ci_subject().as_deref() == Some(subject) && l.state != to) {
        move_landing(conn, &landing, to, Actor::Runner, serde_json::json!({}))?;
        moved.push(landing.id);
    }
    Ok(moved)
}

/// A board's landings for the plan read: every one not settled, oldest first,
/// then those settled at or after `settled_since_ms`, most recent first.
pub(crate) fn landings_for_plan(conn: &Connection, workspace: Uuid, settled_since_ms: i64) -> Result<Vec<Landing>> {
    let ids: Vec<Uuid> = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM landings
                  WHERE workspace_id = ?1 AND (state NOT IN ('landed', 'dropped') OR state_since >= ?2)
                  ORDER BY state IN ('landed', 'dropped'),
                           CASE WHEN state IN ('landed', 'dropped') THEN -state_since ELSE created_at END, id",
            )
            .map_err(map_err)?;
        stmt.query_map(params![uuid_blob(workspace), settled_since_ms], |r| get_uuid(r, 0))
            .map_err(map_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(map_err)?
    };
    ids.into_iter().map(|id| landing_in(conn, id)).collect()
}

#[cfg(test)]
#[path = "landings_tests.rs"]
mod tests;
