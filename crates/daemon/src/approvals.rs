//! A Mac that isn't the runner has an admin action approved on the person's
//! iPhone (ov-858, option (c) of the card; the other halves are
//! `reports/ov-856/design.md` section 3 and `step_up`).
//!
//! The Mac's own Secure Enclave key can't exist on the ad hoc-signed release
//! (no `application-identifier`, no provisioning profile), so such a Mac holds
//! no step-up key, and its admin action fails closed. This is how it is
//! approved instead:
//!
//! 1. The Mac sends `approval.ask` with the exact bytes of the request it wants
//!    to run (`StepUp.action_bytes`). The runner checks it is a plain admin
//!    request from an Admin's device, finds another device of that person with
//!    a trusted step-up key, issues its own 32-byte challenge for exactly
//!    that action, and pushes the person's phone.
//! 2. The phone lists its person's pending approvals (`approval.list`), shows
//!    the action, its target and which Mac asked, asks for Face ID, and signs
//!    the statement for **the Mac's** device id, these action bytes and this
//!    challenge, with its own trusted key (`approval.grant`). The runner
//!    checks that signature exactly as it checks a step-up, and no other.
//! 3. The Mac polls `approval.status`, then sends the original request with
//!    `StepUp { alg: "phone-approval", approval_id, action_bytes }`. `admit`
//!    spends the approval, once, for the asking device, and only if those
//!    action bytes are the ones the phone signed.
//!
//! - **No button grants it.** A grant is a signature over a statement the phone
//!   builds from the bytes it displays; a notification action can't make one
//!   without opening the app and the key's own Face ID prompt.
//! - **One approval, one action, one device.** The id is spent by the request
//!   that uses it, a different digest or device is refused, and an approval
//!   lives [`TTL_MS`]. A denied or expired one signs nothing.
//! - **Same person.** The approver must be another device of the asking
//!   Mac's person. A person's Admin access is checked again when the request
//!   runs (`step_up::may_admin`).

use std::sync::Mutex;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, StepUp, request, result};
use farcooler_transport::Peer;
use prost::Message;
use sha2::{Digest, Sha256};

use crate::people::{Person, person_of_device};
use crate::service::Service;
use crate::step_up::{self, ALG_DEVICE, Context};

/// `StepUp.alg` for a request an iPhone approved.
pub const ALG_APPROVAL: &str = "phone-approval";

/// How long an approval waits to be granted or used.
pub const TTL_MS: i64 = 120_000;

/// How many approvals one Mac may have waiting; the oldest goes first.
pub const LIVE_PER_DEVICE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Pending,
    Granted,
    Denied,
    Used,
}

#[derive(Debug, Clone)]
struct Approval {
    id: String,
    device: String,
    person: Person,
    method: String,
    action_bytes: Vec<u8>,
    nonce: [u8; 32],
    asked_at: i64,
    phase: Phase,
}

impl Approval {
    fn live(&self, now_ms: i64) -> bool {
        now_ms - self.asked_at <= TTL_MS
    }
}

/// The approvals this runner is holding, in memory: a restart forgets them,
/// and the Mac asks again.
#[derive(Default)]
pub struct Approvals(Mutex<Vec<Approval>>);

impl Approvals {
    fn add(&self, approval: Approval, now_ms: i64) {
        let mut all = self.0.lock().unwrap();
        all.retain(|a| now_ms - a.asked_at <= 10 * TTL_MS);
        while all.iter().filter(|a| a.device == approval.device).count() >= LIVE_PER_DEVICE {
            let oldest = all.iter().position(|a| a.device == approval.device).expect("counted");
            all.remove(oldest);
        }
        all.push(approval);
    }

    fn find(&self, id: &str) -> Option<Approval> {
        self.0.lock().unwrap().iter().find(|a| a.id == id).cloned()
    }

    /// Move `id` from `from` to `to`, true when it was in `from` and live.
    fn advance(&self, id: &str, from: Phase, to: Phase, now_ms: i64) -> bool {
        let mut all = self.0.lock().unwrap();
        match all.iter_mut().find(|a| a.id == id && a.phase == from && a.live(now_ms)) {
            Some(a) => {
                a.phase = to;
                true
            }
            None => false,
        }
    }

    /// Spend a granted approval for `device`, once, if its action is `digest`.
    pub(crate) fn spend(&self, id: &str, device: &str, digest: &[u8; 32], now_ms: i64) -> bool {
        let mut all = self.0.lock().unwrap();
        let Some(a) = all.iter_mut().find(|a| a.id == id) else { return false };
        let same = a.device == device && Sha256::digest(&a.action_bytes)[..] == digest[..];
        if a.phase != Phase::Granted || !a.live(now_ms) || !same {
            return false;
        }
        a.phase = Phase::Used;
        true
    }
}

fn refused() -> DomainError {
    DomainError::Conflict { what: "step_up_failed" }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The device's label as the runner wrote it into `authorized_keys`.
fn label_of(svc: &Service, device: &str) -> String {
    let entries = farcooler_fence::read(svc.authorized_keys(), farcooler_fence::AUTHORIZED_KEYS).unwrap_or_default();
    entries.iter().find(|e| e.client_id == device && !e.shell_access).map(|e| farcooler_fence::device_name(&e.label).unwrap_or(&e.label).to_string()).unwrap_or_default()
}

/// What the action acts on, in the runner's own words: a device's label or a
/// member's name. Empty when it names nothing.
fn target_of(svc: &Service, request: &pb::Request) -> String {
    let member = |handle: &str| svc.store.member(handle).map(|m| m.name).unwrap_or_else(|_| handle.to_string());
    match request.payload.as_ref() {
        Some(request::Payload::ClientRevoke(p)) => label_of(svc, &p.client_id),
        Some(request::Payload::ClientSetCap(p)) => label_of(svc, &p.client_id),
        Some(request::Payload::ClientApproveStepUp(p)) => label_of(svc, &p.client_id),
        Some(request::Payload::ClientEnroll(p)) => p.label.clone(),
        Some(request::Payload::MemberSetAccess(p)) => member(&p.handle),
        Some(request::Payload::MemberSetRole(p)) => member(&p.handle),
        Some(request::Payload::MemberRemove(p)) => member(&p.handle),
        Some(request::Payload::MemberSet(p)) => member(&p.handle),
        _ => String::new(),
    }
}

fn state_of(svc: &Service, a: &Approval, now_ms: i64) -> pb::ApprovalState {
    let state = match a.phase {
        Phase::Used | Phase::Granted => "granted",
        Phase::Denied => "denied",
        Phase::Pending if !a.live(now_ms) => "expired",
        Phase::Pending => "pending",
    };
    let target = Request::decode(a.action_bytes.as_slice()).map(|r| target_of(svc, &r)).unwrap_or_default();
    pb::ApprovalState {
        id: a.id.clone(),
        state: if a.phase == Phase::Granted && !a.live(now_ms) { "expired" } else { state }.to_string(),
        method: a.method.clone(),
        action_bytes: a.action_bytes.clone().into(),
        nonce: a.nonce.to_vec().into(),
        device_id: a.device.clone(),
        device_name: label_of(svc, &a.device),
        target_name: target,
        expires_at: a.asked_at + TTL_MS,
    }
}

/// A trusted device key of `person` other than `not`, the phones that can approve.
fn approvers(svc: &Service, person: &Person, not: &str) -> Result<Vec<String>> {
    let Person::Member(handle) = person else { return Ok(Vec::new()) };
    let mut found = Vec::new();
    for device in svc.store.member(handle)?.devices {
        let key = svc.store.step_up_key(&device)?;
        if device != not && key.is_some_and(|k| k.trusted && k.alg == ALG_DEVICE) {
            found.push(device);
        }
    }
    Ok(found)
}

/// `approval.ask`.
pub async fn ask(svc: &Service, peer: &Peer, p: pb::ApprovalAsk) -> Result<result::Value> {
    // The runner's own socket needs no approval.
    let device = peer.client_id.as_deref().ok_or(DomainError::Conflict { what: "step_up_unneeded" })?;
    let bytes = p.action_bytes.to_vec();
    let asked = Request::decode(bytes.as_slice()).map_err(|_| refused())?;
    if asked.encode_to_vec() != bytes || !asked.request_id.is_empty() || asked.step_up.is_some() {
        return Err(refused());
    }
    if !step_up::METHODS.contains(&asked.method.as_str()) {
        return Err(DomainError::Conflict { what: "step_up_unneeded" });
    }
    let person = person_of_device(&svc.store, Some(device))?;
    if !step_up::is_admin(&svc.store, device)? {
        return Err(DomainError::NotYours { what: "not_admin" });
    }
    if approvers(svc, &person, device)?.is_empty() {
        return Err(DomainError::Conflict { what: "no_approver" });
    }
    let now = crate::step_up_rpc::now_ms();
    let approval = Approval {
        id: hex(&step_up::fresh_nonce()?[..16]),
        device: device.to_string(),
        person: person.clone(),
        method: asked.method.clone(),
        action_bytes: bytes,
        nonce: step_up::fresh_nonce()?,
        asked_at: now,
        phase: Phase::Pending,
    };
    let answer = state_of(svc, &approval, now);
    svc.sessions().approvals.add(approval, now);
    tell_phone(svc, &person);
    tracing::info!(device, method = %asked.method, "asked a phone to approve an admin action");
    Ok(result::Value::ApprovalState(answer))
}

/// Push the person's phones, in the relay's own words. Best effort: the phone
/// also lists the approvals whenever it opens.
fn tell_phone(svc: &Service, person: &Person) {
    let Person::Member(handle) = person else { return };
    let Some(account) = svc.store.member(handle).ok().and_then(|m| m.account) else { return };
    let Some(pairing) = crate::push::Pairing::load_in(svc.root_dir()) else { return };
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    let runner = crate::service::stable_host_id(svc.install_id()).to_string();
    let event = hex(&step_up::fresh_nonce().unwrap_or_default()[..16]);
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().unwrap_or_default();
    runtime.spawn(async move {
        crate::push::notify_member(&client, &pairing, &account, crate::push::MemberKind::Approval, &event, Some(&runner), None).await;
    });
}

/// `approval.list`: what the caller's person is being asked, newest first.
pub fn list(svc: &Service, peer: &Peer) -> Result<result::Value> {
    let me = peer.client_id.as_deref().ok_or(DomainError::NotYours { what: "not_yours" })?;
    let person = person_of_device(&svc.store, Some(me))?;
    let may = svc.store.step_up_key(me)?.is_some_and(|k| k.trusted && k.alg == ALG_DEVICE);
    let now = crate::step_up_rpc::now_ms();
    let mut items = Vec::new();
    if may && person != Person::Released {
        for a in svc.sessions().approvals.0.lock().unwrap().iter().rev() {
            if a.person == person && a.device != me && a.phase == Phase::Pending && a.live(now) {
                items.push(state_of(svc, a, now));
            }
        }
    }
    Ok(result::Value::ApprovalList(pb::ApprovalList { items }))
}

/// `approval.grant`: the phone's signature over the Mac's exact action.
pub fn grant(svc: &Service, peer: &Peer, p: pb::ApprovalGrant) -> Result<result::Value> {
    let me = peer.client_id.as_deref().ok_or(DomainError::NotYours { what: "not_yours" })?;
    let now = crate::step_up_rpc::now_ms();
    let a = svc.sessions().approvals.find(&p.id).filter(|a| a.phase == Phase::Pending && a.live(now)).ok_or(DomainError::Conflict { what: "approval_expired" })?;
    // Another device of the same person, holding a trusted device key.
    let key = svc.store.step_up_key(me)?.filter(|k| k.trusted && k.alg == ALG_DEVICE).ok_or(DomainError::Conflict { what: "key_untrusted" })?;
    if me == a.device || person_of_device(&svc.store, Some(me))? != a.person {
        tracing::warn!(device = me, "refused: an approval from another person's device");
        return Err(DomainError::NotYours { what: "not_yours" });
    }
    let pins = step_up::host_pins(svc.ssh_host_dir());
    if pins.is_empty() {
        return Err(DomainError::Conflict { what: "step_up_unavailable" });
    }
    let signed = StepUp {
        action_bytes: a.action_bytes.clone().into(),
        alg: ALG_DEVICE.to_string(),
        nonce: a.nonce.to_vec().into(),
        host_pin: p.host_pin.clone(),
        signature: p.signature.clone(),
        ..Default::default()
    };
    // The statement names the Mac, not the phone: the phone approves for it.
    let context = Context { client_id: &a.device, host_pins: &pins, app_host: farcooler_fence::app_host() };
    step_up::verify(ALG_DEVICE, &key.public_key, &signed, &context).inspect_err(|_| tracing::warn!(device = me, "refused: an approval that didn't verify"))?;
    if !svc.sessions().approvals.advance(&a.id, Phase::Pending, Phase::Granted, now) {
        return Err(DomainError::Conflict { what: "approval_expired" });
    }
    tracing::info!(device = me, mac = %a.device, method = %a.method, "a phone approved an admin action");
    Ok(result::Value::ApprovalState(state_of(svc, &svc.sessions().approvals.find(&a.id).unwrap_or(a), now)))
}

/// `approval.deny`: the phone declines. Anyone of the person's devices may.
pub fn deny(svc: &Service, peer: &Peer, p: pb::ApprovalId) -> Result<result::Value> {
    let me = peer.client_id.as_deref().ok_or(DomainError::NotYours { what: "not_yours" })?;
    let now = crate::step_up_rpc::now_ms();
    let a = svc.sessions().approvals.find(&p.id).ok_or(DomainError::NotFound)?;
    if person_of_device(&svc.store, Some(me))? != a.person {
        return Err(DomainError::NotYours { what: "not_yours" });
    }
    svc.sessions().approvals.advance(&a.id, Phase::Pending, Phase::Denied, now);
    Ok(result::Value::ApprovalState(state_of(svc, &svc.sessions().approvals.find(&a.id).unwrap_or(a), now)))
}

/// `approval.status`: the asking Mac's own approval, to wait on.
pub fn status(svc: &Service, peer: &Peer, p: pb::ApprovalId) -> Result<result::Value> {
    let me = peer.client_id.as_deref().ok_or(DomainError::NotYours { what: "not_yours" })?;
    let a = svc.sessions().approvals.find(&p.id).filter(|a| a.device == me).ok_or(DomainError::NotFound)?;
    Ok(result::Value::ApprovalState(state_of(svc, &a, crate::step_up_rpc::now_ms())))
}

/// The five calls, as the dispatcher runs them.
pub async fn dispatch(svc: &Service, peer: &Peer, req: pb::Request) -> Result<result::Value> {
    use crate::step_up_rpc::payload;
    match req.method.as_str() {
        "approval.ask" => ask(svc, peer, payload(|p| if let request::Payload::ApprovalAsk(p) = p { Some(p) } else { None }, req)?).await,
        "approval.list" => list(svc, peer),
        "approval.grant" => grant(svc, peer, payload(|p| if let request::Payload::ApprovalGrant(p) = p { Some(p) } else { None }, req)?),
        "approval.deny" => deny(svc, peer, payload(|p| if let request::Payload::ApprovalId(p) = p { Some(p) } else { None }, req)?),
        _ => status(svc, peer, payload(|p| if let request::Payload::ApprovalId(p) = p { Some(p) } else { None }, req)?),
    }
}

#[cfg(test)]
#[path = "approvals_tests.rs"]
mod tests;
