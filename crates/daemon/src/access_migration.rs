//! The move from per-device scopes to access that follows the person
//! (ov-856, slice 1 of `reports/ov-856/design.md`, section 5): the store
//! learns whose each device is, what each person may do, and which devices
//! sit below that. **The runner's enforcement does not read any of it yet**:
//! every line, and every call, is what it was.
//!
//! A line's scope is `min(ceiling(access), cap)`. The migration writes the
//! access and the caps so that this reproduces every line it reads, for
//! everyone, the owner included (B1 of the security review). Without a cap
//! for the owner's own read-only iPad, the first rebuild after any change
//! would raise it to `control`, and a `read` device would hold a shell.
//!
//! - **The owner's devices** are those bound to no member and not released,
//!   as `people::person_of` reads them. One added by a member's or a
//!   released device (R3, a teammate's line from before ov-828) is marked
//!   `check`, for the owner to confirm.
//! - **A member's access** is the highest scope among their devices'
//!   lines: `host_admin` Admin, `control` Member, `read` Viewer.
//! - **A cap** is written for every device whose line is below its person's
//!   ceiling: `read` is `viewer`, `control` is `member`.
//! - **Not touched here**: a released device (nobody's, and refused), a
//!   plain shell line, a foreign line, and a line this build can't read.
//!   A forced-command line with no `--client` has no id to bind and reads as
//!   a foreign line; releasing it (S7) belongs with the enforcement slice,
//!   since releasing is a change in what it may do.
//! - **Admin phones keep `host_admin`** for now. Lowering them to `control`
//!   (design section 5, step 5) is a change in behavior and has to come
//!   with the step-up that restores it, so it comes with that.
//!
//! The migration is computed and checked under the fence lock, from the
//! lines as they are read there, and refuses to write anything if the plan
//! would not reproduce a line exactly ([`verify`]). It runs once
//! (`Store::access_migrated`); a failed run changes nothing and tries again
//! at the next start.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use farcooler_core::{DomainError, Result};
use farcooler_fence::{self as fence, Change, Entry, FenceError, Placement};
use farcooler_protocol::v1::Scope;
use farcooler_store::access::{Access, AccessPlan, Cap, OwnerDeviceState};
use farcooler_store::people::Member;
use farcooler_store::Store;

use crate::service::Service;

/// The scope a person's access allows a line, at most.
///
/// Slice 1 reads Admin and Owner as `host_admin`, so today's admin lines
/// (a member's admin phone, the owner's) are reproduced and nothing is
/// lowered. The design's `control` ceiling arrives with step-up.
pub fn ceiling(access: Access) -> Scope {
    match access {
        Access::Owner | Access::Admin => Scope::HostAdmin,
        Access::Member => Scope::Control,
        Access::Viewer => Scope::Read,
    }
}

fn rank(scope: Scope) -> u8 {
    match scope {
        Scope::Unspecified => 0,
        Scope::Read => 1,
        Scope::Control => 2,
        Scope::HostAdmin => 3,
    }
}

fn lower(a: Scope, b: Scope) -> Scope {
    if rank(a) <= rank(b) { a } else { b }
}

fn access_for(scope: Scope) -> Access {
    match scope {
        Scope::HostAdmin => Access::Admin,
        Scope::Control => Access::Member,
        _ => Access::Viewer,
    }
}

/// Whether a line is one this migration reads: ours, restricted, with a
/// device id and a scope this build knows.
fn key_a(entry: &Entry) -> bool {
    !entry.shell_access && !entry.client_id.is_empty() && !matches!(entry.scope, Scope::Unspecified)
}

/// Who owns each device, what each person may do, and each device's cap:
/// what a rebuild reads.
#[derive(Debug, Default, Clone)]
pub struct View {
    owners: BTreeSet<String>,
    member_of: BTreeMap<String, String>,
    access: BTreeMap<String, Access>,
    caps: BTreeMap<String, Cap>,
}

impl View {
    /// What the store holds now.
    pub fn from_store(store: &Store) -> Result<View> {
        let stored = store.stored_access()?;
        let mut view = View {
            owners: stored.owner_devices.into_iter().map(|(d, _)| d).collect(),
            access: stored.member_access.into_iter().collect(),
            caps: stored.caps.into_iter().collect(),
            ..View::default()
        };
        for member in store.members()? {
            for device in member.devices {
                view.member_of.insert(device, member.handle.clone());
            }
        }
        Ok(view)
    }

    /// What `plan` would leave, given today's `members`.
    fn from_plan(plan: &AccessPlan, members: &[Member]) -> View {
        let mut view = View {
            owners: plan.owner_devices.iter().map(|(d, _)| d.clone()).collect(),
            access: plan.member_access.iter().cloned().collect(),
            caps: plan.caps.iter().cloned().collect(),
            ..View::default()
        };
        for member in members {
            for device in &member.devices {
                view.member_of.insert(device.clone(), member.handle.clone());
            }
        }
        view
    }

    /// The access of whoever `device` is bound to, or `None` for a device
    /// nobody is bound to (released, unknown) or a member with no access.
    pub fn access_of(&self, device: &str) -> Option<Access> {
        match self.member_of.get(device) {
            Some(handle) => self.access.get(handle).copied(),
            None => self.owners.contains(device).then_some(Access::Owner),
        }
    }

    /// The scope a rebuild gives `device`'s line: `min(ceiling, cap)`.
    /// `None` when it has no person, so a rebuild leaves its line alone.
    pub fn scope_of(&self, device: &str) -> Option<Scope> {
        let ceiling = ceiling(self.access_of(device)?);
        Some(match self.caps.get(device) {
            Some(Cap::Viewer) => lower(ceiling, Scope::Read),
            Some(Cap::Member) => lower(ceiling, Scope::Control),
            None => ceiling,
        })
    }
}

/// What the store knows that decides whose a device is.
pub struct Facts<'a> {
    pub members: &'a [Member],
    pub released: &'a dyn Fn(&str) -> bool,
    /// `(device, added_by)`, as `Store::added_by_all`.
    pub added_by: &'a [(String, String)],
}

/// The migration for `entries`, by today's rules for whose a device is
/// (`people::person_of`).
pub fn plan(entries: &[Entry], facts: &Facts) -> AccessPlan {
    let member_of: BTreeMap<&str, &str> = facts
        .members
        .iter()
        .flat_map(|m| m.devices.iter().map(move |d| (d.as_str(), m.handle.as_str())))
        .collect();
    let lines: Vec<&Entry> = entries.iter().filter(|e| key_a(e)).collect();
    let released = |id: &str| !member_of.contains_key(id) && (facts.released)(id);
    let owners: BTreeSet<&str> = lines
        .iter()
        .map(|e| e.client_id.as_str())
        .filter(|id| !member_of.contains_key(id) && !released(id))
        .collect();
    let mut out = AccessPlan::default();
    for id in &owners {
        // R3: unbound, and added by a device that isn't the owner's.
        let adder = facts.added_by.iter().find(|(d, _)| d == id).map(|(_, by)| by.as_str()).unwrap_or("");
        let theirs = !adder.is_empty() && (member_of.contains_key(adder) || (facts.released)(adder));
        let state = if theirs { OwnerDeviceState::Check } else { OwnerDeviceState::Confirmed };
        out.owner_devices.push((id.to_string(), state));
    }
    for member in facts.members {
        let highest = lines
            .iter()
            .filter(|e| member.devices.contains(&e.client_id))
            .map(|e| e.scope)
            .max_by_key(|s| rank(*s));
        out.member_access.push((member.handle.clone(), highest.map_or(Access::Member, access_for)));
    }
    let view = View::from_plan(&out, facts.members);
    let mut seen = BTreeSet::new();
    for entry in &lines {
        let id = entry.client_id.as_str();
        let (Some(access), true) = (view.access_of(id), seen.insert(id)) else { continue };
        if rank(entry.scope) < rank(ceiling(access)) {
            let cap = if entry.scope == Scope::Read { Cap::Viewer } else { Cap::Member };
            out.caps.push((id.to_string(), cap));
        }
    }
    out
}

/// The block as a rebuild writes it: every line a person owns, with the
/// scope the view gives it, and every other line as it was, byte for byte.
///
/// A rebuild never decides to raise a line. Today nothing calls it outside
/// [`verify`] and the tests; the enforcement slice rebuilds with it.
pub fn rebuild(entries: &[Entry], view: &View) -> Result<Vec<String>> {
    entries
        .iter()
        .map(|entry| match view.scope_of(&entry.client_id).filter(|_| key_a(entry)) {
            Some(scope) => fence::with_scope(entry, scope).map_err(|_| DomainError::OperationFailed),
            None => Ok(entry.line.clone()),
        })
        .collect()
}

/// Every line a person owns keeps its scope under `view`: what a plan must
/// satisfy before it is written. The device that breaks it, if any.
pub fn verify(entries: &[Entry], view: &View) -> std::result::Result<(), String> {
    for entry in entries.iter().filter(|e| key_a(e)) {
        if let Some(scope) = view.scope_of(&entry.client_id) {
            if scope != entry.scope {
                return Err(entry.client_id.clone());
            }
        }
    }
    Ok(())
}

/// The People view's rows (read only): every device the store has a person
/// for, sorted by device. Empty until the migration has run.
pub fn device_access(store: &Store) -> Result<Vec<farcooler_protocol::v1::DeviceAccess>> {
    use farcooler_protocol::v1::DeviceAccess;
    let stored = store.stored_access()?;
    let caps: BTreeMap<&str, Cap> = stored.caps.iter().map(|(d, c)| (d.as_str(), *c)).collect();
    let access: BTreeMap<&str, Access> = stored.member_access.iter().map(|(h, a)| (h.as_str(), *a)).collect();
    let cap_word = |device: &str| caps.get(device).map(|c| c.as_str().to_string()).unwrap_or_default();
    let mut rows: Vec<DeviceAccess> = stored
        .owner_devices
        .iter()
        .map(|(device, state)| DeviceAccess {
            client_id: device.clone(),
            person: String::new(),
            access: Access::Owner.as_str().to_string(),
            cap: cap_word(device),
            check: *state == OwnerDeviceState::Check,
        })
        .collect();
    if !stored.owner_devices.is_empty() || !access.is_empty() {
        for member in store.members()? {
            let Some(theirs) = access.get(member.handle.as_str()) else { continue };
            for device in &member.devices {
                rows.push(DeviceAccess {
                    client_id: device.clone(),
                    person: member.handle.clone(),
                    access: theirs.as_str().to_string(),
                    cap: cap_word(device),
                    check: false,
                });
            }
        }
    }
    rows.sort_by(|a, b| a.client_id.cmp(&b.client_id));
    Ok(rows)
}

/// Why a run wrote nothing.
enum Skipped {
    Fence(FenceError),
    Store(DomainError),
    Breaks(String),
}

impl From<FenceError> for Skipped {
    fn from(error: FenceError) -> Self {
        Skipped::Fence(error)
    }
}

/// Run the migration once, as the daemon starts. Under the fence lock, from
/// the lines as read there, checked before it writes; the file is never
/// written. Logged, never fatal: a run that can't finish changes nothing and
/// the next start tries again.
pub async fn migrate_at_start(svc: &Arc<Service>) {
    let store = svc.store.clone();
    match store.access_migrated() {
        Ok(false) => {}
        Ok(true) => return,
        Err(err) => {
            tracing::warn!(error = %err.what(), "the access migration couldn't read whether it ran");
            return;
        }
    }
    let path = svc.authorized_keys().to_path_buf();
    let done = tokio::task::spawn_blocking(move || migrate_in(&store, &path)).await;
    match done {
        Ok(Ok(true)) => tracing::info!("moved the per-device scopes into the people's access"),
        Ok(Ok(false)) => {}
        Ok(Err(Skipped::Fence(err))) => tracing::warn!(error = %err, "the access migration couldn't read the fence; it tries again at the next start"),
        Ok(Err(Skipped::Store(err))) => tracing::warn!(error = %err.what(), "the access migration couldn't be written; it tries again at the next start"),
        Ok(Err(Skipped::Breaks(device))) => {
            tracing::error!(device = %device, "the access migration would have changed a device's scope; wrote nothing")
        }
        Err(err) => tracing::warn!(error = %err, "the access migration's task didn't finish"),
    }
}

fn migrate_in(store: &Store, path: &std::path::Path) -> std::result::Result<bool, Skipped> {
    // `update` for its lock, not for a write: `Change::Leave` touches nothing.
    fence::update(path, fence::AUTHORIZED_KEYS, Placement::Last, |entries| {
        let members = store.members().map_err(Skipped::Store)?;
        let added_by = store.added_by_all().map_err(Skipped::Store)?;
        let released = |id: &str| store.device_released(id).unwrap_or(true);
        let plan = plan(entries, &Facts { members: &members, released: &released, added_by: &added_by });
        verify(entries, &View::from_plan(&plan, &members)).map_err(Skipped::Breaks)?;
        let wrote = store.apply_access_plan(&plan).map_err(Skipped::Store)?;
        Ok((Change::Leave, wrote))
    })
}

#[cfg(test)]
#[path = "access_migration_tests.rs"]
mod tests;
