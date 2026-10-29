//! Add Teammate in one write (ov-856, section 8 of the design): `client.enroll`
//! with `for_teammate` and a [`TeammateBind`] writes the teammate's line and
//! binds it to a person with an access, in the fence lock's one hold.
//!
//! - **One request carries the person, the access and the device,** so one
//!   step-up covers all three: they are in the signed bytes, and the runner
//!   runs the request it decodes from them.
//! - **Only the owner makes an Admin.** An Admin adds Members and Viewers, and
//!   never anyone who already is an Admin (`may_manage_member`).
//! - **A person keeps the access they have.** A teammate already here is bound
//!   to by handle, and only when their account is the one the request names (or
//!   they have none yet: added by hand, they take it);
//!   a different access is refused `access_exists`, because changing a
//!   person's access is `member.set_access`, a separate step-up.
//! - **The line fits the access.** A viewer's line is `read`; anyone else's is
//!   `control` or `read` (a `read` line for a Member is capped at Viewer). Never
//!   `host_admin`, and never a shell.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Scope};
use farcooler_store::Store;
use farcooler_store::access::Access;
use farcooler_store::people::Directory;

use crate::access_migration::rank;
use crate::people::Person;
use crate::person_access::{cap_below, may_manage_member};

/// What a teammate's device is bound to, once the request has passed the rules
/// that need no state.
#[derive(Debug, Clone)]
pub(crate) struct Binding {
    handle: String,
    name: String,
    account: String,
    email: String,
    access: Option<Access>,
    scope: Scope,
}

/// The access a request that names none gives: what a teammate is by default.
const DEFAULT: Access = Access::Member;

/// The rules that need no state: the caller is the owner or an Admin, the
/// access is a word the caller may give, and the line fits it. `caller_access`
/// is the caller's person's access, `None` for the owner.
pub(crate) fn plan(caller: &Person, caller_access: Option<Access>, asked: Scope, bind: &pb::TeammateBind) -> Result<Binding> {
    let allowed = match caller {
        Person::Owner => true,
        Person::Member(_) => caller_access == Some(Access::Admin),
        Person::Released => false,
    };
    if !allowed {
        tracing::info!("refused: Add Teammate from a caller who isn't the owner or an Admin");
        return Err(DomainError::NotYours { what: "not_admin" });
    }
    let access = match bind.access.trim() {
        "" => None,
        word => Some(Access::parse(word).ok_or(DomainError::InvalidArgument { what: "access" })?),
    };
    // Admin is the owner's to give, and only the owner's: an Admin who could
    // would be making peers.
    if access == Some(Access::Admin) && *caller != Person::Owner {
        tracing::warn!("refused: an Admin adding an Admin");
        return Err(DomainError::NotYours { what: "admin_is_owners" });
    }
    if !matches!(asked, Scope::Read | Scope::Control) {
        return Err(DomainError::ScopeDenied { needed: "host_admin" });
    }
    if access == Some(Access::Viewer) && asked != Scope::Read {
        return Err(DomainError::Conflict { what: "scope_access" });
    }
    Ok(Binding {
        handle: bind.handle.trim().to_string(),
        name: bind.name.trim().to_string(),
        account: bind.account.trim().to_string(),
        email: bind.email.trim().to_string(),
        access,
        scope: asked,
    })
}

impl Binding {
    /// Bind `device` as the teammate's, under the fence lock and before its
    /// line is written. A person already here keeps their name, their access
    /// and their other devices; a new one is recorded with the access asked.
    pub(crate) fn bind(&self, store: &Store, caller: &Person, device: &str) -> Result<()> {
        if !store.access_migrated()? {
            return Err(DomainError::Conflict { what: "not_migrated" });
        }
        let existing = match store.member(&self.handle) {
            Ok(member) => Some(member),
            Err(DomainError::NotFound) => None,
            Err(e) => return Err(e),
        };
        let recorded = match &existing {
            Some(member) => {
                // The handle is theirs only when the account is: a handle a
                // different person holds is never renamed by an enroll. A
                // person added by hand has no account yet, and takes this one.
                let theirs = member.account.as_deref().unwrap_or_default();
                if !theirs.is_empty() && theirs != self.account {
                    return Err(DomainError::Conflict { what: "handle_taken" });
                }
                if !may_manage_member(store, caller, &self.handle)? {
                    return Err(DomainError::NotYours { what: "not_yours" });
                }
                store.member_access(&self.handle)?
            }
            None => None,
        };
        let access = match (recorded, self.access) {
            (Some(had), Some(asked)) if had != asked => return Err(DomainError::Conflict { what: "access_exists" }),
            (Some(had), _) => had,
            (None, asked) => asked.unwrap_or(DEFAULT),
        };
        let line_ceiling = if access == Access::Viewer { Scope::Read } else { Scope::Control };
        if rank(self.scope) > rank(line_ceiling) {
            return Err(DomainError::Conflict { what: "scope_access" });
        }
        let mut devices = existing.as_ref().map(|m| m.devices.clone()).unwrap_or_default();
        if !devices.iter().any(|d| d == device) {
            devices.push(device.to_string());
        }
        let name = existing.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| self.name.clone());
        store.set_member_releasing(&self.handle, &name, &devices, Directory { account: &self.account, email: &self.email }, None)?;
        if recorded.is_none() {
            store.ensure_member_access(&self.handle, access)?;
        }
        if let Some(cap) = cap_below(self.scope, line_ceiling) {
            store.cap_new_device(device, cap)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "teammate_bind_tests.rs"]
mod tests;
