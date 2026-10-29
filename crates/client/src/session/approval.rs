//! Phone approval, the app's half (ov-858): a Mac that isn't the runner and
//! holds no step-up key asks the person's iPhone to approve one admin action,
//! and the iPhone shows it and signs.
//!
//! - **The Mac** builds the request exactly as a step-up does, sends its bytes
//!   with `approval.ask`, waits on `approval.status`, and sends the same
//!   request with `StepUp { alg: "phone-approval", approval_id }`
//!   ([`Session::approval_finish`]). The runner spends the approval for that
//!   device and those bytes, once.
//! - **The iPhone** lists what its person is being asked. What it shows is
//!   decoded here from the very bytes it signs ([`describe`]), never from the
//!   runner's summary, and the statement it signs names the asking Mac, the
//!   runner's challenge and the runner host key it pinned itself.

use farcooler_protocol::v1::{ApprovalAsk, ApprovalGrant, ApprovalId, ApprovalState, Request, StepUp, request};
use prost::Message;

use super::step_up::{KEPT, Prepared, build, action_bytes, statement};
use super::*;

/// `StepUp.alg` for a request an iPhone approved.
pub const ALG_APPROVAL: &str = "phone-approval";

/// What an approval's bytes say they do, as the phone shows it: the wire
/// method and the ids and words in the request, decoded from `bytes`.
/// `None` for bytes that aren't a plain admin request.
pub fn describe(bytes: &[u8]) -> Option<serde_json::Value> {
    let asked = Request::decode(bytes).ok()?;
    if asked.encode_to_vec() != bytes {
        return None;
    }
    let detail = match asked.payload? {
        request::Payload::ClientRevoke(p) => serde_json::json!({ "clientId": p.client_id, "withAdded": p.with_added }),
        request::Payload::ClientSetCap(p) => serde_json::json!({ "clientId": p.client_id, "cap": p.cap }),
        request::Payload::ClientApproveStepUp(p) => serde_json::json!({ "clientId": p.client_id }),
        request::Payload::ClientEnroll(p) => serde_json::json!({
            "label": p.label,
            "access": p.teammate.as_ref().map(|t| t.access.clone()).unwrap_or_default(),
        }),
        request::Payload::MemberSetAccess(p) => serde_json::json!({ "handle": p.handle, "access": p.access }),
        request::Payload::MemberSetRole(p) => serde_json::json!({ "handle": p.handle, "role": p.role }),
        request::Payload::MemberRemove(p) => serde_json::json!({ "handle": p.handle }),
        request::Payload::MemberSet(p) => serde_json::json!({ "handle": p.handle }),
        _ => return None,
    };
    Some(serde_json::json!({ "method": asked.method, "detail": detail }))
}

/// One approval as the phone is shown it, with the statement to sign.
pub fn item_json(state: &ApprovalState, host_pin: &str) -> Option<serde_json::Value> {
    let action = describe(&state.action_bytes)?;
    let to_sign = statement(host_pin, &state.device_id, &state.action_bytes, &state.nonce);
    Some(serde_json::json!({
        "id": state.id,
        "state": state.state,
        "action": action,
        "deviceId": state.device_id,
        "deviceName": state.device_name,
        "targetName": state.target_name,
        "expiresAt": state.expires_at,
        "statement": farcooler_core::base64::encode(&to_sign),
    }))
}

impl Session {
    /// Ask the person's iPhone to approve `method` with `args` (the ones the
    /// plain call takes). The action is kept for [`Session::approval_finish`].
    pub async fn approval_ask(&self, method: &str, args: &serde_json::Value) -> Result<ApprovalState, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::PHONE_APPROVAL, "approval.ask")?;
        let built = build(method, args)?;
        let bytes = action_bytes(&built);
        let required = vec![farcooler_protocol::capability::PHONE_APPROVAL.to_string()];
        let payload = request::Payload::ApprovalAsk(ApprovalAsk { action_bytes: bytes.clone().into() });
        let state = match self.value_requiring("approval.ask", None, Some(payload), required).await? {
            result::Value::ApprovalState(s) => s,
            other => return Err(wrong("approval_state", &other)),
        };
        let mut kept = self.step_ups.lock().unwrap();
        while kept.len() >= KEPT {
            kept.remove(0);
        }
        kept.push(Prepared { token: state.id.clone(), request: built, action_bytes: bytes, nonce: Vec::new(), host_pin: String::new() });
        Ok(state)
    }

    /// Where an approval stands: `pending`, `granted`, `denied` or `expired`.
    pub async fn approval_status(&self, id: &str) -> Result<ApprovalState, SessionError> {
        let payload = request::Payload::ApprovalId(ApprovalId { id: id.to_string() });
        let required = vec![farcooler_protocol::capability::PHONE_APPROVAL.to_string()];
        match self.value_requiring("approval.status", None, Some(payload), required).await? {
            result::Value::ApprovalState(s) => Ok(s),
            other => Err(wrong("approval_state", &other)),
        }
    }

    /// Send the action `id` asked for, carrying the iPhone's approval. The
    /// action is taken whatever the runner says: an approval is spent once.
    pub async fn approval_finish(&self, id: &str) -> Result<result::Value, SessionError> {
        let prepared = {
            let mut kept = self.step_ups.lock().unwrap();
            let at = kept.iter().position(|p| p.token == id);
            at.map(|i| kept.remove(i))
        };
        let Some(Prepared { mut request, action_bytes, .. }) = prepared else {
            return Err(SessionError::Protocol("that approval was already used or has expired".into()));
        };
        let method = request.method.clone();
        request.step_up = Some(StepUp {
            action_bytes: action_bytes.into(),
            alg: ALG_APPROVAL.to_string(),
            approval_id: id.to_string(),
            ..Default::default()
        });
        let answer = self.client.send(request, crate::deadlines::for_method(&method))?;
        Pending { answer, method }.value().await
    }

    /// What this person's phone is being asked to approve, each with the
    /// statement to sign under `host_pin`. An entry whose bytes aren't a
    /// plain admin request is left out rather than shown.
    pub async fn approval_list(&self, host_pin: &str) -> Result<Vec<serde_json::Value>, SessionError> {
        let required = vec![farcooler_protocol::capability::PHONE_APPROVAL.to_string()];
        match self.value_requiring("approval.list", None, None, required).await? {
            result::Value::ApprovalList(l) => Ok(l.items.iter().filter_map(|s| item_json(s, host_pin)).collect()),
            other => Err(wrong("approval_list", &other)),
        }
    }

    /// Approve `id` with the phone's signature (DER) of the statement
    /// [`item_json`] gave, made with the phone's pin of this runner.
    pub async fn approval_grant(&self, id: &str, host_pin: &str, signature: Vec<u8>) -> Result<ApprovalState, SessionError> {
        let payload = request::Payload::ApprovalGrant(ApprovalGrant { id: id.to_string(), host_pin: host_pin.to_string(), signature: signature.into() });
        let required = vec![farcooler_protocol::capability::PHONE_APPROVAL.to_string()];
        match self.value_requiring("approval.grant", None, Some(payload), required).await? {
            result::Value::ApprovalState(s) => Ok(s),
            other => Err(wrong("approval_state", &other)),
        }
    }

    /// Decline `id`.
    pub async fn approval_deny(&self, id: &str) -> Result<ApprovalState, SessionError> {
        let payload = request::Payload::ApprovalId(ApprovalId { id: id.to_string() });
        let required = vec![farcooler_protocol::capability::PHONE_APPROVAL.to_string()];
        match self.value_requiring("approval.deny", None, Some(payload), required).await? {
            result::Value::ApprovalState(s) => Ok(s),
            other => Err(wrong("approval_state", &other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn revoke() -> Vec<u8> {
        action_bytes(&build("client.revoke", &serde_json::json!({ "clientId": "pad-b", "withAdded": true })).unwrap())
    }

    /// What the phone shows is read from the bytes it signs, and the statement
    /// it signs is the runner's, for the asking Mac.
    #[test]
    fn the_phone_shows_and_signs_the_bytes_it_was_given() {
        let state = ApprovalState { id: "a1".into(), action_bytes: revoke().into(), nonce: vec![9; 32].into(), device_id: "mac-d".into(), ..Default::default() };
        let item = item_json(&state, "SHA256:pin").expect("a plain admin request");
        assert_eq!(item["action"], serde_json::json!({ "method": "client.revoke", "detail": { "clientId": "pad-b", "withAdded": true } }));
        let digest: [u8; 32] = Sha256::digest(revoke()).into();
        let expected = farcooler_protocol::step_up_statement("SHA256:pin", "mac-d", &digest, &[9; 32]);
        assert_eq!(item["statement"], farcooler_core::base64::encode(&expected));
    }

    /// Bytes that aren't a plain admin request, or aren't the way a runner
    /// writes one, are never shown.
    #[test]
    fn bytes_that_are_not_a_plain_admin_request_are_not_shown() {
        let mut padded = revoke();
        padded.extend_from_slice(&[0xf8, 0x01, 0x01]);
        assert!(describe(&padded).is_none(), "an unknown field");
        assert!(describe(&action_bytes(&farcooler_transport::request("daemon.shutdown"))).is_none(), "no payload");
        assert!(describe(&[0xff, 0xff]).is_none(), "not a request");
    }
}
