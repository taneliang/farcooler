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
    /// Held for the test's life. See `TMUX_SERVERS`.
    _permit: tokio::sync::OwnedSemaphorePermit,
    /// The private tmux socket this harness started a server on.
    tmux_socket: String,
}

/// Take the tmux server down with the test that started it.
///
/// Every harness starts its own server and nothing used to stop it, so a suite
/// run left one behind per tmux-using test. They accumulate across runs — a few
/// hundred after an afternoon — and eventually new ones stop starting, at which
/// point `terminal.create` fails with `tmux is unavailable` and the suite appears
/// to have broken in whichever test drew the short straw.
///
/// Synchronous because `Drop` is: it is one `kill-server` against a socket
/// nothing else uses.
impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", &self.tmux_socket, "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// How many of these tests may own a live tmux server at once.
///
/// Every harness starts its OWN server, on its own socket, which is what keeps
/// them isolated. Starting twenty at the same instant is a different matter: the
/// ones that lose the race get `tmux is unavailable` from a server that has not
/// finished coming up, and the failure lands on whichever test happened to be
/// unlucky — so the suite failed somewhere different every run and looked like a
/// bug in the code under test.
///
/// Four is enough to keep the suite quick and few enough that none of them
/// starve.
static TMUX_SERVERS: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));

async fn start(scope: Scope) -> Harness {
    let permit = TMUX_SERVERS.clone().acquire_owned().await.expect("permit");
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("overnightd.sock");
    let service = Arc::new(Service::open_in(dir.path().to_path_buf()).await.expect("service"));
    let tmux_socket = service.tmux.socket().to_string();
    let server = UnixListenerServer::bind(&socket).expect("bind");

    // The watcher is constructed but not run: these tests are about dispatch,
    // and a sampling loop would make them race a tmux that may not be there.
    let watcher = overnight_daemon::watch::Watcher::new(service.clone());

    let cfg = HandshakeConfig { daemon_version: "test".into(), granted_scope: scope };
    tokio::spawn(async move {
        let _ = server.serve(cfg, Factory { service, watcher, scope }).await;
    });

    // The listener is bound before serve() is spawned, so a connect cannot race
    // it — but give the task a turn so the first accept is already pending.
    tokio::task::yield_now().await;
    Harness { _dir: dir, socket, _permit: permit, tmux_socket }
}

#[derive(Clone)]
struct Factory {
    service: Arc<Service>,
    watcher: Arc<overnight_daemon::watch::Watcher>,
    scope: Scope,
}

impl overnight_transport::Handler for Factory {
    fn handle(
        &self,
        req: overnight_protocol::v1::Request,
    ) -> impl std::future::Future<Output = overnight_protocol::v1::Response> + Send {
        let rpc = Rpc::new(self.service.clone(), self.watcher.clone(), self.scope);
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
            adopt_existing: false,
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

/// A workspace, over the wire, ready to have things tiled in it.
async fn a_workspace(
    client: &mut Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>,
    dir: &std::path::Path,
) -> overnight_protocol::v1::Workspace {
    let repo_path = dir.join("demo");
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
            absolute_path: dir.to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    client.call(add).await.expect("add root");

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
            task_name: "tiling".into(),
            branch: "feat/tiling".into(),
            base_revision: "HEAD".into(),
            cli_preset: String::new(),
            adopt_existing: false,
        },
    ));
    let result = client.call(create).await.expect("workspace.create");
    let Some(result::Value::Workspace(workspace)) = result.value else { panic!("wrong result") };
    workspace
}

async fn layout_call(
    client: &mut Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>,
    method: &str,
    workspace: &bytes::Bytes,
    update: overnight_protocol::v1::LayoutUpdate,
) -> overnight_protocol::v1::PaneGroupList {
    let mut req = request(method);
    req.target_resource_id = Some(workspace.clone());
    req.payload = Some(request::Payload::LayoutUpdate(update));
    let result = client.call(req).await.unwrap_or_else(|e| panic!("{method}: {e:?}"));
    let Some(result::Value::PaneGroupList(list)) = result.value else {
        panic!("{method} returned the wrong resource")
    };
    list
}

#[tokio::test]
async fn a_workspace_starts_with_nothing_tiled() {
    // The stated case is four agents with three on screen, and it only works if
    // membership is something you opt into rather than something that happens.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;

    let mut list = request("layout.list");
    list.target_resource_id = Some(workspace.id.clone());
    let result = client.call(list).await.expect("layout.list");
    let Some(result::Value::PaneGroupList(groups)) = result.value else { panic!("wrong result") };
    assert!(groups.items.is_empty(), "nothing tiles until asked");
}

#[tokio::test]
async fn zoom_follows_focus_so_four_agents_can_be_read_one_at_a_time() {
    // The deliberate divergence from tmux, and the reason zoom is useful with
    // more than one agent: moving on keeps you zoomed instead of dropping you
    // back into the grid.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;

    // Two terminals in ONE layout means creating one and splitting it. Creating
    // twice would make two layouts, because a terminal is a pane and a fresh
    // terminal gets a window of its own.
    let mut create = request("terminal.create");
    create.target_resource_id = Some(workspace.id.clone());
    create.payload = Some(request::Payload::TerminalCreate(
        overnight_protocol::v1::TerminalCreate {
            title: "one".into(),
            command_preset: "shell".into(),
            join_active_group: false,
        },
    ));
    let result = client.call(create).await.expect("terminal.create");
    let Some(result::Value::Terminal(one)) = result.value else { panic!("wrong result") };

    let list = layout_call(
        &mut client,
        "layout.split",
        &workspace.id,
        overnight_protocol::v1::LayoutUpdate {
            target: Some(one.id.clone()),
            side: overnight_protocol::v1::SplitSide::Right as i32,
            command_preset: "shell".into(),
            ..Default::default()
        },
    )
    .await;
    let first = one.id.clone();
    let second = list.items[0]
        .panes
        .iter()
        .find(|p| p.terminal_id != first)
        .expect("the split pane")
        .terminal_id
        .clone();

    // Zoom whatever is focused, then check the FOCUSED pane is the zoomed one.
    let zoomed = layout_call(
        &mut client,
        "layout.zoom",
        &workspace.id,
        overnight_protocol::v1::LayoutUpdate { zoom: Some(first.clone()), ..Default::default() },
    )
    .await;
    let pane = zoomed.items[0].panes.iter().find(|p| p.terminal_id == first).expect("first");
    assert!(pane.zoomed && pane.focused);

    let moved = layout_call(
        &mut client,
        "layout.focus",
        &workspace.id,
        overnight_protocol::v1::LayoutUpdate { step: Some(1), ..Default::default() },
    )
    .await;
    let now = moved.items[0].panes.iter().find(|p| p.terminal_id == second).expect("second");
    assert!(now.focused, "focus moved");
    assert!(now.zoomed, "and the zoom came along, which tmux alone would not do");

    let out = layout_call(&mut client, "layout.zoom", &workspace.id, Default::default()).await;
    assert!(!out.items[0].panes.iter().any(|p| p.zoomed), "prefix z is still the way out");
}

#[tokio::test]
async fn tiling_needs_control_and_reading_a_layout_does_not() {
    // An agent has to be able to place its own panes: none of this touches a
    // file or stops a process, so gating it behind host_admin would have made
    // the feature unautomatable for no safety gained.
    let h = start(Scope::Read).await;
    let mut client = connect(&h).await;

    let mut list = request("layout.list");
    list.target_resource_id = Some(bytes::Bytes::copy_from_slice(uuid::Uuid::now_v7().as_bytes()));
    // Read scope reaches the method; the workspace simply does not exist.
    match client.call(list).await {
        Err(ClientError::Daemon { code, .. }) => {
            assert_ne!(code, ErrorCode::ScopeDenied as i32, "read must be enough to look");
        }
        Ok(_) => {}
        other => panic!("unexpected {other:?}"),
    }

    let mut tile = request("layout.split");
    tile.target_resource_id =
        Some(bytes::Bytes::copy_from_slice(uuid::Uuid::now_v7().as_bytes()));
    tile.payload = Some(request::Payload::LayoutUpdate(Default::default()));
    match client.call(tile).await {
        Err(ClientError::Daemon { code, .. }) => {
            assert_eq!(code, ErrorCode::ScopeDenied as i32);
        }
        other => panic!("expected SCOPE_DENIED, got {other:?}"),
    }
}

/// One terminal, made the ordinary way, and where it ends up.
async fn a_terminal(
    client: &mut Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>,
    workspace: &bytes::Bytes,
    title: &str,
) -> overnight_protocol::v1::Terminal {
    let mut create = request("terminal.create");
    create.target_resource_id = Some(workspace.clone());
    create.payload = Some(request::Payload::TerminalCreate(
        overnight_protocol::v1::TerminalCreate {
            title: title.into(),
            command_preset: "shell".into(),
            join_active_group: false,
        },
    ));
    let result = client.call(create).await.expect("terminal.create");
    let Some(result::Value::Terminal(terminal)) = result.value else { panic!("wrong result") };

    // Fix the window's size before anything is asserted about geometry.
    //
    // A window inherits the session's dimensions, and under a parallel test run
    // that arrived late often enough to make the split assertions flaky — a
    // narrow window splits into panes whose left edges tie, and `fresh.left <
    // original.left` is then false for a split that in fact worked. Stating the
    // size makes the arithmetic deterministic rather than dependent on when tmux
    // got round to it.
    let mut viewport = request("layout.viewport");
    viewport.target_resource_id = Some(workspace.clone());
    viewport.payload = Some(request::Payload::LayoutUpdate(
        overnight_protocol::v1::LayoutUpdate {
            columns: Some(120),
            rows: Some(40),
            ..Default::default()
        },
    ));
    client.call(viewport).await.expect("layout.viewport");
    terminal
}

fn split(target: &bytes::Bytes, side: overnight_protocol::v1::SplitSide) -> overnight_protocol::v1::LayoutUpdate {
    overnight_protocol::v1::LayoutUpdate {
        target: Some(target.clone()),
        side: side as i32,
        command_preset: "shell".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_terminal_is_a_pane_in_a_layout_from_the_moment_it_exists() {
    // There is no untiled state any more, and that is the point: an untiled
    // terminal was one no navigation command could reach. A terminal IS a tmux
    // pane, and a pane is always in some window.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let terminal = a_terminal(&mut client, &workspace.id, "one").await;

    let list = layout_call(&mut client, "layout.list", &workspace.id, Default::default()).await;
    assert_eq!(list.items.len(), 1, "one terminal, one layout");
    let group = &list.items[0];
    assert_eq!(group.panes.len(), 1);
    assert_eq!(group.panes[0].terminal_id, terminal.id);
    assert!(group.panes[0].focused, "the only pane holds the keyboard");
    // Geometry that came from tmux rather than from a default.
    assert!(group.columns > 0 && group.rows > 0, "tmux reported a size: {group:?}");
}

#[tokio::test]
async fn splitting_right_puts_the_new_pane_on_the_right() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let first = a_terminal(&mut client, &workspace.id, "one").await;

    let after = layout_call(
        &mut client,
        "layout.split",
        &workspace.id,
        split(&first.id, overnight_protocol::v1::SplitSide::Right),
    )
    .await;

    assert_eq!(after.items.len(), 1, "a split stays in the same layout");
    let group = &after.items[0];
    assert_eq!(group.panes.len(), 2, "{group:?}");
    let original = group.panes.iter().find(|p| p.terminal_id == first.id).expect("original");
    let fresh = group.panes.iter().find(|p| p.terminal_id != first.id).expect("new pane");
    assert!(fresh.left > original.left, "to the right: {group:?}");
    assert_eq!(fresh.top, original.top, "and on the same row");
    assert!(fresh.focused, "you split in order to type in the new pane");
}

#[tokio::test]
async fn splitting_left_puts_the_new_pane_on_the_left() {
    // The `-b` half of the vocabulary, and the reason four drop edges need only
    // two axes: left is right with the new pane placed first.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let first = a_terminal(&mut client, &workspace.id, "one").await;

    let after = layout_call(
        &mut client,
        "layout.split",
        &workspace.id,
        split(&first.id, overnight_protocol::v1::SplitSide::Left),
    )
    .await;
    let group = &after.items[0];
    let original = group.panes.iter().find(|p| p.terminal_id == first.id).expect("original");
    let fresh = group.panes.iter().find(|p| p.terminal_id != first.id).expect("new pane");
    assert!(fresh.left < original.left, "to the left: {group:?}");
}

#[tokio::test]
async fn splitting_downwards_stacks_rather_than_sitting_beside() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let first = a_terminal(&mut client, &workspace.id, "one").await;

    let after = layout_call(
        &mut client,
        "layout.split",
        &workspace.id,
        split(&first.id, overnight_protocol::v1::SplitSide::Bottom),
    )
    .await;
    let group = &after.items[0];
    let original = group.panes.iter().find(|p| p.terminal_id == first.id).expect("original");
    let fresh = group.panes.iter().find(|p| p.terminal_id != first.id).expect("new pane");
    assert!(fresh.top > original.top, "below: {group:?}");
    assert_eq!(fresh.left, original.left, "and in the same column");
}

#[tokio::test]
async fn a_pane_splits_at_any_depth() {
    // The claim the old preset model could not make. Splitting a pane that is
    // itself half of a split has to nest, not flatten — which it does, because
    // tmux keeps the tree and Overnight does not.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let first = a_terminal(&mut client, &workspace.id, "one").await;

    let after = layout_call(
        &mut client,
        "layout.split",
        &workspace.id,
        split(&first.id, overnight_protocol::v1::SplitSide::Right),
    )
    .await;
    let second = after.items[0]
        .panes
        .iter()
        .find(|p| p.terminal_id != first.id)
        .expect("second")
        .terminal_id
        .clone();

    // Split the RIGHT-hand pane downwards: three panes, and the two on the right
    // share a column that the left one does not.
    let after = layout_call(
        &mut client,
        "layout.split",
        &workspace.id,
        split(&second, overnight_protocol::v1::SplitSide::Bottom),
    )
    .await;

    let group = &after.items[0];
    assert_eq!(group.panes.len(), 3, "{group:?}");
    let left = group.panes.iter().find(|p| p.terminal_id == first.id).expect("left");
    let upper = group.panes.iter().find(|p| p.terminal_id == second).expect("upper right");
    let lower = group
        .panes
        .iter()
        .find(|p| p.terminal_id != first.id && p.terminal_id != second)
        .expect("lower right");

    assert_eq!(upper.left, lower.left, "the two on the right share a column");
    assert!(upper.left > left.left, "and that column is right of the first pane");
    assert!(lower.top > upper.top, "the new one is below the pane it split");
    assert_eq!(left.rows, group.rows, "the left pane still spans the full height");
}

#[tokio::test]
async fn breaking_a_pane_out_makes_a_layout_and_dropping_it_back_puts_it_where_asked() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let first = a_terminal(&mut client, &workspace.id, "one").await;

    let after = layout_call(
        &mut client,
        "layout.split",
        &workspace.id,
        split(&first.id, overnight_protocol::v1::SplitSide::Right),
    )
    .await;
    let second = after.items[0]
        .panes
        .iter()
        .find(|p| p.terminal_id != first.id)
        .expect("second")
        .terminal_id
        .clone();

    let broken = layout_call(
        &mut client,
        "layout.break",
        &workspace.id,
        overnight_protocol::v1::LayoutUpdate {
            target: Some(second.clone()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(broken.items.len(), 2, "a pane pulled out is a layout of its own");

    // And dragged back, onto the left edge of the first — which is the whole of
    // drag and drop: one command, and the pane leaves the layout it was in.
    let rejoined = layout_call(
        &mut client,
        "layout.move",
        &workspace.id,
        overnight_protocol::v1::LayoutUpdate {
            terminals: vec![second.clone()],
            target: Some(first.id.clone()),
            side: overnight_protocol::v1::SplitSide::Left as i32,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(rejoined.items.len(), 1, "joining leaves one layout");
    let group = &rejoined.items[0];
    let moved = group.panes.iter().find(|p| p.terminal_id == second).expect("moved");
    let anchor = group.panes.iter().find(|p| p.terminal_id == first.id).expect("anchor");
    assert!(moved.left < anchor.left, "dropped left, so it is on the left: {group:?}");
}

#[tokio::test]
async fn moving_a_pane_that_is_alone_in_its_window_keeps_its_identity() {
    // The bug this test exists for: a terminal's id can rest on its WINDOW rather
    // than its pane — which is what every terminal created before identity moved
    // to the pane looks like, and the only arrangement possible while a window
    // held exactly one pane. Joining that pane to another window makes it inherit
    // the destination's options instead, so the id vanishes, the record derives as
    // `lost`, and its workspace as `error`.
    //
    // Two terminals in separate layouts, then one dragged onto the other, is the
    // ordinary drag-and-drop path — so this was reachable by dragging almost any
    // real terminal.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;

    let first = a_terminal(&mut client, &workspace.id, "one").await;
    let second = a_terminal(&mut client, &workspace.id, "two").await;

    let before = layout_call(&mut client, "layout.list", &workspace.id, Default::default()).await;
    assert_eq!(before.items.len(), 2, "two terminals, two layouts");

    let after = layout_call(
        &mut client,
        "layout.move",
        &workspace.id,
        overnight_protocol::v1::LayoutUpdate {
            terminals: vec![second.id.clone()],
            target: Some(first.id.clone()),
            side: overnight_protocol::v1::SplitSide::Bottom as i32,
            ..Default::default()
        },
    )
    .await;

    assert_eq!(after.items.len(), 1, "the emptied window is gone");
    let group = &after.items[0];
    assert_eq!(group.panes.len(), 2, "both terminals are panes of it: {group:?}");
    assert!(
        group.panes.iter().any(|p| p.terminal_id == second.id),
        "the moved terminal still knows which terminal it is: {group:?}"
    );

    // And the record agrees: still running, not lost.
    let mut list = request("terminal.list");
    list.target_resource_id = Some(workspace.id.clone());
    let result = client.call(list).await.expect("terminal.list");
    let Some(result::Value::TerminalList(terminals)) = result.value else { panic!("wrong result") };
    let moved = terminals.items.iter().find(|t| t.id == second.id).expect("moved terminal");
    assert_eq!(
        moved.state(),
        overnight_protocol::v1::TerminalState::Running,
        "a moved terminal is running, not lost"
    );
}
