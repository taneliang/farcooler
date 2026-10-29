//! Who may change a person's access or a device's cap (ov-856, section 3 of
//! the design: the Access menu and Limit to Viewer on a phone).
//!
//! - **The runner's own socket, outside a pane,** may make either change, as
//!   it always could.
//! - **A remote device** may make it only when its person has stepped up for
//!   exactly this request (`step_up::may_admin` lets it past the scope gate,
//!   and [`authorize`] insists the step-up is for this method). It is then the
//!   owner's, or an Admin's within [`allowed`].
//! - **An Admin manages Members, Viewers and themselves.** They never give
//!   Admin, never touch another Admin or the owner's devices, and may only
//!   lower their own access or cap (S10): an Admin who could raise themselves
//!   would make the step-up worth nothing.
//! - **Decided again under the lock.** The rules are re-read, with the
//!   caller's generation, in the fence lock's one hold, so a demotion that
//!   lands in between refuses the change `access_changed`.

use farcooler_core::{DomainError, Result};
use farcooler_store::Store;
use farcooler_store::access::{Access, Cap};
use farcooler_transport::Peer;

use crate::people::Person;
use crate::person_access::{Authority, Guard, may_manage, may_manage_member, refuse_admin_from_pane, socket_person_only};
use crate::service::Service;

/// The change a request asks for.
pub(crate) enum Change<'a> {
    Access { handle: &'a str, access: Access },
    Cap { device: &'a str, cap: Option<Cap> },
}

/// Whether `caller` may make `change`, as the store stands.
pub(crate) fn allowed(store: &Store, caller: &Person, change: &Change<'_>) -> Result<()> {
    let me = match caller {
        Person::Owner => return Ok(()),
        Person::Released => return Err(DomainError::NotYours { what: "not_admin" }),
        Person::Member(handle) => handle,
    };
    if store.member_access(me)? != Some(Access::Admin) {
        return Err(DomainError::NotYours { what: "not_admin" });
    }
    let refused = DomainError::NotYours { what: "not_yours" };
    match change {
        Change::Access { handle, access } => {
            if handle == me {
                // Themselves: only down.
                return if *access < Access::Admin { Ok(()) } else { Err(refused) };
            }
            if *access >= Access::Admin || !may_manage_member(store, caller, handle)? {
                return Err(refused);
            }
            Ok(())
        }
        Change::Cap { device, cap } => {
            if store.member_of_device(device)?.as_deref() == Some(me.as_str()) {
                // Their own device: held lower, never freed.
                return if cap.is_some() { Ok(()) } else { Err(refused) };
            }
            if !may_manage(store, caller, device)? {
                return Err(refused);
            }
            Ok(())
        }
    }
}

/// Let `peer`'s request for `method` make `change`, or refuse it. The runner's
/// socket answers `None`; a stepped-up device answers the check to run again
/// under the fence lock.
pub(crate) async fn authorize(
    svc: &Service,
    peer: &Peer,
    caller_pid: Option<i32>,
    stepped: Option<&crate::step_up::Stepped>,
    method: &str,
    change: Change<'_>,
) -> Result<Option<Guard>> {
    let Some(device) = peer.client_id.as_deref() else {
        socket_person_only(svc, peer, caller_pid).await?;
        return Ok(None);
    };
    // A device has to have stepped up for this exact request; a line that
    // merely reads as `host_admin` is not a step-up.
    if stepped.is_none_or(|s| s.method != method || s.device != device) {
        tracing::info!(method, "refused: an access change from a device that didn't step up");
        return Err(DomainError::NotYours { what: "socket_only" });
    }
    refuse_admin_from_pane(svc, caller_pid).await?;
    let authority = Authority::read(&svc.store, Some(device))?;
    allowed(&svc.store, &authority.person, &change)?;
    let owned = OwnedChange::of(&change);
    let device = device.to_string();
    Ok(Some(Box::new(move |store: &Store| {
        authority.recheck(store, Some(&device))?;
        allowed(store, &authority.person, &owned.borrow())
    })))
}

/// [`Change`] with its words owned, to wait for the lock.
enum OwnedChange {
    Access { handle: String, access: Access },
    Cap { device: String, cap: Option<Cap> },
}

impl OwnedChange {
    fn of(change: &Change<'_>) -> OwnedChange {
        match change {
            Change::Access { handle, access } => OwnedChange::Access { handle: handle.to_string(), access: *access },
            Change::Cap { device, cap } => OwnedChange::Cap { device: device.to_string(), cap: *cap },
        }
    }

    fn borrow(&self) -> Change<'_> {
        match self {
            OwnedChange::Access { handle, access } => Change::Access { handle, access: *access },
            OwnedChange::Cap { device, cap } => Change::Cap { device, cap: *cap },
        }
    }
}

#[cfg(test)]
#[path = "person_access_change_tests.rs"]
mod tests;
