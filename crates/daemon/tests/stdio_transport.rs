//! The remote transport, exercised the way sshd will drive it.
//!
//! `overnight --host box ...` runs `ssh box overnightd --stdio` and speaks the
//! protocol over the resulting pipes. ssh is not part of what can go wrong
//! there — it is a byte pipe either way — so this spawns the real daemon binary
//! with `--stdio` and talks to its actual stdin and stdout.
//!
//! The failure this is really guarding against: anything at all printed to
//! stdout by the daemon corrupts the very first frame, and the symptom is a
//! handshake that hangs rather than an error that names the cause. A log line,
//! a `println!` left in, a panic message — any of them.

use overnight_protocol::v1::{result, Scope};
use overnight_transport::{request, Client};
use tokio::process::{ChildStdin, ChildStdout, Command};

/// Spawn the daemon in stdio mode against a private runtime directory.
async fn spawn(
    dir: &std::path::Path,
) -> (tokio::process::Child, Client<ChildStdout, ChildStdin>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_overnightd"))
        .arg("--stdio")
        .env("OVERNIGHT_HOME", dir)
        // Deliberately noisy: if any of this reaches stdout the handshake
        // breaks, which is exactly what this test exists to catch.
        .env("RUST_LOG", "debug")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn overnightd --stdio");

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let client = Client::over(stdout, stdin, "test-client", "0.0.0")
        .await
        .expect("handshake over stdio");
    (child, client)
}

#[tokio::test]
async fn the_daemon_serves_the_protocol_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn(dir.path()).await;

    // A remote client gets host_admin, same as a local socket one: ssh has
    // already proved it is the user who owns the database there.
    assert_eq!(client.server_hello().granted_scope, Scope::HostAdmin as i32);
    assert!(!client.server_hello().daemon_version.is_empty());

    let result = client.call(request("daemon.version")).await.expect("daemon.version");
    let Some(result::Value::DaemonVersion(v)) = result.value else { panic!("wrong result") };
    assert!(v.protocol_versions.contains(&overnight_protocol::PROTOCOL_VERSION));
}

#[tokio::test]
async fn several_requests_run_over_one_session() {
    // sshd gives one process per connection, so a session has to survive more
    // than the single call that opened it.
    let dir = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn(dir.path()).await;

    for _ in 0..5 {
        let result = client.call(request("workspace.list")).await.expect("workspace.list");
        let Some(result::Value::WorkspaceList(list)) = result.value else { panic!("wrong result") };
        assert!(list.items.is_empty());
    }
}

#[tokio::test]
async fn a_remote_client_can_write_and_read_back_what_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn(dir.path()).await;

    let repo = tempfile::tempdir().unwrap();
    let mut add = request("repository_root.add");
    add.payload = Some(overnight_protocol::v1::request::Payload::RepositoryRootAdd(
        overnight_protocol::v1::RepositoryRootAdd {
            absolute_path: repo.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    client.call(add).await.expect("repository_root.add");

    let result = client.call(request("repository_root.list")).await.expect("list");
    let Some(result::Value::RepositoryRootList(list)) = result.value else { panic!("wrong result") };
    assert_eq!(list.items.len(), 1);
    // host_admin over ssh, so paths come back.
    assert!(list.items[0].display_path.is_some());
}

#[tokio::test]
async fn host_health_is_reported_by_the_daemon_not_sampled_by_the_client() {
    // A remote client has no tmux to look at, so this has to arrive over the
    // wire or a remote host can never report its own health.
    let dir = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn(dir.path()).await;

    let result = client.call(request("host.health")).await.expect("host.health");
    let Some(result::Value::Host(host)) = result.value else { panic!("wrong result") };
    assert!(!host.platform.is_empty());
    assert_ne!(host.self_health, overnight_protocol::v1::SelfHealth::Unspecified as i32);
    // Degraded must always come with a reason a user can act on.
    if host.self_health == overnight_protocol::v1::SelfHealth::Degraded as i32 {
        assert!(!host.self_health_reasons.is_empty());
    }
}
