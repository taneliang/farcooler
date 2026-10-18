//! Whether cursor took a message typed into its box (ov-632).
//!
//! claude is asked through its hooks and transcript, codex through its
//! rollout. cursor's `beforeSubmitPrompt` hook carries the prompt as typed
//! and reaches the terminal's ring as a user message (`assemble`), and its
//! transcript's user line is its own record (`session_log::cursor` reads that
//! a turn began, not the text). So a message is taken when the ring holds a
//! user message of that text newer than the paste, or, for a pane no hook has
//! reported for (a cursor adopted without the project's hooks), when the
//! text has left the box, which an Enter that didn't send never does, and
//! nobody typed through Far Cooler since the paste (`last_input`).
//!
//! **Not closed:** someone clearing the box by typing straight into tmux
//! between the read-back and the Enter leaves no input mark, so the empty box
//! reads as taken (as `drawn::Expected::shown_on` documents for its race). The
//! hook, where cursor's are installed, is the exact check.

use farcooler_agent_core::event::{AgentEvent, Role, Seq};
use farcooler_core::composer::{self, Composer};
use uuid::Uuid;

use super::{PASTE_POLL, Terminal, Watcher};
use crate::runtime::last_input;

impl Watcher {
    /// The newest event in `terminal`'s ring, `None` while it is empty (the
    /// first event is numbered 0): what a user message must be newer than to
    /// be this paste's.
    pub(super) fn ring_end(&self, terminal: Uuid) -> Option<Seq> {
        self.service.agents().replay(terminal, 0, 0).1.last().map(|e| e.seq)
    }

    /// Whether cursor took `text` within `CONFIRM_SETTLES` of the Enter that
    /// sent it (`pasted` is when the paste began): a user message of that text after `after` in the ring, or the
    /// box empty of it.
    pub(super) async fn cursor_took(&self, to: &Terminal, preset: &str, pasted: i64, after: Option<Seq>, text: &str) -> bool {
        let want = squeezed(text);
        let deadline = tokio::time::Instant::now() + self.timing().compose_confirm;
        loop {
            let heard = self.service.agents().replay(to.id, 0, 0).1.iter().any(|e| {
                after.is_none_or(|after| e.seq > after) && matches!(&e.event, AgentEvent::Message { role: Role::User, text, .. } if squeezed(text) == want)
            });
            if heard {
                return true;
            }
            let typed = last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= pasted);
            if !typed
                && let Ok((screen, _, _)) = self.service.screen(to.id).await
                && composer::read(preset, &screen) == Composer::Empty
            {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(PASTE_POLL).await;
        }
    }
}

/// `text` without its whitespace, as the box wraps and rejoins it.
fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}
