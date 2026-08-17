//! A stdio session relayed into a running daemon keeps the scope it was given.
//!
//! `serve_stdio_session` pipes into the already-running daemon rather than
//! opening a second service, because an agent's transcript lives in the first
//! daemon's memory. That pipe used to discard the scope: the running daemon
//! answers the handshake, and its socket path grants host_admin to a local
//! caller. So a read-enrolled device got host admin on every machine where a
//! daemon was already up — which is every machine in normal use, and therefore
//! every machine the scope was supposed to protect.

use farcooler_protocol::v1::{result, ErrorCode, Scope};
use farcooler_transport::{request, ClientError};

mod common;
use common::{listening_daemon, spawn, spawn_with};

#[tokio::test]
async fn a_relayed_stdio_session_is_not_promoted_to_host_admin() {
    let dir = tempfile::tempdir().unwrap();
    // A daemon on the socket, so the relay branch is the one taken.
    let _listening = listening_daemon(dir.path()).await;

    let (_child, mut client) = spawn_with(dir.path(), &["--scope", "read"]).await;
    assert_eq!(
        client.server_hello().granted_scope,
        Scope::Read as i32,
        "the relay branch discarded the scope"
    );

    // And the running daemon enforces it, which is the half a handshake
    // assertion cannot see: the scope has to reach the dispatcher that refuses,
    // not just the hello that reports.
    match client.call(request("worktree.list")).await {
        Err(ClientError::Daemon { code, .. }) => {
            assert_eq!(code, ErrorCode::ScopeDenied as i32, "a read session reached host paths");
        }
        other => panic!("a relayed read session must not list worktrees: {other:?}"),
    }
}

/// A relayed session that was given no scope still holds host admin.
///
/// The ssh path for every key enrolled before any of this existed: no forced
/// command, so no `--scope`, so nothing to say and no line sent. Silence already
/// means host_admin at the other end, and it has to keep meaning that — a
/// preamble sent anyway would be read as a frame by a daemon built before
/// preambles, which is exactly the process still listening on a runner that has
/// just been upgraded.
#[tokio::test]
async fn a_relayed_session_with_no_scope_still_holds_host_admin() {
    let dir = tempfile::tempdir().unwrap();
    let _listening = listening_daemon(dir.path()).await;

    let (_child, mut client) = spawn(dir.path()).await;
    assert_eq!(client.server_hello().granted_scope, Scope::HostAdmin as i32);

    let result = client.call(request("repository_root.list")).await.expect("repository_root.list");
    let Some(result::Value::RepositoryRootList(_)) = result.value else { panic!("wrong result") };
}

/// A socket client that sends no preamble at all is served exactly as before.
///
/// This is every CLI and every Mac app already installed. They know nothing
/// about a preamble and never will — requiring one would break all of them on
/// upgrade, and it would break them IN FRONT OF the version negotiation built to
/// explain that class of mismatch, so the user would get a hang-up instead of a
/// reason.
#[tokio::test]
async fn a_socket_client_that_says_nothing_is_still_the_owning_user() {
    let dir = tempfile::tempdir().unwrap();
    let _listening = listening_daemon(dir.path()).await;

    let mut client =
        farcooler_transport::Client::connect(dir.path().join("farcoolerd.sock"), "test", "0.0.0")
            .await
            .expect("handshake with no preamble");
    assert_eq!(client.server_hello().granted_scope, Scope::HostAdmin as i32);

    // And host admin in fact, not only in the hello: paths are the thing this
    // scope gates, and `repository_root.list` is nothing but paths.
    let result = client.call(request("repository_root.list")).await.expect("repository_root.list");
    let Some(result::Value::RepositoryRootList(_)) = result.value else { panic!("wrong result") };
}
