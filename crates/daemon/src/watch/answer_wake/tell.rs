//! `terminal tell` (ov-214): a message from the person to the workspace's
//! orchestrator, typed into its TUI and submitted. The Mac title bar's field
//! sent it until ov-264 made that field a search; `farcooler terminal tell`
//! still does.
//!
//! A chat orchestrator takes the message as a prompt on its agent channel
//! (`terminal.agent_prompt`); this is the other kind, the one most people
//! run: claude in a terminal. It's typed only past the answer wake's whole
//! gate, checked against the pane as it is now, and every check fails
//! closed with nothing typed (`tell_into`):
//!
//! 1. the watcher reads the agent Idle, Done or Working, and nobody has typed
//!    there lately (`ready`);
//! 2. the agent in front is proven by its process, its box is recognized and
//!    empty, and bracketed paste is known to be on (`proven_tui`).
//!
//! A working claude is typed into too, as its CLI takes a message mid-turn:
//! it queues it for its next turn (ov-360). The Enter waits on no dialog
//! being up or announced (`dialog` if one is: the text is left in the box),
//! and a queued message is confirmed in claude's queue (`mid_turn`). The
//! reply says which, `Turn::Between` (sent, its next prompt) or
//! `Turn::During` (queued).
//!
//! A message starting with a character an agent's box reads as a mode or a
//! command is refused (`command`): in claude a `/` opens the command picker,
//! which Enter then runs, and a `!` turns the box into a shell, which Enter
//! then runs; codex runs `!` even after a space.
//!
//! Then the text, on one line with every control and invisible character
//! written out (`one_line`), goes in as one bracketed paste; the box is read
//! back until it holds exactly that, and only then is Enter sent. If it
//! never does, or someone types meanwhile, no Enter: the text is left in
//! the box and the caller is told so.
//!
//! Each refusal is an error naming why with a stable word
//! (`DomainError::Conflict`), which the Mac turns into its own sentence:
//! `busy`, `prompt`, `draft`, `typing`, `not_an_agent`, `unfamiliar`,
//! `unproven`, `too_long`, `command`, `not_running`, `paste_left`,
//! `left_at_shell`, `dialog`, `unconfirmed`. `busy` is left only for an
//! agent working that can't be typed into safely mid-turn: codex, or a
//! claude whose session or hooks can't be found.

use farcooler_core::composer;
use farcooler_core::{DomainError, Result};
use farcooler_store::models::{PaneMode, TerminalRole};
use uuid::Uuid;

use super::{Held, PASTE_POLL, PASTE_SETTLES, TOLD_SPACING_MS, Turn, foreground_agent, may_be_typed_to, mid_turn, one_line};
use crate::runtime::{Runtime, last_input};
use crate::watch::{Watcher, now_millis};

/// The longest message typed into a terminal, after it's put on one line.
/// Longer is refused, never cut: claude collapses a long paste into a
/// placeholder the box can't be read back from.
pub(crate) const LONGEST_MESSAGE: usize = 500;

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
    }
}

/// `raw` as it would be typed, or `too_long` past `LONGEST_MESSAGE`,
/// `command` when it starts with what a box reads as a command (this
/// module's docs), or `text` when nothing would be left.
pub(crate) fn told_text(raw: &str) -> Result<String> {
    let line = one_line(raw, usize::MAX);
    if line.trim().is_empty() {
        return Err(DomainError::InvalidArgument { what: "text" });
    }
    if line.chars().count() > LONGEST_MESSAGE {
        return Err(DomainError::Conflict { what: "too_long" });
    }
    if line.starts_with(['/', '!', '#', '@', '&', '$', '?', '\\']) {
        return Err(DomainError::Conflict { what: "command" });
    }
    Ok(line)
}

impl Watcher {
    /// Type `text` into the orchestrator terminal `id` and submit it, past
    /// the answer wake's gate: `Turn::Between` when it's the agent's next
    /// prompt, `Turn::During` when it's queued behind the turn running. See
    /// this module's docs. The native view's composer, for any claude pane,
    /// is `compose_into` (`terminal.compose`, ov-372, ov-367).
    pub(crate) async fn tell_into(&self, id: Uuid, raw: &str) -> Result<Turn> {
        let to = self.service.store.get_terminal(id)?;
        if to.role != TerminalRole::Orchestrator {
            return Err(DomainError::InvalidArgument { what: "terminal" });
        }
        if to.pane_mode == PaneMode::Agent {
            // A chat orchestrator takes a prompt on its own channel.
            return Err(DomainError::InvalidArgument { what: "terminal" });
        }
        if to.pane_mode == PaneMode::Changes || !may_be_typed_to(&to.command_preset, to.role) {
            return Err(DomainError::Conflict { what: "not_an_agent" });
        }
        if !self.service.is_running(&to) {
            return Err(DomainError::Conflict { what: "not_running" });
        }
        let text = told_text(raw)?;
        // A message sent a moment ago: the spacing is for answers, which
        // wait it out on the next pass. This one waits it out here, rather
        // than be refused for it.
        let told = self.told.lock().unwrap_or_else(|e| e.into_inner()).get(&to.id).copied();
        if let Some(left) = told.map(|at| TOLD_SPACING_MS - (now_millis() - at)).filter(|left| *left > 0) {
            tokio::time::sleep(std::time::Duration::from_millis(left as u64)).await;
        }
        // Held through the Enter and the queue's confirmation: no answer or
        // draft pastes into this box meanwhile.
        let _typing = self.typing(to.id).await;
        self.ready(&to).await.map_err(|held| DomainError::Conflict { what: held_word(held) })?;
        let proven = self.proven_tui(&to).await.map_err(|held| DomainError::Conflict { what: held_word(held) })?;
        let witness = match proven.turn {
            Turn::Between => None,
            Turn::During => Some(self.witness(&proven, &to, &text).await.ok_or(DomainError::Conflict { what: "busy" })?),
        };
        let (preset, tty) = (proven.preset, proven.tty.as_str());

        // Not recorded as someone typing (`marks: None`), as an answer isn't.
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let paste: String = crate::pastes::encode_paste(true, &text).iter().map(|b| format!("{b:02x}")).collect();
        let started = now_millis();
        if self.fail_sends_for_tests() {
            return Err(DomainError::OperationFailed);
        }
        runtime.send_bytes_hex(to.id, &paste).await?;
        let typed_since = || last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started);
        let deadline = tokio::time::Instant::now() + PASTE_SETTLES;
        let mut held_exactly = false;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            if typed_since() {
                break;
            }
            if let Ok(Ok((now, _))) = self.box_of(&to, preset).await
                && composer::holds_exactly(&now, &text)
            {
                held_exactly = true;
                break;
            }
        }
        if typed_since() || !held_exactly {
            if foreground_agent(tty).await != Some(preset) {
                return Err(DomainError::Conflict { what: "left_at_shell" });
            }
            return Err(DomainError::Conflict { what: "paste_left" });
        }
        // As `type_into`: a codex turn begun during the read-back gets no
        // Enter, and the text stays in the box (review 1, L1).
        if preset == "codex" && witness.is_none() && super::codex_turn::said_of(proven.pid).await == super::registry_turn::Said::NotIdle {
            return Err(DomainError::Conflict { what: "paste_left" });
        }
        match &witness {
            None => runtime.send_bytes_hex(to.id, "0d").await?,
            Some(witness) => self.enter(&to, preset, witness, &text).await.map_err(|no| match no {
                mid_turn::NoEnter::Dialog => DomainError::Conflict { what: "dialog" },
                mid_turn::NoEnter::Moved => DomainError::Conflict { what: "paste_left" },
                mid_turn::NoEnter::Failed => DomainError::OperationFailed,
            })?,
        }
        self.mark_told(to.id);
        if let Some(witness) = witness
            && !self.queued(&witness, &text).await
        {
            return Err(DomainError::Conflict { what: "unconfirmed" });
        }
        Ok(proven.turn)
    }
}
