//! The three calls step-up adds (ov-856, slice 3): `auth.challenge`,
//! `client.set_step_up_key` and `client.approve_step_up`. The checks of a
//! signature are `step_up`'s; this is who may put a key on a device, and who
//! may trust it (B3).
//!
//! - **A key is a proposal until a person says otherwise.** A device may
//!   propose its own, never a browser (decision 2), and a proposal signs
//!   nothing. The runner's own socket, outside a pane, writes a trusted key
//!   for any device. An enroll stepped up by a trusted key writes the new
//!   device's trusted (`person_access::enroll`).
//! - **An approval names the key.** It carries the six digits the proposing
//!   device shows (`step_up::code_of`), so the person approves the key in
//!   their hand, not a label the proposer chose. It comes from the socket
//!   outside a pane, or from another trusted device of the same person, with
//!   its own step-up. A proposal older than ten minutes is refused.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::Peer;

use crate::people::person_of_device;
use crate::service::Service;
use crate::sessions::Session;
use crate::step_up::{self, Stepped};

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// `auth.challenge`: a nonce for this connection's next step-up.
pub fn challenge(session: &Session) -> Result<result::Value> {
    Ok(result::Value::AuthChallenge(pb::AuthChallenge { nonce: session.challenges().issue(now_ms())?.to_vec().into() }))
}

fn state(device: &str, key: &farcooler_store::step_up::StepUpKeyRow) -> result::Value {
    result::Value::StepUpKeyState(pb::StepUpKeyState {
        client_id: device.to_string(),
        state: if key.trusted { "trusted" } else { "proposed" }.to_string(),
        code: step_up::code_of(&key.alg, &key.public_key),
    })
}

/// Whether the device behind `peer` holds a Key A line that isn't an SSH key
/// of ours: a passkey, which is a browser's (decision 2).
pub(crate) fn is_browser(svc: &Service, device: &str) -> Result<bool> {
    let entries = farcooler_fence::read(svc.authorized_keys(), farcooler_fence::AUTHORIZED_KEYS).map_err(|_| DomainError::OperationFailed)?;
    Ok(entries.iter().any(|e| e.client_id == device && !e.shell_access && !farcooler_fence::holds_ed25519(e)))
}

/// `client.set_step_up_key`.
pub async fn set_key(svc: &Service, peer: &Peer, caller_pid: Option<i32>, p: pb::ClientSetStepUpKey) -> Result<result::Value> {
    let key = p.key.ok_or(DomainError::InvalidArgument { what: "step_up_key" })?;
    step_up::check_key(&key.alg, &key.public_key)?;
    let (device, trusted) = match peer.client_id.as_deref() {
        // The runner's own socket, outside a pane: trusted at once.
        None => {
            crate::person_access::refuse_admin_from_pane(svc, caller_pid).await?;
            (p.client_id.trim().to_string(), true)
        }
        // A device proposes its own, and only its own.
        Some(own) => {
            if !p.client_id.trim().is_empty() && p.client_id.trim() != own {
                return Err(DomainError::NotYours { what: "not_yours" });
            }
            if key.alg == step_up::ALG_WEBAUTHN || is_browser(svc, own)? {
                tracing::warn!(device = own, "refused: a browser proposing a step-up key");
                return Err(DomainError::Conflict { what: "defensive_only" });
            }
            (own.to_string(), false)
        }
    };
    if device.is_empty() {
        return Err(DomainError::InvalidArgument { what: "device" });
    }
    svc.store.put_step_up_key(&device, &key.alg, &key.public_key, trusted, now_ms())?;
    let row = svc.store.step_up_key(&device)?.ok_or(DomainError::OperationFailed)?;
    tracing::info!(device = %device, trusted, "recorded a step-up key");
    Ok(state(&device, &row))
}

/// `client.approve_step_up`: trust `p.client_id`'s proposed key, given its code.
pub async fn approve(svc: &Service, peer: &Peer, caller_pid: Option<i32>, stepped: Option<&Stepped>, p: pb::ClientApproveStepUp) -> Result<result::Value> {
    let target = p.client_id.trim();
    match peer.client_id.as_deref() {
        None => crate::person_access::refuse_admin_from_pane(svc, caller_pid).await?,
        Some(approver) => {
            // Another trusted device of the same person, stepped up for this.
            if !step_up::may_admin(&svc.store, peer, stepped, "client.approve_step_up")? || approver == target {
                return Err(DomainError::Conflict { what: "step_up_needed" });
            }
            let theirs = person_of_device(&svc.store, Some(approver))?;
            if theirs == crate::people::Person::Released || theirs != person_of_device(&svc.store, Some(target))? {
                tracing::warn!(approver, target, "refused: an approval for another person's key");
                return Err(DomainError::NotYours { what: "not_yours" });
            }
        }
    }
    let row = svc.store.step_up_key(target)?.ok_or(DomainError::NotFound)?;
    if step_up::code_of(&row.alg, &row.public_key) != p.code.trim() {
        return Err(DomainError::Conflict { what: "wrong_code" });
    }
    svc.store.trust_step_up_key(target, now_ms())?;
    let row = svc.store.step_up_key(target)?.ok_or(DomainError::OperationFailed)?;
    tracing::info!(device = target, "a step-up key was approved");
    Ok(state(target, &row))
}

/// The three calls, and phone approval's five (ov-858), as the dispatcher runs them.
pub async fn dispatch(svc: &Service, peer: &Peer, caller_pid: Option<i32>, session: &Session, stepped: Option<&Stepped>, req: pb::Request) -> Result<result::Value> {
    if req.method.starts_with("approval.") {
        return crate::approvals::dispatch(svc, peer, req).await;
    }
    match req.method.as_str() {
        "auth.challenge" => challenge(session),
        "client.set_step_up_key" => {
            let p = payload(|p| if let request::Payload::ClientSetStepUpKey(p) = p { Some(p) } else { None }, req)?;
            set_key(svc, peer, caller_pid, p).await
        }
        _ => {
            let p = payload(|p| if let request::Payload::ClientApproveStepUp(p) = p { Some(p) } else { None }, req)?;
            approve(svc, peer, caller_pid, stepped, p).await
        }
    }
}

/// The payload of `req` as `p`, or the refusal for one that isn't.
pub fn payload<T>(extract: impl FnOnce(request::Payload) -> Option<T>, req: pb::Request) -> Result<T> {
    req.payload.and_then(extract).ok_or(DomainError::InvalidArgument { what: "payload" })
}
