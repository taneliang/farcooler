//! The projector as a setting (ov-372): `settings.set_projector` writes
//! `[agents] projector` to the runner's config.toml and turns `agent_rows`
//! on or off in the running daemon at once, so the next connection's hello
//! says so; a daemon started on a config that says on offers it from the
//! start. Each daemon reads its own config.toml, never this Mac's.

use std::path::Path;

use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::{request as call_named, Client};

mod common;
use common::listening_daemon_with_env;

type Socket = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

async fn socket(dir: &Path) -> Socket {
    Client::connect(dir.join("farcoolerd.sock"), "test", "0.0.0").await.expect("socket handshake")
}

fn offers_rows(client: &Socket) -> bool {
    client.server_hello().capabilities.iter().any(|c| c == "agent_rows")
}

async fn set(client: &Socket, on: bool) {
    let mut req = call_named("settings.set_projector");
    req.payload = Some(request::Payload::HostSettings(pb::HostSettings { branch_prefix: String::new(), projector: on }));
    let answer = client.call_with(req, Default::default()).await.expect("settings.set_projector");
    assert!(matches!(answer.value, Some(result::Value::Empty(_))), "{answer:?}");
}

#[tokio::test]
async fn the_setting_turns_rows_on_and_off_without_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let path = config.to_string_lossy().into_owned();
    let mut daemon = listening_daemon_with_env(dir.path(), &[("FARCOOLER_CONFIG", &path)]).await;
    let first = socket(dir.path()).await;
    assert!(!offers_rows(&first), "off by default");
    assert!(first.server_hello().capabilities.iter().any(|c| c == "projector_setting"));

    set(&first, true).await;
    assert!(std::fs::read_to_string(&config).unwrap().contains("projector = true"));
    assert!(offers_rows(&socket(dir.path()).await), "the next hello offers rows");

    set(&first, false).await;
    assert!(!std::fs::read_to_string(&config).unwrap_or_default().contains("projector"));
    assert!(!offers_rows(&socket(dir.path()).await), "and stops");

    // Read back at start.
    set(&first, true).await;
    drop(first);
    daemon.child.kill().await.unwrap();
    let _again = listening_daemon_with_env(dir.path(), &[("FARCOOLER_CONFIG", &path)]).await;
    assert!(offers_rows(&socket(dir.path()).await), "a daemon started on `projector = true` offers rows");
}
