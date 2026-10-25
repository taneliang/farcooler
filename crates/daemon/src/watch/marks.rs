//! Seen and watching, per person (ov-740, the owner's ruling on ov-716 rule
//! 1). Its own file to keep `watch.rs` inside its size budget.
//!
//! **Seen.** `Done` is idle-and-unseen, and `terminal.seen` ends it. The
//! runner's own `Done`, the one its pushes, cards and Needs You read, is the
//! owner's: only the owner's look ends it (`Watcher::mark_seen`), as on a
//! runner with no members. A member's look is written down here instead
//! (`Seen::seen`), and only their own rows read it (`as_seen_by`): a member
//! opening a pane clears nothing for the owner, and the owner's look clears
//! nothing for them (`Seen::cleared`).
//!
//! **Watching.** A client's claim that a pane is on screen silences that
//! pane's notices (`Watcher::report_watching`). It's per person: the owner's
//! push is silenced by the owner's claims alone (`owner_watching`), so a
//! member with the pane open doesn't silence the owner's phone. A member's
//! claim spares that member their own push (`member_watching`), as it
//! spared the owner theirs. Whether someone typed lately is still anyone's
//! (`anyone_watching`): it's about a person at that pane's keyboard, whoever
//! they are.

use std::collections::HashMap;
use std::sync::Mutex;

use farcooler_protocol::v1::{self as pb, AgentActivity};
use uuid::Uuid;

use super::{Watcher, now_millis, watch_is_live};
use crate::people::Person;

/// One client's claim about what it is showing, and when it said so.
///
/// A `Vec` rather than a `HashSet`: it holds a Mac window's tiled layout, which
/// is single digits, and a linear scan of four ids is faster than hashing one.
pub(super) struct Watched {
    /// When this claim arrived, in Unix milliseconds. What `WATCHED_TTL_MS` is
    /// measured against, and the reason a client that goes away silently stops
    /// silencing anything.
    pub(super) at: i64,
    /// The terminals this client said were in front of a person. Empty is a
    /// real and useful claim — "I am looking at nothing" — which is what an app
    /// sends as it backgrounds.
    pub(super) terminals: Vec<Uuid>,
    /// Whose device it is (ov-740): a member's handle, or `None` for the
    /// owner's.
    pub(super) member: Option<String>,
}

/// Write down what one client says it is showing, replacing what it said last.
///
/// Free rather than a method for the reason `card_orphaned` is free: the two
/// properties that matter here — a client's later word REPLACES its earlier one,
/// and a client that stops speaking stops counting — are the properties the
/// whole feature's safety rests on, and neither is testable through a `Watcher`
/// without standing up a `Service`, a SQLite handle and a tmux server to ask a
/// question about a `HashMap`.
///
/// Expired rows are dropped on the way past. There is no timer here to sweep on
/// and no need for one: the map is bounded by the number of devices that have
/// ever connected, `anyone_watching` checks freshness itself so a lingering row
/// silences nothing, and clearing them here means a device revoked months ago
/// does not keep a slot forever.
pub(super) fn record_watch(
    watched: &mut HashMap<String, Watched>,
    client: &str,
    member: Option<&str>,
    terminals: Vec<Uuid>,
    now: i64,
) {
    watched.retain(|_, w| watch_is_live(w.at, now));
    watched.insert(client.to_string(), Watched { at: now, terminals, member: member.map(str::to_string) });
}

/// Whether ANY client's live claim covers this terminal, whoever's it is: a
/// person is at that pane, so nothing automatic types over them. A claim
/// from a client that has gone quiet is skipped rather than removed, so
/// reading this can stay on a shared reference and cannot be the thing that
/// decides a device is gone.
pub(super) fn anyone_watching(watched: &HashMap<String, Watched>, terminal: Uuid, now: i64) -> bool {
    watched.values().any(|w| watch_is_live(w.at, now) && w.terminals.contains(&terminal))
}

/// Whether one of the owner's clients has a live claim on this terminal: a
/// Mac with the pane on screen speaks for the watch on the owner's wrist as
/// much as for itself. Not a member's client (ov-740): their screen isn't
/// the owner's.
pub(super) fn owner_watching(watched: &HashMap<String, Watched>, terminal: Uuid, now: i64) -> bool {
    watched_by(watched, None, terminal, now)
}

/// Whether one of `handle`'s clients has a live claim on this terminal.
pub(super) fn member_watching(watched: &HashMap<String, Watched>, handle: &str, terminal: Uuid, now: i64) -> bool {
    watched_by(watched, Some(handle), terminal, now)
}

fn watched_by(watched: &HashMap<String, Watched>, member: Option<&str>, terminal: Uuid, now: i64) -> bool {
    watched
        .values()
        .any(|w| w.member.as_deref() == member && watch_is_live(w.at, now) && w.terminals.contains(&terminal))
}

/// The members' looks at panes, apart from the owner's (ov-740).
#[derive(Default)]
pub(super) struct Seen {
    /// When each member last opened each pane: (handle, terminal) → ms.
    seen: Mutex<HashMap<(String, Uuid), i64>>,
    /// The `Done` the owner's look ended, by pane: when it began, and when
    /// the `Idle` the look left began. A member who hasn't looked since it
    /// began still reads that `Done` while the pane sits in that `Idle`.
    cleared: Mutex<HashMap<Uuid, (i64, i64)>>,
}

impl Seen {
    /// Whether anyone's look is written down for `terminal`: when not, every
    /// person reads the runner's row as it is, and nobody is looked up.
    fn any(&self, terminal: Uuid) -> bool {
        self.cleared.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&terminal)
            || self.seen.lock().unwrap_or_else(|e| e.into_inner()).keys().any(|(_, t)| *t == terminal)
    }

    /// Forget every look at `terminal`: its activity moved, so whatever was
    /// looked at is over, and the next `Done` begins after every look written
    /// so far. What keeps both maps to the panes with a finished turn on show.
    pub(super) fn forget(&self, terminal: Uuid) {
        self.cleared.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, t), _| *t != terminal);
    }

    /// Keep only the looks at `live` panes: a pane that's gone takes its own.
    pub(super) fn retain(&self, live: &std::collections::HashSet<Uuid>) {
        self.cleared.lock().unwrap_or_else(|e| e.into_inner()).retain(|t, _| live.contains(t));
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, t), _| live.contains(t));
    }

    /// `handle`'s view of `terminal`'s activity, from the runner's.
    fn view(&self, handle: &str, terminal: Uuid, activity: AgentActivity, since: Option<i64>) -> (AgentActivity, Option<i64>) {
        let seen_at = self.seen.lock().unwrap_or_else(|e| e.into_inner()).get(&(handle.to_string(), terminal)).copied();
        let cleared = self.cleared.lock().unwrap_or_else(|e| e.into_inner()).get(&terminal).copied();
        as_seen_by(activity, since, seen_at, cleared)
    }
}

/// What a member reads of a pane whose runner-side activity is `activity`
/// since `since`, when they last opened it at `seen_at`, and the owner's look
/// ended the `Done` that began at the first of `cleared` with the `Idle` that
/// began at the second.
///
/// - A `Done` they've opened since it began is `Idle` to them.
/// - The `Idle` the owner's look left, unchanged since, is still that `Done`
///   to a member who hasn't opened it since the `Done` began.
/// - Anything else is the agent, not anyone's attention, and reads as it is.
pub(super) fn as_seen_by(
    activity: AgentActivity,
    since: Option<i64>,
    seen_at: Option<i64>,
    cleared: Option<(i64, i64)>,
) -> (AgentActivity, Option<i64>) {
    let looked_since = |at: i64| seen_at.is_some_and(|s| s >= at);
    match (activity, since, cleared) {
        (AgentActivity::Done, Some(at), _) if looked_since(at) => (AgentActivity::Idle, since),
        (AgentActivity::Idle, Some(idle), Some((done, left))) if idle == left && !looked_since(done) => {
            (AgentActivity::Done, Some(done))
        }
        _ => (activity, since),
    }
}

impl Watcher {
    /// `terminal.seen` from `person`: the owner's look ends the runner's
    /// `Done` (`mark_seen`); a member's is theirs alone; a released device
    /// has nothing to see with, and its call never gets here (the gate).
    ///
    /// A member's look at a `Done` announces the pane's row, as the owner's
    /// does, so their other devices stop showing it at once. Each connection
    /// reads that row as its person does (`shape_event`): `Idle` to them,
    /// unchanged to everyone else.
    pub async fn mark_seen_by(&self, person: &Person, terminal: Uuid) {
        match person {
            Person::Owner => self.mark_seen(terminal).await,
            Person::Member(handle) => {
                self.seen.seen.lock().unwrap_or_else(|e| e.into_inner()).insert((handle.clone(), terminal), now_millis());
                let done = self.state.lock().await.get(&terminal).filter(|o| o.activity == AgentActivity::Done).cloned();
                if let Some(snapshot) = done {
                    self.announce(terminal, snapshot, None).await;
                }
            }
            Person::Released => {}
        }
    }

    /// Write down that the owner's look ended the `Done` that began at
    /// `done`, leaving the `Idle` that began at `idle`.
    pub(super) fn owner_cleared(&self, terminal: Uuid, done: i64, idle: i64) {
        self.seen.cleared.lock().unwrap_or_else(|e| e.into_inner()).insert(terminal, (done, idle));
    }

    /// `activity`, as `person` reads it: the owner's and a released
    /// device's is the runner's own.
    pub async fn activity_for(&self, person: &Person, terminal: Uuid) -> (AgentActivity, Option<i64>, Option<i64>) {
        let (activity, since, turn) = self.activity(terminal).await;
        match person {
            Person::Member(handle) => {
                let (activity, since) = self.seen.view(handle, terminal, activity, since);
                (activity, since, turn)
            }
            Person::Owner | Person::Released => (activity, since, turn),
        }
    }

    /// The Needs You list's observations as `viewer` reads them (ov-744): a
    /// member's view of each pane someone's look is written down for, so the
    /// owner's look at a member's failed turn leaves the item on their list,
    /// and their own look takes it off. The owner, a released device and a
    /// runner with no members read the runner's own, untouched.
    pub fn observed_as(&self, viewer: &Person, observed: &mut HashMap<Uuid, crate::needs_you::Observation>) {
        let Person::Member(handle) = viewer else { return };
        for (pane, o) in observed.iter_mut().filter(|(pane, _)| self.seen.any(**pane)) {
            let (activity, since) = self.seen.view(handle, *pane, o.activity, Some(o.state_since));
            o.activity = activity;
            o.state_since = since.unwrap_or(o.state_since);
        }
    }

    /// Whether a pane may read differently for some person: only once a
    /// look at it is written down. Until then nobody need be looked up.
    pub fn seen_any(&self, terminal: Uuid) -> bool {
        self.seen.any(terminal)
    }

    /// The person rows about `panes` are read as: `viewer`, asked at most
    /// once, and only when a look at one of them is written down. On a runner
    /// with no members nothing ever is (`mark_seen` writes the owner's look
    /// down only when there are members), so nobody is looked up and every
    /// row is the runner's own. A person the store can't place reads the
    /// runner's rows, as a released device does.
    pub fn viewer_for(
        &self,
        panes: impl IntoIterator<Item = Uuid>,
        viewer: impl FnOnce() -> farcooler_core::Result<Person>,
    ) -> Person {
        match panes.into_iter().any(|pane| self.seen.any(pane)) {
            true => viewer().unwrap_or(Person::Released),
            false => Person::Owner,
        }
    }

    /// `event` as `viewer` reads it: a `terminal_changed` row reshaped for a
    /// member (`shape_for`), every other event as it is. `viewer` is asked
    /// only for a pane someone's look is written down for.
    pub fn shape_event(&self, mut event: pb::Event, viewer: impl FnOnce() -> farcooler_core::Result<Person>) -> pb::Event {
        if let Some(pb::event::Payload::TerminalChanged(row)) = event.payload.as_mut()
            && let Some(id) = crate::wire::parse_id(&row.id)
        {
            let person = self.viewer_for([id], viewer);
            self.shape_for(&person, row);
        }
        event
    }

    /// A pane row in an event, as `person` reads it (`as_seen_by`), so the
    /// `terminal_changed` the owner's look announces doesn't end a member's
    /// `Done`, and the one a turn ending announces says `Idle` to a member
    /// who already opened it.
    pub fn shape_for(&self, person: &Person, row: &mut pb::Terminal) {
        let (Person::Member(handle), Some(id)) = (person, crate::wire::parse_id(&row.id)) else { return };
        let since = crate::wire::millis_of(&row.activity_changed_at);
        let (activity, since) = self.seen.view(handle, id, row.activity(), since);
        row.activity = activity as i32;
        row.activity_changed_at = since.map(crate::wire::timestamp);
    }
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;
