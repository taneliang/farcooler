//! Cursor's `stop` hook as a way to deliver a message (ov-563): the hook
//! waits for the daemon's word, and the daemon answers `{"followup_message":
//! …}` when a message is waiting for that pane, which cursor queues as the
//! next user message. What a message is, and its being settled, are the
//! watcher's (`Watcher::followup_for`); this is only the hook's line to it.
//!
//! The watcher claims the message and hands back what settles it; the line
//! is written, and only a write that succeeded settles it (`Taken`). A hook
//! that had already hung up leaves the message claimed, which the pump records
//! as one it couldn't confirm: never as delivered, and never typed as well.

use std::sync::Arc;

use uuid::Uuid;

use super::HookIngress;

/// A message taken for a pane whose cursor turn just ended.
pub struct Taken {
    /// What to queue as the next user message.
    pub text: String,
    /// Settles the message. Called once the line carrying `text` was written.
    pub delivered: Box<dyn FnOnce() + Send>,
}

/// The watcher's answer for a pane whose cursor turn just ended: a message it
/// claimed for the pane, or `None`.
pub type Followups = Arc<dyn Fn(Uuid) -> Option<Taken> + Send + Sync>;

impl HookIngress {
    /// Where a cursor `stop` hook asks for a message. Set once, by the
    /// watcher; without it every `stop` is answered with no message.
    pub fn set_followups(&self, followups: Followups) {
        *self.followups.lock().unwrap_or_else(|e| e.into_inner()) = Some(followups);
    }

    /// The message for `terminal`'s `stop` hook, claimed now.
    pub(super) fn followup_for(&self, terminal: Uuid) -> Option<Taken> {
        let followups = self.followups.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
        followups(terminal)
    }
}
