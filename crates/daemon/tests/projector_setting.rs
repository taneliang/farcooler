//! No projector setting (ov-830): a runner offers `agent_rows` and
//! `subagent_rows` from the start, with nothing in its config.toml or its
//! environment, and still names `projector_setting`, which apps from before
//! read to recognize a runner that serves rows to whoever views a pane.

use std::path::Path;

use farcooler_transport::Client;

mod common;
use common::listening_daemon_with_env;

type Socket = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

async fn socket(dir: &Path) -> Socket {
    Client::connect(dir.join("farcoolerd.sock"), "test", "0.0.0").await.expect("socket handshake")
}

fn offers(client: &Socket, capability: &str) -> bool {
    client.server_hello().capabilities.iter().any(|c| c == capability)
}

/// Goes red with the setting's gate back (`set_shadowing(load_projector())`
/// at start).
#[tokio::test]
async fn rows_are_offered_with_no_setting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml").to_string_lossy().into_owned();
    let _daemon = listening_daemon_with_env(dir.path(), &[("FARCOOLER_CONFIG", &path)]).await;
    let first = socket(dir.path()).await;
    assert!(offers(&first, "agent_rows"), "offered with nothing set");
    assert!(offers(&first, "subagent_rows"));
    assert!(offers(&first, "projector_setting"), "kept for apps from before");
}
