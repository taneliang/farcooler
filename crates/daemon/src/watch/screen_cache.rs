//! A screen is read when it moved, not once a second.
//!
//! The sampling loop classifies every shell pane by what is on its screen, and
//! reading a screen is a `capture-pane`: a process, per pane, per tick. An idle
//! pane's screen is the same string every time, so this keeps the last one and
//! hands it back for as long as tmux's own numbers say nothing happened: the
//! `ScreenStamp` that comes with the inventory's `list-panes`, which costs no
//! process. The classifier sees exactly what it would have seen, only without
//! asking tmux to say it again.
//!
//! A screen is taken as unchanged only when ALL of these hold.
//!
//! - The stamp is the same: the window's last-activity second, the scrollback
//!   length, the cursor and the program's pid.
//! - The geometry is the same, because a resize redraws without any output.
//! - The capture happened in a LATER second than the window's last activity.
//!   `window_activity` is whole seconds, so output in the same second as a
//!   capture may have come after it; the next tick reads again, and from then
//!   on the second is behind us.
//! - The capture is under `MAX_AGE` old. A backstop for whatever the three
//!   above cannot see (an activity bump tmux did not make): wrong for at most
//!   this long, and spread over `STAGGER` seconds by pane so a fleet does not
//!   re-read all at once.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use farcooler_core::Result;
use farcooler_core::inventory::{RuntimeSnapshot, ScreenStamp, TaggedPane};
use uuid::Uuid;

use crate::runtime::Runtime;

/// The longest a screen is reused, before the stagger.
const MAX_AGE: Duration = Duration::from_secs(20);

/// Seconds of spread, by the pane's number, added to `MAX_AGE`.
const STAGGER: u64 = 10;

/// What a capture was taken under.
pub(super) struct Taken {
    stamp: ScreenStamp,
    columns: u32,
    rows: u32,
    /// The Unix second the capture STARTED in.
    second: u64,
    at: Instant,
    screen: (String, u32, u32),
}

/// The last screen read for each terminal. A std mutex, held across a map
/// operation and never across an await.
#[derive(Default)]
pub(super) struct ScreenCache {
    taken: Mutex<HashMap<Uuid, Taken>>,
}

fn now_second() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `%12` is pane 12: a number to spread the backstop by.
fn stagger_of(pane: &TaggedPane) -> Duration {
    let n: u64 = pane.pane_id.trim_start_matches('%').parse().unwrap_or(0);
    Duration::from_secs(n % STAGGER)
}

impl Taken {
    /// Whether this capture still stands for `pane` as it is now.
    pub(super) fn stands_for(&self, pane: &TaggedPane, age_limit: Duration) -> bool {
        pane.stamp.unchanged_since(&self.stamp)
            && (pane.columns, pane.rows) == (self.columns, self.rows)
            && self.second > pane.stamp.activity
            && self.at.elapsed() < age_limit + stagger_of(pane)
    }
}

#[cfg(test)]
impl Taken {
    /// A capture of `pane` as it stood, started in Unix second `second` and just
    /// now finished.
    pub(super) fn for_test(pane: &TaggedPane, second: u64) -> Self {
        Self {
            stamp: pane.stamp,
            columns: pane.columns,
            rows: pane.rows,
            second,
            at: Instant::now(),
            screen: (String::new(), pane.columns, pane.rows),
        }
    }
}

#[cfg(test)]
impl ScreenCache {
    pub(super) fn hold(&self, id: Uuid, taken: Taken) {
        self.taken.lock().unwrap().insert(id, taken);
    }

    pub(super) fn holds(&self, id: Uuid) -> bool {
        self.taken.lock().unwrap().contains_key(&id)
    }
}

impl ScreenCache {
    /// The terminal's screen: the one already held while it stands, else a
    /// fresh `capture-pane`. Errors are not kept.
    pub(super) async fn read(&self, runtime: &Runtime, snapshot: &RuntimeSnapshot, id: Uuid) -> Result<(String, u32, u32)> {
        let pane = snapshot.claimants(id).into_iter().next().cloned();
        if let Some(pane) = &pane {
            let held = self.taken.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(taken) = held.get(&id).filter(|t| t.stands_for(pane, MAX_AGE)) {
                return Ok(taken.screen.clone());
            }
        }
        let second = now_second();
        let at = Instant::now();
        let screen = runtime.screen(id).await?;
        if let Some(pane) = pane {
            self.taken.lock().unwrap_or_else(|e| e.into_inner()).insert(
                id,
                Taken { stamp: pane.stamp, columns: pane.columns, rows: pane.rows, second, at, screen: screen.clone() },
            );
        }
        Ok(screen)
    }

    /// Forget the terminals that are gone, so one that comes back is read.
    pub(super) fn retain(&self, live: &HashSet<Uuid>) {
        self.taken.lock().unwrap_or_else(|e| e.into_inner()).retain(|id, _| live.contains(id));
    }
}
