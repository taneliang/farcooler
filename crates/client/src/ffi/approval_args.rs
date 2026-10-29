//! Phone approval, as an app passes it (ov-858): `approval.ask`,
//! `approval.status` and `approval.finish` on the Mac that asks, and
//! `approval.list`, `approval.grant` and `approval.deny` on the iPhone that
//! answers. The app owns the key and the face; these carry the bytes, as
//! `step_up_args` does. Binary values travel as standard base64.

use farcooler_core::base64;
use serde_json::{Value, json};

use crate::session::{Session, SessionError, step_up};

fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default()
}

/// `{id, state, expiresAt}`: an approval as the Mac waits on it.
fn waiting(state: &farcooler_protocol::v1::ApprovalState) -> Value {
    json!({ "id": state.id, "state": state.state, "expiresAt": state.expires_at })
}

pub(super) async fn call(session: &Session, method: &str, args: &Value) -> Result<Value, SessionError> {
    match method {
        // `{method, args}`: ask the person's iPhone to approve that action.
        "approval.ask" => Ok(waiting(&session.approval_ask(text(args, "method"), args.get("args").unwrap_or(&Value::Null)).await?)),
        "approval.status" => Ok(waiting(&session.approval_status(text(args, "id")).await?)),
        // Send the approved action; answers what the plain call answers.
        "approval.finish" => step_up::value_json(&session.approval_finish(text(args, "id")).await?),
        // `{hostPin}`: the pending approvals, each with the statement to sign.
        "approval.list" => Ok(json!({ "items": session.approval_list(text(args, "hostPin")).await? })),
        // `{id, hostPin, signature}`: the phone's DER signature, base64.
        "approval.grant" => {
            let signature = base64::decode(text(args, "signature")).filter(|b| !b.is_empty());
            let signature = signature.ok_or_else(|| SessionError::Protocol("approval.grant needs a signature in base64".into()))?;
            Ok(waiting(&session.approval_grant(text(args, "id"), text(args, "hostPin"), signature).await?))
        }
        _ => Ok(waiting(&session.approval_deny(text(args, "id")).await?)),
    }
}
