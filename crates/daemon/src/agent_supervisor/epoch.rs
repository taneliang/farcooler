//! Epochs that are never reused across daemon lives (ov-380).
//!
//! A new daemon numbers every transcript from zero again. An epoch that also
//! restarted at zero gave the new life the same epoch the old one had handed a
//! client, so that client kept a cursor into a stream that no longer existed.
//! The boot time is larger than any earlier life's. Never zero, which iOS
//! reads as "no session", and far below 2^53, so JSON and a Kotlin `Long`
//! carry it exactly.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// This daemon life's epoch, in Unix milliseconds at first use: what every
/// terminal with something to show reports.
pub(super) fn boot() -> u64 {
    static BOOT: OnceLock<u64> = OnceLock::new();
    *BOOT.get_or_init(|| {
        let millis = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        millis.max(1)
    })
}
