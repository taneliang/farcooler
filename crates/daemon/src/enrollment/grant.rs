//! Who may call `client.enroll`, and the runner's record of every call
//! (ov-828).
//!
//! **The owner's own `control` device may grant.** A phone is enrolled at
//! `control` (the Mac's ceremony asks for it, and nothing raises it), and the
//! browser is added from a phone: so with `client.enroll` at `host_admin`,
//! every phone's grant was refused at the scope gate, the runner logged
//! nothing, and the reply named the runner pending. That is what the owner hit
//! pairing a browser with Cosmo on Oct 26.
//!
//! What a `control` device may grant is bounded by what it already holds, so
//! no device widens its own access:
//!
//! - **not a shell line.** A plain line has no forced command and so no
//!   scope; it is `host_admin`'s alone, as before.
//! - **not above its own scope.** A `control` device grants `read` or
//!   `control`, never `host_admin`.
//! - **not from a member's device.** A member's device at `control` could
//!   otherwise add lines to the owner's `authorized_keys`, even released ones,
//!   each with a node key the tunnel would admit. Members grant only at
//!   `host_admin`, as before (ov-719).
//!
//! `control` already types into the owner's shells, which can write any
//! file, `authorized_keys` included; what this adds over that route is that
//! the runner writes the line itself, with a scope and a forced command, and
//! logs it.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{ClientEnroll, Request, Scope, request, result};
use farcooler_transport::Peer;

use crate::people::Person;

/// Whether a caller at `caller`, who is `who`, may grant what `request`
/// asks.
///
/// A device below `host_admin` that isn't the owner's is refused
/// `not_yours: not_owners_device`, so a member's phone can say "only the
/// owner can add devices here" rather than "update Far Cooler" (ov-828 review
/// N4). A grant wider than `control` is refused `scope_denied` naming
/// `host_admin`, the scope that may, as a runner from before ov-828 refuses
/// every `control` caller.
pub fn may_grant(caller: Scope, who: &Person, request: &ClientEnroll) -> Result<()> {
    if caller == Scope::HostAdmin {
        return Ok(());
    }
    if !matches!(who, Person::Owner) {
        return Err(DomainError::NotYours { what: "not_owners_device" });
    }
    let asked = Scope::try_from(request.scope).unwrap_or(Scope::Unspecified);
    let within_control = matches!(asked, Scope::Read | Scope::Control);
    if caller == Scope::Control && !request.shell_access && within_control {
        return Ok(());
    }
    Err(DomainError::ScopeDenied { needed: "host_admin" })
}

/// One `client.enroll`, logged at INFO when it is asked and when it is
/// answered, whatever answered it: the scope gate, the people gate or the
/// fence. Device ids and flags only, never a key: the ids are the names this
/// runner's own `authorized_keys` writes in plain text.
pub struct Attempt {
    by: String,
    client: String,
}

impl Attempt {
    /// The attempt in `req`, logged, or `None` for any other method.
    ///
    /// Keyed on the method, so another method's request carrying this
    /// payload is not logged as an enrollment. The device id is the caller's
    /// text, logged before anything has judged it and by a caller as narrow
    /// as `read`, so one the fence would refuse is logged as `(invalid)`, and
    /// a usable one Debug-quoted.
    pub fn of(req: &Request, peer: &Peer) -> Option<Attempt> {
        if req.method != "client.enroll" {
            return None;
        }
        let none = ClientEnroll::default();
        let p = match req.payload.as_ref() {
            Some(request::Payload::ClientEnroll(p)) => p,
            _ => &none,
        };
        let by = peer.client_id.clone().unwrap_or_else(|| "-".into());
        let client = if farcooler_fence::usable_client_id(&p.client_id) { p.client_id.clone() } else { "(invalid)".into() };
        let scope = Scope::try_from(p.scope).map(|s| s.as_str_name()).unwrap_or("unknown");
        tracing::info!(
            by = ?by,
            by_scope = peer.scope.as_str_name(),
            client = ?client,
            scope,
            shell = p.shell_access,
            tunnel = !p.node_key.is_empty(),
            teammate = p.for_teammate,
            "a device asked to enroll another"
        );
        Some(Attempt { by, client })
    }

    /// The answer, logged: written, already there, or the refusal's word.
    pub fn answered(self, outcome: &Result<result::Value>) {
        let Attempt { by, client } = self;
        match outcome {
            Ok(result::Value::ClientEnroll(done)) => tracing::info!(
                by = ?by,
                client = ?client,
                outcome = if done.already_enrolled { "already_enrolled" } else { "enrolled" },
                tunnel = !done.conn_blob.is_empty(),
                "an enrollment was answered"
            ),
            Ok(_) => tracing::info!(by = ?by, client = ?client, outcome = "unexpected", "an enrollment was answered"),
            Err(e) => tracing::info!(
                by = ?by,
                client = ?client,
                outcome = farcooler_core::error::word(e.code()),
                what = e.what(),
                "an enrollment was refused"
            ),
        }
    }
}

#[cfg(test)]
#[path = "grant_tests.rs"]
mod tests;
