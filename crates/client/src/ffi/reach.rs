//! Which runner a connect config names, and the session to it.
//!
//! An app on the runner's own machine names the daemon's socket (ov-372):
//! `{"socket": "<runtime dir>/farcoolerd.sock"}`. The Mac's native agent view
//! reads its local runner that way, one connection for every pane, where the
//! rest of the Mac runs the CLI. Anything else is an ssh destination
//! (`parse_destination`).

use serde_json::{Value, json};

use super::{BAD_CONFIG, connect_failure, parse_destination};
use crate::session::Session;

/// The session `config` names, or the connect line's failure.
pub(super) async fn open(config: &str) -> Result<Session, Value> {
    if let Some(socket) = socket_of(config) {
        return Session::connect_local(std::path::Path::new(&socket)).await.map_err(|e| connect_failure(&e));
    }
    match parse_destination(config) {
        Ok(destination) => Session::connect_ssh(&destination).await.map_err(|e| connect_failure(&e)),
        // A config this boundary could not read never left the device, so
        // it is no failure of the runner's; it gets a word of its own rather
        // than none, on `trouble`'s terms.
        Err(message) => Err(json!({ "error": message, "trouble": BAD_CONFIG })),
    }
}

/// The `socket` a config names, when it names one.
fn socket_of(config: &str) -> Option<String> {
    let value: Value = serde_json::from_str(config).ok()?;
    value.get("socket").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_names_the_local_runner_and_nothing_else_does() {
        assert_eq!(socket_of(r#"{"socket":"/tmp/fc/farcoolerd.sock"}"#).as_deref(), Some("/tmp/fc/farcoolerd.sock"));
        assert_eq!(socket_of(r#"{"socket":""}"#), None);
        assert_eq!(socket_of(r#"{"host":"box","user":"me"}"#), None);
    }

    #[tokio::test]
    async fn a_socket_nobody_answers_is_a_failure_with_a_word() {
        let failure = open(r#"{"socket":"/nonexistent/farcoolerd.sock"}"#).await.err().expect("no daemon there");
        assert!(failure["trouble"].is_string(), "{failure}");
    }
}
