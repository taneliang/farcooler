//! Messages the runner holds for a pane until claude can take them (ov-468),
//! on the rows a view shows.
//!
//! A held message is a `Queued` row, `held:<id>`, Waiting, so a view says
//! Queued for it as it does for one in claude's own queue, and settles the
//! echo it drew when it sent it. Once the runner types it, the row is taken
//! back: claude's own record, a queued row or a turn, shows it from then on.
//! One the runner gives up on goes Withdrawn, with a notice saying why. One
//! pasted into the box whose Enter waits for the turn's end is `InBox`, which
//! a view says as "Waiting to send" (ov-827).

use super::fold::{LINE_CHARS, Projection, squeeze};
use super::rows::*;

/// The row id of the held message `id`.
fn row_id(id: &str) -> String {
    format!("held:{id}")
}

impl Projection {
    /// The runner holds `text` as `id`, since `at_ms`: its Queued row, made
    /// once however often this is said.
    pub fn runner_held(&mut self, id: &str, text: &str, at_ms: i64) {
        let row = row_id(id);
        if self.index.contains_key(&row) {
            return;
        }
        let queued = Queued { text: squeeze(text, LINE_CHARS), state: QueuedState::Waiting, at_ms: Some(at_ms) };
        self.push(row, self.newest_turn, false, RowKind::Queued(queued));
    }

    /// `text`, `id`, is in the agent's box since `at_ms`, its Enter waiting
    /// for the turn's end (ov-827): an `InBox` row, made once however often
    /// this is said, or a held row moved to it.
    pub fn runner_in_box(&mut self, id: &str, text: &str, at_ms: i64) {
        if let Some(&i) = self.index.get(&row_id(id)) {
            self.set_queued(i, QueuedState::InBox);
            return;
        }
        let queued = Queued { text: squeeze(text, LINE_CHARS), state: QueuedState::InBox, at_ms: Some(at_ms) };
        self.push(row_id(id), self.newest_turn, false, RowKind::Queued(queued));
    }

    /// The runner typed `id`: its row is taken back.
    pub fn runner_told(&mut self, id: &str) {
        if let Some(&i) = self.index.get(&row_id(id)) {
            self.retract(i);
        }
    }

    /// The runner gave `id` up: Withdrawn, and a notice, `why`, after it. A
    /// message never held here (a projection rebuilt since) gets the notice
    /// alone. Said once: told again (a replay racing the live call, ov-830),
    /// it changes nothing.
    pub fn runner_dropped(&mut self, id: &str, why: &str, at_ms: i64) {
        if !self.runner_dropped.insert(id.to_string()) {
            return;
        }
        if let Some(&i) = self.index.get(&row_id(id)) {
            self.set_queued(i, QueuedState::Withdrawn);
        }
        self.notice(NoticeKind::NotSent, squeeze(why, LINE_CHARS), Some(at_ms), false);
    }
}
