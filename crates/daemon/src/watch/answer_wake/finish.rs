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
//! - nobody has typed there since the paste began, or since the last key
//!   read, the box unchanged by it (below);
//! - for claude, every check a mid-turn Enter makes, whether or not it reads
//!   as working: the session quiet, no call in flight, no ask held
//!   (`witness`), and the same again under the fence (`enter`). A box holding
//!   text hides the line that says claude is working (it drops "esc to
//!   interrupt"), so the screen can't be trusted to say it's between turns.
//!   A claude no hook was ever heard from (started by hand) is pressed
//!   Enter on only when its session registry says it's between turns, as
//!   `type_into`'s own between-turns Enter is (`between_unhooked`, ov-711);
//!   until then it waits. A hooked one whose hooks can't vouch waits;
//! - for codex, which says nothing through hooks, between turns as both the
//!   watcher and a fresh capture read it, and as its rollout says
//!   (`codex_turn`), as any Enter into codex is. codex
//!   keeps "esc to interrupt" with text in its box (measured on 0.153.4), so
//!   that reading holds. Any other agent's screen with text in the box was
//!   never measured, so its answer settles as a paste left in the box.
//!
//! **A key typed since the paste** (ov-827). A dialog answered from inside
//! Far Cooler, typed in the pane from the Mac or a phone, marks input as
//! typing in the box does. The box tells them apart: still holding exactly
//! the text, the key went elsewhere and the wait goes on, its checks counting
//! keys from the last one read; changed, it settles as a paste left in the
//! box. Before ov-827 any key settled it so, and a notice a dialog's answer
//! followed was left in the box for good. Nobody may have typed for
//! `QUIET_MS` (`ready`), longer than a compose's 3 s, and a claude no hook is
//! heard from is read between turns twice, `idle_twice` apart, before its
//! Enter.
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
        // codex's rollout first, so the screen is the newest read. The paste
        // is in the box because an earlier pass proved the agent, so a
        // rollout seen on this process and missed now is a lost one.
        let said = if preset == "codex" { super::codex_turn::said_held(pid, false).await.0 } else { super::registry_turn::Said::Nothing };
        let turn = match self.box_shows(to, preset, &composer::drawn::Expected::plain(text)).await {
            Ok(Ok((true, Turn::Between))) => seen,
            Ok(Ok((true, Turn::During))) => Turn::During,
            Ok(Err(held)) => return Pass::Waiting(held),
            Err(_) => return Pass::Waiting(Held::Unfamiliar),
            Ok(Ok(_)) => return self.settle(wake, Some(task), Some(PASTE_LEFT.into())),
        };
        // A screen between turns while codex's rollout says one runs: wait.
        let turn = match (preset, turn) {
            ("codex", Turn::Between) if said == super::registry_turn::Said::NotIdle => Turn::During,
            _ => turn,
        };
        // A key typed since the paste that left the box exactly the text (it
        // was just read so) went somewhere else: a dialog's answer, from the
        // Mac or a phone. Not the end of the wait (ov-827, as ov-816 made it
        // for a compose); nobody has typed for `QUIET_MS` (`ready`), and the
        // checks below count keys only from the last one read.
        let since = last_input(self.service.root_dir(), to.id).map_or(pasted, |at| pasted.max(at + 1));
        let proven = super::Proven { preset, tty: String::new(), pid, turn, held: false, background: farcooler_core::activity::Background::None };
        let witness = match (super::queues_mid_turn(preset), turn) {
            (true, _) => match self.witness(&proven, to, text).await {
                Some(witness) => Some(witness.pasted_at(since)),
                // No hook was ever heard from it (ov-711): claude's own
                // registry may still say it's between turns, read so twice
                // `idle_twice` apart, as a waiting compose's plain Enter is:
                // one stray frame read idle mid-turn sends no key (ov-827).
                None if turn == Turn::Between && self.between_unhooked(to, &proven, since, text).await => {
                    tokio::time::sleep(self.timing().idle_twice).await;
                    if !self.between_unhooked(to, &proven, since, text).await {
                        return Pass::Waiting(Held::Busy);
                    }
                    None
                }
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
            // No fence: for a claude that no hook was ever heard from
            // (`between_unhooked`), a dialog drawn between `still_between`'s
            // reads and this key would take it, and so would one a background
            // subagent raises. The same window `type_into`'s between-turns
            // Enter has always had; accepted (ov-711 review, finding 3).
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
        self.settle(wake, Some(task), told(wake.kind, to, turn))
    }

    /// `wake`, pasted at `pasted` and never entered, is past its deadline:
    /// its text is taken back out of the box, so it isn't left there as a
    /// draft for someone to send hours later, or to hold every message after
    /// it out (ov-711), and `given_up` says why it wasn't delivered. Taken
    /// back by Bring Here's clear (`bring`): only claude's box, only while it
    /// reads exactly the text, nobody having typed there since the paste,
    /// and a row at a time, each read back. Anything else leaves it, as
    /// before: "Paste left in the composer; not sent". Skipped while another
    /// send holds the box, to be tried on the next pass.
    pub(super) async fn give_up_paste(&self, wake: &PendingWake, task: &Task, pasted: i64, given_up: String) -> Pass {
        let Some((to, text)) = self.addressed(wake, task).await.unwrap_or_default() else {
            return self.settle(wake, Some(task), Some(PASTE_LEFT.into()));
        };
        // This pass holds `wake_pump`, so it never waits for the box: the
        // guard taken here is held to the end of the take-back.
        let Some(typing) = self.try_typing(to.id) else { return Pass::Skipped };
        // A person's own words are theirs to send again.
        let person = self.route_of(wake) == super::route::Kind::Prompt;
        let record = match self.take_back(&to, &text, pasted, typing).await {
            TakenBack::Cleared if person => {
                format!("{given_up} Far Cooler took your message back out of the agent's box, so it wasn't sent. Send it again when the agent is ready.")
            }
            TakenBack::Cleared => format!("{given_up} Far Cooler took it back out of the agent's box, so nothing is left there to send by mistake."),
            TakenBack::Partly => format!(
                "{given_up} Far Cooler began taking it back out of the agent's box but only partly cleared it. Check the box before sending anything there."
            ),
            TakenBack::Left => PASTE_LEFT.into(),
        };
        self.settle(wake, Some(task), Some(record))
    }

    /// Clear `text`, pasted into `to` at `pasted`, from its box, holding the
    /// box (`typing`) throughout. See `give_up_paste`. On a task of its own,
    /// as `bring_draft` is, so a dropped pass can't stop a clear between a
    /// ctrl+u and the ctrl+y that would put it back.
    async fn take_back(&self, to: &Terminal, text: &str, pasted: i64, typing: tokio::sync::OwnedMutexGuard<()>) -> TakenBack {
        if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= pasted) {
            return TakenBack::Left;
        }
        let Some(me) = self.me.upgrade() else { return TakenBack::Left };
        let (to, text) = (to.clone(), text.to_string());
        let run = async move {
            let _typing = typing;
            let to = me.bring_gate(to.id)?;
            let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            let drawn = match me.bring_held(&to, None).await? {
                (drawn, _) if !drawn.is_empty() && squeeze(&drawn) == squeeze(&text) => drawn,
                _ => return Ok(false),
            };
            me.bring_held(&to, Some(drawn)).await.map(|(_, cleared)| cleared)
        };
        match tokio::spawn(run).await.unwrap_or(Err(farcooler_core::DomainError::OperationFailed)) {
            Ok(true) => TakenBack::Cleared,
            Err(farcooler_core::DomainError::Conflict { what: "partly" }) => TakenBack::Partly,
            _ => TakenBack::Left,
        }
    }

    /// Whether claude in `to`, `proven`'s, that no hook was ever heard from, so
    /// none can vouch for a fenced Enter, is proven between turns by what
    /// `type_into`'s own between-turns Enter rests on (`still_between`):
    /// nobody typed since the paste at `pasted`, a fresh capture reads Idle
    /// with the box showing exactly `text`, and claude's session registry
    /// says idle. With text in the box the screen alone can't say a turn
    /// isn't running (see this module's docs), so a claude with no registry
    /// never passes. A hooked claude with no witness waits, as it always has.
    ///
    /// Why (ov-711): the owner's orchestrator is claude started by hand in an
    /// adopted shell pane, so no hook is ever heard from it. A notice marked
    /// pasted to it was never finished, idle or not, and at the give-up was
    /// left typed in its box, where it sat for sixteen hours and kept every
    /// later notice out (`Held::Draft`).
    async fn between_unhooked(&self, to: &Terminal, proven: &super::Proven, pasted: i64, text: &str) -> bool {
        let pid = proven.pid;
        let session = super::registry_turn::config_of(pid).await.and_then(|config| super::mid_turn::transcript_in(&config, pid));
        let Some((session, _)) = session else { return false };
        let asks = self.service.hooks().asks();
        if asks.hooked(&session) || asks.is_holding(to.id) {
            return false;
        }
        self.still_between(to, proven, pasted, &composer::drawn::Expected::plain(text), false).await.is_ok()
    }
}

/// What a give-up's take-back came to.
enum TakenBack {
    /// The box is empty of it.
    Cleared,
    /// Some rows went and couldn't be put back (`bring`'s `partly`).
    Partly,
    /// Not touched, or put back whole.
    Left,
}
