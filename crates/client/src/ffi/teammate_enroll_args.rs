//! Add Teammate from a phone, as an app passes it (ov-854): `teammate.enroll`.
//!
//! Not a wire method: one name for the whole of one runner's half (read who is
//! there, write the key `for_teammate`, bind it, take it off if that fails), so
//! the iPhone and Android hold the Mac's order of operations once, here.

use serde_json::Value;

use crate::session::teammate::{TeammateDevice, TeammateMember};
use crate::session::{Session, SessionError};

/// `{publicKey, label, clientId, scope, nodeKey, handle, name, account, email}`:
/// enroll that device for that teammate. `scope` is `read` or `control` and
/// nothing else: a teammate is never given a shell or a wider grant (ruling
/// R-65), so a request for one never leaves the phone. `nodeKey` may be empty.
///
/// Answers `{outcome, ...}` (see `TeammateOutcome::json`): a refusal on this
/// runner is an outcome, not an error, because the app asks each runner in turn
/// and says what became of each.
pub(super) async fn enroll(session: &Session, args: &Value) -> Result<Value, SessionError> {
    const METHOD: &str = "teammate.enroll";
    let field = |name: &str| args.get(name).and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let required = |name: &str| match field(name) {
        "" => Err(SessionError::Protocol(format!("{METHOD} needs a {name}"))),
        value => Ok(value),
    };
    let scope = required("scope")?;
    if !matches!(scope, "read" | "control") {
        return Err(SessionError::Protocol("a teammate is enrolled at read or control".into()));
    }
    let device = TeammateDevice {
        public_key: required("publicKey")?,
        label: field("label"),
        client_id: required("clientId")?,
        scope,
        node_key: field("nodeKey"),
    };
    let member = TeammateMember { handle: required("handle")?, name: required("name")?, account: required("account")?, email: field("email") };
    Ok(session.enroll_teammate(&device, &member).await.json())
}
