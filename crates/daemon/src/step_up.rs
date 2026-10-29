//! Step-up on the runner (ov-856, slice 3 of `reports/ov-856/design.md`
//! sections 3 and 6, with the security review's B2 and B3 and S1, S6 and S9).
//!
//! An admin action from a remote device carries a signature, made with the
//! person's own user verification, over exactly that action. This module is
//! the runner's half: the challenges, the verifiers and the admission of one
//! request. The app's half is a later slice.
//!
//! - **Step-up is not scope.** [`admit`] hands the dispatcher a [`Stepped`]
//!   next to the request; the connection's [`Peer`] keeps the scope its line
//!   has, so every `== HostAdmin` test still means "this line is
//!   `host_admin`" (B2). [`may_admin`] is the one predicate that lets a
//!   `control` line past an admin method's gate, and it names the method.
//! - **Step-up never writes a shell line.** `client.enroll` refuses
//!   `shell_access` and `host_admin` from any remote device, stepped up or not
//!   (`person_access::enroll`).
//! - **A request is signed as the runner will run it** (S1). `action_bytes` is
//!   the request with `request_id` and `step_up` cleared; the runner decodes
//!   the request it executes from those bytes, and refuses it unless they
//!   re-encode to themselves (an unknown field is dropped by decoding, so it
//!   would not) and the outer request, so cleared, is the same message.
//! - **A challenge is one assertion** (S9): 32 random bytes, tied to the
//!   session that asked, good for [`CHALLENGE_MS`], at most [`LIVE_CHALLENGES`]
//!   live at once. It is spent before the signature is checked, and stays
//!   spent when the action then fails.
//! - **Two verifiers.** `p256-sha256`: an ECDSA P-256 signature (DER) over the
//!   statement, from a Secure Enclave or Keystore key. `webauthn-es256`: a
//!   passkey's assertion, whose client data must name this app's origin, a
//!   challenge that is the statement's hash, and whose authenticator data must
//!   carry this app's RP ID hash and the user-presence and user-verification
//!   flags.
//! - **Only a trusted key signs** (B3), and a browser's key (WebAuthn) only
//!   for the defensive actions in [`DEFENSIVE`] (owner's decision 2).
//! - **A key must be asked for.** A device that holds a trusted key and is the
//!   owner's or an Admin's must step up for the methods in [`METHODS`]
//!   ([`needs_step_up`]); one with no trusted key keeps what it had, and
//!   gains nothing new.

use std::collections::VecDeque;
use std::sync::Mutex;

use base64ct::{Base64UrlUnpadded, Encoding};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{Request, StepUp};
use farcooler_store::Store;
use farcooler_store::access::Access;
use farcooler_transport::Peer;
use p256::ecdsa::signature::hazmat::PrehashVerifier;
use p256::ecdsa::{Signature, VerifyingKey};
use prost::Message;
use sha2::{Digest, Sha256};

use crate::people::{Person, person_of_device};
use crate::service::Service;
use crate::sessions::Session;

/// A device key (Secure Enclave, Keystore): ECDSA P-256 over SHA-256 of the
/// statement, DER.
pub const ALG_DEVICE: &str = "p256-sha256";
/// A browser's passkey (WebAuthn, ES256).
pub const ALG_WEBAUTHN: &str = "webauthn-es256";

/// The methods that take a step-up from a remote device. `step_up` on any
/// other is refused (S9).
pub const METHODS: [&str; 6] = ["client.revoke", "client.enroll", "member.set", "member.remove", "member.set_role", "client.approve_step_up"];

/// What a browser's key may do (decision 2): take access away. Never add,
/// bind, raise, propose or approve a key.
pub const DEFENSIVE: [&str; 2] = ["client.revoke", "member.remove"];

/// How long a challenge lives.
pub const CHALLENGE_MS: i64 = 60_000;

/// How many challenges one session may hold; the oldest goes first.
pub const LIVE_CHALLENGES: usize = 4;

/// One session's live challenges.
#[derive(Default)]
pub struct Challenges(Mutex<VecDeque<([u8; 32], i64)>>);

impl Challenges {
    /// A fresh challenge, issued at `now_ms`.
    pub fn issue(&self, now_ms: i64) -> Result<[u8; 32]> {
        self.issue_with(fresh_nonce()?, now_ms)
    }

    pub(crate) fn issue_with(&self, nonce: [u8; 32], now_ms: i64) -> Result<[u8; 32]> {
        let mut live = self.0.lock().unwrap();
        live.retain(|(_, at)| now_ms - at <= CHALLENGE_MS);
        while live.len() >= LIVE_CHALLENGES {
            live.pop_front();
        }
        live.push_back((nonce, now_ms));
        Ok(nonce)
    }

    /// Spend `nonce`: true once, and only within [`CHALLENGE_MS`] of its issue.
    pub fn take(&self, nonce: &[u8], now_ms: i64) -> bool {
        let mut live = self.0.lock().unwrap();
        let Some(at) = live.iter().position(|(n, _)| n.as_slice() == nonce) else { return false };
        let (_, issued) = live.remove(at).expect("position is in range");
        now_ms - issued <= CHALLENGE_MS
    }
}

fn fresh_nonce() -> Result<[u8; 32]> {
    use std::io::Read as _;
    let mut nonce = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut nonce)).map_err(|_| DomainError::OperationFailed)?;
    Ok(nonce)
}

/// A request a person approved with a gesture: the method, the digest of the
/// bytes signed, and who signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stepped {
    pub method: String,
    pub digest: [u8; 32],
    pub device: String,
    /// The key was a browser's passkey.
    pub browser: bool,
}

/// Whether the person behind `peer`'s device, stepped up for `method`, may
/// take an admin action on a `control` line (B2): the line is at least
/// `control`, the person is the owner or an Admin, and the step-up is for
/// exactly this method. Who the action touches is the handler's
/// (`person_access::may_manage`).
pub fn may_admin(store: &Store, peer: &Peer, stepped: Option<&Stepped>, method: &str) -> Result<bool> {
    let Some(stepped) = stepped.filter(|s| s.method == method) else { return Ok(false) };
    if peer.client_id.as_deref() != Some(stepped.device.as_str()) || crate::access_migration::rank(peer.scope) < crate::access_migration::rank(farcooler_protocol::v1::Scope::Control) {
        return Ok(false);
    }
    // Approving a key is every trusted device's own business, an admin or not.
    Ok(method == "client.approve_step_up" || is_admin(store, &stepped.device)?)
}

fn is_admin(store: &Store, device: &str) -> Result<bool> {
    Ok(match person_of_device(store, Some(device))? {
        Person::Owner => true,
        Person::Member(handle) => store.member_access(&handle)? == Some(Access::Admin),
        Person::Released => false,
    })
}

/// Whether `device` must step up for `method`: it is a remote owner's or
/// Admin's with a trusted key. A device with none keeps what it had.
pub fn needs_step_up(store: &Store, device: &str, method: &str) -> Result<bool> {
    if !METHODS.contains(&method) {
        return Ok(false);
    }
    Ok(store.step_up_key(device)?.is_some_and(|k| k.trusted) && (method == "client.approve_step_up" || is_admin(store, device)?))
}

/// The six digits a proposing device shows and an approver checks: derived
/// from the key, so an approval names the key it trusts and not a label.
pub fn code_of(alg: &str, public_key: &[u8]) -> String {
    let digest = Sha256::new().chain_update(b"farcooler-step-up-code-v1").chain_update(alg.as_bytes()).chain_update([0]).chain_update(public_key).finalize();
    format!("{:06}", u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000)
}

/// A key as the runner will hold it: a known algorithm and a P-256 point.
pub fn check_key(alg: &str, public_key: &[u8]) -> Result<()> {
    if alg != ALG_DEVICE && alg != ALG_WEBAUTHN {
        return Err(DomainError::InvalidArgument { what: "step_up_key" });
    }
    verifying_key(public_key).map(|_| ()).map_err(|_| DomainError::InvalidArgument { what: "step_up_key" })
}

fn verifying_key(public_key: &[u8]) -> std::result::Result<VerifyingKey, ()> {
    if public_key.len() != 65 || public_key[0] != 4 {
        return Err(());
    }
    VerifyingKey::from_sec1_bytes(public_key).map_err(|_| ())
}

fn failed() -> DomainError {
    DomainError::Conflict { what: "step_up_failed" }
}

/// What the runner needs to check one assertion.
pub struct Context<'a> {
    pub client_id: &'a str,
    /// The runner's own sshd host-key fingerprints; the statement must name one.
    pub host_pins: &'a [String],
    /// The app host a passkey signs for (`fence::passkey::app_host`).
    pub app_host: &'a str,
}

/// The signature of `step_up` against `key`, over what `step_up` and `context`
/// say it is for. A WebAuthn assertion answers its counter, for the caller to
/// record. Nothing here reads the store or the clock.
pub fn verify(alg: &str, public_key: &[u8], step_up: &StepUp, context: &Context) -> Result<Option<u32>> {
    if step_up.alg != alg {
        return Err(failed());
    }
    if !context.host_pins.iter().any(|pin| *pin == step_up.host_pin) {
        return Err(DomainError::Conflict { what: "host_pin_mismatch" });
    }
    let key = verifying_key(public_key).map_err(|_| failed())?;
    let digest: [u8; 32] = Sha256::digest(&step_up.action_bytes).into();
    let statement = farcooler_protocol::step_up_statement(&step_up.host_pin, context.client_id, &digest, &step_up.nonce);
    let signature = Signature::from_der(&step_up.signature).map_err(|_| failed())?;
    match alg {
        ALG_DEVICE => {
            key.verify_prehash(&Sha256::digest(&statement), &signature).map_err(|_| failed())?;
            Ok(None)
        }
        ALG_WEBAUTHN => {
            let count = webauthn_checks(step_up, &statement, context.app_host)?;
            let mut signed = step_up.authenticator_data.to_vec();
            signed.extend_from_slice(&Sha256::digest(&step_up.client_data_json));
            key.verify_prehash(&Sha256::digest(&signed), &signature).map_err(|_| failed())?;
            Ok(Some(count))
        }
        _ => Err(failed()),
    }
}

/// What a passkey's assertion must say before its signature means anything
/// (S6): answers the counter.
fn webauthn_checks(step_up: &StepUp, statement: &[u8], app_host: &str) -> Result<u32> {
    let data: serde_json::Value = serde_json::from_slice(&step_up.client_data_json).map_err(|_| failed())?;
    let field = |name: &str| data.get(name).and_then(|v| v.as_str());
    let challenge = Base64UrlUnpadded::encode_string(&Sha256::digest(statement));
    let from_here = field("type") == Some("webauthn.get")
        && field("origin") == Some(format!("https://{app_host}").as_str())
        && field("challenge") == Some(challenge.as_str())
        && data.get("topOrigin").is_none()
        && data.get("crossOrigin").is_none_or(|v| v == &serde_json::Value::Bool(false));
    let auth = &step_up.authenticator_data;
    // rpIdHash (32), flags (1), signCount (4); extensions may follow.
    if !from_here || auth.len() < 37 || auth[..32] != Sha256::digest(app_host.as_bytes())[..] {
        return Err(failed());
    }
    const USER_PRESENT: u8 = 0x01;
    const USER_VERIFIED: u8 = 0x04;
    if auth[32] & USER_PRESENT == 0 || auth[32] & USER_VERIFIED == 0 {
        return Err(failed());
    }
    Ok(u32::from_be_bytes([auth[33], auth[34], auth[35], auth[36]]))
}

/// `request` as it is signed: the request with its `request_id` and `step_up`
/// cleared, serialized.
pub fn action_bytes(request: &Request) -> Vec<u8> {
    Request { request_id: Default::default(), step_up: None, ..request.clone() }.encode_to_vec()
}

/// The request to execute, decoded from the signed bytes (S1). Refused unless
/// the bytes are the way this runner would write that request, which an
/// unknown field or another spelling is not, and unless the outer request,
/// cleared the same way, is that same message.
fn signed_request(outer: &Request, bytes: &[u8]) -> Result<Request> {
    let inner = Request::decode(bytes).map_err(|_| failed())?;
    if inner.encode_to_vec() != bytes || !inner.request_id.is_empty() || inner.step_up.is_some() {
        tracing::warn!(method = %outer.method, "refused a step-up whose bytes aren't a plain request");
        return Err(failed());
    }
    let cleared = Request { request_id: Default::default(), step_up: None, ..outer.clone() };
    if cleared != inner {
        tracing::warn!(method = %outer.method, "refused a step-up that signed another request");
        return Err(failed());
    }
    Ok(Request { request_id: outer.request_id.clone(), ..inner })
}

/// One request's admission. Without a `step_up`, `req` is as it came, unless
/// its device must step up for it (`step_up_needed`). With one: the method must
/// take it, the device must hold a trusted key and be an owner's or an Admin
/// (approving a key excepted), the challenge is spent, the signature is
/// checked, and the request returned is the one the signature covers.
pub async fn admit(svc: &Service, peer: &Peer, session: &Session, req: Request, now_ms: i64) -> Result<(Request, Option<Stepped>)> {
    let Some(step_up) = req.step_up.as_ref() else {
        if let Some(device) = peer.client_id.as_deref()
            && needs_step_up(&svc.store, device, &req.method)?
        {
            tracing::info!(method = %req.method, "refused: this device must step up for that");
            return Err(DomainError::Conflict { what: "step_up_needed" });
        }
        return Ok((req, None));
    };
    if !METHODS.contains(&req.method.as_str()) {
        tracing::info!(method = %req.method, "refused: a step-up on a method that takes none");
        return Err(DomainError::Conflict { what: "step_up_unneeded" });
    }
    // The runner's own socket needs none, and a device that is named in a
    // preamble is that device (S4).
    let Some(device) = peer.client_id.as_deref() else {
        return Err(DomainError::Conflict { what: "step_up_unneeded" });
    };
    let key = svc.store.step_up_key(device)?.filter(|k| k.trusted).ok_or(DomainError::Conflict { what: "key_untrusted" })?;
    // The nonce is spent before the signature is looked at, and stays spent.
    if !session.challenges().take(&step_up.nonce, now_ms) {
        tracing::info!(method = %req.method, "refused: a step-up with a challenge that is spent, expired or not this session's");
        return Err(DomainError::Conflict { what: "challenge_expired" });
    }
    let executed = signed_request(&req, &step_up.action_bytes)?;
    let pins = svc.host_pins();
    if pins.is_empty() {
        return Err(DomainError::Conflict { what: "step_up_unavailable" });
    }
    let context = Context { client_id: device, host_pins: &pins, app_host: farcooler_fence::app_host() };
    let counter = verify(&key.alg, &key.public_key, step_up, &context).inspect_err(|_| tracing::warn!(device, method = %req.method, "refused: a step-up that didn't verify"))?;
    let browser = key.alg == ALG_WEBAUTHN;
    if browser && !DEFENSIVE.contains(&req.method.as_str()) {
        tracing::warn!(device, method = %req.method, "refused: a browser's passkey for an action that isn't defensive");
        return Err(DomainError::Conflict { what: "defensive_only" });
    }
    if let Some(count) = counter {
        svc.store.use_sign_count(device, count)?;
    }
    let digest: [u8; 32] = Sha256::digest(&step_up.action_bytes).into();
    let stepped = Stepped { method: req.method.clone(), digest, device: device.to_string(), browser };
    if !may_admin(&svc.store, peer, Some(&stepped), &req.method)? {
        return Err(DomainError::NotYours { what: "not_admin" });
    }
    Ok((executed, Some(stepped)))
}

#[cfg(test)]
#[path = "step_up_tests.rs"]
mod tests;
