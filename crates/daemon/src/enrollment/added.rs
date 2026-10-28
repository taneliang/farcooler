//! Who added each device, and what follows from it (ov-838, the ov-828
//! review's N1, N5 and N6).
//!
//! - **Every line records its enroller** (`Store::record_added_by`, written
//!   under the fence lock before the line). `client.list` carries it as
//!   `added_by`, so a client can list, beside a lost phone, the devices it
//!   added and those they added in turn, and `client.revoke` with
//!   `with_added` takes them all in one write ([`revoke_keeping`]).
//! - **A device below `host_admin` adds at most [`ADD_LIMIT`] a day**
//!   ([`may_add`]), and never under an id a revoke marked: that id's live
//!   ssh master, if one is still open, would be served again as the new
//!   device.
//! - **A device below `host_admin` never starts this runner's tunnel**
//!   ([`Caller::may_start_tunnel`]).

use std::sync::Arc;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{ClientList, ClientRevoke, Scope};

use super::Refusal;
use crate::service::Service;
use crate::sessions::{Session, Sessions};

/// How many devices one device below `host_admin` may add in [`ADD_WINDOW`].
/// A person pairs a browser or two; a client adding more than this in a day
/// is misbehaving, or isn't the person's.
pub const ADD_LIMIT: u32 = 5;

/// The window [`ADD_LIMIT`] counts over.
pub const ADD_WINDOW: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Who asked for an enrollment over a connection: the session it arrived
/// on, and the scope that session's line grants.
#[derive(Clone)]
pub struct Caller {
    pub session: Arc<Session>,
    pub scope: Scope,
}

impl Caller {
    pub fn new(session: Arc<Session>, scope: Scope) -> Self {
        Caller { session, scope }
    }

    /// Whether this caller's enroll may start this runner's tunnel when none
    /// is running: `host_admin` only (the ruling on the ov-828 review's N5).
    /// Below it, a device is admitted to a tunnel that is already running and
    /// pairs as direct otherwise. Starting one mints a tunnel identity that
    /// persists, joins the relay network, and makes this host's sshd
    /// reachable from it for good; that's the runner's shape, which
    /// `host_admin` decides, and not a side effect a phone's grant should
    /// have. This runner's own socket (no caller) may.
    pub fn may_start_tunnel(&self) -> bool {
        self.scope == Scope::HostAdmin
    }

    fn below_host_admin(&self) -> bool {
        self.scope != Scope::HostAdmin
    }
}

/// Whether `caller` may add a line under `client_id`, asked under the fence
/// lock when a line is about to be written. `host_admin` and this runner's
/// socket always may; below it, an id that already has a line (`has_line`)
/// or a marked id is `device_taken` and the
/// [`ADD_LIMIT`]th device in a day is `add_limit`. A store that can't count
/// refuses.
pub(super) fn may_add(store: &farcooler_store::Store, sessions: &Sessions, caller: Option<&Caller>, client_id: &str, has_line: bool) -> Result<()> {
    let Some(caller) = caller.filter(|c| c.below_host_admin()) else { return Ok(()) };
    if has_line {
        tracing::warn!(client = %client_id, "refused: a device below host_admin enrolling under an id that already has a line");
        return Err(DomainError::Conflict { what: "device_taken" });
    }
    if sessions.is_marked(client_id) {
        tracing::warn!(client = %client_id, "refused: a device below host_admin taking an id a revoke marked");
        return Err(DomainError::Conflict { what: "device_taken" });
    }
    let by = caller.session.client_id().unwrap_or("");
    let since = super::now_millis() - ADD_WINDOW.as_millis() as i64;
    if store.added_by_since(by, since)? >= ADD_LIMIT {
        tracing::warn!(by = %by, limit = ADD_LIMIT, "refused: a device added as many devices as it may today");
        return Err(DomainError::Conflict { what: "add_limit" });
    }
    Ok(())
}

/// `list` with each line's `added_by` filled in from the store. A store
/// that can't be read leaves them empty and says so: the list is still the
/// file's, which is what decides who may log in.
pub(super) fn with_added_by(svc: &Service, mut list: ClientList) -> ClientList {
    match svc.store.added_by_all() {
        Ok(rows) => {
            let by: std::collections::HashMap<String, String> = rows.into_iter().collect();
            for item in list.items.iter_mut().filter(|i| !i.client_id.is_empty()) {
                if let Some(added_by) = by.get(&item.client_id) {
                    item.added_by = added_by.clone();
                }
            }
        }
        Err(e) => tracing::warn!(error = %e.what(), "who added each device couldn't be read; the list names none"),
    }
    list
}

/// After a revoke's write: what the store kept about `device` alone goes
/// (the ov-828 re-review's R4). Its line is gone, so a row left would only
/// wait on a bind that can't come. A store that can't be written is
/// logged: every row it leaves keeps the device refused.
pub(super) fn forget(svc: &Service, device: &str) {
    if let Err(e) = svc.store.forget_revoked_device(device) {
        tracing::warn!(client = %device, error = %e.what(), "a revoked device's pending rows stayed");
    }
}

/// `client.revoke`, with `with_added` (ov-838): the device, and every device
/// it added, and those they added, that still has a line and that `keep`
/// lets go, in one write, decided under the fence lock. A member's device
/// passes a `keep` that spares the owner's devices; a `keep` that errs
/// refuses the revoke, never spares. With `with_added`, a
/// store that can't say who added whom refuses the revoke rather than taking
/// less than it was asked to.
pub async fn revoke_keeping(
    svc: &Service,
    request: &ClientRevoke,
    keep: impl Fn(&str) -> Result<bool> + Send + 'static,
) -> Result<ClientList> {
    let root = request.client_id.clone();
    let with_added = request.with_added;
    let store = svc.store.clone();
    super::revoke_targets(svc, request, move |entries| {
        let mut gone = vec![root.clone()];
        if with_added {
            let rows = store.added_by_all().map_err(Refusal)?;
            for id in farcooler_store::device_added_by::added_under(&rows, &root) {
                if entries.iter().any(|e| e.client_id == id) && keep(&id).map_err(Refusal)? {
                    gone.push(id);
                }
            }
        }
        Ok(gone)
    })
    .await
}

/// `client.revoke` as `person` asked it: what a member's device revokes
/// along with the device it names spares the owner's devices (ov-838). The
/// gate holds the device it names (`touches_the_owners_device`); this holds
/// what that device added.
pub async fn revoke_as(svc: &Service, request: &ClientRevoke, person: &crate::people::Person) -> Result<ClientList> {
    let spare_owners = person.is_someone_else();
    let store = svc.store.clone();
    revoke_keeping(svc, request, move |id| if spare_owners { store.device_is_owners(id).map(|owners| !owners) } else { Ok(true) }).await
}

#[cfg(test)]
#[path = "added_tests.rs"]
mod tests;
