//! `terminal tell` (ov-214): a message from the person to the workspace's
//! orchestrator, typed into its TUI and submitted. The Mac title bar's field
//! sent it until ov-264 made that field a search; `farcooler terminal tell`
//! still does.
//!
//! Since ov-620 a tell is a Prompt (`route`): the person's own words, typed
//! as their turn through compose's gate, checks and confirmation
//! (`compose`), only into the workspace's orchestrator. A tell from inside
//! one of the runner's panes may be an agent's (`Author::Agent`): it goes as
//! an agent message, by the orchestrator's mailbox, never typed as the
//! person's turn (`route`), and keeps the rule tells had before: nothing a
//! box reads as a command or a mode first (`/ ! # @ & $ ? \`). Its refusals
//! are compose's stable words (`DomainError::Conflict`), which the Mac and the
//! CLI turn into their own sentences: `busy`, `prompt`, `draft`, `typing`,
//! `not_an_agent`, `unfamiliar`, `unproven`, `too_long`, `command`,
//! `not_running`, `paste_left`, `left_at_shell`, `dialog`, `unconfirmed`,
//! `handoff`, `unsupported`, `unconfirmable`, `backslash`. A claude that
//! can't be typed to safely yet holds it, answered Queued (`compose::held`).

use farcooler_core::{DomainError, Result};
use farcooler_store::models::TerminalRole;
use uuid::Uuid;

use super::{Held, Turn};
use super::route::{Author, Delivered, Delivery};
use crate::watch::Watcher;

/// The word a refusal names, for `Held`.
pub(crate) fn held_word(held: Held) -> &'static str {
    match held {
        Held::Busy => "busy",
        Held::Prompt => "prompt",
        Held::Draft => "draft",
        Held::Typing => "typing",
        Held::NotAnAgent => "not_an_agent",
        Held::Unfamiliar => "unfamiliar",
        Held::Unproven => "unproven",
        Held::Followup => "followup",
        Held::FollowupLimit => "followup_limit",
    }
}

impl Watcher {
    /// Type `raw` into the orchestrator terminal `id` and submit it, as the
    /// person's own turn: `Turn::Between` when it's the agent's next prompt,
    /// `Turn::During` when it's queued behind the turn running or held until
    /// it can be. See this module's docs.
    pub(crate) async fn tell_into(&self, id: Uuid, raw: &str, author: Author) -> Result<Turn> {
        let to = self.service.store.get_terminal(id)?;
        if to.role != TerminalRole::Orchestrator {
            return Err(DomainError::InvalidArgument { what: "terminal" });
        }
        match self.deliver(id, Delivery { author, ..Delivery::prompt(raw) }).await? {
            Delivered::Sent => Ok(Turn::Between),
            _ => Ok(Turn::During),
        }
    }
}
