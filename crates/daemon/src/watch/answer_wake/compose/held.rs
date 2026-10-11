//! A message composed while claude can't be typed to safely yet, held here
//! and typed once it can (ov-468).
//!
//! claude takes a message mid-turn into a queue of its own, but the runner
//! presses that Enter only under `mid_turn`'s checks: no call in flight, no
//! dialog announced, a turn boundary seen since the daemon started. A send
//! that fails one of them, or that is a command, which waits for the turn to
//! end, used to be refused as `busy`. Now it's answered Queued, as claude's
//! own queue would answer it, and held in order behind any message already
//! held for that pane. Every `RETRY` the oldest goes through the whole gate
//! again (`compose_checked`), and when it passes it's typed: into claude's
//! queue mid-turn, or as the next prompt. The view's Queued echo stays until
//! the transcript shows it (`NativePaneModel.settleQueued`).
//!
//! A held message that the gate refuses for any other reason (a draft left
//! in the box, the agent gone), or that waits past `GIVE_UP`, is dropped
//! with a warning, its files with it. Held messages live in memory: a
//! daemon restart drops them. codex is never held; it refuses mid-turn as
//! before (`compose::codex`).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use farcooler_core::{DomainError, Result};
use uuid::Uuid;

use super::super::Turn;
use crate::watch::Watcher;

/// What `compose_locked` refuses with for a send that should wait rather
/// than be refused. Never sent to a client: `compose_or_hold` holds it.
pub(super) const HOLD: &str = "hold";

/// How often the oldest held message tries the gate again.
const RETRY: Duration = Duration::from_millis(500);

/// How long a message is held before it's dropped: a long turn's worth, as
/// a message between agents waits (`messages::GIVE_UP_AFTER_MS`).
const GIVE_UP: Duration = Duration::from_secs(4 * 60 * 60);

/// One held message, as the request carried it.
pub(crate) struct Waiting {
    raw: String,
    images: Vec<(String, Vec<u8>)>,
    files: Vec<PathBuf>,
    since: Instant,
}

/// Each pane's held messages, oldest first. A pane is in the map while its
/// messages are being told, the one being tried included, so a send that
/// arrives meanwhile goes behind them.
pub(crate) type HeldComposes = std::sync::Mutex<HashMap<Uuid, VecDeque<Waiting>>>;

impl Watcher {
    /// `compose_checked`, or, when it says to wait or messages are already
    /// held for `id`, the message held and answered Queued (`Turn::During`).
    pub(super) async fn compose_or_hold(&self, id: Uuid, raw: &str, images: &[(String, Vec<u8>)], files: &[PathBuf]) -> Result<Turn> {
        if self.holds_composed(id) {
            self.compose_request(id, raw, images, files)?;
            self.hold_composed(id, raw, images, files);
            return Ok(Turn::During);
        }
        match self.compose_checked(id, raw, images, files).await {
            Err(DomainError::Conflict { what: HOLD }) => {
                self.hold_composed(id, raw, images, files);
                Ok(Turn::During)
            }
            other => other,
        }
    }

    /// Whether messages are held for `id`.
    fn holds_composed(&self, id: Uuid) -> bool {
        self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&id)
    }

    /// Hold a message for `id`, behind any held already, and start telling
    /// them when nothing is.
    fn hold_composed(&self, id: Uuid, raw: &str, images: &[(String, Vec<u8>)], files: &[PathBuf]) {
        let waiting = Waiting { raw: raw.to_string(), images: images.to_vec(), files: files.to_vec(), since: Instant::now() };
        let first = {
            let mut held = self.composed_held.lock().unwrap_or_else(|e| e.into_inner());
            let first = !held.contains_key(&id);
            held.entry(id).or_default().push_back(waiting);
            first
        };
        // A pane already in the map has its teller running, perhaps trying
        // a message it took off the line, which finds this one after.
        if first && let Some(me) = self.me.upgrade() {
            tokio::spawn(tell_held(me, id));
        }
    }

    /// The oldest message held for `id`, taken off the line, or `None` with
    /// the pane gone from the map when there's none.
    fn next_held(&self, id: Uuid) -> Option<Waiting> {
        let mut held = self.composed_held.lock().unwrap_or_else(|e| e.into_inner());
        let next = held.get_mut(&id).and_then(VecDeque::pop_front);
        if next.is_none() {
            held.remove(&id);
        }
        next
    }

    /// Put `waiting` back at the head of `id`'s line, to try again.
    fn hold_again(&self, id: Uuid, waiting: Waiting) {
        self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).entry(id).or_default().push_front(waiting);
    }
}

/// Tell `id`'s held messages in order, each once the gate lets it through,
/// until none is left. Once the last is told the pane leaves the map at
/// once, so a send right after it isn't held behind nothing.
async fn tell_held(me: Arc<Watcher>, id: Uuid) {
    loop {
        tokio::time::sleep(RETRY).await;
        loop {
            let Some(waiting) = me.next_held(id) else { return };
            match me.compose_checked(id, &waiting.raw, &waiting.images, &waiting.files).await {
                Err(DomainError::Conflict { what: HOLD }) if waiting.since.elapsed() < GIVE_UP => {
                    me.hold_again(id, waiting);
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(terminal = %id, error = %e, "a held message wasn't sent");
                    if !matches!(e, DomainError::Conflict { what: "unconfirmed" }) {
                        for file in &waiting.files {
                            let _ = std::fs::remove_file(file);
                        }
                    }
                }
            }
        }
    }
}
