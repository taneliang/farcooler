//! No projector setting (ov-830): a runner offers `agent_rows` and
//! `subagent_rows` from the start, with nothing in its config.toml or its
//! environment, and `settings.set_projector`, kept for apps from before,
//! answers and changes nothing: rows stay offered, and no config is written.

use std::path::Path;

use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::{request as call_named, Client};

mod common;
use common::listening_daemon_with_env;

type Socket = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

async fn socket(dir: &Path) -> Socket {
    Client::connect(dir.join("farcoolerd.sock"), "test", "0.0.0").await.expect("socket handshake")
}

fn offers(client: &Socket, capability: &str) -> bool {
    client.server_hello().capabilities.iter().any(|c| c == capability)
}

async fn set(client: &Socket, on: bool) {
    let mut req = call_named("settings.set_projector");
    req.payload = Some(request::Payload::HostSettings(pb::HostSettings { branch_prefix: String::new(), projector: on }));
    let answer = client.call_with(req, Default::default()).await.expect("settings.set_projector");
    assert!(matches!(answer.value, Some(result::Value::Empty(_))), "{answer:?}");
}

/// Goes red with the setting's gate back (`set_shadowing(load_projector())`
/// at start, or `set_projector` withholding `agent_rows`).
#[tokio::test]
async fn rows_are_offered_with_no_setting_and_the_old_switch_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let path = config.to_string_lossy().into_owned();
    let _daemon = listening_daemon_with_env(dir.path(), &[("FARCOOLER_CONFIG", &path)]).await;
    let first = socket(dir.path()).await;
    assert!(offers(&first, "agent_rows"), "offered with nothing set");
    assert!(offers(&first, "subagent_rows"));
    assert!(offers(&first, "projector_setting"), "kept for apps from before");

    // An app from before turns its view off, then on: neither changes what
    // this runner serves, and neither writes its config.
    set(&first, false).await;
    assert!(offers(&socket(dir.path()).await, "agent_rows"), "still offered after an old app's off");
    set(&first, true).await;
    assert!(!std::fs::read_to_string(&config).unwrap_or_default().contains("projector"), "nothing written");
}
