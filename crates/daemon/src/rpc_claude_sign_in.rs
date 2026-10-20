//! Claude sign-in's routes (ov-584): `claude.sign_in`,
//! `claude.sign_in_finish`, `claude.sign_in_cancel` and `claude.sign_out`,
//! behind `claude_sign_in`. Who may is `people::may_sign_in`, checked on
//! every call; the sign-in itself is `claude_sign_in`.
//!
//! A sign-in is named by its session id, but the id is no authority: finish
//! and cancel act only on the calling member's own sign-in, started from
//! this same device.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{Request, request, result};
use farcooler_transport::Peer;

use crate::people::{Person, may_sign_in, person_of};
use crate::service::Service;
use crate::task_ops::required_id;
use crate::watch::Watcher;

fn payload_missing() -> DomainError {
    DomainError::InvalidArgument { what: "payload" }
}

/// The calling member's handle, once they may act on their own login.
async fn caller(svc: &Service, peer: &Peer, caller_pid: Option<i32>) -> Result<String> {
    let Person::Member(me) = person_of(&svc.store, peer)? else { return Err(DomainError::NotYours { what: "not_yours" }) };
    may_sign_in(svc, peer, caller_pid, &me).await?;
    Ok(me)
}

/// One sign-in route, as `Rpc::dispatch` hands it over.
pub(crate) async fn dispatch(svc: &Service, watcher: &Watcher, peer: &Peer, caller_pid: Option<i32>, req: Request) -> Result<result::Value> {
    let device = peer.client_id.as_deref();
    match (req.method.as_str(), req.payload) {
        ("claude.sign_in", Some(request::Payload::ClaudeSignInStart(p))) => {
            let handle = p.handle.trim();
            may_sign_in(svc, peer, caller_pid, handle).await?;
            Ok(result::Value::ClaudeSignIn(svc.start_sign_in(handle, device).await?))
        }
        ("claude.sign_in_finish", Some(request::Payload::ClaudeSignInFinish(p))) => {
            let me = caller(svc, peer, caller_pid).await?;
            let session = required_id(&p.session_id)?;
            let login = svc.finish_sign_in(&me, session, device, &p.code).await?;
            watcher.announce_fleet_changed();
            watcher.announce_needs_you();
            Ok(result::Value::MemberClaudeLogin(login))
        }
        ("claude.sign_in_cancel", Some(request::Payload::ClaudeSignInCancel(p))) => {
            let me = caller(svc, peer, caller_pid).await?;
            let session = required_id(&p.session_id)?;
            Ok(result::Value::MemberClaudeLogin(svc.cancel_sign_in(&me, session, device)?))
        }
        ("claude.sign_out", Some(request::Payload::ClaudeSignOut(p))) => {
            let handle = p.handle.trim();
            may_sign_in(svc, peer, caller_pid, handle).await?;
            let login = svc.sign_out(handle).await?;
            watcher.announce_fleet_changed();
            watcher.announce_needs_you();
            Ok(result::Value::MemberClaudeLogin(login))
        }
        ("claude.sign_in" | "claude.sign_in_finish" | "claude.sign_in_cancel" | "claude.sign_out", _) => Err(payload_missing()),
        _ => Err(DomainError::CapabilityUnsupported { needed: "a newer Far Cooler" }),
    }
}
