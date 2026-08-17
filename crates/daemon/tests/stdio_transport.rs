//! The remote transport, exercised the way sshd will drive it.
//!
//! `farcooler --host box ...` runs `ssh box farcoolerd --stdio` and speaks the
//! protocol over the resulting pipes. ssh is not part of what can go wrong
//! there — it is a byte pipe either way — so this spawns the real daemon binary
//! with `--stdio` and talks to its actual stdin and stdout.
//!
//! The failure this is really guarding against: anything at all printed to
//! stdout by the daemon corrupts the very first frame, and the symptom is a
//! handshake that hangs rather than an error that names the cause. A log line,
//! a `println!` left in, a panic message — any of them.

use farcooler_protocol::v1::{
    request, result, AgentSubscribe, ErrorCode, PaneMode, RepositoryRegister, RepositoryRootAdd,
    Scope, SetPaneMode, TerminalCreate, WorkspaceCreate,
};
use farcooler_transport::{request, Client, ClientError};
use tokio::process::{ChildStdin, ChildStdout};

mod common;
use common::{spawn, spawn_with, stdio_command};

#[tokio::test]
async fn the_daemon_serves_the_protocol_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn(dir.path()).await;

    // A remote client whose key names no scope gets host_admin, same as a local
    // socket one: ssh has already proved it is the user who owns the database
    // there. See `no_scope_argument_still_means_host_admin`.
    assert_eq!(client.server_hello().granted_scope, Scope::HostAdmin as i32);
    assert!(!client.server_hello().daemon_version.is_empty());

    let result = client.call(request("daemon.version")).await.expect("daemon.version");
    let Some(result::Value::DaemonVersion(v)) = result.value else { panic!("wrong result") };
    assert!(v.protocol_versions.contains(&farcooler_protocol::PROTOCOL_VERSION));
}

// ---- the scope the forced command gave this session ----

/// A forced command's scope is the session's scope.
///
/// sshd runs the command in the `authorized_keys` entry and ignores whatever the
/// client asked for, so `--scope` is the one thing in this process's arguments
/// that the connecting device cannot choose. It used to be ignored entirely, and
/// every stdio session got host_admin — so a device enrolled to read would have
/// held full host administration and the word would have been decorative.
#[tokio::test]
async fn a_scope_in_the_arguments_is_the_scope_of_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, client) = spawn_with(dir.path(), &["--scope", "read"]).await;
    assert_eq!(client.server_hello().granted_scope, Scope::Read as i32);
}

/// And the handshake is not the enforcement.
///
/// `ServerHello` only REPORTS the scope; the table in `rpc.rs` is what refuses.
/// The two used to be wired to different things — the handshake to the argument,
/// the dispatcher to a `Scope::HostAdmin` literal — which is the shape of bug
/// that passes a scope test and grants everything.
#[tokio::test]
async fn a_read_session_is_refused_a_control_method() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn_with(dir.path(), &["--scope", "read"]).await;

    client.call(request("repository.list")).await.expect("repository.list is a read method");

    match client.call(request("daemon.shutdown")).await {
        Err(ClientError::Daemon { code, .. }) => {
            assert_eq!(
                code,
                ErrorCode::ScopeDenied as i32,
                "a read session reached a host_admin method"
            );
        }
        other => panic!("a read session must not reach daemon.shutdown: {other:?}"),
    }
}

/// No scope means host_admin, and that is honest rather than lax.
///
/// A key with no forced command lets the connecting device write the whole
/// command line, so it could pass any `--scope` it liked. Defaulting to less
/// would protect nothing and would break every entry enrolled before this
/// existed.
#[tokio::test]
async fn no_scope_argument_still_means_host_admin() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, client) = spawn(dir.path()).await;
    assert_eq!(client.server_hello().granted_scope, Scope::HostAdmin as i32);
}

/// A scope nobody recognizes refuses the session rather than rounding up.
///
/// Absence is a decision; a misspelling is a mistake. Promoting one to host
/// admin turns a typo in someone's `authorized_keys` into privilege escalation,
/// and it would do it silently, on the one line nobody re-reads.
#[tokio::test]
async fn a_scope_nobody_recognizes_refuses_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = stdio_command(dir.path(), &["--scope", "reed"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn farcoolerd --stdio");

    let status = tokio::time::timeout(std::time::Duration::from_secs(30), child.wait())
        .await
        .expect("a refused session must exit rather than serve")
        .expect("wait");
    // Specifically the usage exit code, not merely non-zero: closing this
    // session's stdin also ends the process non-zero, so `!success()` alone
    // would pass against a daemon that had happily served the whole session.
    assert_eq!(status.code(), Some(2), "an unknown scope must refuse the session outright");
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
    add.payload = Some(farcooler_protocol::v1::request::Payload::RepositoryRootAdd(
        farcooler_protocol::v1::RepositoryRootAdd {
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
    assert_ne!(host.self_health, farcooler_protocol::v1::SelfHealth::Unspecified as i32);
    // Degraded must always come with a reason a user can act on.
    if host.self_health == farcooler_protocol::v1::SelfHealth::Degraded as i32 {
        assert!(!host.self_health_reasons.is_empty());
    }
}

// ---- the agent channel, over the same transport a phone actually uses ----
//
// `rpc_over_socket.rs` proves these against the Unix socket. Mirrored here
// against a real `farcoolerd --stdio` process rather than in-process dispatch,
// because the whole point of this file is that nothing about the daemon's
// behavior may depend on which listener accepted the connection.

/// A workspace, over stdio, ready to hold a terminal.
async fn a_workspace(
    client: &mut Client<ChildStdout, ChildStdin>,
    dir: &std::path::Path,
) -> farcooler_protocol::v1::Workspace {
    let repo_path = dir.join("demo");
    std::fs::create_dir(&repo_path).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git")
            .args(&args)
            .current_dir(&repo_path)
            .status()
            .unwrap();
    }

    let mut add = request("repository_root.add");
    add.payload = Some(request::Payload::RepositoryRootAdd(RepositoryRootAdd {
        absolute_path: dir.to_string_lossy().into_owned(),
        typed_confirmation: String::new(),
    }));
    client.call(add).await.expect("add root");

    let mut register = request("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(RepositoryRegister {
        relative_path: repo_path.to_string_lossy().into_owned(),
    }));
    let result = client.call(register).await.expect("register");
    let Some(result::Value::Repository(repository)) = result.value else { panic!("wrong result") };

    let mut create = request("workspace.create");
    create.target_resource_id = Some(repository.id.clone());
    create.payload = Some(request::Payload::WorkspaceCreate(WorkspaceCreate {
        task_name: "stdio".into(),
        branch: "feat/stdio".into(),
        base_revision: "HEAD".into(),
        terminal_preset: String::new(),
        adopt_existing: false,
    }));
    let result = client.call(create).await.expect("workspace.create");
    let Some(result::Value::Workspace(workspace)) = result.value else { panic!("wrong result") };
    workspace
}

/// One terminal in that workspace, over stdio.
async fn a_terminal(
    client: &mut Client<ChildStdout, ChildStdin>,
    workspace: &bytes::Bytes,
    title: &str,
) -> farcooler_protocol::v1::Terminal {
    let mut create = request("terminal.create");
    create.target_resource_id = Some(workspace.clone());
    create.payload = Some(request::Payload::TerminalCreate(TerminalCreate {
        title: title.into(),
        command_preset: "shell".into(),
        join_active_group: false,
    }));
    let result = client.call(create).await.expect("terminal.create");
    let Some(result::Value::Terminal(terminal)) = result.value else { panic!("wrong result") };
    terminal
}

#[tokio::test]
async fn a_terminal_reports_its_pane_mode_to_a_client_over_stdio() {
    // Clients render pane mode; they must never infer it from a command line —
    // and that has to hold for the transport a phone actually uses, not only
    // the Unix socket.
    let home = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn(home.path()).await;
    let repo_root = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, repo_root.path()).await;
    let terminal = a_terminal(&mut client, &workspace.id, "claude").await;

    assert_eq!(terminal.pane_mode, PaneMode::Terminal as i32);
}

#[tokio::test]
async fn an_agent_subscribe_from_a_cursor_is_accepted_over_stdio() {
    // A client attaches to a PANE, not to a session: subscribing before any
    // agent has ever run there must be accepted, with nothing to replay,
    // rather than refused — over stdio exactly as it is over the socket.
    let home = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn(home.path()).await;
    let repo_root = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, repo_root.path()).await;
    let terminal = a_terminal(&mut client, &workspace.id, "claude").await;

    let mut req = request("terminal.agent_subscribe");
    req.payload = Some(request::Payload::AgentSubscribe(AgentSubscribe {
        epoch: 0,
        terminal_id: terminal.id.clone(),
        from_seq: 0,
    }));
    let result = client.call(req).await;
    assert!(
        result.is_ok(),
        "subscribe must be accepted even before a session exists, over stdio: {result:?}"
    );

    let Some(result::Value::AgentEventBatch(batch)) = result.unwrap().value else {
        panic!("wrong result")
    };
    assert!(batch.events.is_empty(), "nothing has happened on this pane yet");
}

#[tokio::test]
async fn a_pane_mode_toggle_is_refused_over_stdio_the_same_way() {
    // Authorization and precondition outcomes must not differ by transport: an
    // unknown terminal id is refused with the same code whether the client
    // reached the daemon over the Unix socket or over ssh's stdio pipes.
    let home = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn(home.path()).await;

    let mut req = request("terminal.set_pane_mode");
    req.payload = Some(request::Payload::SetPaneMode(SetPaneMode {
        terminal_id: bytes::Bytes::copy_from_slice(uuid::Uuid::now_v7().as_bytes()),
        pane_mode: PaneMode::Agent as i32,
        force: false,
    }));
    match client.call(req).await {
        Err(ClientError::Daemon { code, retryable, .. }) => {
            assert_eq!(
                code,
                ErrorCode::NotFound as i32,
                "an unknown terminal must fail identically on both adapters"
            );
            assert!(!retryable, "retrying an unknown terminal can never succeed");
        }
        other => panic!("expected NOT_FOUND, got {other:?}"),
    }
}
