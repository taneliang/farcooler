//! Access follows the person, enforced live (ov-856, slice 2 of
//! `reports/ov-856/design.md`, sections 2 and 9).
//!
//! - **Each request is checked live.** [`live_peer`] gives a connection's
//!   request the scope `min(line, ceiling(access(person)), cap(device))`, read
//!   from the store per call, so a demotion bites the next request on a
//!   connection that is still open, and a runner rolled back before the lines
//!   were rewritten still serves no more than the store says. Missing rows
//!   lower nothing: a runner that hasn't moved its scopes over serves its lines.
//! - **Authority is re-read where the write is.** [`Authority`] is who a
//!   request was decided for and their `access_generation`; enroll, revoke and
//!   `member.set` check it again inside the fence lock and refuse
//!   `access_changed` on a mismatch (S2).
//! - **An Admin manages Members, Viewers and themselves** ([`may_manage`]).
//! - **The socket, not a pane.** Admin requests from inside a pane are refused
//!   `person_only` ([`refuse_admin_from_pane`], S4), and `member.set_access` and
//!   `client.set_cap` come from the runner's own socket alone.
//! - **A change to access or a cap rewrites the scope word and nothing else**,
//!   in the order its direction asks (S3): a demotion lowers the lines first and
//!   commits the store only once the file is written, so a failed write changes
//!   nothing and a rollback can never serve a stale, higher line; a promotion
//!   records first and then raises the lines, so a failed write leaves the
//!   lower line. Either way the person's open sessions close quietly and
//!   reconnect at the new scope ([`Sessions::reconnect`](crate::sessions::Sessions::reconnect)).

use std::collections::BTreeSet;
use std::sync::Arc;

use farcooler_core::{DomainError, Result};
use farcooler_fence::{self as fence, Change, Entry, Placement};
use farcooler_protocol::v1::{self as pb, ClientEnroll, Scope, result};
use farcooler_store::Store;
use farcooler_store::access::{Access, Cap};
use farcooler_transport::Peer;

use crate::access_migration::{View, ceiling, lower, rank, rebuild, relined};
use crate::enrollment::{Refusal, attributed, blocking};
use crate::people::{Person, person_of_device};
use crate::service::Service;
use crate::sessions::Session;

/// The scope a cap holds a device to.
fn cap_scope(cap: Cap) -> Scope {
    match cap {
        Cap::Viewer => Scope::Read,
        Cap::Member => Scope::Control,
    }
}

/// The cap that keeps a line at `scope` below the owner's ceiling, or none
/// for a line that isn't below it.
pub fn owner_cap(scope: Scope) -> Option<Cap> {
    cap_below(scope, ceiling(Access::Owner))
}

/// The cap that reproduces a line at `scope` under `ceiling`, or none.
fn cap_below(scope: Scope, ceiling: Scope) -> Option<Cap> {
    match (rank(scope) < rank(ceiling), scope) {
        (false, _) => None,
        (true, Scope::Read) => Some(Cap::Viewer),
        (true, _) => Some(Cap::Member),
    }
}

/// What `peer`'s request may do right now: its line's scope, lowered to its
/// person's ceiling and to its device's cap. `Read` when the store can't be
/// read, since a guess must not be wider than what's recorded.
pub fn effective_scope(store: &Store, peer: &Peer) -> Scope {
    let Some(device) = peer.client_id.as_deref() else { return peer.scope };
    let read = || -> Result<Scope> {
        let mut scope = peer.scope;
        if let Some(handle) = store.member_of_device(device)? {
            if let Some(access) = store.member_access(&handle)? {
                scope = lower(scope, ceiling(access));
            }
        }
        if let Some(cap) = store.device_cap(device)? {
            scope = lower(scope, cap_scope(cap));
        }
        Ok(scope)
    };
    read().unwrap_or_else(|err| {
        tracing::warn!(error = %err.what(), "a device's access couldn't be read; serving it at read");
        lower(peer.scope, Scope::Read)
    })
}

/// `peer`, at the scope it may use now ([`effective_scope`]).
pub fn live_peer(store: &Store, peer: &Peer) -> Peer {
    Peer { scope: effective_scope(store, peer), ..peer.clone() }
}

/// Who a request was decided for, and where their access stood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    pub person: Person,
    pub generation: u32,
}

fn person_key(person: &Person) -> &str {
    match person {
        Person::Owner => "",
        Person::Member(handle) => handle,
        Person::Released => "(released)",
    }
}

impl Authority {
    /// Who `device` is now (`None` is the local socket) and their generation.
    pub fn read(store: &Store, device: Option<&str>) -> Result<Authority> {
        let person = person_of_device(store, device)?;
        let generation = store.access_generation(person_key(&person))?;
        Ok(Authority { person, generation })
    }

    /// Refused `access_changed` unless `device` is the same person at the same
    /// generation as when this was read.
    pub fn recheck(&self, store: &Store, device: Option<&str>) -> Result<()> {
        if Authority::read(store, device)? != *self {
            tracing::info!("refused: the caller's access changed while the request waited");
            return Err(DomainError::Conflict { what: "access_changed" });
        }
        Ok(())
    }
}

/// Whether `caller` may manage `device`: the owner manages everything; an
/// Admin manages Members' and Viewers' devices and their own, never the
/// owner's or another Admin's (S2). A device nobody holds is anyone's to
/// manage. A member with no access recorded (a runner that hasn't moved its
/// scopes over) has the rules it had.
pub fn may_manage(store: &Store, caller: &Person, device: &str) -> Result<bool> {
    let me = match caller {
        Person::Owner => return Ok(true),
        Person::Released => return Ok(false),
        Person::Member(handle) => handle,
    };
    match store.member_of_device(device)? {
        Some(holder) if holder == *me => Ok(true),
        Some(holder) => Ok(store.member_access(&holder)? != Some(Access::Admin)),
        None => Ok(!store.device_is_owners(device)?),
    }
}

/// Whether `caller` may manage the member `handle`: the owner does; a member
/// does themselves and anyone who isn't an Admin. A handle nobody holds is
/// anyone's to manage (its removal is a no-op or a retry).
pub fn may_manage_member(store: &Store, caller: &Person, handle: &str) -> Result<bool> {
    Ok(match caller {
        Person::Owner => true,
        Person::Released => false,
        Person::Member(me) => me == handle || store.member_access(handle)? != Some(Access::Admin),
    })
}

/// Refuse `person_only` an admin request from a caller inside a pane, or one
/// the runner can't tell is a person (S4): a prompt-injected agent in a pane
/// reaches the socket as the owner, and these are the calls where that
/// matters most. The same walk `member.set_role` does.
pub async fn refuse_admin_from_pane(svc: &Service, caller_pid: Option<i32>) -> Result<()> {
    if caller_pid.and_then(|pid| crate::people::pane_of(svc, pid)).is_some() {
        tracing::info!("refused: an admin request from inside a pane");
        return Err(DomainError::Conflict { what: "person_only" });
    }
    if crate::caller_seat::caller_seat(svc, caller_pid).await != crate::caller_seat::Seat::Person {
        tracing::info!("refused: an admin request from a caller who isn't a person");
        return Err(DomainError::Conflict { what: "person_only" });
    }
    Ok(())
}

/// `client.enroll`, as the dispatcher runs it: the grant rules (`may_grant`),
/// what the caller's person may hand out, who the new line is bound to, and
/// the authority the request was decided under, rechecked in the lock.
pub async fn enroll(svc: &Arc<Service>, peer: &Peer, caller_pid: Option<i32>, session: &Arc<Session>, p: ClientEnroll) -> Result<result::Value> {
    refuse_admin_from_pane(svc, caller_pid).await?;
    let authority = Authority::read(&svc.store, peer.client_id.as_deref())?;
    let person = authority.person.clone();
    crate::enrollment::may_grant(peer.scope, &person, &p)?;
    let asked = Scope::try_from(p.scope).unwrap_or(Scope::Unspecified);
    let access = match &person {
        Person::Member(handle) => svc.store.member_access(handle)?,
        _ => None,
    };
    // Never above what the person it will be inherited from may hold.
    if access.is_some_and(|access| rank(asked) > rank(ceiling(access))) {
        return Err(DomainError::ScopeDenied { needed: "host_admin" });
    }
    let by_member = match person {
        // A member's own `control` device adds a device of theirs (ov-838, R-88),
        // capped where its line is below their ceiling.
        Person::Member(handle) if peer.scope != Scope::HostAdmin => {
            let svc = svc.clone();
            let cap = cap_below(asked, ceiling(access.unwrap_or(Access::Member)));
            Some(Box::new(move |id: &str| {
                svc.store.record_member_device_added(id, &handle)?;
                cap.map_or(Ok(()), |cap| svc.store.cap_new_device(id, cap))
            }) as crate::enrollment::MemberEnrolled)
        }
        Person::Member(handle) => {
            let svc = svc.clone();
            Some(Box::new(move |id: &str| svc.store.record_member_enrolled(id, &handle)) as crate::enrollment::MemberEnrolled)
        }
        // The owner's Add Teammate: nobody's until bound (ov-828 S1).
        _ if p.for_teammate => {
            let svc = svc.clone();
            Some(Box::new(move |id: &str| svc.store.record_teammate_pending(id)) as crate::enrollment::MemberEnrolled)
        }
        _ => None,
    };
    let caller = crate::enrollment::Caller::new(session.clone(), peer.scope).with_authority(authority);
    Ok(result::Value::ClientEnroll(crate::enrollment::enroll_as(svc, &p, by_member, Some(caller)).await?))
}

/// `client.revoke`: the device, and with `with_added` what it added, that the
/// caller's person manages ([`may_manage`]), by a caller still who they were.
pub async fn revoke(svc: &Service, device: Option<&str>, request: &pb::ClientRevoke) -> Result<pb::ClientList> {
    revoke_held(svc, device, request, || ()).await
}

/// [`revoke`], running `held` just before the fence lock is taken.
pub(crate) async fn revoke_held(svc: &Service, device: Option<&str>, request: &pb::ClientRevoke, held: impl FnOnce() + Send + 'static) -> Result<pb::ClientList> {
    let authority = Authority::read(&svc.store, device)?;
    let person = authority.person.clone();
    let (store, caller_device, root) = (svc.store.clone(), device.map(str::to_string), request.client_id.clone());
    let guard: crate::enrollment::Guard = {
        let (store, person) = (store.clone(), person.clone());
        Box::new(move |_| {
            authority.recheck(&store, caller_device.as_deref())?;
            if !may_manage(&store, &person, &root)? {
                return Err(DomainError::NotYours { what: "not_yours" });
            }
            Ok(())
        })
    };
    let keep_person = person.clone();
    let keep = move |id: &str| may_manage(&store, &keep_person, id);
    crate::enrollment::revoke_guarded(svc, request, keep, guard, held).await
}

/// `member.set`, decided under the fence lock (ov-832, S2): the devices named
/// must have lines, the caller's person must manage everyone it touches, the
/// caller must still be who they were, and the members' access and caps are
/// recorded with the binding. The sweep that revokes what the write released
/// is the caller's, after the lock.
pub(crate) async fn member_set(
    svc: &Service,
    device: Option<&str>,
    authority: Authority,
    p: pb::MemberSet,
    held: impl FnOnce() + Send + 'static,
) -> Result<farcooler_store::people::MemberWrite> {
    let (store, path, device) = (svc.store.clone(), svc.authorized_keys().to_path_buf(), device.map(str::to_string));
    blocking(move || {
        held();
        fence::update(&path, fence::AUTHORIZED_KEYS, Placement::Last, |entries| {
            authority.recheck(&store, device.as_deref())?;
            // Only devices this runner enrolled: a typo bound to a member would
            // leave their real phone the owner's.
            let named: Vec<&str> = p.client_ids.iter().map(|d| d.trim()).filter(|d| !d.is_empty()).collect();
            if !named.iter().all(|d| entries.iter().any(|e| !e.client_id.is_empty() && e.client_id == *d)) {
                return Err(Refusal(DomainError::InvalidArgument { what: "device" }));
            }
            let handle = p.handle.trim();
            let before = match store.member(handle) {
                Ok(member) => Some(member),
                Err(DomainError::NotFound) => None,
                Err(e) => return Err(Refusal(e)),
            };
            // An Admin manages Members, Viewers and themselves: not another
            // Admin, and not a device bound to one (S2).
            let touched = named.iter().map(|d| d.to_string()).chain(before.iter().flat_map(|m| m.devices.clone()));
            for id in touched {
                if !may_manage(&store, &authority.person, &id)? {
                    return Err(Refusal(DomainError::NotYours { what: "not_yours" }));
                }
            }
            if !may_manage_member(&store, &authority.person, handle)? {
                return Err(Refusal(DomainError::NotYours { what: "not_yours" }));
            }
            let who = farcooler_store::people::Directory { account: &p.account, email: &p.email };
            // What the caller read as theirs, checked with the write (ov-786).
            let read = p.devices_read.as_ref().map(|r| r.client_ids.as_slice());
            let write = store.set_member_releasing(handle, &p.name, &p.client_ids, who, read)?;
            if store.access_migrated()? {
                record_binding(&store, entries, &write.member)?;
            }
            Ok((Change::Leave, write))
        })
    })
    .await
}

/// The access and caps a member's devices need now they're bound: a member
/// with no access takes the highest scope among their lines (Member with
/// none), and a device whose line is below their ceiling is capped to it, so
/// the access and the caps together reproduce every line.
fn record_binding(store: &Store, entries: &[Entry], member: &farcooler_store::people::Member) -> Result<()> {
    let key_a = |id: &str| entries.iter().find(|e| !e.shell_access && e.client_id == id && !matches!(e.scope, Scope::Unspecified));
    if store.member_access(&member.handle)?.is_none() {
        let highest = member.devices.iter().filter_map(|d| key_a(d)).map(|e| e.scope).max_by_key(|s| rank(*s));
        let access = match highest {
            Some(Scope::HostAdmin) => Access::Admin,
            Some(Scope::Read) => Access::Viewer,
            _ => Access::Member,
        };
        store.ensure_member_access(&member.handle, access)?;
    }
    let access = store.member_access(&member.handle)?.unwrap_or(Access::Member);
    for device in &member.devices {
        if let Some(cap) = key_a(device).and_then(|e| cap_below(e.scope, ceiling(access))) {
            store.cap_new_device(device, cap)?;
        }
    }
    Ok(())
}

/// What an edit to access or a cap works out from the view as it stands.
struct Next {
    /// The view once the change is made.
    view: View,
    /// Raising: records first, then lines. Lowering: lines first.
    raising: bool,
    /// The devices whose line may rise (a promotion or an uncap names them).
    raise: BTreeSet<String>,
    /// Devices whose sessions reconnect.
    reconnect: BTreeSet<String>,
}

/// Computes what an edit does from the current view.
type NextFn = Box<dyn Fn(&View) -> Result<Next> + Send>;
/// Records an edit in the store.
type WriteFn = Box<dyn Fn(&Store) -> Result<()> + Send>;

/// An edit, and the store write that records it.
struct Edit {
    next: NextFn,
    write: WriteFn,
}

/// Apply `edit` under the fence lock, in the order its direction asks, and
/// reconnect the sessions it touches.
async fn apply(svc: &Service, edit: Edit, held: impl FnOnce() + Send + 'static) -> Result<()> {
    let (store, path) = (svc.store.clone(), svc.authorized_keys().to_path_buf());
    let raising = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reconnect = blocking(move || {
        held();
        fence::update_then(
            &path,
            fence::AUTHORIZED_KEYS,
            Placement::Last,
            |entries| {
                let entries = attributed(entries);
                let next = (edit.next)(&View::from_store(&store)?)?;
                let rebuilt = rebuild(&entries, &next.view, &next.raise)?;
                let (relined, changed) = relined(&entries, rebuilt);
                // A promotion is recorded before its lines are raised: if the
                // file can't be written, the device keeps the lower line.
                if next.raising {
                    (edit.write)(&store)?;
                    raising.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                let change = match changed {
                    true => {
                        let (ours, foreign) = crate::enrollment::sorted(&relined);
                        Change::Write { entries: ours, foreign }
                    }
                    false => Change::Leave,
                };
                Ok::<_, Refusal>((change, next.reconnect))
            },
            // A demotion is recorded only once its lines are written: a failed
            // write changes nothing, and a rollback never serves a stale,
            // higher line.
            |_| match raising.load(std::sync::atomic::Ordering::SeqCst) {
                true => Ok(()),
                false => (edit.write)(&store).map_err(Refusal),
            },
        )
    })
    .await?;
    for device in &reconnect {
        let closed = svc.sessions().reconnect(device);
        tracing::info!(client = %device, closed, "closed a device's sessions after its access changed (access_changed)");
    }
    Ok(())
}

/// `member.set_access`: give `handle` `access`.
pub async fn set_access(svc: &Service, handle: &str, access: Access, held: impl FnOnce() + Send + 'static) -> Result<()> {
    let handle = handle.to_string();
    svc.store.member(&handle)?;
    let (h1, h2) = (handle.clone(), handle);
    let edit = Edit {
        next: Box::new(move |now| {
            let before = now.access_of_member(&h1);
            let raising = before.is_some_and(|before| access > before);
            let devices = now.devices_of(&h1);
            Ok(Next {
                view: now.with_access(&h1, access),
                raising,
                raise: if raising { devices.clone() } else { BTreeSet::new() },
                reconnect: devices,
            })
        }),
        write: Box::new(move |store| store.set_member_access(&h2, access).map(|_| ())),
    };
    apply(svc, edit, held).await
}

/// `client.set_cap`: hold `device` at `cap`, or `None` to clear it.
pub async fn set_cap(svc: &Service, device: &str, cap: Option<Cap>, held: impl FnOnce() + Send + 'static) -> Result<()> {
    let (d1, d2) = (device.to_string(), device.to_string());
    let edit = Edit {
        next: Box::new(move |now| {
            let Some(old) = now.scope_of(&d1) else { return Err(DomainError::InvalidArgument { what: "device" }) };
            let view = now.with_cap(&d1, cap);
            let raising = view.scope_of(&d1).is_some_and(|new| rank(new) > rank(old));
            Ok(Next {
                raise: if raising { BTreeSet::from([d1.clone()]) } else { BTreeSet::new() },
                reconnect: BTreeSet::from([d1.clone()]),
                view,
                raising,
            })
        }),
        write: Box::new(move |store| store.set_device_cap(&d2, cap).map(|_| ())),
    };
    apply(svc, edit, held).await
}

/// The two socket-only calls, as the dispatcher runs them: refused unless the
/// runner's own socket asked (a device named in a preamble is that device, S4),
/// and unless a person did (never a pane).
pub async fn socket_person_only(svc: &Service, peer: &Peer, caller_pid: Option<i32>) -> Result<()> {
    if peer.client_id.is_some() {
        tracing::info!(by = ?peer.client_id, "refused: an access change from a device, not the runner's socket");
        return Err(DomainError::NotYours { what: "socket_only" });
    }
    refuse_admin_from_pane(svc, caller_pid).await
}

#[cfg(test)]
#[path = "person_access_tests.rs"]
mod tests;
