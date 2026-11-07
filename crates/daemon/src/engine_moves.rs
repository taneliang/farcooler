//! ov-948: a card the engine moved is told to open boards the way a hand move
//! is. The store moves the card inside the `workflow_apply` transaction and
//! calls this after the commit, so clients never hear of a move that rolled
//! back. A status move bumps no `resource_version`, so these events are the
//! only signal a board gets.
use crate::watch::Watcher;
use crate::watch::task_notice::TaskEvent;
use farcooler_store::BoardMove;
use farcooler_store::models::{Actor, TaskStatus};

fn closed(status: TaskStatus) -> bool { matches!(status, TaskStatus::Done | TaskStatus::Cancelled) }
fn is_an_item(status: TaskStatus) -> bool { matches!(status, TaskStatus::NeedsDecision | TaskStatus::InReview) }

impl Watcher {
    pub(crate) fn announce_engine_moves(&self, moves: &[BoardMove]) {
        let svc = &self.service;
        for m in moves {
            let Ok(task) = svc.store.get_task(m.task) else { continue };
            self.announce_task_changed(&task, None, Actor::Runner);
            let before = m.from.unwrap_or(m.to);
            if closed(before) != closed(m.to) {
                if let Some(lane) = task.worktree_id { self.announce_worktree_changed(lane); }
                let _ = crate::task_starts::announce_dependents(svc, self, &task, Actor::Runner);
            }
            if before != m.to && (is_an_item(before) || is_an_item(m.to)) { self.announce_needs_you(); }
            if before != m.to { self.task_event(&task, TaskEvent::Moved { to: m.to }, Actor::Runner); }
        }
    }
}
