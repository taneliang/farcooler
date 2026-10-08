//! Finishing an answer a dialog stopped short of its Enter (ov-385). See
//! `answer_wake`'s docs, "A dialog in the way".
//!
//! The row is claimed and marked pasted (`Store::mark_wake_pasted`): its text
//! went into the box, and a dialog came up before the Enter. A later pass
//! that reaches it past checks 1 and 2 (`ready`, so no dialog is on the
//! screen the watcher reads) presses the Enter only when:
//! - the agent in front is the one the pane runs, proven by its process
//!   (`proven_agent`);
//! - a fresh capture reads the box holding exactly the text, with no dialog
//!   over it;
//! - nobody has typed there since the paste began;
//! - for claude, every check a mid-turn Enter makes, whether or not it reads
//!   as working: the session quiet, no call in flight, no ask held
//!   (`witness`), and the same again under the fence (`enter`). A box holding
//!   text hides the line that says claude is working (it drops "esc to
//!   interrupt"), so the screen can't be trusted to say it's between turns.
//!   A claude whose session or hooks can't be found waits, and in the end
//!   settles as a paste left in the box;
//! - for codex, which says nothing through hooks, between turns as both the
//!   watcher and a fresh capture read it, and as its rollout says
//!   (`codex_turn`), as any Enter into codex is. codex
//!   keeps "esc to interrupt" with text in its box (measured on 0.153.4), so
//!   that reading holds. Any other agent's screen with text in the box was
//!   never measured, so its answer settles as a paste left in the box.
//!
//! **A limit.** A dialog answered from inside Far Cooler, typed in the pane
//! from the Mac or a phone, is a key typed since the paste: Far Cooler can't
//! tell a key that went to the dialog from one that went to the box, so the
//! answer settles "Paste left in the composer; not sent". It's finished only
//! when the dialog went without a key Far Cooler saw: a tool call that needed
//! no answer, an ask answered through the hook, or a key typed in a terminal
//! attached to tmux directly.
//!
//! A box holding anything else, or nothing, settles "Paste left in the
//! composer; not sent", as a box that never matched always has. The text is
//! never pasted again. The mark comes off before the Enter goes, so a crash
//! from there reads as one mid-typing: "Couldn't confirm", never a second Enter.

use farcooler_core::composer;
use farcooler_store::PendingWake;
use farcooler_store::models::{Task, Terminal};

use super::{Held, PASTE_LEFT, Pass, Turn, couldnt_confirm, mid_turn, told};
use crate::runtime::{Runtime, last_input};
use crate::watch::Watcher;

impl Watcher {
    /// A dialog came up between `wake`'s paste, begun at `pasted` (Unix ms),
    /// and its Enter: mark the row pasted and wait for the dialog to go, or
    /// settle as a paste left in the box when the mark can't be written.
    pub(super) fn paste_waits(&self, wake: &PendingWake, task: &Task, pasted: i64) -> Pass {
        match self.service.store.mark_wake_pasted(wake, pasted) {
            Ok(true) => Pass::Waiting(Held::Prompt),
            _ => self.settle(wake, Some(task), Some(PASTE_LEFT.into())),
        }
    }

    /// Press the Enter for `wake`, whose `text` was pasted into `to` at
    /// `pasted`, if this module's checks all pass now.
    /// `seen` is the turn the watcher reads (`ready`).
    pub(super) async fn finish_paste(
        &self,
        wake: &PendingWake,
        task: &Task,
        to: &Terminal,
        text: &str,
        pasted: i64,
        seen: Turn,
    ) -> Pass {
        let (preset, _, pid) = match self.proven_agent(to).await {
            Ok(proven) => proven,
            Err(held) => return Pass::Waiting(held),
        };
        if !matches!(preset, "claude" | "codex") {
            return self.settle(wake, Some(task), Some(PASTE_LEFT.into()));
        }
        let turn = match self.box_of(to, preset).await {
            Ok(Ok((now, Turn::Between))) if composer::holds_exactly(&now, text) => seen,
            Ok(Ok((now, Turn::During))) if composer::holds_exactly(&now, text) => Turn::During,
            Ok(Err(held)) => return Pass::Waiting(held),
            Err(_) => return Pass::Waiting(Held::Unfamiliar),
            Ok(Ok(_)) => return self.settle(wake, Some(task), Some(PASTE_LEFT.into())),
        };
        // A screen between turns while codex's rollout says one runs: wait.
        let turn = match (preset, turn) {
            ("codex", Turn::Between) if super::codex_turn::said_of(pid).await == super::registry_turn::Said::NotIdle => Turn::During,
            _ => turn,
        };
        if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= pasted) {
            return self.settle(wake, Some(task), Some(PASTE_LEFT.into()));
        }
        let proven = super::Proven { preset, tty: String::new(), pid, turn, held: false };
        let witness = match (super::queues_mid_turn(preset), turn) {
            (true, _) => match self.witness(&proven, to, text).await {
                Some(witness) => Some(witness.pasted_at(pasted)),
                None => return Pass::Waiting(Held::Busy),
            },
            (false, Turn::Between) => None,
            (false, Turn::During) => return Pass::Waiting(Held::Busy),
        };
        match self.service.store.take_wake_paste(wake) {
            Ok(true) => {}
            Ok(false) => return Pass::Settled,
            Err(_) => return Pass::Waiting(Held::Busy),
        }
        // From here, as in `type_into`, nothing is retried but a dialog that
        // comes up again: the text is still in the box, so the mark goes back.
        let entered = match &witness {
            None => {
                let runtime = Runtime { marks: None, ..self.service.runtime() };
                if self.fail_sends_for_tests() {
                    Err(mid_turn::NoEnter::Failed)
                } else {
                    runtime.send_bytes_hex(to.id, "0d").await.map_err(|_| mid_turn::NoEnter::Failed)
                }
            }
            Some(witness) => self.enter(to, preset, witness, text).await,
        };
        match entered {
            Ok(()) => {}
            Err(mid_turn::NoEnter::Failed) => return self.settle(wake, Some(task), Some(couldnt_confirm(wake.kind))),
            Err(mid_turn::NoEnter::Dialog) => return self.paste_waits(wake, task, pasted),
            Err(mid_turn::NoEnter::Moved) => return self.settle(wake, Some(task), Some(PASTE_LEFT.into())),
        }
        self.mark_told(to.id);
        // What claude's transcript says took it, not the screen's turn: a box
        // holding text reads as Idle while claude works.
        let turn = match &witness {
            None => turn,
            Some(witness) => match self.queued_how(witness, text).await {
                Some(took) => took,
                None => return self.settle(wake, Some(task), Some(couldnt_confirm(wake.kind))),
            },
        };
        self.settle(wake, Some(task), Some(told(wake.kind, to, turn)))
    }
}
