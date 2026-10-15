//! `client.rekey` (ov-569): this device moves its own line on a runner to the
//! new key a phone's "Make a New Key" made.
//!
//! The runner rewrites the line the connection authenticated as, never one the
//! request names, and serves it at `read`; see `enrollment::rekey` in
//! `crates/daemon`. A runner too old to have it advertises no `rekey`
//! capability, and is refused here before anything is sent.

use farcooler_protocol::capability::REKEY;
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::{Value, json};

use super::results::wrong;
use super::{Session, SessionError, enrolled_client_json, require};

impl Session {
    /// Move this device's line to `public_key`, with the client id and proof
    /// `crate::device_keys::rekey_proof` made. Answers `{"client": …}`, the
    /// line as the runner now reads it.
    pub async fn rekey_client(&self, public_key: &str, client_id: &str, proof: &str) -> Result<Value, SessionError> {
        const METHOD: &str = "client.rekey";
        require(self.capabilities(), REKEY, METHOD)?;
        let payload = request::Payload::ClientRekey(pb::ClientRekey {
            public_key: public_key.to_string(),
            client_id: client_id.to_string(),
            proof: proof.to_string(),
        });
        match self.value(METHOD, None, Some(payload)).await? {
            result::Value::ClientRekey(r) => Ok(json!({ "client": r.client.as_ref().map(enrolled_client_json) })),
            other => Err(wrong("client_rekey", &other)),
        }
    }
}
