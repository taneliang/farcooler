//! What the runner holds for a pane that its transcript can't tell a
//! projector (ov-830): the live held ask's request hook, which carries the
//! hold's id (`HELD_ASK`), and the messages the runner holds or gave up on
//! (`runner_holds`). A projector opens only while somebody views its pane, so
//! these are kept here whether or not one is open, and replayed into each
//! projector a build makes. Without them, an ask held while nobody looked
//! had no way to be answered once a view opened, and a message given up on
//! meanwhile never said why.

use std::collections::HashMap;
use std::sync::Mutex;

use farcooler_core::session_log::projector::held::HELD_ASK;
use farcooler_core::session_log::projector::{Projection, SessionProjector};
use serde_json::Value;
use uuid::Uuid;

use super::Held;

/// The most held or given-up messages kept per pane. Each is one row and a
/// notice on a rebuilt projection, so the oldest given-up ones go first.
const MOST_MESSAGES: usize = 32;

/// How long a given-up message's notice is kept for a projector opened
/// later, at most. It is let go sooner, at the pane's next prompt: a notice
/// is said in the turn it's folded into, the newest, so one kept past that
/// would say "not sent" under a later reply, and read as that one failing.
/// The bound is for a pane with no hooks to say its prompts.
const DROPPED_KEPT_MS: i64 = 10 * 60 * 1000;

/// One message the runner holds for a pane, or gave up on.
#[derive(Debug, Clone, PartialEq)]
struct Message {
    id: String,
    /// Its text and when it was held: `None` for one given up on that this
    /// runner never showed held (its notice stands alone).
    held: Option<(String, i64)>,
    /// Whether it is pasted in the agent's box, its Enter waiting (ov-827).
    in_box: bool,
    /// Why it was given up on, and when.
    dropped: Option<(String, i64)>,
}

/// One pane's holds.
#[derive(Default)]
struct PaneHolds {
    /// The live held ask: its hold id and the request hook that carried it.
    ask: Option<(String, Held)>,
    /// The hold that ended last: its request, folded on a thread of its own,
    /// can arrive after a quick answer ended it, and is then not kept.
    gone: Option<String>,
    messages: Vec<Message>,
}

impl PaneHolds {
    fn is_empty(&self) -> bool {
        self.ask.is_none() && self.gone.is_none() && self.messages.is_empty()
    }
}

/// Every pane's holds.
#[derive(Default)]
pub(crate) struct Holds(Mutex<HashMap<Uuid, PaneHolds>>);

impl Holds {
    fn with<R>(&self, terminal: Uuid, f: impl FnOnce(&mut PaneHolds) -> R) -> R {
        let mut all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let pane = all.entry(terminal).or_default();
        let r = f(pane);
        if pane.is_empty() {
            all.remove(&terminal);
        }
        r
    }

    /// A hook routed to `terminal`: kept when it holds an ask. A new prompt
    /// lets the given-up messages go (`DROPPED_KEPT_MS`).
    pub(crate) fn hook(&self, terminal: Uuid, held: &Held) {
        if held.event == "UserPromptSubmit" {
            self.with(terminal, |p| p.messages.retain(|m| m.dropped.is_none()));
        }
        if let Some(id) = held.payload.get(HELD_ASK).and_then(Value::as_str) {
            let id = id.to_string();
            let held = Held { codex: held.codex, event: held.event.clone(), payload: held.payload.clone(), at: held.at };
            self.with(terminal, |p| {
                if p.gone.as_deref() != Some(id.as_str()) {
                    p.ask = Some((id, held));
                }
            });
        }
    }

    /// The hold `id` on `terminal` ended, however it did: its ask isn't
    /// replayed again.
    pub(crate) fn ask_gone(&self, terminal: Uuid, id: &str) {
        self.with(terminal, |p| {
            if p.ask.as_ref().is_some_and(|(held, _)| held == id) {
                p.ask = None;
            }
            p.gone = Some(id.to_string());
        });
    }

    pub(crate) fn held(&self, terminal: Uuid, id: &str, text: &str, at: i64) {
        self.with(terminal, |p| {
            if !p.messages.iter().any(|m| m.id == id) {
                p.messages.push(Message { id: id.to_string(), held: Some((text.to_string(), at)), in_box: false, dropped: None });
                trim(&mut p.messages);
            }
        });
    }

    /// `id` is in the agent's box, its Enter waiting for the turn's end: held
    /// as that, or a held one moved to it (ov-827).
    pub(crate) fn in_box(&self, terminal: Uuid, id: &str, text: &str, at: i64) {
        self.with(terminal, |p| {
            match p.messages.iter_mut().find(|m| m.id == id) {
                Some(m) => m.in_box = true,
                None => {
                    p.messages.push(Message { id: id.to_string(), held: Some((text.to_string(), at)), in_box: true, dropped: None });
                    trim(&mut p.messages);
                }
            }
        });
    }

    /// Typed: claude's own record shows it from now on.
    pub(crate) fn told(&self, terminal: Uuid, id: &str) {
        self.with(terminal, |p| p.messages.retain(|m| m.id != id));
    }

    pub(crate) fn dropped(&self, terminal: Uuid, id: &str, why: &str, at: i64) {
        self.with(terminal, |p| {
            match p.messages.iter_mut().find(|m| m.id == id) {
                Some(m) => m.dropped = Some((why.to_string(), at)),
                None => p.messages.push(Message { id: id.to_string(), held: None, in_box: false, dropped: Some((why.to_string(), at)) }),
            }
            trim(&mut p.messages);
        });
    }

    pub(crate) fn forget(&self, terminal: Uuid) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
    }

    /// Fold `terminal`'s holds into a projector just built, after the hooks
    /// its build held (`building`): the held ask, unless one of those was
    /// that same request, then every held message and why one wasn't sent.
    pub(crate) fn replay(&self, terminal: Uuid, session: &mut SessionProjector, building: &[Held]) {
        let (ask, messages) = {
            let mut all = self.0.lock().unwrap_or_else(|e| e.into_inner());
            let Some(p) = all.get_mut(&terminal) else { return };
            let since = super::now_ms() - DROPPED_KEPT_MS;
            p.messages.retain(|m| m.dropped.as_ref().is_none_or(|(_, at)| *at >= since));
            (p.ask.as_ref().map(|(id, h)| (id.clone(), h.clone())), p.messages.clone())
        };
        if let Some((id, ask)) = ask {
            let seen = building.iter().any(|h| h.payload.get(HELD_ASK).and_then(Value::as_str) == Some(id.as_str()));
            if !seen {
                super::apply(session, ask.codex, &ask.event, &ask.payload, ask.at);
            }
        }
        replay_messages(session.projection_mut(), &messages);
    }
}

fn replay_messages(p: &mut Projection, messages: &[Message]) {
    for m in messages {
        if let Some((text, at)) = &m.held {
            if m.in_box {
                p.runner_in_box(&m.id, text, *at);
            } else {
                p.runner_held(&m.id, text, *at);
            }
        }
        if let Some((why, at)) = &m.dropped {
            p.runner_dropped(&m.id, why, *at);
        }
    }
}

/// At most `MOST_MESSAGES`, the oldest given-up ones going first: a message
/// still held is never dropped from here.
fn trim(messages: &mut Vec<Message>) {
    while messages.len() > MOST_MESSAGES {
        match messages.iter().position(|m| m.dropped.is_some()) {
            Some(i) => {
                messages.remove(i);
            }
            None => return,
        }
    }
}

#[cfg(test)]
#[path = "session_projectors_holds_tests.rs"]
mod tests;
