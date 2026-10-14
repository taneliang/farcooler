//! `client.set_node_key`: a device already enrolled on a runner adds its own
//! tunnel key to its own line, so a fleet paired before the tunnel existed
//! gains it without pairing again (ov-485).
//!
//! The runner names the line from the connection, never from the request, and
//! serves it at `read` — see `enrollment::set_node_key` and the scope table in
//! `crates/daemon/src/rpc.rs`. A runner too old to have a tunnel advertises no
//! `tunnel` capability, and is refused here before anything is sent.

use farcooler_protocol::capability::TUNNEL;
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::{Value, json};

use super::results::wrong;
use super::{Session, SessionError, require};

impl Session {
    /// Register this device's tailcat node PUBLIC key on its own line.
    ///
    /// Answers `{"connBlob": <token>}`: the runner's tunnel token when it is
    /// serving one now, and empty when it is not yet — the line is written
    /// either way, and the runner's next start admits the key.
    pub async fn set_node_key(&self, node_key: &str) -> Result<Value, SessionError> {
        const METHOD: &str = "client.set_node_key";
        require(self.capabilities(), TUNNEL, METHOD)?;
        let payload = request::Payload::ClientSetNodeKey(pb::ClientSetNodeKey {
            // Ignored by the runner, which reads the connection's own id. Sent
            // empty rather than guessed, so a log never shows a claimed id.
            client_id: String::new(),
            node_key: node_key.to_string(),
        });
        match self.value(METHOD, None, Some(payload)).await? {
            result::Value::ClientSetNodeKey(r) => Ok(json!({ "connBlob": r.conn_blob })),
            other => Err(wrong("client_set_node_key", &other)),
        }
    }
}
