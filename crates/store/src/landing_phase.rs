//! What a landing is doing, in one word every surface reads (ov-503, ov-872).
//!
//! The engine's landing groups say it from their item's phase and step
//! (`workflow_overlay.rs`, ov-958). The old landings, from before the engine
//! landed, are history (ov-960): nothing moves them again, so each reads as
//! where it ended, from its stored state alone (`history`).
//!
//! | Stored state | Reads as | Why |
//! |---|---|---|
//! | landed | Landed | |
//! | dropped | Dropped | |
//! | pushed, green | Landed | its SHA reached the base; the CI it waited on is no longer followed |
//! | red | CI failed | its SHA reached the base, and CI on it failed |
//! | open, gating | Dropped | nothing was pushed, and nothing will push it now |

use crate::landings::{Landing, LandingState};

/// An agent with no move for this long is stalled.
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

/// A landing's phase, and since when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub phase: LandingPhase,
    pub since: i64,
}

/// An old landing's phase: where it ended, since its last move. The table in
/// the module docs.
pub fn history(landing: &Landing) -> Derived {
    let phase = match landing.state {
        LandingState::Landed | LandingState::Pushed | LandingState::Green => LandingPhase::Landed,
        LandingState::Red => LandingPhase::CiFailed,
        LandingState::Dropped | LandingState::Open | LandingState::Gating => LandingPhase::Dropped,
    };
    Derived { phase, since: landing.state_since }
}

#[cfg(test)]
#[path = "landing_phase_tests.rs"]
mod tests;
