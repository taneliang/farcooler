//! Step-up, as an app passes it (ov-856, slice 4): `step_up.begin`,
//! `step_up.finish` and `step_up.propose`. The app owns the key and the face;
//! these carry the bytes. Binary values travel as standard base64.

use farcooler_core::base64;
use serde_json::{Value, json};

use crate::session::{Session, SessionError, step_up};

fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default()
}

fn bytes(method: &str, args: &Value, key: &str) -> Result<Vec<u8>, SessionError> {
    base64::decode(text(args, key)).filter(|b| !b.is_empty()).ok_or_else(|| SessionError::Protocol(format!("{method} needs a {key} in base64")))
}

/// An admin action signed with this device's step-up key: `begin` builds it and
/// answers the statement to sign, `finish` sends it with the signature, and
/// `propose` offers this device's key for approval.
pub(super) async fn call(session: &Session, method: &str, args: &Value) -> Result<Value, SessionError> {
    match method {
        "step_up.begin" => begin(session, args).await,
        "step_up.finish" => finish(session, args).await,
        "client.approve_step_up" => step_up::value_json(&session.approve_step_up_plain(args).await?),
        _ => propose(session, args).await,
    }
}

/// `{method, args, clientId, hostPin}`: prepare an admin action. Answers
/// `{token, statement}`, the statement in base64 for the device key to sign.
/// `args` are the ones the plain call takes.
pub(super) async fn begin(session: &Session, args: &Value) -> Result<Value, SessionError> {
    let method = text(args, "method");
    let inner = args.get("args").cloned().unwrap_or(Value::Null);
    let (token, statement) = session.step_up_begin(method, &inner, text(args, "clientId"), text(args, "hostPin")).await?;
    Ok(json!({ "token": token, "statement": base64::encode(&statement) }))
}

/// `{token, signature}`: send the action `token` prepared, signed (DER, base64).
/// Answers what the plain call answers.
pub(super) async fn finish(session: &Session, args: &Value) -> Result<Value, SessionError> {
    const METHOD: &str = "step_up.finish";
    let signature = bytes(METHOD, args, "signature")?;
    step_up::value_json(&session.step_up_finish(text(args, "token"), signature).await?)
}

/// `{publicKey}`: propose this device's key (the 65-byte point, base64).
/// Answers `{clientId, state, code}`.
pub(super) async fn propose(session: &Session, args: &Value) -> Result<Value, SessionError> {
    let key = bytes("step_up.propose", args, "publicKey")?;
    Ok(step_up::key_state_json(&session.step_up_propose(key).await?))
}
