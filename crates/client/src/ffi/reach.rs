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
        if both(config) || !socket.starts_with('/') {
            return Err(json!({ "error": "a socket is an absolute path, named without a host or a token", "trouble": BAD_CONFIG }));
        }
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

/// A config naming a socket and an ssh destination too: two paths to one
/// runner, with nothing choosing between them (the header's "never both").
fn both(config: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(config) else { return false };
    ["host", "token"].iter().any(|key| value.get(*key).is_some_and(|v| !v.is_null()))
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
    async fn a_socket_beside_a_host_or_a_token_or_relative_is_refused() {
        for config in [
            r#"{"socket":"/tmp/fc/farcoolerd.sock","host":"box"}"#,
            r#"{"socket":"/tmp/fc/farcoolerd.sock","token":"tc-x"}"#,
            r#"{"socket":"farcoolerd.sock"}"#,
        ] {
            let failure = open(config).await.err().expect(config);
            assert_eq!(failure["trouble"], BAD_CONFIG, "{config}");
        }
    }

    #[tokio::test]
    async fn a_socket_nobody_answers_is_a_failure_with_a_word() {
        let failure = open(r#"{"socket":"/nonexistent/farcoolerd.sock"}"#).await.err().expect("no daemon there");
        assert!(failure["trouble"].is_string(), "{failure}");
    }
}
