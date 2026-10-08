//! Bring Here (ov-369, R-28): the draft a person left in a terminal-mode
//! claude's own box, read for a native composer to take, then cleared, so a
//! send from the composer has one draft and not two. `terminal.compose`
//! refuses a box holding text (`draft`); this is how a client gets back to a
//! send without typing over it.
//!
//! **Two calls**, so the text is never in neither place. A read (no
//! `expected`) types nothing and answers the box's text; the client puts it
//! in its composer. A clear (`expected`, that text) empties the box only
//! while it still reads exactly that. A clear that fails leaves the text in
//! both: the composer and the box.
//!
//! **What the keys do**, measured on claude 2.1.292 in a sandbox:
//! - ctrl+u deletes back to the start of the row the cursor is on, a wrapped
//!   row counting as a row; at a row's start it takes the line break and the
//!   line before. On an empty box it does nothing.
//! - ctrl+y pastes back all that a run of ctrl+u's deleted, line breaks too.
//!   claude says `Ctrl+Y to paste deleted text` above the box meanwhile.
//! - Neither does anything on a permission dialog, and neither interrupts a
//!   turn: both behave the same mid-turn. (Esc would stop a turn, two open
//!   the rewind picker, and ctrl+c can exit: none is pressed.)
//! - Keys sent back to back can be taken together, so each waits for the box
//!   to change.
//!
//! **The gate.** Every check fails closed, nothing typed; each refusal is a
//! `DomainError::Conflict` naming why with a stable word:
//! 1. a terminal-mode pane launched as an agent, running (`not_an_agent`,
//!    `not_running`; a chat pane has a prompt channel of its own);
//! 2. nothing else typing into its box (`Watcher::typing`, waited for up to
//!    `TYPING_WAIT`: `sending`);
//! 3. claude in front, proven by its process (`not_an_agent`; codex and
//!    cursor are `unsupported`);
//! 4. its box on screen, idle or working (a dialog or a panel is `prompt`;
//!    anything else `unfamiliar`);
//! 5. the draft read whole (`composer::draft`): not a collapsed paste or an
//!    image (`pasted`), not so tall claude may be hiding rows (`too_tall`),
//!    claude's cursor after its last character (`cursor`). An empty box
//!    answers no text, nothing to bring.
//!
//! A clear also needs the box to read exactly `expected` (`changed`), and no
//! key in the pane in the last `TYPED_WITHIN_MS` (`typing`).
//!
//! **The clear.** ctrl+u, then the box read until it changes. Each read must
//! show the start of `expected`. Anything else is a key someone typed, or a
//! tall draft's hidden rows scrolling in: ctrl+y puts it all back, and the
//! clear is refused as `typing`, `changed` or `too_tall`. So is a key typed
//! in the pane through a client meanwhile. It ends when the box reads empty,
//! or after the box's rows and two more: then it's put back, `changed`. A
//! box that can't be put back (a dialog came up, or the yank didn't show)
//! is `partly`: claude's own hint says ctrl+y there brings it back.
//!
//! **After.** The time the box was emptied is kept per terminal
//! (`Watcher::brought`): `typed_lately` ignores keys from before it, which
//! made the draft now in the composer. Without that, the send that follows
//! would wait out the 15 s a key holds an automatic send for (ov-407).

use std::time::Duration;

use farcooler_core::composer::draft::{self, Draft, Held};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, AgentActivity};
use farcooler_store::models::{PaneMode, Terminal};
use uuid::Uuid;

use super::tell::held_word;
use super::{PASTE_POLL, may_be_typed_to};
use crate::runtime::{Runtime, last_input};
use crate::watch::{Watcher, now_millis};

/// How recently a person's key in the pane stops a clear: Stop's window
/// (`interrupt`), not an automatic send's 15 s. Bring Here is the person's
/// own act, often right after they typed in the box, and it reads the box
/// back after every key.
pub(crate) const TYPED_WITHIN_MS: i64 = super::interrupt::TYPED_WITHIN_MS;

/// The longest a clear waits for another send to let go of the box.
const TYPING_WAIT: Duration = Duration::from_secs(10);

/// The longest the box takes to show a key.
const KEY_SETTLES: Duration = Duration::from_millis(1_500);

/// ctrl+u and ctrl+y.
const CLEAR_ROW: &str = "15";
const PUT_BACK: &str = "19";

/// The box after a key.
enum After {
    /// Shown, as this.
    Shown(Draft),
    /// Someone typed through a client since the clear began.
    Typed,
    /// No box: a dialog, a panel, or claude gone.
    Gone,
}

impl Watcher {
    /// `terminal.bring_draft`'s request, answered.
    pub(crate) async fn bring_draft_wire(&self, p: &pb::BringDraft) -> Result<pb::BroughtDraft> {
        let id = crate::wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
        let expected = (!p.expected.is_empty()).then(|| p.expected.clone());
        let (text, cleared) = self.bring_draft(id, expected).await?;
        Ok(pb::BroughtDraft { text, cleared })
    }

    /// The draft in the claude in terminal `id`'s box, and whether it was
    /// cleared: read with nothing typed, or with `expected`, cleared while it
    /// still reads exactly that. See this module's docs. On a task of its
    /// own, so a caller that goes away can't stop a clear between a ctrl+u
    /// and the ctrl+y that would put it back.
    pub(crate) async fn bring_draft(&self, id: Uuid, expected: Option<String>) -> Result<(String, bool)> {
        let Some(me) = self.me.upgrade() else { return Err(DomainError::OperationFailed) };
        tokio::spawn(async move { me.bring_here(id, expected).await }).await.unwrap_or(Err(DomainError::OperationFailed))
    }

    async fn bring_here(&self, id: Uuid, expected: Option<String>) -> Result<(String, bool)> {
        let to = self.service.store.get_terminal(id)?;
        if to.pane_mode == PaneMode::Agent {
            return Err(DomainError::InvalidArgument { what: "terminal" });
        }
        if to.pane_mode == PaneMode::Changes || !may_be_typed_to(&to.command_preset, to.role) {
            return Err(conflict("not_an_agent"));
        }
        if !self.service.is_running(&to) {
            return Err(conflict("not_running"));
        }
        let Ok(_typing) = tokio::time::timeout(TYPING_WAIT, self.typing(to.id)).await else {
            return Err(conflict("sending"));
        };
        let Some(held) = self.draft_in(&to).await? else { return Ok((String::new(), false)) };
        let Some(expected) = expected else { return Ok((held.text, false)) };
        if held.text != expected {
            return Err(conflict("changed"));
        }
        if last_input(self.service.root_dir(), to.id).is_some_and(|at| now_millis() - at < TYPED_WITHIN_MS) {
            return Err(conflict("typing"));
        }
        self.clear(&to, &held).await?;
        self.brought.lock().unwrap_or_else(|e| e.into_inner()).insert(to.id, now_millis());
        Ok((held.text, true))
    }

    /// The gate's checks 3 to 5: the draft in `to`'s box, `None` when it's
    /// empty.
    async fn draft_in(&self, to: &Terminal) -> Result<Option<Held>> {
        let (preset, _, _) = self.proven_agent(to).await.map_err(|held| conflict(held_word(held)))?;
        if preset != "claude" {
            return Err(conflict("unsupported"));
        }
        let (screen, columns, rows) = self.service.screen(to.id).await?;
        match self.service.registry().classify(preset, &screen) {
            AgentActivity::Idle | AgentActivity::Working => {}
            AgentActivity::Blocked => return Err(conflict("prompt")),
            _ => return Err(conflict("unfamiliar")),
        }
        let held = match draft::claude(&screen, columns) {
            Draft::Empty => return Ok(None),
            Draft::Unrecognized => return Err(conflict("unfamiliar")),
            Draft::Holds(held) => held,
        };
        if held.placeholder {
            return Err(conflict("pasted"));
        }
        if held.rows >= draft::max_rows(rows) {
            return Err(conflict("too_tall"));
        }
        if !held.cursor_at_end {
            return Err(conflict("cursor"));
        }
        Ok(Some(held))
    }

    /// Empty `to`'s box of `held`, a row at a time, each read back. See this
    /// module's docs, "The clear".
    async fn clear(&self, to: &Terminal, held: &Held) -> Result<()> {
        let started = now_millis();
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let mut shown = held.text.clone();
        for _ in 0..held.rows + 2 {
            runtime.send_bytes_hex(to.id, CLEAR_ROW).await?;
            #[cfg(test)]
            if let Some(run) = self.after_clear_key.lock().unwrap_or_else(|e| e.into_inner()).take() {
                run();
            }
            match self.after_key(to, &shown, started).await {
                After::Shown(Draft::Empty) => return Ok(()),
                After::Shown(Draft::Holds(now)) if draft::is_prefix_of(&now.text, &held.text) => shown = now.text,
                After::Shown(Draft::Holds(_)) => {
                    let typed = last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started);
                    return self.put_back(to, &runtime, held, if typed { "typing" } else { "too_tall" }).await;
                }
                After::Typed => return self.put_back(to, &runtime, held, "typing").await,
                After::Shown(Draft::Unrecognized) | After::Gone => return Err(conflict("partly")),
            }
        }
        self.put_back(to, &runtime, held, "changed").await
    }

    /// Read `to`'s box until it shows something other than `shown`, or for
    /// `KEY_SETTLES`.
    async fn after_key(&self, to: &Terminal, shown: &str, started: i64) -> After {
        let deadline = tokio::time::Instant::now() + KEY_SETTLES;
        let mut last = After::Gone;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started) {
                return After::Typed;
            }
            let Ok((screen, columns, _)) = self.service.screen(to.id).await else { continue };
            if !matches!(self.service.registry().classify("claude", &screen), AgentActivity::Idle | AgentActivity::Working) {
                last = After::Gone;
                continue;
            }
            let now = draft::claude(&screen, columns);
            match &now {
                Draft::Holds(held) if held.text == shown => last = After::Shown(now),
                _ => return After::Shown(now),
            }
        }
        last
    }

    /// ctrl+y, and the box read until it holds `held` again: then `word`,
    /// else `partly`.
    async fn put_back(&self, to: &Terminal, runtime: &Runtime, held: &Held, word: &'static str) -> Result<()> {
        runtime.send_bytes_hex(to.id, PUT_BACK).await?;
        let whole: String = held.text.chars().filter(|c| !c.is_whitespace()).collect();
        let deadline = tokio::time::Instant::now() + KEY_SETTLES;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            let Ok((screen, columns, _)) = self.service.screen(to.id).await else { continue };
            if let Draft::Holds(now) = draft::claude(&screen, columns)
                && now.text.chars().filter(|c| !c.is_whitespace()).collect::<String>().contains(&whole)
            {
                return Err(conflict(word));
            }
        }
        Err(conflict("partly"))
    }
}

fn conflict(what: &'static str) -> DomainError {
    DomainError::Conflict { what }
}
