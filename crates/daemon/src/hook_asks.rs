//! Permission asks a claude TUI is waiting on, held while a phone may answer.
//!
//! Claude runs its `PermissionRequest` hook while it draws its own dialog. The
//! hook connects to `hook_ingress`, and `serve` holds the connection open here
//! for up to `LONGEST_HOLD`, so the lock screen and the watch can answer the
//! same question the keyboard can. This is the state of those held asks and
//! nothing else: no sockets, and no screens. It is told what a screen shows;
//! it never looks.
//!
//! **At most one ask per terminal.** A device finds its ask by the id it was
//! shown, but the keyboard's answer is only ever seen per pane (the dialog
//! leaves the screen, or a turn ends), so two asks held on one pane could not
//! be told apart by the signal that withdraws them. A newer ask on the same
//! pane therefore withdraws the older one, whose dialog is left to the
//! keyboard.
//!
//! **Every ask ends exactly once**, however it ends: a device answers, the
//! dialog leaves the screen, a turn ends or begins, a newer ask supersedes it,
//! its hook goes away, or its terminal is forgotten. All of those go through
//! one `settle`, which removes the entry under the lock, so no two of them can
//! both end the same ask. `settle` is also the only place a `Resolved` is
//! recorded, which is what tells every surface to stop offering buttons.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use farcooler_agent::event::AgentEvent;
use farcooler_agent_hooks::wire::Decision;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::hook_ingress::EventSink;

/// What every held ask's id starts with, so `terminal.agent_answer` can tell
/// an answer for a held hook from one for an ACP shim without asking both.
pub const HOOK_ASK_PREFIX: &str = "hook-ask-";

/// How long a device's answer waits to hear that it reached the hook.
///
/// A phone is told "sent" only once `serve` has written the verdict to the
/// hook's socket. The glance surfaces must not report a success they cannot
/// confirm, so an answer that could not be written is refused, not accepted.
const ACK_BOUND: Duration = Duration::from_secs(2);

/// Consecutive samples without the dialog, after it was seen, that mean the
/// keyboard answered. Two, as the watcher's own `CONFIRMATIONS`: one sample can
/// catch a redraw halfway.
const DIALOG_GONE_AFTER: u8 = 2;

/// How a held ask ended, as `serve` hears it.
#[derive(Debug)]
pub struct Settled {
    /// `None` is "no decision": the hook prints nothing and claude keeps its
    /// own dialog, or has already taken the keyboard's answer.
    pub decision: Option<Decision>,
    /// Sent by `serve` once the verdict is on the hook's socket. Present only
    /// for a device's answer, which is the only ending anybody waits on.
    pub ack: Option<oneshot::Sender<()>>,
}

/// Why a device's answer was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerRefused {
    /// Nothing is held under that id on that terminal: it was answered,
    /// withdrawn, or never existed.
    NotHeld,
    /// The ask offers `allow` and `deny`, and this was neither. The ask stays
    /// held.
    UnknownOption,
    /// The ask was settled, but the hook's connection never confirmed the
    /// verdict landed.
    NotDelivered,
}

struct Held {
    id: String,
    since: Instant,
    seen_dialog: bool,
    absent_samples: u8,
    reply: oneshot::Sender<Settled>,
}

pub struct HookAsks {
    held: Mutex<HashMap<Uuid, Held>>,
    /// `HookIngress`'s sink, shared, so a `Resolved` lands in the same ring as
    /// the `Permission` it ends. `None` until `listen` has installed it.
    sink: Arc<Mutex<Option<EventSink>>>,
}

impl HookAsks {
    pub fn new(sink: Arc<Mutex<Option<EventSink>>>) -> Self {
        Self { held: Mutex::new(HashMap::new()), sink }
    }

    /// Hold a new ask on `terminal`, superseding any older one there.
    ///
    /// Returns the ask's id, which a device echoes back, and the channel its
    /// ending arrives on.
    pub fn hold(&self, terminal: Uuid) -> (String, oneshot::Receiver<Settled>) {
        let id = format!("{HOOK_ASK_PREFIX}{}", Uuid::now_v7());
        let (reply, rx) = oneshot::channel();
        let held = Held { id: id.clone(), since: Instant::now(), seen_dialog: false, absent_samples: 0, reply };
        let older = self.lock().insert(terminal, held);
        if let Some(older) = older {
            self.end(terminal, older, Settled { decision: None, ack: None }, "", true, "superseded");
        }
        (id, rx)
    }

    /// A device's answer to the ask held under `id` on `terminal`.
    ///
    /// `decider` names the device, and a deny says it: "Denied from iPhone".
    /// Returns once the verdict is on the hook's socket, or refuses.
    pub async fn answer(
        &self,
        terminal: Uuid,
        id: &str,
        option: &str,
        decider: &str,
    ) -> Result<(), AnswerRefused> {
        if !self.lock().get(&terminal).is_some_and(|held| held.id == id) {
            return Err(AnswerRefused::NotHeld);
        }
        let decision = match option {
            "allow" => Decision::Allow,
            "deny" => Decision::Deny { message: format!("Denied from {decider}") },
            _ => return Err(AnswerRefused::UnknownOption),
        };
        let (ack, landed) = oneshot::channel();
        let settled = Settled { decision: Some(decision), ack: Some(ack) };
        if !self.settle(terminal, Some(id), settled, option, true, "answered") {
            // Ended by something else between the look above and now.
            return Err(AnswerRefused::NotHeld);
        }
        match tokio::time::timeout(ACK_BOUND, landed).await {
            Ok(Ok(())) => Ok(()),
            _ => Err(AnswerRefused::NotDelivered),
        }
    }

    /// What one sample of `terminal`'s screen showed: claude's permission
    /// dialog, or not.
    pub fn saw_screen(&self, terminal: Uuid, dialog_up: bool) {
        let gone = {
            let mut held = self.lock();
            let Some(ask) = held.get_mut(&terminal) else { return };
            if dialog_up {
                ask.seen_dialog = true;
                ask.absent_samples = 0;
                false
            } else if ask.seen_dialog {
                ask.absent_samples = ask.absent_samples.saturating_add(1);
                ask.absent_samples >= DIALOG_GONE_AFTER
            } else {
                // Not drawn yet: claude draws the dialog as it runs the hook,
                // and a sample can land between the two.
                false
            }
        };
        if gone {
            self.settle(terminal, None, Settled { decision: None, ack: None }, "", true, "dialog left the screen");
        }
    }

    /// A turn ended or began on `terminal`, which it cannot do with a dialog
    /// up.
    pub fn turn_boundary(&self, terminal: Uuid) {
        self.settle(terminal, None, Settled { decision: None, ack: None }, "", true, "turn boundary");
    }

    /// `serve` giving up on its own ask: the hold ran out, or the hook went
    /// away.
    pub fn withdraw(&self, terminal: Uuid, id: &str) {
        self.settle(terminal, Some(id), Settled { decision: None, ack: None }, "", true, "withdrawn");
    }

    /// `terminal`'s row is gone. Its ask ends with no decision and no
    /// `Resolved`: the ring goes with the row, and recording into it would
    /// bring an entry back for a terminal nothing can reach.
    pub fn forget(&self, terminal: Uuid) {
        self.settle(terminal, None, Settled { decision: None, ack: None }, "", false, "forgotten");
    }

    /// Whether an ask is held on `terminal`. For tests and for logs.
    pub fn is_holding(&self, terminal: Uuid) -> bool {
        self.lock().contains_key(&terminal)
    }

    /// End the ask held on `terminal`, if there is one and, when `id` is
    /// given, it is that ask. The one way any ask ends; returns whether this
    /// call was the one that ended it.
    ///
    /// The entry leaves the map under the lock, so of two endings racing for
    /// one ask exactly one finds it.
    fn settle(
        &self,
        terminal: Uuid,
        id: Option<&str>,
        settled: Settled,
        chosen: &str,
        record: bool,
        why: &str,
    ) -> bool {
        let held = {
            let mut held = self.lock();
            if !held.get(&terminal).is_some_and(|ask| id.is_none_or(|id| ask.id == id)) {
                return false;
            }
            held.remove(&terminal)
        };
        let Some(held) = held else { return false };
        self.end(terminal, held, settled, chosen, record, why);
        true
    }

    /// Tell `serve` how an ask it holds ended and, when `record`, tell every
    /// surface too, with a `Resolved` naming what was `chosen` ("" for no
    /// decision).
    ///
    /// Only for an entry already out of the map, which is what makes the one
    /// `Resolved` per ask a property rather than a hope. The sink is called
    /// with no lock held: it reaches `AgentSupervisor::record`, which takes
    /// locks of its own.
    fn end(&self, terminal: Uuid, held: Held, settled: Settled, chosen: &str, record: bool, why: &str) {
        tracing::debug!(%terminal, id = %held.id, held_for = ?held.since.elapsed(), why, "a held ask ended");
        // `serve` may be gone already (its hook hung up); the ending is still
        // an ending, and the surfaces still need to hear it.
        let _ = held.reply.send(settled);
        if !record {
            return;
        }
        let sink = self.sink.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(sink) = sink {
            sink(terminal, vec![AgentEvent::Resolved { id: held.id, chosen: chosen.to_string() }]);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Held>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Recorded = Arc<Mutex<Vec<(Uuid, AgentEvent)>>>;

    fn ledger() -> (HookAsks, Recorded) {
        let recorded: Recorded = Arc::default();
        let into = recorded.clone();
        let sink: EventSink = Arc::new(move |terminal, events| {
            let mut into = into.lock().unwrap();
            into.extend(events.into_iter().map(|e| (terminal, e)));
        });
        (HookAsks::new(Arc::new(Mutex::new(Some(sink)))), recorded)
    }

    /// Every `Resolved` recorded for `id`, as what was chosen.
    fn resolved(recorded: &Recorded, id: &str) -> Vec<String> {
        recorded
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::Resolved { id: r, chosen } if r == id => Some(chosen.clone()),
                _ => None,
            })
            .collect()
    }

    /// What `serve` does with an ending: take it, acknowledge it as written,
    /// and hand back the decision.
    fn a_hook_waiting_on(rx: oneshot::Receiver<Settled>) -> tokio::task::JoinHandle<Option<Decision>> {
        tokio::spawn(async move {
            let settled = rx.await.expect("an ending arrives");
            if let Some(ack) = settled.ack {
                let _ = ack.send(());
            }
            settled.decision
        })
    }

    #[tokio::test]
    async fn an_allow_from_a_device_decides_the_held_ask() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = asks.hold(pane);
        assert!(id.starts_with(HOOK_ASK_PREFIX));
        let hook = a_hook_waiting_on(rx);
        assert_eq!(asks.answer(pane, &id, "allow", "iPhone").await, Ok(()));
        assert_eq!(hook.await.unwrap(), Some(Decision::Allow));
        assert_eq!(resolved(&recorded, &id), ["allow"]);
        assert!(!asks.is_holding(pane));
    }

    #[tokio::test]
    async fn a_deny_names_the_device_that_sent_it() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = asks.hold(pane);
        let hook = a_hook_waiting_on(rx);
        assert_eq!(asks.answer(pane, &id, "deny", "iPhone").await, Ok(()));
        assert_eq!(
            hook.await.unwrap(),
            Some(Decision::Deny { message: "Denied from iPhone".to_string() })
        );
        assert_eq!(resolved(&recorded, &id), ["deny"]);
    }

    #[tokio::test]
    async fn an_option_the_ask_never_offered_is_refused_and_the_ask_stays_held() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, _rx) = asks.hold(pane);
        assert_eq!(
            asks.answer(pane, &id, "allow_always", "iPhone").await,
            Err(AnswerRefused::UnknownOption)
        );
        assert!(asks.is_holding(pane), "a wrong button must not cost the ask");
        assert!(resolved(&recorded, &id).is_empty());
    }

    #[tokio::test]
    async fn a_second_answer_to_the_same_ask_is_refused() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = asks.hold(pane);
        let hook = a_hook_waiting_on(rx);
        assert_eq!(asks.answer(pane, &id, "deny", "iPhone").await, Ok(()));
        hook.await.unwrap();
        assert_eq!(asks.answer(pane, &id, "allow", "iPad").await, Err(AnswerRefused::NotHeld));
        assert_eq!(resolved(&recorded, &id), ["deny"], "the first answer stands");
    }

    #[tokio::test]
    async fn an_answer_naming_an_id_nobody_holds_is_refused() {
        let (asks, _) = ledger();
        let pane = Uuid::now_v7();
        let (_id, _rx) = asks.hold(pane);
        assert_eq!(
            asks.answer(pane, "hook-ask-nobody", "allow", "iPhone").await,
            Err(AnswerRefused::NotHeld)
        );
        assert!(asks.is_holding(pane), "someone else's id must not end this ask");
        let (other, _rx) = asks.hold(Uuid::now_v7());
        assert_eq!(
            asks.answer(pane, &other, "allow", "iPhone").await,
            Err(AnswerRefused::NotHeld),
            "an id held on another pane is not this pane's"
        );
    }

    #[tokio::test]
    async fn a_newer_ask_on_the_same_pane_withdraws_the_older() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (older, older_rx) = asks.hold(pane);
        let (newer, _newer_rx) = asks.hold(pane);
        assert_ne!(older, newer);
        let settled = older_rx.await.expect("the older ask hears its ending");
        assert_eq!(settled.decision, None, "the older dialog is left to the keyboard");
        assert_eq!(resolved(&recorded, &older), [""]);
        assert!(resolved(&recorded, &newer).is_empty());
        assert_eq!(
            asks.answer(pane, &older, "allow", "iPhone").await,
            Err(AnswerRefused::NotHeld)
        );
        assert!(asks.is_holding(pane));
    }

    /// Each way an ask can end, and how many `Resolved` it leaves: one, except
    /// `forget`, whose ring is deleted with the row and gets none.
    #[tokio::test]
    async fn every_ask_is_resolved_exactly_once() {
        let (asks, recorded) = ledger();
        let mut ids = Vec::new();

        let pane = Uuid::now_v7();
        let (id, rx) = asks.hold(pane);
        let hook = a_hook_waiting_on(rx);
        asks.answer(pane, &id, "allow", "iPhone").await.unwrap();
        hook.await.unwrap();
        asks.withdraw(pane, &id);
        asks.turn_boundary(pane);
        ids.push(("answer", id));

        let pane = Uuid::now_v7();
        let (id, _rx) = asks.hold(pane);
        asks.withdraw(pane, &id);
        asks.withdraw(pane, &id);
        asks.turn_boundary(pane);
        ids.push(("withdraw", id));

        let pane = Uuid::now_v7();
        let (id, _rx) = asks.hold(pane);
        let (newer, _newer_rx) = asks.hold(pane);
        asks.withdraw(pane, &id);
        ids.push(("supersede", id));

        asks.turn_boundary(pane);
        asks.turn_boundary(pane);
        ids.push(("turn boundary", newer));

        let pane = Uuid::now_v7();
        let (id, _rx) = asks.hold(pane);
        for up in [true, false, false, false, false] {
            asks.saw_screen(pane, up);
        }
        asks.withdraw(pane, &id);
        ids.push(("dialog gone", id));

        for (ending, id) in &ids {
            assert_eq!(resolved(&recorded, id).len(), 1, "{ending} resolved {id} other than once");
        }

        let pane = Uuid::now_v7();
        let (id, rx) = asks.hold(pane);
        asks.forget(pane);
        asks.forget(pane);
        asks.withdraw(pane, &id);
        assert_eq!(rx.await.expect("an ending arrives").decision, None);
        assert!(resolved(&recorded, &id).is_empty(), "forget records nothing into a ring that is gone");
    }

    #[tokio::test]
    async fn the_dialog_leaving_withdraws_the_ask_only_after_it_was_seen() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = asks.hold(pane);
        asks.saw_screen(pane, true);
        asks.saw_screen(pane, false);
        assert!(asks.is_holding(pane));
        asks.saw_screen(pane, false);
        assert!(!asks.is_holding(pane), "two samples without the dialog: the keyboard answered");
        assert_eq!(rx.await.expect("an ending arrives").decision, None);
        assert_eq!(resolved(&recorded, &id), [""]);
    }

    #[tokio::test]
    async fn a_dialog_missing_for_one_sample_does_not_withdraw_the_ask() {
        let (asks, _) = ledger();
        let pane = Uuid::now_v7();
        let (_id, _rx) = asks.hold(pane);
        for up in [true, false, true, false, true] {
            asks.saw_screen(pane, up);
        }
        assert!(asks.is_holding(pane), "a one-frame redraw is not an answer");
    }

    #[tokio::test]
    async fn a_dialog_not_yet_drawn_does_not_withdraw_the_ask() {
        let (asks, _) = ledger();
        let pane = Uuid::now_v7();
        let (_id, _rx) = asks.hold(pane);
        for _ in 0..10 {
            asks.saw_screen(pane, false);
        }
        assert!(asks.is_holding(pane), "a dialog never seen cannot have left");
    }

    #[tokio::test]
    async fn a_turn_boundary_withdraws_a_held_ask() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = asks.hold(pane);
        asks.turn_boundary(Uuid::now_v7());
        assert!(asks.is_holding(pane), "another pane's turn is not this one's");
        asks.turn_boundary(pane);
        assert!(!asks.is_holding(pane));
        assert_eq!(rx.await.expect("an ending arrives").decision, None);
        assert_eq!(resolved(&recorded, &id), [""]);
    }

    #[tokio::test]
    async fn forgetting_a_terminal_withdraws_its_ask() {
        let (asks, _) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = asks.hold(pane);
        asks.forget(pane);
        assert!(!asks.is_holding(pane));
        assert_eq!(rx.await.expect("the hook is released").decision, None);
        assert_eq!(asks.answer(pane, &id, "allow", "iPhone").await, Err(AnswerRefused::NotHeld));
    }
}
