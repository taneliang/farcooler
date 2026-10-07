//! A draft held behind a dialog (ov-385).
//!
//! Ask the Orchestrator, Reverse and Discuss paste a draft into a terminal
//! orchestrator's box and never press Enter (`draft_into`). While a dialog is
//! up (a permission, a question, a menu) there's no box to paste into, and the
//! draft used to be refused, so the apps copied it to the clipboard. A client
//! that asks (`hold_behind_dialog`) now has it held here instead:
//! - **One per terminal.** A newer draft for the same terminal replaces a
//!   waiting one: the box takes one draft. Pasted at once, the newer one
//!   leaves the older reading as withdrawn; held too, it takes the older
//!   one's place, and the client that sent the older finds another id there.
//! - **Delivered** on the first tick that finds the dialog gone and the box
//!   free, through the same checks and the same paste a draft gets now
//!   (`paste_draft`), with no Enter. Then it reads as sent.
//! - **Withdrawn** by the person (`withdraw_draft`), or **expired** when it
//!   hasn't gone in after `GIVE_UP_AFTER_MS`, its terminal is gone, or
//!   someone typed in the pane after the dialog closed: they've moved on, and
//!   a draft landing later would go in ahead of whatever they type next.
//!   **Failed** when the paste itself failed past every check; it may have
//!   reached the box, so it's never tried again.
//! - **Never beside an answer.** A pass runs under the answer wake's own
//!   lock as well (`wake_pump`), skipping the tick while answers are typed,
//!   so an answer and a draft waiting behind one dialog can't both paste.
//! - **Said** through `Terminal.draft_hold`, on every terminal event and
//!   reply, and kept `ENDED_KEPT_MS` after it ends, so the client that sent it
//!   can say how it ended.
//!
//! In memory, not in the store: a draft is the start of a sentence the person
//! is about to finish, worth nothing an hour on, and a restart drops it as it
//! drops a client's connection. The client then reads no hold on the terminal
//! and says the draft wasn't sent. One pass at a time, under `draft_pump`, so
//! a withdrawal and a paste never cross, and a draft is pasted at most once.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, DraftHoldState};
use farcooler_store::models::Terminal;
use uuid::Uuid;

use super::{GIVE_UP_AFTER_MS, Held};
use crate::runtime::{Runtime, last_input};
use crate::watch::{Watcher, now_millis};

/// How long a hold that ended stays on its terminal, for the client that
/// sent it to read how it ended.
pub(crate) const ENDED_KEPT_MS: i64 = 10 * 60 * 1_000;

/// One terminal's held draft.
#[derive(Debug, Clone)]
pub(crate) struct DraftHold {
    id: Uuid,
    /// As it's pasted: `draft_text` already.
    text: String,
    state: DraftHoldState,
    held_ms: i64,
    ended_ms: i64,
    /// When a pass first found no dialog in the way: a key typed from then on
    /// went to the box, not the dialog.
    closed_ms: Option<i64>,
}

impl DraftHold {
    fn wire(&self) -> pb::DraftHold {
        pb::DraftHold {
            id: bytes::Bytes::copy_from_slice(self.id.as_bytes()),
            state: self.state as i32,
            held_ms: self.held_ms,
            expires_ms: self.held_ms + GIVE_UP_AFTER_MS,
            ended_ms: self.ended_ms,
        }
    }

    fn end(&mut self, state: DraftHoldState, now: i64) {
        self.state = state;
        self.ended_ms = now;
    }
}

/// What `draft_into` did with a draft.
#[derive(Debug)]
pub(crate) enum Drafted {
    /// In the box now.
    Pasted,
    /// Held behind a dialog, as this hold.
    Held(pb::DraftHold),
}

impl Watcher {
    /// The hold on `terminal`, as `Terminal.draft_hold` carries it.
    pub(crate) fn draft_hold(&self, terminal: Uuid) -> Option<pb::DraftHold> {
        self.draft_holds.lock().unwrap_or_else(|e| e.into_inner()).get(&terminal).map(DraftHold::wire)
    }

    /// Hold `text` (as `draft_text` made it) for `to`, replacing any hold
    /// waiting there. Called under `draft_pump`.
    pub(super) fn hold_draft(&self, to: &Terminal, text: String) -> pb::DraftHold {
        let now = now_millis();
        let hold =
            DraftHold { id: Uuid::now_v7(), text, state: DraftHoldState::Waiting, held_ms: now, ended_ms: 0, closed_ms: None };
        let wire = hold.wire();
        self.draft_holds.lock().unwrap_or_else(|e| e.into_inner()).insert(to.id, hold);
        self.announce_draft_hold(to.id);
        wire
    }

    /// A draft went into `terminal`'s box directly: a hold waiting there is
    /// replaced by it. Called under `draft_pump`.
    pub(super) fn draft_replaced(&self, terminal: Uuid) {
        let replaced = {
            let mut holds = self.draft_holds.lock().unwrap_or_else(|e| e.into_inner());
            match holds.get_mut(&terminal) {
                Some(hold) if hold.state == DraftHoldState::Waiting => {
                    hold.end(DraftHoldState::Withdrawn, now_millis());
                    true
                }
                _ => false,
            }
        };
        if replaced {
            self.announce_draft_hold(terminal);
        }
    }

    /// The person withdrew the hold `hold` on `terminal`: it's never pasted.
    /// Answers with the hold as it now is, which is how it ended when it
    /// already had. `NotFound` for a hold this runner doesn't have.
    pub(crate) async fn withdraw_draft(&self, terminal: Uuid, hold: Uuid) -> Result<pb::DraftHold> {
        let _one_pass = self.draft_pump.lock().await;
        let (wire, ended) = {
            let mut holds = self.draft_holds.lock().unwrap_or_else(|e| e.into_inner());
            let Some(held) = holds.get_mut(&terminal).filter(|h| h.id == hold) else {
                return Err(DomainError::NotFound);
            };
            let ended = held.state == DraftHoldState::Waiting;
            if ended {
                held.end(DraftHoldState::Withdrawn, now_millis());
            }
            (held.wire(), ended)
        };
        if ended {
            self.announce_draft_hold(terminal);
        }
        Ok(wire)
    }

    /// Make every hold `by` ms older, for tests of the expiry.
    #[cfg(test)]
    pub(crate) fn age_draft_holds_for_tests(&self, by: i64) {
        for hold in self.draft_holds.lock().unwrap_or_else(|e| e.into_inner()).values_mut() {
            hold.held_ms -= by;
        }
    }

    /// A tick's two pumps, off the caller's path and in one task: the
    /// answers (`pump_wakes`, when any may wait), then the held drafts. One
    /// after the other rather than racing for `wake_pump`, so neither can
    /// starve the other: whichever was spawned first used to win the lock.
    pub(crate) fn spawn_pumps(&self) {
        let wakes = self.wakes_hint.load(std::sync::atomic::Ordering::SeqCst);
        let drafts = !self.draft_holds.lock().unwrap_or_else(|e| e.into_inner()).is_empty();
        if !wakes && !drafts {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if wakes {
                    me.pump_wakes().await;
                }
                me.pump_draft_holds().await;
            });
        }
    }

    /// Try every waiting hold once: paste it if the dialog has gone and the
    /// box is free, expire it past `GIVE_UP_AFTER_MS`, and forget one that
    /// ended `ENDED_KEPT_MS` ago. A tick with no hold costs a map lookup.
    pub(crate) async fn pump_draft_holds(&self) {
        if self.draft_holds.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            return;
        }
        // Never while an answer may be typed into the same pane: after it.
        // `wake_pump` first, so the wait holds nothing a person's draft or
        // withdrawal (`draft_into`, `withdraw_draft`, which take only
        // `draft_pump`) would queue behind.
        let _no_answer = self.wake_pump.lock().await;
        let Ok(_one_pass) = self.draft_pump.try_lock() else { return };
        let now = now_millis();
        let waiting: Vec<(Uuid, DraftHold)> = {
            let mut holds = self.draft_holds.lock().unwrap_or_else(|e| e.into_inner());
            holds.retain(|_, h| h.state == DraftHoldState::Waiting || now - h.ended_ms < ENDED_KEPT_MS);
            holds.iter().filter(|(_, h)| h.state == DraftHoldState::Waiting).map(|(t, h)| (*t, h.clone())).collect()
        };
        for (terminal, hold) in waiting {
            let typed_since = |closed: i64| last_input(self.service.root_dir(), terminal).is_some_and(|at| at >= closed);
            let set_closed = |closed: Option<i64>| {
                if let Some(held) = self.draft_holds.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&terminal) {
                    held.closed_ms = closed;
                }
            };
            let ended = match self.service.store.get_terminal(terminal) {
                Err(_) => Some(DraftHoldState::Expired),
                Ok(_) if now - hold.held_ms > GIVE_UP_AFTER_MS => Some(DraftHoldState::Expired),
                Ok(to) => match self.paste_draft(&to, &hold.text).await {
                    Ok(Ok(())) => Some(DraftHoldState::Sent),
                    // Past the gate, and the send failed: it may have reached
                    // the box, so it's never pasted again.
                    Ok(Err(_)) => Some(DraftHoldState::Failed),
                    // A dialog again: a key typed now answers it, not the box.
                    Err(Held::Prompt) => {
                        set_closed(None);
                        None
                    }
                    // No dialog, and something else in the way. A key typed
                    // since the pass that first saw the dialog gone is the
                    // person moving on; from that pass, a key will be.
                    Err(_) => match hold.closed_ms {
                        Some(closed) if typed_since(closed) => Some(DraftHoldState::Expired),
                        Some(_) => None,
                        None => {
                            set_closed(Some(now_millis()));
                            None
                        }
                    },
                },
            };
            let Some(state) = ended else { continue };
            if let Some(held) = self.draft_holds.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&terminal) {
                held.end(state, now_millis());
            }
            self.announce_draft_hold(terminal);
        }
    }

    /// The gate and the paste `draft_into` uses: `ready`, then `proven_tui`,
    /// then one bracketed paste and no Enter. Not recorded as someone typing
    /// (`marks: None`), as an answer isn't. `Err` with why it can't now,
    /// nothing typed; `Ok` with how the send went.
    pub(super) async fn paste_draft(&self, to: &Terminal, text: &str) -> std::result::Result<Result<()>, Held> {
        if !self.service.is_running(to) {
            return Err(Held::NotAnAgent);
        }
        self.ready(to).await?;
        self.proven_tui(to).await?;
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let paste: String = crate::pastes::encode_paste(true, text).iter().map(|b| format!("{b:02x}")).collect();
        if self.fail_sends_for_tests() {
            return Ok(Err(DomainError::OperationFailed));
        }
        Ok(runtime.send_bytes_hex(to.id, &paste).await)
    }

    /// Say `terminal` changed, so every client reads its hold: its whole
    /// state as last observed, as any terminal event carries it, else a
    /// fleet re-read for a terminal never observed.
    fn announce_draft_hold(&self, terminal: Uuid) {
        let Some(me) = self.me.upgrade() else { return };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        runtime.spawn(async move {
            let observed = me.state.lock().await.get(&terminal).cloned();
            match observed {
                Some(observed) => me.announce(terminal, observed, None).await,
                None => me.announce_fleet_changed(),
            }
        });
    }
}
