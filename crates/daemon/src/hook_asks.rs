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
//!
//! **Sessions heard from** (ov-360). Apart from the held asks, this keeps,
//! per session id, that a hook arrived from it at all and when its last gate
//! (`PermissionRequest`, for a tool, a question or a plan alike) began. It's
//! what lets the daemon press Enter in a working claude: a dialog claude
//! raises mid-turn takes the Enter meant for a queued message, and the gate
//! is heard before the dialog is drawn (`answer_wake::mid_turn`). Kept by
//! session, not terminal: a claude started by hand routes to no terminal,
//! and its session id is known from its own registry. A session never heard
//! from has no such signal, and isn't typed into mid-turn.
//!
//! **The fence** (ov-360). Per session, a lock and a mark: a tool call in
//! flight. Claude's `PreToolUse` hook marks it, then takes the lock, then is
//! answered (`tool_starting`); claude draws no permission dialog before that
//! answer. The mid-turn Enter takes the same lock, checks the mark, and holds
//! the lock until its key has landed (`answer_wake::mid_turn`). So an Enter
//! and a dialog can't overlap: whichever takes the lock first finishes
//! first. `PostToolUse`, `PostToolUseFailure`, `Stop` and `UserPromptSubmit`
//! end the mark (`tool_ended`); a turn starting or ending has no tool call
//! in flight.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use farcooler_agent::event::AgentEvent;
use farcooler_agent_hooks::wire::{Decision, LONGEST_HOLD};
use tokio::sync::{oneshot, watch};
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
    /// When it was held, by the wall clock, for the needs-you item's `since`.
    /// Beside `since` rather than instead of it: the hold's own timing wants a
    /// clock that never jumps.
    at: SystemTime,
    /// claude's `tool_name`, as the hook's payload gave it, for the lock
    /// screen's "Bash · Billing". Unchecked here; `push::WireAsk` decides
    /// what may cross the relay.
    tool: Option<String>,
    /// When the hold ends, by the wall clock: `at + hold`. `at` is stamped
    /// before the hold's own timer starts, so the real end is never earlier.
    until: SystemTime,
    seen_dialog: bool,
    absent_samples: u8,
    /// Whether its `Permission` was recorded. Only an offered ask is owed a
    /// `Resolved`.
    offered: bool,
    reply: oneshot::Sender<Settled>,
}

/// The ask open on one terminal, as a notice carries it (ov-57 T0 C1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAsk {
    pub id: String,
    pub tool: Option<String>,
    /// When the daemon stops holding it, by the wall clock.
    pub until: SystemTime,
}

pub struct HookAsks {
    held: Mutex<HashMap<Uuid, Held>>,
    /// Per session id: when a hook was last heard from it, and when its last
    /// gate began. See this module's docs, "Sessions heard from".
    sessions: Mutex<HashMap<String, Heard>>,
    /// `HookIngress`'s sink, shared, so a `Resolved` lands in the same ring as
    /// the `Permission` it ends. `None` until `listen` has installed it.
    sink: Arc<Mutex<Option<EventSink>>>,
    /// Bumped whenever the set of open (offered) asks changes: an offer, or
    /// the end of an offered ask. The watcher subscribes, so a pane that stays
    /// blocked can tell the relay its ask came or went (`kind:"ask"`).
    changes: watch::Sender<u64>,
}

/// What was last heard from one session.
#[derive(Debug, Clone)]
struct Heard {
    at: Instant,
    gate: Option<Instant>,
    /// A `PreToolUse` came and no end of the call yet.
    tool_in_flight: bool,
    /// The fence's lock.
    fence: Arc<tokio::sync::Mutex<()>>,
}

/// The longest a `PreToolUse` waits on the fence's lock. Above the longest
/// the mid-turn Enter can hold it (`answer_wake::mid_turn::LONGEST_FENCE`),
/// so a `PreToolUse` is never answered while an Enter holds it: the Enter
/// gives up, and lets go, first. A hook kept waiting is first told so with a
/// hold (`hook_ingress::fence`), which gives it this long.
pub const FENCE_HOLD: Duration = Duration::from_secs(10);

/// Sessions remembered before the oldest are let go: far more than a runner
/// runs at once.
const SESSIONS_KEPT: usize = 512;

impl HookAsks {
    pub fn new(sink: Arc<Mutex<Option<EventSink>>>) -> Self {
        Self {
            held: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            sink,
            changes: watch::Sender::new(0),
        }
    }

    /// A hook arrived from `session`; `gate` when it's one the agent waits
    /// on, which it raises as it puts up a dialog. Recorded before anything
    /// else is done with the hook.
    pub fn heard(&self, session: &str, gate: bool) {
        let now = Instant::now();
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if sessions.len() >= SESSIONS_KEPT && !sessions.contains_key(session) {
            let oldest = sessions.iter().min_by_key(|(_, h)| h.at).map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                sessions.remove(&oldest);
            }
        }
        let heard = sessions.entry(session.to_string()).or_insert_with(|| Heard {
            at: now,
            gate: None,
            tool_in_flight: false,
            fence: Arc::default(),
        });
        heard.at = now;
        if gate {
            heard.gate = Some(now);
        }
    }

    /// The fence's lock for `session`, or `None` for a session never heard
    /// from.
    pub fn fence(&self, session: &str) -> Option<Arc<tokio::sync::Mutex<()>>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(session).map(|h| h.fence.clone())
    }

    /// A `PreToolUse` from `session`: mark the call in flight, and hand back
    /// the fence its answer waits for. `None` for a session never heard from.
    pub fn mark_tool_starting(&self, session: &str) -> Option<Arc<tokio::sync::Mutex<()>>> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let heard = sessions.get_mut(session)?;
        heard.tool_in_flight = true;
        Some(heard.fence.clone())
    }

    /// `mark_tool_starting`, then wait (up to `FENCE_HOLD`) for any Enter
    /// holding the fence to let go. The hook is answered after this returns.
    pub async fn tool_starting(&self, session: &str) {
        if let Some(fence) = self.mark_tool_starting(session) {
            let _ = tokio::time::timeout(FENCE_HOLD, fence.lock()).await;
        }
    }

    /// The tool call in flight in `session`, if any, is over.
    pub fn tool_ended(&self, session: &str) {
        if let Some(heard) = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get_mut(session) {
            heard.tool_in_flight = false;
        }
    }

    /// Whether `session` has a tool call in flight.
    pub fn tool_in_flight(&self, session: &str) -> bool {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(session).is_some_and(|h| h.tool_in_flight)
    }

    /// Whether any hook was ever heard from `session` by this daemon.
    pub fn hooked(&self, session: &str) -> bool {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).contains_key(session)
    }

    /// Whether `session` began a gate at or after `since`: a dialog may be up,
    /// or about to be drawn.
    pub fn gated_since(&self, session: &str, since: Instant) -> bool {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session).and_then(|h| h.gate).is_some_and(|gate| gate >= since)
    }

    /// Told whenever an ask is offered or an offered ask ends. What changed
    /// is not said; `open_on` is the answer.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn changed(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// `hold_for` with no tool and the longest hold, for tests and callers
    /// that know neither.
    pub fn hold(&self, terminal: Uuid) -> (String, oneshot::Receiver<Settled>) {
        self.hold_for(terminal, None, LONGEST_HOLD)
    }

    /// Hold a new ask on `terminal`, superseding any older one there, for
    /// `hold`: the ingress's own timer, which is what makes `until` true.
    ///
    /// Returns the ask's id, which a device echoes back, and the channel its
    /// ending arrives on.
    pub fn hold_for(
        &self,
        terminal: Uuid,
        tool: Option<&str>,
        hold: Duration,
    ) -> (String, oneshot::Receiver<Settled>) {
        let id = format!("{HOOK_ASK_PREFIX}{}", Uuid::now_v7());
        let (reply, rx) = oneshot::channel();
        let at = SystemTime::now();
        let held = Held {
            id: id.clone(),
            since: Instant::now(),
            at,
            tool: tool.map(str::to_string),
            until: at + hold,
            seen_dialog: false,
            absent_samples: 0,
            offered: false,
            reply,
        };
        let sink = self.sink();
        let mut asks = self.lock();
        if let Some(older) = asks.insert(terminal, held) {
            let was_open = older.offered;
            let settled = Settled { decision: None, ack: None };
            end(terminal, older, settled, "", true, "superseded", sink.as_ref());
            if was_open {
                self.changed();
            }
        }
        (id, rx)
    }

    /// Offer the ask held under `id` on `terminal` to every surface, by
    /// recording `permission`. Returns whether it was still held to offer.
    ///
    /// Recorded under the ledger's lock, so no ending can come between the
    /// check and the record: every ending removes the entry under that same
    /// lock, and records its `Resolved` before letting go of it. So a `Permission` always
    /// precedes its `Resolved`, and an ask that ended before it was offered
    /// is never offered at all, which is also what keeps an offer after
    /// `forget` from recreating the terminal's ring. The sink takes
    /// `AgentSupervisor`'s locks, never this one, so holding this across it
    /// cannot deadlock.
    pub fn offer(&self, terminal: Uuid, id: &str, permission: AgentEvent) -> bool {
        let sink = self.sink();
        let mut held = self.lock();
        let Some(ask) = held.get_mut(&terminal).filter(|ask| ask.id == id) else {
            return false;
        };
        ask.offered = true;
        if let Some(sink) = sink {
            sink(terminal, vec![permission]);
        }
        self.changed();
        true
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

    /// Every ask held and offered, as (terminal, id, when it was held), for
    /// the needs-you list.
    ///
    /// Offered only: an ask not yet offered has no `Permission` in its
    /// terminal's ring, so no surface has its options to answer with. At most
    /// one per terminal, as the ledger holds them.
    pub fn open(&self) -> Vec<(Uuid, String, SystemTime)> {
        self.lock()
            .iter()
            .filter(|(_, ask)| ask.offered)
            .map(|(terminal, ask)| (*terminal, ask.id.clone(), ask.at))
            .collect()
    }

    /// The ask held and offered on `terminal`, if there is one: the same
    /// filter as `open`, with what a notice carries.
    pub fn open_on(&self, terminal: Uuid) -> Option<OpenAsk> {
        self.lock()
            .get(&terminal)
            .filter(|ask| ask.offered)
            .map(|ask| OpenAsk { id: ask.id.clone(), tool: ask.tool.clone(), until: ask.until })
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
    /// one ask exactly one finds it. Its `Resolved` is recorded before the
    /// lock is let go: `forget` takes this lock before the terminal's ring
    /// is deleted, so no `Resolved` can land after and bring the ring back.
    fn settle(
        &self,
        terminal: Uuid,
        id: Option<&str>,
        settled: Settled,
        chosen: &str,
        record: bool,
        why: &str,
    ) -> bool {
        let sink = self.sink();
        let mut asks = self.lock();
        if !asks.get(&terminal).is_some_and(|ask| id.is_none_or(|id| ask.id == id)) {
            return false;
        }
        let Some(held) = asks.remove(&terminal) else { return false };
        let was_open = held.offered;
        end(terminal, held, settled, chosen, record, why, sink.as_ref());
        if was_open {
            self.changed();
        }
        true
    }

    /// The sink, cloned out, so no call holds its lock and the ledger's at
    /// once.
    fn sink(&self) -> Option<EventSink> {
        self.sink.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Held>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Tell `serve` how an ask it held ended and, when `record`, tell every
/// surface too, with a `Resolved` naming what was `chosen` ("" for no
/// decision).
///
/// Only for an entry already out of the map, and only with the ledger's lock
/// still held by the caller: that is what makes the one `Resolved` per ask a
/// property rather than a hope, and what keeps it from landing after
/// `forget`. The sink reaches `AgentSupervisor::record`, which takes its own
/// locks and never this ledger's, so recording here cannot deadlock.
fn end(
    terminal: Uuid,
    held: Held,
    settled: Settled,
    chosen: &str,
    record: bool,
    why: &str,
    sink: Option<&EventSink>,
) {
    tracing::debug!(%terminal, id = %held.id, held_for = ?held.since.elapsed(), why, "a held ask ended");
    // `serve` may be gone already (its hook hung up); the ending is still an
    // ending, and the surfaces still need to hear it.
    let _ = held.reply.send(settled);
    if !record || !held.offered {
        return;
    }
    if let Some(sink) = sink {
        sink(terminal, vec![AgentEvent::Resolved { id: held.id, chosen: chosen.to_string() }]);
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

    fn a_permission(id: &str) -> AgentEvent {
        AgentEvent::Permission { id: id.to_string(), tool_call: String::new(), options: vec![] }
    }

    /// Everything recorded for `pane`, as (kind, id), in order.
    fn story(recorded: &Recorded, pane: Uuid) -> Vec<(&'static str, String)> {
        recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| *t == pane)
            .filter_map(|(_, e)| match e {
                AgentEvent::Permission { id, .. } => Some(("Permission", id.clone())),
                AgentEvent::Resolved { id, .. } => Some(("Resolved", id.clone())),
                _ => None,
            })
            .collect()
    }

    /// An ask, held and offered, as `serve` leaves one.
    fn offered(asks: &HookAsks, pane: Uuid) -> (String, oneshot::Receiver<Settled>) {
        let (id, rx) = asks.hold(pane);
        assert!(asks.offer(pane, &id, a_permission(&id)), "a fresh ask is still held");
        (id, rx)
    }

    /// An offered ask is open until it settles, and then it isn't.
    #[tokio::test]
    async fn open_lists_a_held_ask_and_forgets_it_once_settled() {
        let (asks, _recorded) = ledger();
        let pane = Uuid::now_v7();
        let before = SystemTime::now();
        let (id, rx) = offered(&asks, pane);
        let open = asks.open();
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!((open[0].0, open[0].1.as_str()), (pane, id.as_str()));
        assert!(open[0].2 >= before, "held at a wall-clock time before the hold");
        let hook = a_hook_waiting_on(rx);
        asks.answer(pane, &id, "allow", "iPhone").await.unwrap();
        hook.await.unwrap();
        assert_eq!(asks.open(), vec![], "a settled ask is still listed");
    }

    /// What a notice carries about an open ask: its tool, and when its hold
    /// ends, which is `at + hold` (ov-57 T0 C1). Not open until offered.
    #[tokio::test]
    async fn open_reports_tool_and_hold_end() {
        let (asks, _recorded) = ledger();
        let pane = Uuid::now_v7();
        let hold = Duration::from_secs(60);
        let (id, _rx) = asks.hold_for(pane, Some("Bash"), hold);
        assert_eq!(asks.open_on(pane), None, "held but not offered is not open");
        assert!(asks.offer(pane, &id, a_permission(&id)));
        let open = asks.open_on(pane).expect("an offered ask is open");
        let at = asks.open()[0].2;
        assert_eq!(open.id, id);
        assert_eq!(open.tool.as_deref(), Some("Bash"));
        assert_eq!(open.until, at + hold, "the hold ends `hold` after it began");
        assert_eq!(asks.open_on(Uuid::now_v7()), None, "another pane has none");
        asks.withdraw(pane, &id);
        assert_eq!(asks.open_on(pane), None, "an ended ask is not open");
    }

    /// The watcher hears every change to what is open: an offer, and an
    /// offered ask ending. Not a hold that nobody was offered.
    #[tokio::test]
    async fn offers_and_endings_are_announced() {
        let (asks, _recorded) = ledger();
        let mut changes = asks.subscribe();
        let pane = Uuid::now_v7();
        let (quiet, _rx) = asks.hold(pane);
        asks.withdraw(pane, &quiet);
        assert!(!changes.has_changed().unwrap(), "nothing was open, so nothing changed");
        let (_first, _rx) = offered(&asks, pane);
        assert!(changes.has_changed().unwrap(), "an offer is a change");
        changes.borrow_and_update();
        let (_newer, _newer_rx) = asks.hold(pane);
        assert!(changes.has_changed().unwrap(), "a superseded offer is a change");
        changes.borrow_and_update();
        asks.turn_boundary(pane);
        assert!(!changes.has_changed().unwrap(), "the newer one was never offered");

        // An offered ask ending through `settle`, each way it can.
        let (id, _rx) = offered(&asks, pane);
        changes.borrow_and_update();
        asks.withdraw(pane, &id);
        assert!(changes.has_changed().unwrap(), "an offered ask withdrawn is a change");
        changes.borrow_and_update();
        let (_id, _rx) = offered(&asks, pane);
        changes.borrow_and_update();
        asks.turn_boundary(pane);
        assert!(changes.has_changed().unwrap(), "an offered ask ended at a turn is a change");
    }

    /// One pane holds one ask, so the newer one is what's open.
    #[tokio::test]
    async fn a_newer_ask_on_the_same_terminal_replaces_the_older_in_open() {
        let (asks, _recorded) = ledger();
        let pane = Uuid::now_v7();
        let (_older, _older_rx) = offered(&asks, pane);
        let (newer, _newer_rx) = offered(&asks, pane);
        let ids: Vec<_> = asks.open().into_iter().map(|(t, id, _)| (t, id)).collect();
        assert_eq!(ids, vec![(pane, newer)]);
    }

    /// The surfaces read a `Permission` as pending until a `Resolved` for it
    /// follows, so a `Resolved` that lands first leaves a button nothing ever
    /// clears. In sequence only: the interleavings are
    /// `an_ask_ended_before_it_was_offered_is_never_offered` and
    /// `every_resolved_is_recorded_under_the_ledgers_lock`.
    #[tokio::test]
    async fn a_superseded_offer_is_resolved_after_its_permission() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (older, _rx) = offered(&asks, pane);
        let (newer, _newer_rx) = asks.hold(pane);
        assert_eq!(story(&recorded, pane), [("Permission", older.clone()), ("Resolved", older)]);
        assert!(!asks.offer(pane, "hook-ask-nobody", a_permission("hook-ask-nobody")));
        let _ = newer;
    }

    /// Every `Resolved` is recorded with the ledger's lock still held.
    ///
    /// `forget` takes that lock before the terminal's ring is deleted, so a
    /// `Resolved` recorded under it cannot land after the deletion and bring
    /// the ring back. This sink checks the lock at the moment it is called,
    /// for every ending that records one.
    #[tokio::test]
    async fn every_resolved_is_recorded_under_the_ledgers_lock() {
        let asks_cell: Arc<std::sync::OnceLock<std::sync::Weak<HookAsks>>> = Arc::default();
        let outside: Arc<Mutex<Vec<String>>> = Arc::default();
        let (cell, out) = (asks_cell.clone(), outside.clone());
        let sink: EventSink = Arc::new(move |_, events| {
            let asks = cell.get().and_then(std::sync::Weak::upgrade).expect("the ledger");
            for event in events {
                if let AgentEvent::Resolved { id, .. } = event {
                    if asks.held.try_lock().is_ok() {
                        out.lock().unwrap().push(id);
                    }
                }
            }
        });
        let asks = Arc::new(HookAsks::new(Arc::new(Mutex::new(Some(sink)))));
        asks_cell.set(Arc::downgrade(&asks)).unwrap();

        let pane = Uuid::now_v7();
        let (id, rx) = offered(&asks, pane);
        let hook = a_hook_waiting_on(rx);
        asks.answer(pane, &id, "allow", "iPhone").await.unwrap();
        hook.await.unwrap();
        let (id, _rx) = offered(&asks, pane);
        let (_newer, _newer_rx) = offered(&asks, pane);
        asks.withdraw(pane, &id);
        asks.turn_boundary(pane);
        let (_id, _rx) = offered(&asks, pane);
        for up in [true, false, false] {
            asks.saw_screen(pane, up);
        }
        assert!(!asks.is_holding(pane));
        assert_eq!(*outside.lock().unwrap(), Vec::<String>::new(), "recorded with the lock released");
    }

    /// An ask ended between its hold and its offer (a newer ask, a turn
    /// boundary) is never offered, and so is owed no `Resolved`.
    #[tokio::test]
    async fn an_ask_ended_before_it_was_offered_is_never_offered() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (older, older_rx) = asks.hold(pane);
        let (newer, _newer_rx) = asks.hold(pane);
        assert!(!asks.offer(pane, &older, a_permission(&older)), "the older ask was superseded");
        assert_eq!(older_rx.await.expect("it still hears its ending").decision, None);
        asks.turn_boundary(pane);
        assert!(!asks.offer(pane, &newer, a_permission(&newer)), "the newer one ended at the turn");
        assert_eq!(story(&recorded, pane), [], "nothing offered, nothing taken back");
    }

    /// `forget` goes with the terminal's ring. An offer after it must not
    /// bring that ring back.
    #[tokio::test]
    async fn an_offer_after_forget_records_nothing() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, _rx) = asks.hold(pane);
        asks.forget(pane);
        assert!(!asks.offer(pane, &id, a_permission(&id)));
        assert_eq!(story(&recorded, pane), []);
    }

    #[tokio::test]
    async fn an_allow_from_a_device_decides_the_held_ask() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = offered(&asks, pane);
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
        let (id, rx) = offered(&asks, pane);
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
        let (id, _rx) = offered(&asks, pane);
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
        let (id, rx) = offered(&asks, pane);
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
        let (_id, _rx) = offered(&asks, pane);
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
        let (older, older_rx) = offered(&asks, pane);
        let (newer, _newer_rx) = offered(&asks, pane);
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
        let (id, rx) = offered(&asks, pane);
        let hook = a_hook_waiting_on(rx);
        asks.answer(pane, &id, "allow", "iPhone").await.unwrap();
        hook.await.unwrap();
        asks.withdraw(pane, &id);
        asks.turn_boundary(pane);
        ids.push(("answer", id));

        let pane = Uuid::now_v7();
        let (id, _rx) = offered(&asks, pane);
        asks.withdraw(pane, &id);
        asks.withdraw(pane, &id);
        asks.turn_boundary(pane);
        ids.push(("withdraw", id));

        let pane = Uuid::now_v7();
        let (id, _rx) = offered(&asks, pane);
        let (newer, _newer_rx) = offered(&asks, pane);
        asks.withdraw(pane, &id);
        ids.push(("supersede", id));

        asks.turn_boundary(pane);
        asks.turn_boundary(pane);
        ids.push(("turn boundary", newer));

        let pane = Uuid::now_v7();
        let (id, _rx) = offered(&asks, pane);
        for up in [true, false, false, false, false] {
            asks.saw_screen(pane, up);
        }
        asks.withdraw(pane, &id);
        ids.push(("dialog gone", id));

        for (ending, id) in &ids {
            assert_eq!(resolved(&recorded, id).len(), 1, "{ending} resolved {id} other than once");
        }

        let pane = Uuid::now_v7();
        let (id, rx) = offered(&asks, pane);
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
        let (id, rx) = offered(&asks, pane);
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
        let (_id, _rx) = offered(&asks, pane);
        for up in [true, false, true, false, true] {
            asks.saw_screen(pane, up);
        }
        assert!(asks.is_holding(pane), "a one-frame redraw is not an answer");
    }

    #[tokio::test]
    async fn a_dialog_not_yet_drawn_does_not_withdraw_the_ask() {
        let (asks, _) = ledger();
        let pane = Uuid::now_v7();
        let (_id, _rx) = offered(&asks, pane);
        for _ in 0..10 {
            asks.saw_screen(pane, false);
        }
        assert!(asks.is_holding(pane), "a dialog never seen cannot have left");
    }

    #[tokio::test]
    async fn a_turn_boundary_withdraws_a_held_ask() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = offered(&asks, pane);
        asks.turn_boundary(Uuid::now_v7());
        assert!(asks.is_holding(pane), "another pane's turn is not this one's");
        asks.turn_boundary(pane);
        assert!(!asks.is_holding(pane));
        assert_eq!(rx.await.expect("an ending arrives").decision, None);
        assert_eq!(resolved(&recorded, &id), [""]);
    }

    #[tokio::test]
    async fn an_ask_nobody_was_offered_ends_without_a_resolved() {
        let (asks, recorded) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = asks.hold(pane);
        asks.withdraw(pane, &id);
        assert!(!asks.is_holding(pane));
        assert_eq!(rx.await.expect("an ending arrives").decision, None);
        assert!(resolved(&recorded, &id).is_empty(), "nothing was offered, so nothing is taken back");
    }

    #[test]
    fn a_session_is_heard_and_its_gates_dated() {
        let (asks, _) = ledger();
        assert!(!asks.hooked("s1"));
        let before = Instant::now();
        asks.heard("s1", false);
        assert!(asks.hooked("s1") && !asks.hooked("s2"));
        assert!(!asks.gated_since("s1", before), "a hook that isn't a gate");
        asks.heard("s1", true);
        assert!(asks.gated_since("s1", before));
        let after = Instant::now() + Duration::from_millis(1);
        assert!(!asks.gated_since("s1", after), "a gate from before");
        assert!(!asks.gated_since("s2", before));
        for n in 0..SESSIONS_KEPT + 5 {
            asks.heard(&format!("x{n}"), false);
        }
        assert_eq!(asks.sessions.lock().unwrap().len(), SESSIONS_KEPT);
    }

    /// A `PreToolUse` marks the call at once, then waits for the fence an
    /// Enter holds, and is answered when it's let go; the call's end clears
    /// the mark.
    #[tokio::test]
    async fn a_tool_call_is_marked_then_waits_for_the_fence() {
        let (asks, _) = ledger();
        let asks = Arc::new(asks);
        asks.heard("s1", false);
        let fence = asks.fence("s1").expect("a fence");
        let held = fence.lock().await;
        let starting = tokio::spawn({
            let asks = asks.clone();
            async move { asks.tool_starting("s1").await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(asks.tool_in_flight("s1"), "marked before the wait");
        assert!(!starting.is_finished(), "answered while the Enter held the fence");
        drop(held);
        tokio::time::timeout(Duration::from_secs(1), starting).await.expect("answered once let go").unwrap();
        asks.tool_ended("s1");
        assert!(!asks.tool_in_flight("s1"));
        assert!(asks.fence("unheard").is_none());
        asks.tool_starting("unheard").await;
        assert!(!asks.tool_in_flight("unheard"));
    }

    #[tokio::test]
    async fn forgetting_a_terminal_withdraws_its_ask() {
        let (asks, _) = ledger();
        let pane = Uuid::now_v7();
        let (id, rx) = offered(&asks, pane);
        asks.forget(pane);
        assert!(!asks.is_holding(pane));
        assert_eq!(rx.await.expect("the hook is released").decision, None);
        assert_eq!(asks.answer(pane, &id, "allow", "iPhone").await, Err(AnswerRefused::NotHeld));
    }
}
