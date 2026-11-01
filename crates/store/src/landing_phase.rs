//! What a landing is doing, in one word every surface reads (ov-503, ov-872).
//!
//! A landing's stored `state` says little on its own: an open landing could be
//! waiting on a ticket, working with a lander, or forgotten. The **phase** is
//! derived on the runner, on every plan read, from the state, the tickets on
//! the landing, its lander and the clock, so the Mac, the phones and the CLI
//! agree. Nothing here is stored.
//!
//! | Phase | When |
//! |---|---|
//! | Waiting for the earlier landing | the collecting landing, while a landing ahead of it is still on its way |
//! | Waiting for main | the collecting landing with a ticket ready, nothing ahead and main's CI red |
//! | Waiting for tickets | the collecting landing with nothing ahead and no ticket ready, or a ticket on a landing started by hand isn't ready |
//! | Integrating | its tickets are ready and no SHA is out |
//! | Gating locally | stored `gating` |
//! | Waiting for CI | stored `pushed` |
//! | CI failed | stored `red` |
//! | Ready to land | stored `green`, not yet landed |
//! | Landed, Dropped | stored `landed`, `dropped` |
//! | Stalled | open or gating for 2 h with no open lander |
//!
//! **Never held (ov-523).** A landing isn't held for a ticket: one that leaves
//! the landing stage comes off an unpushed landing in the same write
//! (`collecting.rs`).

use rusqlite::{Connection, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::landings::LandingState;
use crate::models::uuid_blob;
use crate::ticket_lines::Stage;

/// A landing with no open lander and no move for this long is stalled.
pub const STALLED_AFTER_MS: i64 = 2 * 60 * 60_000;

/// What a landing is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandingPhase {
    /// The collecting landing, while the landing ahead is still on its way.
    WaitingForPreviousLanding,
    /// The collecting landing, with a ticket ready and nothing ahead, while
    /// main's CI is red.
    WaitingForMain,
    WaitingForTickets,
    Integrating,
    GatingLocally,
    WaitingForCi,
    CiFailed,
    /// Green, and not (yet) landed: a pull-request workspace's landing, which
    /// the orchestrator lands by hand.
    ReadyToLand,
    Landed,
    Stalled,
    Dropped,
}

/// A ticket on a landing, as the phase reads it.
#[derive(Debug, Clone, Copy)]
pub struct TicketOn {
    pub task: Uuid,
    pub stage: Stage,
    pub stage_since: i64,
    /// When it last left the landing stage, if it has: the moment a ticket
    /// that blocks the landing stopped being ready, which its later steps
    /// (building to review and back) do not move.
    pub left_ready_at: Option<i64>,
}

/// What the phase of a landing is read from.
#[derive(Debug, Clone)]
pub struct PhaseInput<'a> {
    pub state: LandingState,
    pub state_since: i64,
    /// A lander is recorded and hasn't finished.
    pub lander_open: bool,
    pub lander_started_at: Option<i64>,
    /// When a ticket was last put on the landing.
    pub last_ticket_on: Option<i64>,
    pub tickets: &'a [TicketOn],
    /// The board's collecting landing, not one started by hand.
    pub collecting: bool,
    /// Main's CI reads failed.
    pub main_red: bool,
    /// Another landing on the board is ahead of the collecting one.
    pub ahead: bool,
    pub now: i64,
}

/// A landing's phase, and since when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub phase: LandingPhase,
    pub since: i64,
}

/// The phase table in the module docs, as one function of its inputs.
pub fn derive(i: &PhaseInput) -> Derived {
    let settled = |phase| Derived { phase, since: i.state_since };
    match i.state {
        LandingState::Landed => return settled(LandingPhase::Landed),
        LandingState::Dropped => return settled(LandingPhase::Dropped),
        LandingState::Pushed => return settled(LandingPhase::WaitingForCi),
        LandingState::Red => return settled(LandingPhase::CiFailed),
        LandingState::Green => return settled(LandingPhase::ReadyToLand),
        LandingState::Open | LandingState::Gating => {}
    }
    let open: Vec<&TicketOn> = i.tickets.iter().filter(|t| !t.stage.is_closed()).collect();
    let blockers: Vec<&TicketOn> = open.iter().copied().filter(|t| t.stage != Stage::Landing).collect();
    // The clock starts at the last move of the landing itself or of its
    // membership, and no earlier than the first moment a ticket still blocking
    // it stopped being ready.
    let floor = i.state_since.max(i.last_ticket_on.unwrap_or(0));
    // The collecting landing waits behind the one ahead, or for a ticket to
    // depart with. It is waiting by design, so it is never stalled.
    if i.collecting && i.state == LandingState::Open && !i.lander_open {
        let ready = open.iter().any(|t| t.stage == Stage::Landing);
        let phase = match (i.ahead, ready && i.main_red) {
            (true, _) => LandingPhase::WaitingForPreviousLanding,
            (false, true) => LandingPhase::WaitingForMain,
            (false, false) => LandingPhase::WaitingForTickets,
        };
        return Derived { phase, since: floor.max(open.iter().map(|t| t.stage_since).max().unwrap_or(0)) };
    }
    let (phase, since) = if i.state == LandingState::Gating {
        (LandingPhase::GatingLocally, i.state_since)
    } else if !blockers.is_empty() {
        let oldest = blockers.iter().map(|t| t.left_ready_at.unwrap_or(t.stage_since)).min().unwrap_or(floor);
        (LandingPhase::WaitingForTickets, floor.max(oldest))
    } else {
        let started = if i.lander_open { i.lander_started_at.unwrap_or(0) } else { 0 };
        let latest_ready = open.iter().map(|t| t.stage_since).max().unwrap_or(0);
        (LandingPhase::Integrating, floor.max(started).max(latest_ready))
    };
    let stalled = !i.lander_open && i.now - since >= STALLED_AFTER_MS;
    Derived { phase: if stalled { LandingPhase::Stalled } else { phase }, since }
}

/// When `task` last left the landing stage, from its own timeline.
pub(crate) fn left_ready_at(conn: &Connection, task: Uuid) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT max(at) FROM line_events WHERE task_id = ?1 AND kind = 'stage' AND json_extract(extra, '$.from') = 'landing'",
        params![uuid_blob(task)],
        |r| r.get(0),
    )
    .map_err(map_err)
}

/// When a ticket was last put on `landing`.
pub(crate) fn last_ticket_on(conn: &Connection, landing: Uuid) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT max(at) FROM line_events WHERE landing_id = ?1 AND kind = 'ticket_on'",
        params![uuid_blob(landing)],
        |r| r.get(0),
    )
    .map_err(map_err)
}

#[cfg(test)]
#[path = "landing_phase_tests.rs"]
mod tests;
