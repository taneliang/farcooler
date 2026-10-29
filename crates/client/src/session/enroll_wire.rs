//! The `client.enroll` request and its answer, as an app and a step-up both spell them (ov-856).

use super::*;

/// The `client.enroll` request an app asks for: also what a step-up signs
/// (`session::step_up`), so the plain call and the signed one are one spelling.
///
/// `scope` is a word, and an unknown one is refused here rather than sent:
/// a key with no scope at all already means host_admin to sshd, so rounding a
/// misspelling up would turn a typo into the whole runner. The pairing rule
/// (`shell_access` demands `host_admin`) is NOT re-checked: the daemon owns
/// every rule about what may be written into `authorized_keys`.
#[allow(clippy::too_many_arguments)]
pub(super) fn enroll_payload(
    public_key: &str,
    label: &str,
    client_id: &str,
    scope: &str,
    shell_access: bool,
    node_key: &str,
    for_teammate: bool,
    teammate: Option<farcooler_protocol::v1::TeammateBind>,
) -> Result<request::Payload, SessionError> {
    let Some(scope) = scope_from_word(scope) else {
        return Err(SessionError::Protocol("a device is enrolled at read, control or host_admin".into()));
    };
    Ok(request::Payload::ClientEnroll(farcooler_protocol::v1::ClientEnroll {
        public_key: public_key.to_string(),
        label: label.to_string(),
        client_id: client_id.to_string(),
        scope: scope as i32,
        // False is the restricted line and the proto's default; true is the
        // plain line, which the daemon refuses unless the scope beside it is
        // host_admin.
        shell_access,
        // Empty means "this device asked for no tunnel", which is NOT an
        // unusable key: the daemon reads absence and presence differently.
        node_key: node_key.to_string(),
        for_teammate,
        step_up_key: None,
        teammate,
    }))
}

/// `client.enroll`'s answer as an app reads it. `alreadyEnrolled` is its own
/// field rather than an error: it is the ordinary outcome of enrolling a Mac on
/// itself. `connBlob` is this runner's tunnel token, empty for an enrollment
/// that carried no node key.
pub(super) fn enroll_json(r: &farcooler_protocol::v1::ClientEnrollResult) -> serde_json::Value {
    json!({
        "client": r.client.as_ref().map(enrolled_client_json),
        "alreadyEnrolled": r.already_enrolled,
        "connBlob": r.conn_blob,
    })
}
