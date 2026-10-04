//! Letting go of a task held until a time, when the time comes (ov-212).
//!
//! On the watcher's tick: the store answers the earliest hold from an index,
//! so asking every second costs nothing, and a hold is let go within a
//! second of its time. The store writes the note and queues the
//! orchestrator's wake in the same transaction; the answer pump tells it
//! (`answer_wake`, as a hold that ended).

use farcooler_store::models::Actor;

use super::Watcher;

impl Watcher {
    /// Let go of every hold due at `now` (Unix milliseconds), and announce
    /// each task, as the runner. `now` is the caller's, so a test can stand
    /// at any time.
    pub(crate) fn release_due_holds(&self, now: i64) {
        let store = &self.service.store;
        match store.next_hold_due() {
            Ok(Some(due)) if due <= now => {}
            Ok(_) => return,
            Err(e) => {
                tracing::warn!(error = %e, "couldn't read when the next held task is due");
                return;
            }
        }
        match store.release_due_holds(now) {
            Ok(released) => {
                if !released.is_empty() {
                    // Each is a wake to tell its orchestrator (`answer_wake`).
                    self.wakes_hint.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                for (task, _) in released {
                    self.announce_task_changed(&task, None, Actor::Runner);
                }
            }
            Err(e) => tracing::warn!(error = %e, "couldn't let go of a held task whose time came"),
        }
    }
}
