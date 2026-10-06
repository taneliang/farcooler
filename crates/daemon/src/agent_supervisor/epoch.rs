//! Epochs that are never reused, across toggles or across daemon lives (ov-380).
//!
//! A shim survives a daemon restart and reconnects to the new daemon, which
//! numbers its transcript from zero again. A counter that also restarted at
//! zero gave the new life the same epoch the old one had handed a client, so
//! that client kept a cursor into a stream that no longer existed. Seeding from
//! the boot time makes a new life's epochs larger than any an earlier life
//! issued: a restart takes far longer than the handful of toggles one life
//! counts. Never zero, which iOS reads as "no session", and far below 2^53, so
//! JSON and a Kotlin `Long` carry it exactly.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// This daemon life's first epoch, in Unix milliseconds at first use. Also what
/// a terminal with no shim of its own reports: a hook-fed transcript never
/// establishes anything, and its epoch used to stay 0 for every life.
pub(super) fn boot() -> u64 {
    static BOOT: OnceLock<u64> = OnceLock::new();
    *BOOT.get_or_init(|| {
        let millis = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        millis.max(1)
    })
}

// A clock stepped backwards between two daemon lives could seed a later life
// below an earlier one's epochs, and a client that held one of those would be
// matched into the wrong stream. It takes a step back larger than the epochs
// the earlier life issued, which count toggles; negligible, and not guarded.

/// The epoch after `current`: one past it, and never below this life's start.
pub(super) fn next(current: u64) -> u64 {
    next_after(current, boot())
}

/// The smallest epoch a terminal is ever given: zero means no session.
pub(super) const FIRST_ALLOWED: u64 = 1;

/// `next` for a life that started at `boot`.
pub(super) fn next_after(current: u64, boot: u64) -> u64 {
    current.max(boot).max(FIRST_ALLOWED) + 1
}
