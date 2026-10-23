//! A push about a member's item goes to that member (ov-633,
//! `needs_you::concern`). Its own file to keep `watch.rs` inside its size
//! budget.
//!
//! The owner's notice always goes: it moves the live card and a task's
//! thread. For a member's item waiting on a person (a blocked agent, a failed
//! turn, a decision, a block on the board, a review) the member's own
//! installs are told through `/v1/notify/member`, in the relay's fixed words,
//! and the owner's notice goes without a banner, **but only once the relay
//! says it sent the member's push** (`"sent": true`, `Route::settle`).
//! Nobody's phone is left silent:
//!
//! - a member added by hand, with no relay account, can't be pushed, so the
//!   owner is buzzed as before;
//! - a push the relay sent nowhere (the member has no install, or none keeps
//!   the class on, or they've left the team) says `"sent": false`, and the
//!   owner is buzzed;
//! - a relay that doesn't take the push (one older than `needs_you` answers
//!   400, or it can't be reached), or is older than `sent`, leaves the owner
//!   buzzed;
//! - a push the relay throttles (the member was pushed within two minutes)
//!   says `"sent": true`, and is accepted: they were told of something of
//!   theirs moments ago, and their list holds the rest.
//!
//! A member's agent finishing isn't waiting on anybody: the owner's notice
//! is quiet and no member push goes. Nor does one about the member's own
//! write (ov-712): they moved the card, so the owner is told instead.
//!
//! On a runner with no members, and for the owner's items and nobody's,
//! nothing changes.

use farcooler_store::models::Task;
use uuid::Uuid;

use super::task_notice::Class;
use super::{Tapped, Watcher};
use crate::needs_you::concern;
use crate::push::{MemberKind, Opens};

/// What one notice about an item does: whether the owner's copy alerts, and
/// the member push that, once the relay takes it, quiets it.
#[derive(Debug)]
pub(super) struct Route {
    alert: bool,
    member: Option<MemberPush>,
}

#[derive(Debug)]
struct MemberPush {
    account: String,
    kind: MemberKind,
    about: String,
    /// What the member's tap opens (ov-705).
    opens: Opens,
}

impl Route {
    /// The owner's notice as it always was.
    fn owner(alert: bool) -> Route {
        Route { alert, member: None }
    }

    /// Send the member push, if there is one, and say whether the owner's
    /// notice alerts: not once the relay says it sent the member's push. A
    /// test's tap with no pairing counts as taken, as `deliver_push` counts
    /// the owner's.
    pub(super) async fn settle(self, watcher: &Watcher, pairing: Option<&crate::push::Pairing>) -> bool {
        let Some(push) = self.member else { return self.alert };
        let event = crate::push::needs_you_event_id(&push.account, &push.about, crate::review::now_millis());
        watcher.tap(Tapped {
            kind: Some("member"),
            title: push.account.clone(),
            subtitle: push.about.clone(),
            event: Some(push.kind.word()),
            level: push.kind.class(),
            ..Tapped::default()
        });
        let Some(pairing) = pairing else { return false };
        let runner = crate::service::stable_host_id(watcher.service.install_id()).to_string();
        let taken = crate::push::notify_member(&watcher.push, pairing, &push.account, push.kind, &event, Some(&runner), Some(&push.opens)).await;
        !taken
    }
}

impl Watcher {
    /// The route of the owner's agent notice about `terminal`, which would
    /// have alerted as `alert`.
    pub(super) fn route_agent_notice(&self, terminal: Uuid, status: &str, failed: bool, alert: bool) -> Route {
        // A working card never alerts, and is refreshed often.
        if !alert || status == "working" {
            return Route::owner(alert);
        }
        let waiting = status == "blocked" || failed;
        self.route(concern::whose_terminal(&self.service.store, terminal), waiting.then_some(("blocked", terminal.to_string(), Opens::Terminal(terminal))), None)
    }

    /// The route of the owner's task notice of `class` about `task`, which is
    /// `whose` (`concern::whose_task`, read once for the live event too), news
    /// of `author`'s own writes when that's a member (ov-712).
    pub(super) fn route_task_notice(&self, task: &Task, whose: Option<concern::Whose>, class: Class, author: Option<&str>) -> Route {
        let waiting = matches!(class, Class::Decision | Class::Blocked | Class::Review);
        self.route(whose, waiting.then_some((class.event(), task.id.to_string(), Opens::Task(task.key.clone()))), author)
    }

    /// `push_to`'s answer as a route. `waiting` is the class, subject and
    /// what a tap opens of an item waiting on a person, `None` for news that
    /// isn't. `author`, a member whose own write this is, isn't pushed about
    /// it: they did it. The owner is told instead, as for anyone's write.
    fn route(&self, whose: Option<concern::Whose>, waiting: Option<(&'static str, String, Opens)>, author: Option<&str>) -> Route {
        let Some(handle) = concern::push_to(whose.as_ref()).member else { return Route::owner(true) };
        let Some((class, subject, opens)) = waiting else { return Route::owner(false) };
        if author == Some(handle.as_str()) {
            return Route::owner(true);
        }
        // No relay account (a member added by hand): theirs can't be pushed,
        // so the owner is, as before.
        let Some(account) = self.service.store.member(&handle).ok().and_then(|m| m.account) else { return Route::owner(true) };
        Route { alert: true, member: Some(MemberPush { account, kind: MemberKind::NeedsYou(class), about: format!("{subject}:{class}"), opens }) }
    }
}

#[cfg(test)]
#[path = "member_push_tests.rs"]
mod tests;
