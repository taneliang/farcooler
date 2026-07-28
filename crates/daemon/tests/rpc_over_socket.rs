//! The daemon as a client actually meets it: a real Unix socket, the real
//! dispatch table, the real store.
//!
//! The unit tests around `rpc` prove the scope table in isolation. This proves
//! the thing those tests cannot: that a request crossing the wire reaches the
//! service, that a result comes back shaped as the client expects, and that a
//! refusal arrives as a specific code rather than a dropped connection.
//!
//! tmux is not required. Every method here reads or writes durable intent; the
//! ones that need a live pane are covered by `crates/tmux`'s live tests.

use std::sync::Arc;

use overnight_daemon::{rpc::Rpc, service::Service};
use overnight_protocol::v1::{ErrorCode, Scope, request, result};
use overnight_transport::{Client, ClientError, HandshakeConfig, UnixListenerServer, request};

/// A daemon on a private socket with a private database.
///
/// The service is opened at an EXPLICIT directory, never through
/// `OVERNIGHT_HOME`. The environment is process-global and these tests run in
/// parallel, so routing through it would have each test reading whichever
/// database the most recent `start()` happened to point at — which is exactly
/// the failure this harness produced the first time it was written.
struct Harness {
    _dir: tempfile::TempDir,
    socket: std::path::PathBuf,
}

async fn start(scope: Scope) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("overnightd.sock");
    let service = Arc::new(Service::open_in(dir.path().to_path_buf()).await.expect("service"));
    let server = UnixListenerServer::bind(&socket).expect("bind");

    let cfg = HandshakeConfig { daemon_version: "test".into(), granted_scope: scope };
    tokio::spawn(async move {
        let _ = server.serve(cfg, Factory { service, scope }).await;
    });

    // The listener is bound before serve() is spawned, so a connect cannot race
    // it — but give the task a turn so the first accept is already pending.
    tokio::task::yield_now().await;
    Harness { _dir: dir, socket }
}

#[derive(Clone)]
struct Factory {
    service: Arc<Service>,
    scope: Scope,
}

impl overnight_transport::Handler for Factory {
    fn handle(
        &self,
        req: overnight_protocol::v1::Request,
    ) -> impl std::future::Future<Output = overnight_protocol::v1::Response> + Send {
        let rpc = Rpc::new(self.service.clone(), self.scope);
        async move { overnight_transport::Handler::handle(&rpc, req).await }
    }
}

async fn connect(h: &Harness) -> Client<
    tokio::net::unix::OwnedReadHalf,
    tokio::net::unix::OwnedWriteHalf,
> {
    Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect")
}

#[tokio::test]
async fn a_client_can_ask_the_daemon_what_it_is() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    assert_eq!(client.server_hello().daemon_version, "test");

    let result = client.call(request("daemon.version")).await.expect("daemon.version");
    let Some(result::Value::DaemonVersion(v)) = result.value else {
        panic!("expected a DaemonVersion, got {:?}", result.value);
    };
    assert!(v.protocol_versions.contains(&overnight_protocol::PROTOCOL_VERSION));
    assert!(!v.capabilities.is_empty());
}

#[tokio::test]
async fn listing_an_empty_host_returns_empty_lists_rather_than_an_error() {
    // A fresh install is not a failure, and a client that has to distinguish
    // "no workspaces" from "the call broke" will get it wrong.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let result = client.call(request("workspace.list")).await.expect("workspace.list");
    let Some(result::Value::WorkspaceList(list)) = result.value else { panic!("wrong result") };
    assert!(list.items.is_empty());

    let result = client.call(request("terminal.list")).await.expect("terminal.list");
    let Some(result::Value::TerminalList(list)) = result.value else { panic!("wrong result") };
    assert!(list.items.is_empty());
}

#[tokio::test]
async fn a_read_scoped_client_is_refused_a_mutation_with_a_specific_code() {
    // The point of the scope table: the refusal must be actionable, not a
    // closed socket or a generic failure.
    let h = start(Scope::Read).await;
    let mut client = connect(&h).await;

    let mut req = request("repository.register");
    req.payload = Some(request::Payload::RepositoryRegister(
        overnight_protocol::v1::RepositoryRegister { relative_path: "/tmp".into() },
    ));

    match client.call(req).await {
        Err(ClientError::Daemon { code, retryable, .. }) => {
            assert_eq!(code, ErrorCode::ScopeDenied as i32);
            assert!(!retryable, "retrying a scope denial can never succeed");
        }
        other => panic!("expected a scope denial, got {other:?}"),
    }

    // And the connection survives it, so the client can carry on.
    assert!(client.call(request("daemon.version")).await.is_ok());
}

#[tokio::test]
async fn a_read_scoped_client_is_refused_the_admin_reads_too() {
    // Repository roots carry paths, so listing them is host_admin even though
    // it is a read.
    let h = start(Scope::Read).await;
    let mut client = connect(&h).await;

    match client.call(request("repository_root.list")).await {
        Err(ClientError::Daemon { code, .. }) => {
            assert_eq!(code, ErrorCode::ScopeDenied as i32)
        }
        other => panic!("expected a scope denial, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unknown_method_is_refused_rather_than_hanging() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    // `terminal.write` deliberately does not exist: terminal input is a frame
    // on the terminal channel, not a request. A client that tries it must find
    // out, not block.
    match client.call(request("terminal.write")).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::NotFound as i32),
        other => panic!("expected NOT_FOUND, got {other:?}"),
    }
}

#[tokio::test]
async fn a_mutation_without_its_target_is_refused() {
    // Every single-resource mutation is addressed by the envelope's target. A
    // missing one must not be treated as "any" or "the first".
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    match client.call(request("workspace.archive")).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::NotFound as i32),
        other => panic!("expected NOT_FOUND, got {other:?}"),
    }
}

#[tokio::test]
async fn adding_a_repository_root_persists_and_is_listed_back() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let repo = tempfile::tempdir().unwrap();
    let mut req = request("repository_root.add");
    req.payload = Some(request::Payload::RepositoryRootAdd(
        overnight_protocol::v1::RepositoryRootAdd {
            absolute_path: repo.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));

    let result = client.call(req).await.expect("repository_root.add");
    let Some(result::Value::RepositoryRoot(root)) = result.value else { panic!("wrong result") };
    assert!(!root.id.is_empty());
    // host_admin, so the path comes back.
    assert!(root.display_path.is_some());

    let result = client.call(request("repository_root.list")).await.expect("list");
    let Some(result::Value::RepositoryRootList(list)) = result.value else { panic!("wrong result") };
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].id, root.id);
}

#[tokio::test]
async fn a_second_client_sees_what_the_first_one_wrote() {
    // The reason the daemon exists: one owner of durable state, many clients.
    let h = start(Scope::HostAdmin).await;
    let mut first = connect(&h).await;

    let repo = tempfile::tempdir().unwrap();
    let mut req = request("repository_root.add");
    req.payload = Some(request::Payload::RepositoryRootAdd(
        overnight_protocol::v1::RepositoryRootAdd {
            absolute_path: repo.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    first.call(req).await.expect("add");

    let mut second = connect(&h).await;
    let result = second.call(request("repository_root.list")).await.expect("list");
    let Some(result::Value::RepositoryRootList(list)) = result.value else { panic!("wrong result") };
    assert_eq!(list.items.len(), 1, "a separate connection must see the same state");
}

#[tokio::test]
async fn a_root_is_removed_only_with_its_exact_name() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let repo = tempfile::tempdir().unwrap();
    let name = repo.path().file_name().unwrap().to_string_lossy().into_owned();

    let mut add = request("repository_root.add");
    add.payload = Some(request::Payload::RepositoryRootAdd(
        overnight_protocol::v1::RepositoryRootAdd {
            absolute_path: repo.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    let result = client.call(add).await.expect("add");
    let Some(result::Value::RepositoryRoot(root)) = result.value else { panic!("wrong result") };

    // The wrong name is refused. Removing a root revokes access to a whole
    // tree, so a near-miss must not go through.
    let mut wrong = request("repository_root.remove");
    wrong.target_resource_id = Some(root.id.clone());
    wrong.payload = Some(request::Payload::TypedConfirmation(
        overnight_protocol::v1::TypedConfirmation { typed_confirmation: "nope".into() },
    ));
    match client.call(wrong).await {
        Err(ClientError::Daemon { code, .. }) => {
            assert_eq!(code, ErrorCode::ConfirmationRequired as i32)
        }
        other => panic!("expected a confirmation refusal, got {other:?}"),
    }

    // Still there.
    let result = client.call(request("repository_root.list")).await.expect("list");
    let Some(result::Value::RepositoryRootList(list)) = result.value else { panic!("wrong result") };
    assert_eq!(list.items.len(), 1);

    let mut right = request("repository_root.remove");
    right.target_resource_id = Some(root.id.clone());
    right.payload = Some(request::Payload::TypedConfirmation(
        overnight_protocol::v1::TypedConfirmation { typed_confirmation: name },
    ));
    client.call(right).await.expect("remove");

    let result = client.call(request("repository_root.list")).await.expect("list");
    let Some(result::Value::RepositoryRootList(list)) = result.value else { panic!("wrong result") };
    assert!(list.items.is_empty());

    // Nothing on disk was touched.
    assert!(repo.path().exists(), "removing a root must not delete anything");
}

#[tokio::test]
async fn removing_a_root_leaves_no_orphaned_repositories() {
    // A repository exists only as a member of a root. Leaving one behind would
    // strand a row pointing at a tree Overnight may no longer touch.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let dir = tempfile::tempdir().unwrap();
    let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
    let repo_path = dir.path().join("demo");
    std::fs::create_dir(&repo_path).unwrap();
    std::process::Command::new("git")
        .args(["init", "-q", "."])
        .current_dir(&repo_path)
        .status()
        .unwrap();

    let mut add = request("repository_root.add");
    add.payload = Some(request::Payload::RepositoryRootAdd(
        overnight_protocol::v1::RepositoryRootAdd {
            absolute_path: dir.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    let result = client.call(add).await.expect("add");
    let Some(result::Value::RepositoryRoot(root)) = result.value else { panic!("wrong result") };

    let mut register = request("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        overnight_protocol::v1::RepositoryRegister {
            relative_path: repo_path.to_string_lossy().into_owned(),
        },
    ));
    client.call(register).await.expect("register");

    let mut remove = request("repository_root.remove");
    remove.target_resource_id = Some(root.id.clone());
    remove.payload = Some(request::Payload::TypedConfirmation(
        overnight_protocol::v1::TypedConfirmation { typed_confirmation: name },
    ));
    client.call(remove).await.expect("remove");

    let result = client.call(request("repository.list")).await.expect("list");
    let Some(result::Value::RepositoryList(list)) = result.value else { panic!("wrong result") };
    assert!(list.items.is_empty(), "the repository must go with its root");
}

#[tokio::test]
async fn a_root_with_workspaces_under_it_is_refused_with_an_actionable_reason() {
    // Not RUNNING_PROCESSES: nothing is running. The distinction matters
    // because the two have different remedies, and a client can only say
    // "remove the worktrees first" if it is told that is the problem.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let dir = tempfile::tempdir().unwrap();
    let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
    let repo_path = dir.path().join("demo");
    std::fs::create_dir(&repo_path).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
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
    add.payload = Some(request::Payload::RepositoryRootAdd(
        overnight_protocol::v1::RepositoryRootAdd {
            absolute_path: dir.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    let result = client.call(add).await.expect("add");
    let Some(result::Value::RepositoryRoot(root)) = result.value else { panic!("wrong result") };

    let mut register = request("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        overnight_protocol::v1::RepositoryRegister {
            relative_path: repo_path.to_string_lossy().into_owned(),
        },
    ));
    let result = client.call(register).await.expect("register");
    let Some(result::Value::Repository(repository)) = result.value else { panic!("wrong result") };

    let mut create = request("workspace.create");
    create.target_resource_id = Some(repository.id.clone());
    create.payload = Some(request::Payload::WorkspaceCreate(
        overnight_protocol::v1::WorkspaceCreate {
            task_name: "a task".into(),
            branch: "feat/x".into(),
            base_revision: "HEAD".into(),
            cli_preset: String::new(),
        },
    ));
    client.call(create).await.expect("workspace.create");

    let mut remove = request("repository_root.remove");
    remove.target_resource_id = Some(root.id.clone());
    remove.payload = Some(request::Payload::TypedConfirmation(
        overnight_protocol::v1::TypedConfirmation { typed_confirmation: name },
    ));
    match client.call(remove).await {
        Err(ClientError::Daemon { code, retryable, .. }) => {
            assert_eq!(code, ErrorCode::WorkspacesExist as i32);
            assert!(!retryable, "retrying cannot help; the user has to act");
        }
        other => panic!("expected WORKSPACES_EXIST, got {other:?}"),
    }
}
