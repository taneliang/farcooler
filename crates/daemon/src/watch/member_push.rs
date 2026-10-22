//! A push about a member's item goes to that member (ov-633,
//! `needs_you::concern`). Its own file to keep `watch.rs` inside its size
//! budget.
//!
//! The owner's notice always goes: it moves the live card and a task's
//! thread. For a member's item it goes with `alert: false`, so the owner's
//! phone doesn't buzz about someone else's agent. If it would have alerted
//! about something waiting on a person (a blocked agent, a failed turn, a
//! decision, a block on the board, a review), the member's own installs hear
//! it instead, through `/v1/notify/member` with the relay's fixed words. A
//! member added by hand, with no relay account, gets no push and sees the
//! item when their app next reads the list.
//!
//! On a runner with no members, and for the owner's items and nobody's,
//! nothing changes.

use farcooler_store::models::Task;
use uuid::Uuid;

use super::task_notice::Class;
use super::{Tapped, Watcher};
use crate::needs_you::concern;

impl Watcher {
    /// Whether the owner's agent notice about `terminal` alerts, given that
    /// it would have (`alert`); and tell its member when it is theirs.
    pub(super) fn route_agent_notice(&self, terminal: Uuid, status: &str, failed: bool, alert: bool) -> bool {
        // A working card never alerts, and is refreshed often.
        if !alert || status == "working" {
            return alert;
        }
        let to = concern::push_to(concern::whose_terminal(&self.service.store, terminal).as_ref());
        if let Some(handle) = to.member
            && (status == "blocked" || failed)
        {
            self.push_member(handle, format!("{terminal}:{status}"));
        }
        to.owner_alerts
    }

    /// Whether the owner's task notice of `class` about `task` alerts; and
    /// tell its member when it is theirs.
    pub(super) fn route_task_notice(&self, task: &Task, class: Class) -> bool {
        let to = concern::push_to(concern::whose_task(&self.service.store, task).as_ref());
        if let Some(handle) = to.member
            && matches!(class, Class::Decision | Class::Blocked | Class::Review)
        {
            self.push_member(handle, format!("{}:{}", task.id, class.event()));
        }
        to.owner_alerts
    }

    /// Ask the relay to tell `handle`'s phones that something needs them,
    /// detached as every push is. A test's tap hears `kind: "member"` with
    /// the handle as its title.
    fn push_member(&self, handle: String, about: String) {
        let me = self.me.clone();
        tokio::spawn(async move {
            let Some(watcher) = me.upgrade() else { return };
            let Some(pairing) = watcher.audience() else { return };
            watcher.tap(Tapped { kind: Some("member"), title: handle.clone(), subtitle: about.clone(), ..Tapped::default() });
            let Some(pairing) = pairing else { return };
            let Some(account) = watcher.service.store.member(&handle).ok().and_then(|m| m.account) else { return };
            let event = crate::push::needs_you_event_id(&handle, &about, crate::review::now_millis());
            let runner = crate::service::stable_host_id(watcher.service.install_id()).to_string();
            crate::push::notify_member(&watcher.push, &pairing, &account, crate::push::MemberKind::NeedsYou, &event, Some(&runner))
                .await;
        });
    }
}

#[cfg(test)]
#[path = "member_push_tests.rs"]
mod tests;
