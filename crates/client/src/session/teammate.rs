//! Add Teammate from a phone (ov-854): one runner's half of it, the Mac's
//! `TeammateEnrollment.enrollOne` written once for both phones.
//!
//! A device bound to nobody is the OWNER's on a runner, so a teammate's key
//! must never stay on one unbound. The runner helps (a `for_teammate` line is
//! nobody's and released until bound, ov-828), and this does the rest in the
//! Mac's order: read who is there, write Key A for the teammate, bind it to
//! their member row, and take the key off again if either write fails.
//!
//! **Only a `host_admin` device can do this.** `member.set` and `client.revoke`
//! are `host_admin` at the runner's scope gate, and the phones are normally
//! enrolled at `control` (ov-828). A phone below that is told so before
//! anything is written: a key it could write and not bind, or write and not
//! take off, would sit on the runner doing nothing and nobody could say why.
//! The refusal is `needs_admin`, and the runner's own gate stays the authority.
//!
//! Nothing here keeps a journal, as the Mac's does: a phone that quits between
//! the write and the bind leaves a `for_teammate` line the runner holds
//! released, which grants nothing, and Add Teammate again re-binds it.

use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::{Value, json};

use super::*;

/// Who the teammate is, as the confirmation left it. The handle is a
/// suggestion the runner may suffix (`bob-2`); the account and email are the
/// team directory's.
pub struct TeammateMember<'a> {
    pub handle: &'a str,
    pub name: &'a str,
    pub account: &'a str,
    pub email: &'a str,
}

/// The device being added, as its offer carried it.
pub struct TeammateDevice<'a> {
    pub public_key: &'a str,
    pub label: &'a str,
    pub client_id: &'a str,
    /// `read` or `control`: a teammate is never given more (ruling R-65).
    pub scope: &'a str,
    pub node_key: &'a str,
}

/// What became of one runner's half of Add Teammate. The word is what crosses
/// the FFI as `outcome`; the apps own the sentence.
#[derive(Debug, PartialEq, Eq)]
pub enum TeammateOutcome {
    /// The key is on the runner and bound to the teammate. `tunnel` is the
    /// runner's token when it admitted the device's node key, else empty.
    Written { tunnel: String },
    /// This phone isn't a `host_admin` device there. Nothing was written.
    NeedsAdmin,
    /// The runner is too old to keep a member's account, or to enroll at all.
    /// Nothing was written.
    TooOld,
    /// Who is on the runner couldn't be read, so nothing was written.
    CouldNotRead,
    /// The device is already another member's there. Left exactly as it was.
    SomeoneElses,
    /// The write failed, and may have landed: the key was taken off again.
    /// `still_there` is true when that failed too, which the app says.
    NotAdded { still_there: bool },
    /// The key went on and could not be bound, so it was taken off again.
    NotRecorded { still_there: bool },
    /// The device was bound before and its record could not be updated; its
    /// key is left as it was.
    NotUpdated,
}

impl TeammateOutcome {
    /// The FFI's answer: `{"outcome": word, ...}`.
    pub fn json(&self) -> Value {
        match self {
            Self::Written { tunnel } => json!({ "outcome": "written", "tunnel": tunnel }),
            Self::NeedsAdmin => json!({ "outcome": "needs_admin" }),
            Self::TooOld => json!({ "outcome": "too_old" }),
            Self::CouldNotRead => json!({ "outcome": "could_not_read" }),
            Self::SomeoneElses => json!({ "outcome": "someone_elses" }),
            Self::NotAdded { still_there } => json!({ "outcome": "not_added", "stillThere": still_there }),
            Self::NotRecorded { still_there } => json!({ "outcome": "not_recorded", "stillThere": still_there }),
            Self::NotUpdated => json!({ "outcome": "not_updated" }),
        }
    }
}

impl Session {
    /// Whether this connection holds the scope `member.set` and `client.revoke`
    /// need. A grant this build has no word for reads as yes, as the apps'
    /// `mayAdministerRunner` does: the runner's gate refuses it if it isn't.
    pub fn may_administer(&self) -> bool {
        !matches!(self.granted_scope(), "read" | "control")
    }

    /// Enroll `device` at `device.scope` on this runner for the teammate
    /// `member`, and bind it to them. See the module for the order and why.
    pub async fn enroll_teammate(&self, device: &TeammateDevice<'_>, member: &TeammateMember<'_>) -> TeammateOutcome {
        use farcooler_protocol::capability as cap;
        if !self.may_administer() {
            return TeammateOutcome::NeedsAdmin;
        }
        // A runner that can't keep the teammate's account is refused here,
        // before any key is written (the Mac's re-review R4).
        let serves = |c: &str| self.can(c);
        if !serves(cap::ENROLLMENT) || !serves(cap::REPRESENTATIVES) || !serves(cap::MEMBER_ACCOUNTS) {
            return TeammateOutcome::TooOld;
        }
        let Ok(people) = self.members().await else { return TeammateOutcome::CouldNotRead };
        let holder = people.members.iter().find(|m| m.client_ids.iter().any(|d| d == device.client_id));
        if holder.is_some_and(|h| !h.account.is_empty() && h.account != member.account) {
            return TeammateOutcome::SomeoneElses;
        }

        let written = match self.write_for_teammate(device).await {
            Ok(written) => written,
            Err(_) => return self.take_off(device.client_id, |still_there| TeammateOutcome::NotAdded { still_there }).await,
        };

        let mut bound = self.bind(device.client_id, member, holder, &people).await;
        // Their devices changed between the read and the bind: another Add
        // Teammate for them (ov-786). Read again and bind once more from what
        // is there now. Whoever the fresh read shows holding the device, it is
        // theirs to keep and its key is never taken off here.
        let mut fresh_holder = None;
        if matches!(&bound, Err(SessionError::Refused { what, .. }) if what == "devices_changed") {
            if let Ok(again) = self.members().await {
                fresh_holder = again.members.iter().find(|m| m.client_ids.iter().any(|d| d == device.client_id)).cloned();
                if fresh_holder.as_ref().is_some_and(|h| !h.account.is_empty() && h.account != member.account) {
                    return TeammateOutcome::SomeoneElses;
                }
                bound = self.bind(device.client_id, member, fresh_holder.as_ref(), &again).await;
            }
        }
        if bound.is_err() {
            // Bound before this bind, by an earlier ceremony or one alongside
            // it: nothing to undo, and its key is not ours to remove.
            if holder.is_some() || fresh_holder.is_some() {
                return TeammateOutcome::NotUpdated;
            }
            return self.take_off(device.client_id, |still_there| TeammateOutcome::NotRecorded { still_there }).await;
        }
        TeammateOutcome::Written { tunnel: written }
    }

    /// Key A for the teammate, `for_teammate`, never a shell line. A key that
    /// was already here keeps its lines, which may be wider than what was
    /// chosen and may include a Mac's Key B line under the same id, so it is
    /// taken off, both lines at once, and written again (the Mac's F8/R3).
    /// Answers the runner's tunnel token, empty when it gave none.
    async fn write_for_teammate(&self, device: &TeammateDevice<'_>) -> Result<String, SessionError> {
        let write = || {
            self.enroll_client_for(device.public_key, device.label, device.client_id, device.scope, false, device.node_key, true)
        };
        let mut answer = write().await?;
        if answer["alreadyEnrolled"].as_bool().unwrap_or(false) {
            self.revoke_client_with(device.client_id, false).await?;
            answer = write().await?;
        }
        Ok(answer["connBlob"].as_str().unwrap_or_default().to_string())
    }

    /// `member.set` binding `client_id` to the teammate. A teammate already
    /// here (by device, or by account) keeps their handle, name and other
    /// devices and gains this one, since `member.set` replaces a member's
    /// devices. What was read is sent along (`devices_read`) when the runner
    /// checks it, so a list built from a stale read is refused rather than
    /// taking off a device added since.
    async fn bind(
        &self,
        client_id: &str,
        member: &TeammateMember<'_>,
        holder: Option<&pb::Member>,
        people: &pb::MemberList,
    ) -> Result<(), SessionError> {
        let existing = holder.or_else(|| people.members.iter().find(|m| !member.account.is_empty() && m.account == member.account));
        let handle = match existing {
            Some(e) => e.handle.clone(),
            None => unique_handle(member.handle, people.members.iter().map(|m| m.handle.as_str())),
        };
        let mut devices = existing.map(|e| e.client_ids.clone()).unwrap_or_default();
        if !devices.iter().any(|d| d == client_id) {
            devices.push(client_id.to_string());
        }
        let read = people.members.iter().find(|m| m.handle == handle).map(|m| m.client_ids.clone()).unwrap_or_default();
        let checked = self.can(farcooler_protocol::capability::MEMBER_DEVICES_READ);
        let payload = request::Payload::MemberSet(pb::MemberSet {
            handle,
            name: existing.map(|e| e.name.clone()).unwrap_or_else(|| member.name.trim().to_string()),
            client_ids: devices,
            account: member.account.trim().to_string(),
            email: member.email.trim().to_string(),
            devices_read: checked.then(|| pb::MemberDevicesRead { client_ids: read }),
        });
        let mut required = vec![
            farcooler_protocol::capability::REPRESENTATIVES.to_string(),
            farcooler_protocol::capability::MEMBER_ACCOUNTS.to_string(),
        ];
        if checked {
            required.push(farcooler_protocol::capability::MEMBER_DEVICES_READ.to_string());
        }
        match self.value_requiring("member.set", None, Some(payload), required).await? {
            result::Value::MemberList(_) => Ok(()),
            other => Err(wrong("member_list", &other)),
        }
    }

    /// Take the key off after a write or a bind that failed, and the devices
    /// it added with it. When that fails too the outcome says so: never silent.
    async fn take_off(&self, client_id: &str, outcome: impl Fn(bool) -> TeammateOutcome) -> TeammateOutcome {
        let gone = self.revoke_client_with(client_id, true).await.is_ok();
        outcome(!gone)
    }
}

/// `chosen`, or with `-2`, `-3` and so on until no member here has it, cut so
/// the suffix still fits the runner's 32. The handles the runner keeps for
/// itself are never given out. The apps suggest a handle with the same rule
/// (`TeammateNames`); this one decides, on the runner's own list.
pub fn unique_handle<'a>(chosen: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    const RESERVED: [&str; 6] = ["owner", "orchestrator", "rep", "runner", "manager", "user"];
    let taken: std::collections::HashSet<&str> = taken.into_iter().collect();
    if !taken.contains(chosen) && !RESERVED.contains(&chosen) {
        return chosen.to_string();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            let stem: String = chosen.chars().take(32 - suffix.len()).collect();
            format!("{}{suffix}", stem.trim_end_matches('-'))
        })
        .find(|candidate| !taken.contains(candidate.as_str()))
        .unwrap_or_else(|| chosen.to_string())
}

#[cfg(test)]
#[path = "teammate_tests.rs"]
mod tests;
