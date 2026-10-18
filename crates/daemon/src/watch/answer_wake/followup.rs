//! A message for a cursor pane, taken when its turn ends (ov-563): an
//! agent's, and a person's too, since the followup is the same role:user
//! turn as typing (ov-627, `queued`).
//!
//! Cursor's `stop` hook may answer `{"followup_message": …}`, which cursor
//! queues as the next user message (`hook_ingress::followup`, which asks
//! here). So a message for a cursor pane that is working isn't typed into its
//! box: it waits, and the turn's end takes it. An idle pane is typed to, as
//! the pump does for any pane; `claim_wake` is what lets only one of the two
//! have a message.
//!
//! The message is claimed as it's taken and settled only once the line to the
//! hook was written (`hook_ingress::followup`). A claimed message that never
//! settles (the hook had hung up, or the daemon stopped between) reads as
//! "couldn't confirm", never as a second delivery.

use super::deliver::Way;
use crate::hook_ingress::followup::Taken;
use super::*;

impl Watcher {
    /// The oldest message waiting for `terminal`, claimed, as the text to
    /// queue as its next turn and what settles it once that's written. `None` when it isn't a cursor pane or
    /// nothing is waiting. The wake switch, the give-up time and a claim by
    /// the pump are honored as `wake()` honors them.
    pub(crate) fn followup_for(&self, terminal: Uuid) -> Option<Taken> {
        let store = &self.service.store;
        let to = store.get_terminal(terminal).ok()?;
        if Way::of(&to.command_preset, terminal) != Way::CursorFollowup {
            return None;
        }
        for wake in store.pending_message_wakes().ok()? {
            if wake.claimed_at.is_some() || now_millis() - wake.enqueued_at > give_up_after(wake.kind) {
                continue;
            }
            let Ok(task) = store.get_task(wake.task) else { continue };
            let mine = match wake.to {
                Some(id) => id == terminal,
                None => self.service.live_orchestrator(task.workspace_id).ok().flatten().is_some_and(|o| o.id == terminal),
            };
            if !mine || !store.get_workspace(task.workspace_id).is_ok_and(|w| w.wake_on_answer) {
                continue;
            }
            if !store.claim_wake(&wake).unwrap_or(false) {
                continue;
            }
            let text = self.message_text(&wake, &task);
            self.followup_taken(terminal);
            let this = self.me.clone();
            let delivered = Box::new(move || {
                if let Some(this) = this.upgrade() {
                    this.settle(&wake, Some(&task), None);
                }
            });
            return Some(Taken { text, delivered });
        }
        None
    }
}
