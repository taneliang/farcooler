//! The app's half of step-up (ov-856, slice 4): ask the runner for a nonce,
//! build the exact request an admin action will send, hand the app the
//! statement to sign with its device key, and send the request with the
//! signature. The key and the face are the app's; every byte that is signed
//! is spelled here, once, for the Mac, the iPhone and Android.
//!
//! - **Two calls, one action.** [`Session::step_up_begin`] fetches a nonce,
//!   builds the request and answers the statement. [`Session::step_up_finish`]
//!   sends that same request with the signature. The runner decodes the
//!   request it executes from the signed `action_bytes`, so what is built at
//!   `begin` is what runs.
//! - **One prompt is one action.** A nonce is spent by its signature, and a
//!   prepared action is taken by `finish`: a second `finish` on a token is
//!   refused, and at most [`KEPT`] actions wait at once.
//! - **The pin is the app's.** `host_pin` is the app's own pinned fingerprint
//!   of the runner's host key (never something this session heard from the
//!   runner), and `client_id` is this device's own: both go into the
//!   statement, which is why the runner can tell a relayed signature.

use farcooler_protocol::v1::{StepUp, request};
use prost::Message;
use sha2::{Digest, Sha256};

use super::*;

/// How many prepared actions wait for a signature; the oldest is dropped.
const KEPT: usize = 4;

/// The algorithm a Secure Enclave or Keystore key signs with.
pub const ALG: &str = "p256-sha256";

/// An admin action built and waiting for its signature.
pub(super) struct Prepared {
    token: String,
    request: farcooler_protocol::v1::Request,
    action_bytes: Vec<u8>,
    nonce: Vec<u8>,
    host_pin: String,
}

/// The request for `method` with `args`, or the refusal for a method that
/// takes no step-up from an app. The arguments are the ones the plain call
/// takes, camelCase, so an app changes only which function it calls.
pub(super) fn build(method: &str, args: &serde_json::Value) -> Result<farcooler_protocol::v1::Request, SessionError> {
    let text = |key: &str| args.get(key).and_then(|v| v.as_str()).map(str::trim).unwrap_or_default().to_string();
    let flag = |key: &str| args.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    let need = |key: &str| match text(key) {
        value if value.is_empty() => Err(SessionError::Protocol(format!("{method} needs a {key}"))),
        value => Ok(value),
    };
    let payload = match method {
        "client.revoke" => request::Payload::ClientRevoke(farcooler_protocol::v1::ClientRevoke {
            client_id: need("clientId")?,
            with_added: flag("withAdded"),
        }),
        "client.enroll" => enroll_payload(
            &need("publicKey")?,
            &text("label"),
            &need("clientId")?,
            &need("scope")?,
            flag("shellAccess"),
            &text("nodeKey"),
            flag("forTeammate"),
        )?,
        "client.approve_step_up" => {
            request::Payload::ClientApproveStepUp(farcooler_protocol::v1::ClientApproveStepUp { client_id: need("clientId")?, code: need("code")? })
        }
        "member.set_role" => {
            request::Payload::MemberSetRole(farcooler_protocol::v1::MemberSetRole { handle: need("handle")?, role: need("role")? })
        }
        other => return Err(SessionError::Protocol(format!("{other} takes no step-up from an app"))),
    };
    let mut built = farcooler_transport::request(method);
    built.required_capabilities = vec![farcooler_protocol::capability::STEP_UP.to_string()];
    built.payload = Some(payload);
    Ok(built)
}

/// The request as it is signed: `request_id` and `step_up` cleared.
pub(super) fn action_bytes(built: &farcooler_protocol::v1::Request) -> Vec<u8> {
    farcooler_protocol::v1::Request { request_id: Default::default(), step_up: None, ..built.clone() }.encode_to_vec()
}

/// What the key signs for `action_bytes`, as the runner rebuilds it.
pub(super) fn statement(host_pin: &str, client_id: &str, action_bytes: &[u8], nonce: &[u8]) -> Vec<u8> {
    let digest: [u8; 32] = Sha256::digest(action_bytes).into();
    farcooler_protocol::step_up_statement(host_pin, client_id, &digest, nonce)
}

impl Session {
    /// Prepare `method` for a signature: the statement to sign, and a token
    /// to send it back with. Refused without a round trip on a runner that
    /// doesn't advertise `step_up`.
    pub async fn step_up_begin(
        &self,
        method: &str,
        args: &serde_json::Value,
        client_id: &str,
        host_pin: &str,
    ) -> Result<(String, Vec<u8>), SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::STEP_UP, "step_up")?;
        if client_id.is_empty() || host_pin.is_empty() {
            return Err(SessionError::Protocol("step_up needs this device's clientId and the runner's hostPin".into()));
        }
        let built = build(method, args)?;
        let nonce = match self.value("auth.challenge", None, None).await? {
            result::Value::AuthChallenge(c) => c.nonce.to_vec(),
            other => return Err(wrong("auth_challenge", &other)),
        };
        let bytes = action_bytes(&built);
        let to_sign = statement(host_pin, client_id, &bytes, &nonce);
        let token = uuid::Uuid::now_v7().to_string();
        let mut kept = self.step_ups.lock().unwrap();
        while kept.len() >= KEPT {
            kept.remove(0);
        }
        kept.push(Prepared { token: token.clone(), request: built, action_bytes: bytes, nonce, host_pin: host_pin.to_string() });
        Ok((token, to_sign))
    }

    /// Send the action `token` prepared, with `signature` (the DER ECDSA
    /// signature of the statement `step_up_begin` answered). The token is
    /// spent whatever the runner says: a nonce is good for one assertion.
    pub async fn step_up_finish(&self, token: &str, signature: Vec<u8>) -> Result<result::Value, SessionError> {
        let prepared = {
            let mut kept = self.step_ups.lock().unwrap();
            let at = kept.iter().position(|p| p.token == token);
            at.map(|i| kept.remove(i))
        };
        let Some(Prepared { mut request, action_bytes, nonce, host_pin, .. }) = prepared else {
            return Err(SessionError::Protocol("that step-up was already used or has expired".into()));
        };
        let method = request.method.clone();
        request.step_up = Some(StepUp {
            action_bytes: action_bytes.into(),
            alg: ALG.to_string(),
            nonce: nonce.into(),
            host_pin,
            signature: signature.into(),
            authenticator_data: Default::default(),
            client_data_json: Default::default(),
        });
        let answer = self.client.send(request, crate::deadlines::for_method(&method))?;
        Pending { answer, method }.value().await
    }

    /// `client.approve_step_up` without a signature: what a device with no
    /// trusted key of its own sends, while the transition lasts. A runner that
    /// wants one answers `step_up_needed`, and the app then signs.
    pub async fn approve_step_up_plain(&self, args: &serde_json::Value) -> Result<result::Value, SessionError> {
        let built = build("client.approve_step_up", args)?;
        let payload = built.payload;
        self.value("client.approve_step_up", None, payload).await
    }

    /// Propose this device's own step-up key (`client.set_step_up_key`):
    /// `public_key` is the 65-byte uncompressed P-256 point. Answers the key's
    /// state (`proposed`) and the six digits the device shows and an approver
    /// checks. A proposal signs nothing until a person approves it.
    pub async fn step_up_propose(&self, public_key: Vec<u8>) -> Result<farcooler_protocol::v1::StepUpKeyState, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::STEP_UP, "client.set_step_up_key")?;
        let required = vec![farcooler_protocol::capability::STEP_UP.to_string()];
        let payload = request::Payload::ClientSetStepUpKey(farcooler_protocol::v1::ClientSetStepUpKey {
            client_id: String::new(),
            key: Some(farcooler_protocol::v1::StepUpKey { alg: ALG.to_string(), public_key: public_key.into() }),
        });
        match self.value_requiring("client.set_step_up_key", None, Some(payload), required).await? {
            result::Value::StepUpKeyState(s) => Ok(s),
            other => Err(wrong("step_up_key_state", &other)),
        }
    }
}

/// A signed action's answer as an app reads it, in the shape the plain call
/// answers: the device list, an enrollment, the people, or a key's state.
pub(crate) fn value_json(value: &result::Value) -> Result<serde_json::Value, SessionError> {
    Ok(match value {
        result::Value::ClientList(l) => enrolled_json(&l.items),
        result::Value::ClientEnroll(r) => enroll_json(r),
        result::Value::MemberList(l) => crate::members_json::members_json(l),
        result::Value::StepUpKeyState(s) => key_state_json(s),
        other => return Err(wrong("step_up_answer", other)),
    })
}

/// A key's state: `{clientId, state, code}`.
pub(crate) fn key_state_json(s: &farcooler_protocol::v1::StepUpKeyState) -> serde_json::Value {
    json!({ "clientId": s.client_id, "state": s.state, "code": s.code })
}
