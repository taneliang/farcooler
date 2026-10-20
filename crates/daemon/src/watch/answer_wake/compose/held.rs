//! A message composed while claude can't be typed to safely yet, held and
//! typed once it can (ov-468).
//!
//! claude takes a message mid-turn into a queue of its own, but the runner
//! presses that Enter only under `mid_turn`'s checks: no call in flight, no
//! dialog announced, a turn boundary seen since the daemon started. A send
//! that fails one of them, or that is a command, which waits for the turn to
//! end, used to be refused as `busy`. Now it's answered Queued, as claude's
//! own queue would answer it, and held behind any message already held for
//! that pane.
//!
//! **Kept.** Each is a row in the store (`Store::hold_compose`), so a
//! restart picks the line up where it was (`resume_held_composes`), as
//! answer wakes are kept. A row is claimed before its message is typed, so
//! one a restart finds claimed is never typed again: it may have reached
//! claude, and the pane says so.
//!
//! **In order.** A pane's slot (`compose_slot`) is held from a send's first
//! look at the line through its typing or its hold, and by the teller for
//! each try, so two sends can't pass each other.
//!
//! **Told.** Every `RETRY` the pane's teller tries the oldest through the
//! whole gate again (`compose_checked`). A draft in the box, someone typing
//! or a dialog up keeps it held, as the gate's own wait does. Told, its row
//! goes. Given up (the agent gone, `GIVE_UP_MS` passed, or a refusal once
//! typing began), it goes too, and the pane's rows say so.
//!
//! **Shown.** While held, the pane's rows carry it as Queued
//! (`Projection::runner_held`), which settles the Queued echo a view drew
//! when it sent it; typed, that row is taken back, and claude's own shows
//! it; given up, it's Withdrawn, with a notice saying why. codex is never
//! held; it refuses mid-turn as before (`compose::codex`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use farcooler_store::ComposeHold;
use uuid::Uuid;

use super::super::Turn;
use crate::watch::{Watcher, now_millis};

/// What `compose_locked` refuses with for a send that should wait rather
/// than be refused. Never sent to a client: `compose_or_hold` holds it.
pub(super) const HOLD: &str = "hold";

/// How often a pane's oldest held message tries the gate again.
pub(crate) const RETRY: Duration = Duration::from_millis(500);

/// How long a message is held before it's given up: a long turn's worth, as
/// a message between agents waits (`messages::GIVE_UP_AFTER_MS`).
pub(crate) const GIVE_UP_MS: i64 = 4 * 60 * 60 * 1_000;

/// The gate's refusals that keep a held message held: nothing was typed,
/// and each passes on its own. Not `unconfirmable`: `session_of` returns it
/// on a registry read that finds no live claude, which includes a read torn
/// by claude's own write (ov-645), and the held message is then given up.
const STILL_HELD: &[&str] = &[HOLD, "draft", "typing", "prompt"];

/// The panes with a teller running, each pane's slot, and whether the line
/// kept across a restart has been picked up.
#[derive(Default)]
pub(crate) struct Tellers {
    running: HashSet<Uuid>,
    slots: HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>,
    resumed: bool,
    /// A test's pause after a send the gate said to hold, before it's held:
    /// says so on the first, waits on the second.
    #[cfg(test)]
    pub(crate) pause_before_hold: Option<(tokio::sync::oneshot::Sender<()>, tokio::sync::oneshot::Receiver<()>)>,
}

pub(crate) type HeldComposes = std::sync::Mutex<Tellers>;

impl Watcher {
    /// `compose_checked`, or, when it says to wait or messages are already
    /// held for `id`, the message held and answered Queued (`Turn::During`).
    pub(super) async fn compose_or_hold(&self, id: Uuid, raw: &str, images: &[(String, Vec<u8>)], files: &[PathBuf]) -> Result<Turn> {
        let _slot = self.compose_slot(id).await;
        if self.service.store.holds_compose(id)? {
            self.compose_request(id, raw, images, files)?;
            self.hold_composed(id, raw, images, files)?;
            return Ok(Turn::During);
        }
        match self.compose_checked(id, raw, images, files).await {
            Err(DomainError::Conflict { what: HOLD }) => {
                #[cfg(test)]
                self.pause_before_hold_for_tests().await;
                self.hold_composed(id, raw, images, files)?;
                Ok(Turn::During)
            }
            other => other,
        }
    }

    #[cfg(test)]
    async fn pause_before_hold_for_tests(&self) {
        let pause = self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pause_before_hold.take();
        if let Some((said, go)) = pause {
            let _ = said.send(());
            let _ = go.await;
        }
    }

    /// `id`'s slot: one send, or one teller's try, at a time.
    async fn compose_slot(&self, id: Uuid) -> tokio::sync::OwnedMutexGuard<()> {
        let slot = Arc::clone(self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).slots.entry(id).or_default());
        slot.lock_owned().await
    }

    /// `compose_slot`, for a test that holds it to wait out a try in flight.
    #[cfg(test)]
    pub(in crate::watch) async fn compose_slot_for_test(&self, id: Uuid) -> tokio::sync::OwnedMutexGuard<()> {
        self.compose_slot(id).await
    }

    /// Hold a message for `id`, behind any held already, show it on the
    /// pane's rows, and start its teller if none runs.
    fn hold_composed(&self, id: Uuid, raw: &str, images: &[(String, Vec<u8>)], files: &[PathBuf]) -> Result<()> {
        let files: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        let hold = self.service.store.hold_compose(id, raw, images, &files)?;
        show_held(&hold);
        self.start_teller(id);
        Ok(())
    }

    /// A teller for `id`, unless one runs.
    fn start_teller(&self, id: Uuid) {
        let started = self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).running.insert(id);
        if started && let Some(me) = self.me.upgrade() {
            tokio::spawn(tell_held(me, id));
        }
    }

    /// Once, after a start: a teller for every pane with messages kept
    /// held across the restart.
    pub(crate) fn resume_held_composes(&self) {
        if std::mem::replace(&mut self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).resumed, true) {
            return;
        }
        for id in self.service.store.compose_hold_terminals().unwrap_or_default() {
            self.start_teller(id);
        }
    }

    /// Try `id`'s held messages, oldest first, until one must wait or none
    /// is left: `false` once none is, and the teller has stopped.
    async fn tell_held_once(&self, id: Uuid) -> bool {
        let _slot = self.compose_slot(id).await;
        loop {
            let hold = match self.service.store.first_compose_hold(id) {
                Ok(Some(hold)) => hold,
                Ok(None) => {
                    self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).running.remove(&id);
                    return false;
                }
                Err(_) => return true,
            };
            show_held(&hold);
            if hold.claimed_at.is_some() {
                self.give_up(&hold, "claimed");
                continue;
            }
            if now_millis() - hold.enqueued_at >= GIVE_UP_MS {
                self.give_up(&hold, "expired");
                continue;
            }
            if self.service.store.claim_compose_hold(hold.id, true).is_err() {
                return true;
            }
            let files: Vec<PathBuf> = hold.files.iter().map(PathBuf::from).collect();
            match self.compose_checked(id, &hold.text, &hold.images, &files).await {
                Err(DomainError::Conflict { what }) if STILL_HELD.contains(&what) => {
                    let _ = self.service.store.claim_compose_hold(hold.id, false);
                    return true;
                }
                Ok(_) => {
                    let _ = self.service.store.drop_compose_hold(hold.id);
                    crate::session_projectors::global().runner_hold(id, |p| p.runner_told(&hold.id.to_string()));
                }
                Err(e) => {
                    tracing::warn!(terminal = %id, error = %e, "a held message wasn't sent");
                    let what = match &e {
                        DomainError::Conflict { what } => what,
                        DomainError::NotFound => "not_running",
                        _ => "failed",
                    };
                    if what != "unconfirmed" {
                        for file in &files {
                            let _ = std::fs::remove_file(file);
                        }
                    }
                    self.give_up(&hold, what);
                }
            }
        }
    }

    /// Give `hold` up, for `what`: gone from the line, and the pane's rows
    /// say why.
    fn give_up(&self, hold: &ComposeHold, what: &str) {
        let _ = self.service.store.drop_compose_hold(hold.id);
        let agent = self.service.store.get_terminal(hold.terminal).map(|t| agent_name(&t.command_preset)).unwrap_or("The agent");
        let why = why_not_sent(agent, &hold.text, what);
        let (id, typed) = (hold.id.to_string(), matches!(what, "claimed" | "unconfirmed"));
        crate::session_projectors::global().runner_hold(hold.terminal, |p| {
            // Typed, or it may have been: claude's own rows show it if it took it.
            if typed {
                p.runner_told(&id);
            }
            p.runner_dropped(&id, &why, now_millis());
        });
    }
}

/// Tell `id`'s held messages in order, each once the gate lets it through,
/// until none is left.
async fn tell_held(me: Arc<Watcher>, id: Uuid) {
    loop {
        tokio::time::sleep(me.timing().held_retry).await;
        if !me.tell_held_once(id).await {
            return;
        }
    }
}

/// `hold` on its pane's rows, as Queued.
fn show_held(hold: &ComposeHold) {
    let text = echo(hold);
    crate::session_projectors::global().runner_hold(hold.terminal, |p| p.runner_held(&hold.id.to_string(), &text, hold.enqueued_at));
}

/// A held message's words, as a view's echo draws them: each image as
/// `[Image]`, then the text.
fn echo(hold: &ComposeHold) -> String {
    let text = hold.text.trim();
    let mut parts = vec!["[Image]"; hold.images.len()];
    if !text.is_empty() {
        parts.push(text);
    }
    parts.join(" ")
}

/// The agent a preset launches, by name.
fn agent_name(preset: &str) -> &'static str {
    match preset.split(':').next() {
        Some("claude") => "Claude",
        Some("codex") => "Codex",
        _ => "The agent",
    }
}

/// What the pane's rows say of a held message given up for `what`.
pub(crate) fn why_not_sent(agent: &str, text: &str, what: &str) -> String {
    let mut quoted: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();
    if quoted.chars().count() < text.split_whitespace().collect::<Vec<_>>().join(" ").chars().count() {
        quoted.push('…');
    }
    match what {
        "claimed" => format!("“{quoted}” may not have reached {agent}: the runner restarted as it was typed. Check the terminal before sending it again."),
        "unconfirmed" => format!("{agent} didn’t confirm it took “{quoted}”. Check the terminal before sending it again."),
        "expired" => format!("Not sent: “{quoted}”. {agent} didn’t take it within 4 hours."),
        "not_running" | "not_an_agent" => format!("Not sent: “{quoted}”. {agent} isn’t running in this pane anymore."),
        "paste_left" | "dialog" => format!("Not sent: “{quoted}”. It didn’t land as typed, so it was left in {agent}’s box."),
        "left_at_shell" => format!("Not sent: “{quoted}”. {agent} quit as it was typed."),
        "queued_unseen" => format!("{agent} took “{quoted}” into its queue but never showed it ran. Check the terminal before sending it again."),
        _ => format!("Not sent: “{quoted}”. The runner couldn’t type it."),
    }
}
