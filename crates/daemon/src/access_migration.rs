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
//! would not reproduce a line exactly ([`verify`]). The first run derives the
//! tables from the lines alone (`Store::access_migrated`); a failed run
//! changes nothing and tries again at the next start.
//!
//! **Every later start refreshes the tables** (slice 2, the slice 1 review's
//! SF1): it adds what the lines have and the tables lack (a member added, a
//! device an older build enrolled), drops what no line backs, and keeps every
//! row that already has a say. Then it lowers any line that sits above what
//! the tables say: a reconcile never raises one.

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

pub(crate) fn rank(scope: Scope) -> u8 {
    match scope {
        Scope::Unspecified => 0,
        Scope::Read => 1,
        Scope::Control => 2,
        Scope::HostAdmin => 3,
    }
}

pub(crate) fn lower(a: Scope, b: Scope) -> Scope {
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

/// The device ids that have more than one Key A line. `enroll` never writes a
/// second line in one shape under an id, so only a hand edit does; which of
/// the two a session arrived on is a guess, so such an id is left out of the
/// plan, `verify` and `rebuild` and its lines stay as they are.
fn duplicated(entries: &[Entry]) -> BTreeSet<&str> {
    let mut seen = BTreeSet::new();
    entries.iter().filter(|e| key_a(e)).filter(|e| !seen.insert(e.client_id.as_str())).map(|e| e.client_id.as_str()).collect()
}

/// Who owns each device, what each person may do, and each device's cap:
/// what a rebuild reads.
#[derive(Debug, Default, Clone)]
pub struct View {
    owners: BTreeSet<String>,
    released: BTreeSet<String>,
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
            released: store.released_all()?,
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
    /// A released device is checked before any owner row, as `person_of`
    /// does: a stale row for an id that was revoked and enrolled again for a
    /// teammate must not read as the owner's (the slice 1 review's SF2).
    pub fn access_of(&self, device: &str) -> Option<Access> {
        match self.member_of.get(device) {
            Some(handle) => self.access.get(handle).copied(),
            None if self.released.contains(device) => None,
            None => self.owners.contains(device).then_some(Access::Owner),
        }
    }

    /// A stale owner row, for a test of what a released device reads as.
    #[cfg(test)]
    pub(crate) fn owners_insert_for_tests(&mut self, device: &str) {
        self.owners.insert(device.to_string());
    }

    /// `handle`'s access in this view.
    pub fn access_of_member(&self, handle: &str) -> Option<Access> {
        self.access.get(handle).copied()
    }

    /// This view, with `handle`'s access as `access`: what a change would
    /// leave.
    pub fn with_access(&self, handle: &str, access: Access) -> View {
        let mut view = self.clone();
        view.access.insert(handle.to_string(), access);
        view
    }

    /// This view, with `device`'s cap as `cap`.
    pub fn with_cap(&self, device: &str, cap: Option<Cap>) -> View {
        let mut view = self.clone();
        match cap {
            Some(cap) => view.caps.insert(device.to_string(), cap),
            None => view.caps.remove(device),
        };
        view
    }

    /// Every device `person` holds: a member's handle, or the owner's with
    /// the empty string.
    pub fn devices_of(&self, person: &str) -> BTreeSet<String> {
        if person.is_empty() {
            return self.owners.iter().filter(|d| !self.member_of.contains_key(*d) && !self.released.contains(*d)).cloned().collect();
        }
        self.member_of.iter().filter(|(_, h)| *h == person).map(|(d, _)| d.clone()).collect()
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
    /// The access already stored, which a refresh keeps (empty on the first
    /// run).
    pub access: &'a BTreeMap<String, Access>,
    /// The caps already stored: they win over a cap worked out from a line.
    pub caps: &'a BTreeMap<String, Cap>,
}

/// The migration for `entries`, by today's rules for whose a device is
/// (`people::person_of`).
pub fn plan(entries: &[Entry], facts: &Facts) -> AccessPlan {
    let member_of: BTreeMap<&str, &str> = facts
        .members
        .iter()
        .flat_map(|m| m.devices.iter().map(move |d| (d.as_str(), m.handle.as_str())))
        .collect();
    let twice = duplicated(entries);
    let lines: Vec<&Entry> = entries.iter().filter(|e| key_a(e) && !twice.contains(e.client_id.as_str())).collect();
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
        let access = facts.access.get(&member.handle).copied();
        out.member_access.push((member.handle.clone(), access.unwrap_or_else(|| highest.map_or(Access::Member, access_for))));
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
/// **A rebuild never raises a line unless it is told to** (B1 of the
/// security review). A line moves to the view's scope only downward; one of
/// the `raise` devices, the ones a promotion or an uncap names, may also
/// move up, as far as `control` and no further (`host_admin` arrives with
/// step-up). A line that doesn't change is returned as it was, so one this
/// build can't rewrite is no obstacle until it has to be lowered.
pub fn rebuild(entries: &[Entry], view: &View, raise: &BTreeSet<String>) -> Result<Vec<String>> {
    let (lines, stuck) = rebuild_skipping(entries, view, raise);
    match stuck.is_empty() {
        true => Ok(lines),
        false => Err(DomainError::OperationFailed),
    }
}

/// [`rebuild`], except that a line this build can't rewrite (an unusual option
/// set, another channel's program) is returned as it was and named in the
/// second list instead of failing every other line with it (slice 2 review,
/// N3). The caller decides whether that matters: it does for a device the
/// change names, and not for anyone else's.
pub fn rebuild_skipping(entries: &[Entry], view: &View, raise: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    let twice = duplicated(entries);
    let mut stuck = Vec::new();
    let lines = entries
        .iter()
        .map(|entry| {
            let Some(target) = view.scope_of(&entry.client_id).filter(|_| key_a(entry) && !twice.contains(entry.client_id.as_str())) else {
                return entry.line.clone();
            };
            let up = lower(target, Scope::Control);
            let scope = if raise.contains(&entry.client_id) && rank(up) > rank(entry.scope) { up } else { lower(entry.scope, target) };
            if scope == entry.scope {
                return entry.line.clone();
            }
            fence::with_scope(entry, scope).unwrap_or_else(|_| {
                stuck.push(entry.client_id.clone());
                entry.line.clone()
            })
        })
        .collect();
    (lines, stuck)
}

/// `entries` with the lines of `rebuilt`, and whether any differs: what a
/// write of them would be.
pub fn relined(entries: &[Entry], rebuilt: Vec<String>) -> (Vec<Entry>, bool) {
    let changed = entries.iter().zip(&rebuilt).any(|(entry, line)| entry.line != *line);
    (entries.iter().zip(rebuilt).map(|(entry, line)| Entry { line, ..entry.clone() }).collect(), changed)
}

/// Every line a person owns keeps its scope under `view`: what a plan must
/// satisfy before it is written. The device that breaks it, if any.
pub fn verify(entries: &[Entry], view: &View) -> std::result::Result<(), String> {
    let twice = duplicated(entries);
    for entry in entries.iter().filter(|e| key_a(e) && !twice.contains(e.client_id.as_str())) {
        if let Some(scope) = view.scope_of(&entry.client_id) {
            if scope != entry.scope {
                return Err(entry.client_id.clone());
            }
        }
    }
    Ok(())
}

/// The People view's rows: every device the store has a person for, sorted
/// by device. Empty until the migration has run. `only` is whose rows to tell:
/// `None` is everyone's (the owner's view), `Some(handle)` one member's own.
///
/// Each row says what its device's step-up key is (`trusted`, `proposed`, or
/// `expired` for a proposal past its ten minutes) and, for a proposal, the six
/// digits to check: what lets another device of the same person approve it.
pub fn device_access(store: &Store, only: Option<&str>, now_ms: i64) -> Result<Vec<farcooler_protocol::v1::DeviceAccess>> {
    use farcooler_protocol::v1::DeviceAccess;
    let stored = store.stored_access()?;
    let caps: BTreeMap<&str, Cap> = stored.caps.iter().map(|(d, c)| (d.as_str(), *c)).collect();
    let access: BTreeMap<&str, Access> = stored.member_access.iter().map(|(h, a)| (h.as_str(), *a)).collect();
    let cap_word = |device: &str| caps.get(device).map(|c| c.as_str().to_string()).unwrap_or_default();
    // The key's state, code and time, for a row.
    let key = |device: &str, row: DeviceAccess| -> Result<DeviceAccess> {
        let Some(k) = store.step_up_key(device)? else { return Ok(row) };
        let expired = !k.trusted && now_ms - k.registered_at > farcooler_store::step_up::PROPOSAL_MS;
        Ok(DeviceAccess {
            step_up: if k.trusted { "trusted" } else if expired { "expired" } else { "proposed" }.to_string(),
            step_up_code: if k.trusted || expired { String::new() } else { crate::step_up::code_of(&k.alg, &k.public_key) },
            step_up_at: k.registered_at,
            ..row
        })
    };
    let mut rows: Vec<DeviceAccess> = Vec::new();
    if only.is_none() {
        for (device, state) in &stored.owner_devices {
            rows.push(key(
                device,
                DeviceAccess {
                    client_id: device.clone(),
                    person: String::new(),
                    access: Access::Owner.as_str().to_string(),
                    cap: cap_word(device),
                    check: *state == OwnerDeviceState::Check,
                    ..Default::default()
                },
            )?);
        }
    }
    if !stored.owner_devices.is_empty() || !access.is_empty() {
        for member in store.members()?.into_iter().filter(|m| only.is_none_or(|h| h == m.handle)) {
            let Some(theirs) = access.get(member.handle.as_str()) else { continue };
            for device in &member.devices {
                rows.push(key(
                    device,
                    DeviceAccess {
                        client_id: device.clone(),
                        person: member.handle.clone(),
                        access: theirs.as_str().to_string(),
                        cap: cap_word(device),
                        ..Default::default()
                    },
                )?);
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

/// Run the migration as the daemon starts: the first time it derives the
/// tables from the lines, every later start refreshes them and lowers any
/// line above them. Under the fence lock, from the lines as read there,
/// checked before it writes. Logged, never fatal: a run that can't finish
/// changes nothing and the next start tries again.
pub async fn migrate_at_start(svc: &Arc<Service>) {
    let store = svc.store.clone();
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

/// One run, answering whether it was the first.
///
/// The lines come first and the store second, both inside the lock: a line
/// lowered with the tables not yet refreshed is the safe way round, and a
/// failed file write changes nothing at all.
fn migrate_in(store: &Store, path: &std::path::Path) -> std::result::Result<bool, Skipped> {
    let first = !store.access_migrated().map_err(Skipped::Store)?;
    fence::update_then(
        path,
        fence::AUTHORIZED_KEYS,
        Placement::Last,
        |entries| {
            let members = store.members().map_err(Skipped::Store)?;
            let added_by = store.added_by_all().map_err(Skipped::Store)?;
            let released = |id: &str| store.device_released(id).unwrap_or(true);
            let stored = store.stored_access().map_err(Skipped::Store)?;
            let (access, caps): (BTreeMap<_, _>, BTreeMap<_, _>) = match first {
                true => Default::default(),
                false => (stored.member_access.into_iter().collect(), stored.caps.into_iter().collect()),
            };
            let plan = plan(entries, &Facts { members: &members, released: &released, added_by: &added_by, access: &access, caps: &caps });
            let mut view = View::from_plan(&plan, &members);
            view.caps.extend(caps);
            if first {
                verify(entries, &view).map_err(Skipped::Breaks)?;
            }
            let (rebuilt, stuck) = rebuild_skipping(entries, &view, &BTreeSet::new());
            if !stuck.is_empty() {
                tracing::warn!(devices = ?stuck, "lines this build can't rewrite were left as they are; re-pair these devices");
            }
            let (lowered, changed) = relined(entries, rebuilt);
            let live: BTreeSet<String> = entries.iter().filter(|e| key_a(e)).map(|e| e.client_id.clone()).collect();
            let change = match changed {
                true => {
                    let (ours, foreign) = crate::enrollment::sorted(&lowered);
                    Change::Write { entries: ours, foreign }
                }
                false => Change::Leave,
            };
            Ok((change, (plan, live)))
        },
        |(plan, live)| {
            match first {
                true => store.apply_access_plan(plan).map(|_| ()),
                false => store.refresh_access(plan, live),
            }
            .map_err(Skipped::Store)
        },
    )
    .map(|_| first)
}

#[cfg(test)]
#[path = "access_migration_tests.rs"]
mod tests;
