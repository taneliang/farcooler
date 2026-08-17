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

use farcooler_daemon::{rpc::Rpc, service::Service};
use farcooler_protocol::v1::{ErrorCode, Scope, request, result};
use farcooler_transport::{Client, ClientError, HandshakeConfig, UnixListenerServer, request};

/// A daemon on a private socket with a private database.
///
/// The service is opened at an EXPLICIT directory, never through
/// `FARCOOLER_HOME`. The environment is process-global and these tests run in
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
    /// Held so a test can subscribe to what the RPC layer broadcasts.
    ///
    /// The harness server does not stream events to clients, so a test that
    /// cares whether a mutation ANNOUNCED — rather than merely succeeding —
    /// listens here instead.
    watcher: Arc<farcooler_daemon::watch::Watcher>,
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
    let socket = dir.path().join("farcoolerd.sock");
    let service = Arc::new(Service::open_in(dir.path().to_path_buf()).await.expect("service"));
    let tmux_socket = service.tmux.socket().to_string();
    let server = UnixListenerServer::bind(&socket).expect("bind");

    // The watcher is constructed but not run: these tests are about dispatch,
    // and a sampling loop would make them race a tmux that may not be there.
    let watcher = farcooler_daemon::watch::Watcher::new(service.clone());

    let observed = watcher.clone();
    tokio::spawn(async move {
        // Every connection to this harness gets the scope the test asked for.
        // Nothing here sends a preamble — that is the stdio relay's business,
        // and `a_relayed_session_keeps_its_scope.rs` covers it.
        let _ = server
            .serve(move |_| {
                Some((
                    HandshakeConfig { daemon_version: "test".into(), granted_scope: scope },
                    Factory { service: service.clone(), watcher: watcher.clone(), scope },
                ))
            })
            .await;
    });

    // The listener is bound before serve() is spawned, so a connect cannot race
    // it — but give the task a turn so the first accept is already pending.
    tokio::task::yield_now().await;
    Harness { _dir: dir, socket, _permit: permit, tmux_socket, watcher: observed }
}

#[derive(Clone)]
struct Factory {
    service: Arc<Service>,
    watcher: Arc<farcooler_daemon::watch::Watcher>,
    scope: Scope,
}

impl farcooler_transport::Handler for Factory {
    fn handle(
        &self,
        req: farcooler_protocol::v1::Request,
    ) -> impl std::future::Future<Output = farcooler_protocol::v1::Response> + Send {
        // Nothing waits on this harness's stop signal: the test server has no
        // process to end, and `daemon.shutdown` is exercised where it matters —
        // against a real daemon, by `daemon ensure`.
        let rpc = Rpc::new(
            self.service.clone(),
            self.watcher.clone(),
            self.scope,
            Arc::new(tokio::sync::Notify::new()),
        );
        async move { farcooler_transport::Handler::handle(&rpc, req).await }
    }
}

async fn connect(h: &Harness) -> Client<
    tokio::net::unix::OwnedReadHalf,
    tokio::net::unix::OwnedWriteHalf,
> {
    Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect")
}

/// A registered repository with one empty commit, ready to branch from.
///
/// The `TempDir` comes back with it and must be held: dropping it deletes the
/// repository out from under the daemon, which turns a later assertion into a
/// confusing `worktree_missing` rather than the thing being tested.
async fn registered_repository(
    client: &mut Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>,
) -> (tempfile::TempDir, bytes::Bytes) {
    let dir = tempfile::tempdir().unwrap();
    let repo_path = dir.path().join("demo");
    std::fs::create_dir(&repo_path).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["config", "commit.gpgsign", "false"],
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
        farcooler_protocol::v1::RepositoryRootAdd {
            absolute_path: dir.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    client.call(add).await.expect("repository_root.add");

    let mut register = request("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo_path.to_string_lossy().into_owned(),
        },
    ));
    let result = client.call(register).await.expect("repository.register");
    let Some(result::Value::Repository(repository)) = result.value else {
        panic!("wrong result")
    };
    (dir, repository.id)
}

/// Every terminal the daemon knows about, whichever workspace it belongs to.
///
/// `Workspace` carries no terminals of its own — they are a separate list — so
/// asserting that a worktree opened with one means asking for them.
async fn terminals(
    client: &mut Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>,
) -> Vec<farcooler_protocol::v1::Terminal> {
    let result = client.call(request("terminal.list")).await.expect("terminal.list");
    let Some(result::Value::TerminalList(list)) = result.value else { panic!("wrong result") };
    list.items
}

/// Create a workspace, asking for `preset` in its opening terminal.
async fn create_workspace(
    client: &mut Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>,
    repository: bytes::Bytes,
    task: &str,
    branch: &str,
    preset: &str,
) -> farcooler_protocol::v1::Workspace {
    let mut create = request("workspace.create");
    create.target_resource_id = Some(repository);
    create.payload = Some(request::Payload::WorkspaceCreate(
        farcooler_protocol::v1::WorkspaceCreate {
            task_name: task.into(),
            branch: branch.into(),
            base_revision: "HEAD".into(),
            terminal_preset: preset.into(),
            adopt_existing: false,
        },
    ));
    let result = client.call(create).await.expect("workspace.create");
    let Some(result::Value::Workspace(ws)) = result.value else { panic!("wrong result") };
    ws
}

#[tokio::test]
async fn creating_a_workspace_with_a_preset_opens_a_terminal_in_it() {
    // The review: "When a new worktree is created, it should just open a
    // terminal as well." Done here rather than in each client, because a
    // worktree with nothing running in it is a directory, and a rule
    // implemented three times is a rule three clients can disagree about.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let (_dir, repository) = registered_repository(&mut client).await;

    let ws = create_workspace(&mut client, repository, "add auth", "feat/add-auth", "shell").await;

    let opened: Vec<_> =
        terminals(&mut client).await.into_iter().filter(|t| t.workspace_id == ws.id).collect();
    assert_eq!(opened.len(), 1, "the worktree came with a terminal");
    assert_eq!(opened[0].title, "shell", "titled after the preset, as the CLI does");
}

#[tokio::test]
async fn creating_a_workspace_with_no_preset_opens_nothing() {
    // Empty means none, which is what keeps every existing caller's behavior
    // unchanged and gives `--no-terminal` something to mean: the task flow
    // creates its own agent terminal a moment later and must not also get a
    // shell it never asked for.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let (_dir, repository) = registered_repository(&mut client).await;

    let ws = create_workspace(&mut client, repository, "add auth", "feat/add-auth", "").await;
    assert!(
        terminals(&mut client).await.iter().all(|t| t.workspace_id != ws.id),
        "nothing was asked for, so nothing was opened"
    );
}

#[tokio::test]
async fn removing_a_worktree_closes_the_terminals_in_it() {
    // The review: "Deleting a worktree should just automatically close existing
    // terminals." It used to refuse with RunningProcesses, and the Mac app
    // rendered that as "Stop the terminals in this workspace before removing
    // it" — telling the user to go and do by hand the thing they had asked for.
    //
    // The worktree here is clean, so no typed confirmation is needed. That a
    // DIRTY one still demands its name is covered by
    // `service::remove_worktree_tests::a_dirty_worktree_demands_the_name`.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let (_dir, repository) = registered_repository(&mut client).await;

    let ws = create_workspace(&mut client, repository, "doomed", "feat/doomed", "shell").await;
    assert_eq!(
        terminals(&mut client).await.iter().filter(|t| t.workspace_id == ws.id).count(),
        1,
        "there has to be something to close for this to prove anything"
    );

    let mut remove = request("workspace.remove_worktree");
    remove.target_resource_id = Some(ws.id.clone());
    client.call(remove).await.expect("removal closes the terminals rather than refusing");

    assert!(
        terminals(&mut client).await.iter().all(|t| t.workspace_id != ws.id),
        "the terminal records went with the worktree"
    );
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
    assert!(v.protocol_versions.contains(&farcooler_protocol::PROTOCOL_VERSION));
    assert!(!v.capabilities.is_empty());
}

#[tokio::test]
async fn host_get_carries_this_machines_settings() {
    // The client applies the branch prefix — the composer shows you the branch
    // it is about to create, so a daemon-side prefix would make that preview a
    // lie — which means the client has to be told what it is. It rides
    // `host.get` rather than a call of its own because every client already
    // makes this one.
    //
    // Asserts the field is POPULATED, not what it contains. `load_branch_prefix`
    // reads the real `$HOME`, and this harness deliberately avoids
    // process-global environment (see `start`) because these tests run in
    // parallel. What the value should be for a given file is covered
    // hermetically by `config::branch_prefix_from`'s own tests.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let result = client.call(request("host.get")).await.expect("host.get");
    let Some(result::Value::Host(host)) = result.value else {
        panic!("expected a Host, got {:?}", result.value);
    };
    assert!(host.settings.is_some(), "a client cannot apply a prefix it was never sent");
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
        farcooler_protocol::v1::RepositoryRegister { relative_path: "/tmp".into() },
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

    match client.call(request("workspace.hide")).await {
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
        farcooler_protocol::v1::RepositoryRootAdd {
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
        farcooler_protocol::v1::RepositoryRootAdd {
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
        farcooler_protocol::v1::RepositoryRootAdd {
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
        farcooler_protocol::v1::TypedConfirmation { typed_confirmation: "nope".into() },
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
        farcooler_protocol::v1::TypedConfirmation { typed_confirmation: name },
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
    // strand a row pointing at a tree Far Cooler may no longer touch.
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
        farcooler_protocol::v1::RepositoryRootAdd {
            absolute_path: dir.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    let result = client.call(add).await.expect("add");
    let Some(result::Value::RepositoryRoot(root)) = result.value else { panic!("wrong result") };

    let mut register = request("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo_path.to_string_lossy().into_owned(),
        },
    ));
    client.call(register).await.expect("register");

    let mut remove = request("repository_root.remove");
    remove.target_resource_id = Some(root.id.clone());
    remove.payload = Some(request::Payload::TypedConfirmation(
        farcooler_protocol::v1::TypedConfirmation { typed_confirmation: name },
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
    add.payload = Some(request::Payload::RepositoryRootAdd(
        farcooler_protocol::v1::RepositoryRootAdd {
            absolute_path: dir.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    let result = client.call(add).await.expect("add");
    let Some(result::Value::RepositoryRoot(root)) = result.value else { panic!("wrong result") };

    let mut register = request("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo_path.to_string_lossy().into_owned(),
        },
    ));
    let result = client.call(register).await.expect("register");
    let Some(result::Value::Repository(repository)) = result.value else { panic!("wrong result") };

    let mut create = request("workspace.create");
    create.target_resource_id = Some(repository.id.clone());
    create.payload = Some(request::Payload::WorkspaceCreate(
        farcooler_protocol::v1::WorkspaceCreate {
            task_name: "a task".into(),
            branch: "feat/x".into(),
            base_revision: "HEAD".into(),
            terminal_preset: String::new(),
            adopt_existing: false,
        },
    ));
    client.call(create).await.expect("workspace.create");

    let mut remove = request("repository_root.remove");
    remove.target_resource_id = Some(root.id.clone());
    remove.payload = Some(request::Payload::TypedConfirmation(
        farcooler_protocol::v1::TypedConfirmation { typed_confirmation: name },
    ));
    match client.call(remove).await {
        Err(ClientError::Daemon { code, retryable, .. }) => {
            assert_eq!(code, ErrorCode::WorkspacesExist as i32);
            assert!(!retryable, "retrying cannot help; the user has to act");
        }
        other => panic!("expected WORKSPACES_EXIST, got {other:?}"),
    }
}

/// Regression for a bug the first cut of the reconciler introduced: since
/// registering a repository now auto-adopts its main checkout as a workspace
/// (see `crates/daemon/src/reconcile.rs`), that row's `is_main_checkout` flag
/// exempts it from `WorkspacesExist` -- otherwise no root could ever be
/// removed again, because `workspace.remove_worktree` refuses the main
/// checkout on purpose. But the row still existed, and `terminals.workspace_id`
/// carries a foreign key with no `ON DELETE CASCADE`, so deleting the
/// repository underneath a surviving terminal row failed with a `ResourceConflict`
/// ("resource version is stale") -- a misleading error for what was actually a
/// constraint violation, and one that left the root permanently unremovable.
/// A dead terminal record -- created, then stopped -- must not be able to do
/// that: `remove_root` has to clear it along with everything else.
#[tokio::test]
async fn removing_a_root_survives_a_stopped_terminal_in_the_main_checkout() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let dir = tempfile::tempdir().unwrap();
    let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
    let repo_path = dir.path().join("demo");
    std::fs::create_dir(&repo_path).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo_path).status().unwrap();
    }

    let mut add = request("repository_root.add");
    add.payload = Some(request::Payload::RepositoryRootAdd(
        farcooler_protocol::v1::RepositoryRootAdd {
            absolute_path: dir.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    let result = client.call(add).await.expect("add");
    let Some(result::Value::RepositoryRoot(root)) = result.value else { panic!("wrong result") };

    let mut register = request("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo_path.to_string_lossy().into_owned(),
        },
    ));
    let result = client.call(register).await.expect("register");
    let Some(result::Value::Repository(repository)) = result.value else { panic!("wrong result") };

    // `workspace.main` is gone -- the reconciler adopted the main checkout
    // the moment `repository.register` ran, synchronously. Find that row by
    // its worktree path rather than by a method that no longer exists; the
    // wire `Workspace` carries no `is_main_checkout` flag, only the path.
    let result = client.call(request("workspace.list")).await.expect("workspace.list");
    let Some(result::Value::WorkspaceList(list)) = result.value else { panic!("wrong result") };
    let canonical_repo_path = repo_path.canonicalize().unwrap().to_string_lossy().into_owned();
    let workspace = list
        .items
        .into_iter()
        .find(|w| {
            w.worktree_path.as_deref() == Some(canonical_repo_path.as_str())
                && w.repository_id == repository.id
        })
        .expect("the reconciler must have adopted the main checkout");

    let terminal = a_terminal(&mut client, &workspace.id, "shell").await;
    let mut stop = request("terminal.stop");
    stop.target_resource_id = Some(terminal.id.clone());
    client.call(stop).await.expect("terminal.stop");

    let mut remove = request("repository_root.remove");
    remove.target_resource_id = Some(root.id.clone());
    remove.payload = Some(request::Payload::TypedConfirmation(
        farcooler_protocol::v1::TypedConfirmation { typed_confirmation: name },
    ));
    client.call(remove).await.expect("a stopped terminal's record must not block this");

    let result = client.call(request("repository.list")).await.expect("list");
    let Some(result::Value::RepositoryList(list)) = result.value else { panic!("wrong result") };
    assert!(
        list.items.is_empty(),
        "the repository, its main checkout and its terminal all go with the root"
    );
}

/// The other half of the fix above: a LIVE terminal is still somebody's work,
/// and must refuse the removal cleanly rather than either deleting its only
/// record or failing with the same unactionable `ResourceConflict` a dead one
/// used to produce. `RunningProcesses` is the existing vocabulary for exactly
/// this situation in `remove_terminal` and `remove_worktree`.
#[tokio::test]
async fn removing_a_root_is_refused_while_a_terminal_is_actually_running() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let dir = tempfile::tempdir().unwrap();
    let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
    let repo_path = dir.path().join("demo");
    std::fs::create_dir(&repo_path).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo_path).status().unwrap();
    }

    let mut add = request("repository_root.add");
    add.payload = Some(request::Payload::RepositoryRootAdd(
        farcooler_protocol::v1::RepositoryRootAdd {
            absolute_path: dir.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    let result = client.call(add).await.expect("add");
    let Some(result::Value::RepositoryRoot(root)) = result.value else { panic!("wrong result") };

    let mut register = request("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo_path.to_string_lossy().into_owned(),
        },
    ));
    let result = client.call(register).await.expect("register");
    let Some(result::Value::Repository(repository)) = result.value else { panic!("wrong result") };

    // `workspace.main` is gone -- the reconciler adopted the main checkout
    // the moment `repository.register` ran, synchronously. Find that row by
    // its worktree path rather than by a method that no longer exists; the
    // wire `Workspace` carries no `is_main_checkout` flag, only the path.
    let result = client.call(request("workspace.list")).await.expect("workspace.list");
    let Some(result::Value::WorkspaceList(list)) = result.value else { panic!("wrong result") };
    let canonical_repo_path = repo_path.canonicalize().unwrap().to_string_lossy().into_owned();
    let workspace = list
        .items
        .into_iter()
        .find(|w| {
            w.worktree_path.as_deref() == Some(canonical_repo_path.as_str())
                && w.repository_id == repository.id
        })
        .expect("the reconciler must have adopted the main checkout");

    // Left running -- never stopped.
    let _terminal = a_terminal(&mut client, &workspace.id, "shell").await;

    let mut remove = request("repository_root.remove");
    remove.target_resource_id = Some(root.id.clone());
    remove.payload = Some(request::Payload::TypedConfirmation(
        farcooler_protocol::v1::TypedConfirmation { typed_confirmation: name },
    ));
    match client.call(remove).await {
        Err(ClientError::Daemon { code, retryable, .. }) => {
            assert_eq!(code, ErrorCode::RunningProcesses as i32);
            assert!(!retryable, "stopping the terminal is the remedy, not a retry");
        }
        other => panic!("expected RUNNING_PROCESSES, got {other:?}"),
    }

    // Still there afterwards: a refusal must not have half-applied anything.
    let result = client.call(request("repository.list")).await.expect("list");
    let Some(result::Value::RepositoryList(list)) = result.value else { panic!("wrong result") };
    assert_eq!(list.items.len(), 1, "a refused removal changes nothing");
}

/// A workspace, over the wire, ready to have things tiled in it.
async fn a_workspace(
    client: &mut Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>,
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
    add.payload = Some(request::Payload::RepositoryRootAdd(
        farcooler_protocol::v1::RepositoryRootAdd {
            absolute_path: dir.to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    client.call(add).await.expect("add root");

    let mut register = request("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo_path.to_string_lossy().into_owned(),
        },
    ));
    let result = client.call(register).await.expect("register");
    let Some(result::Value::Repository(repository)) = result.value else { panic!("wrong result") };

    let mut create = request("workspace.create");
    create.target_resource_id = Some(repository.id.clone());
    create.payload = Some(request::Payload::WorkspaceCreate(
        farcooler_protocol::v1::WorkspaceCreate {
            task_name: "tiling".into(),
            branch: "feat/tiling".into(),
            base_revision: "HEAD".into(),
            terminal_preset: String::new(),
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
    update: farcooler_protocol::v1::LayoutUpdate,
) -> farcooler_protocol::v1::PaneGroupList {
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
        farcooler_protocol::v1::TerminalCreate {
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
        farcooler_protocol::v1::LayoutUpdate {
            target: Some(one.id.clone()),
            side: farcooler_protocol::v1::SplitSide::Right as i32,
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
        farcooler_protocol::v1::LayoutUpdate { zoom: Some(first.clone()), ..Default::default() },
    )
    .await;
    let pane = zoomed.items[0].panes.iter().find(|p| p.terminal_id == first).expect("first");
    assert!(pane.zoomed && pane.focused);

    let moved = layout_call(
        &mut client,
        "layout.focus",
        &workspace.id,
        farcooler_protocol::v1::LayoutUpdate { step: Some(1), ..Default::default() },
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
) -> farcooler_protocol::v1::Terminal {
    let mut create = request("terminal.create");
    create.target_resource_id = Some(workspace.clone());
    create.payload = Some(request::Payload::TerminalCreate(
        farcooler_protocol::v1::TerminalCreate {
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
        farcooler_protocol::v1::LayoutUpdate {
            columns: Some(120),
            rows: Some(40),
            ..Default::default()
        },
    ));
    client.call(viewport).await.expect("layout.viewport");
    terminal
}

fn split(target: &bytes::Bytes, side: farcooler_protocol::v1::SplitSide) -> farcooler_protocol::v1::LayoutUpdate {
    farcooler_protocol::v1::LayoutUpdate {
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
        split(&first.id, farcooler_protocol::v1::SplitSide::Right),
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
        split(&first.id, farcooler_protocol::v1::SplitSide::Left),
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
        split(&first.id, farcooler_protocol::v1::SplitSide::Bottom),
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
    // tmux keeps the tree and Far Cooler does not.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let first = a_terminal(&mut client, &workspace.id, "one").await;

    let after = layout_call(
        &mut client,
        "layout.split",
        &workspace.id,
        split(&first.id, farcooler_protocol::v1::SplitSide::Right),
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
        split(&second, farcooler_protocol::v1::SplitSide::Bottom),
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
        split(&first.id, farcooler_protocol::v1::SplitSide::Right),
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
        farcooler_protocol::v1::LayoutUpdate {
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
        farcooler_protocol::v1::LayoutUpdate {
            terminals: vec![second.clone()],
            target: Some(first.id.clone()),
            side: farcooler_protocol::v1::SplitSide::Left as i32,
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
        farcooler_protocol::v1::LayoutUpdate {
            terminals: vec![second.id.clone()],
            target: Some(first.id.clone()),
            side: farcooler_protocol::v1::SplitSide::Bottom as i32,
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
        farcooler_protocol::v1::TerminalState::Running,
        "a moved terminal is running, not lost"
    );
}

#[tokio::test]
async fn a_terminal_reports_its_pane_mode_to_a_client() {
    // Clients render pane mode; they must never infer it from a command line.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let terminal = a_terminal(&mut client, &workspace.id, "claude").await;

    assert_eq!(terminal.pane_mode, farcooler_protocol::v1::PaneMode::Terminal as i32);
}

#[tokio::test]
async fn a_pane_split_with_the_changes_preset_is_a_changes_pane() {
    // The whole reason the diff stopped being a client-side tile: it is a pane
    // like any other, made by the same verb, and it says so on the wire. A
    // client that had to infer it from a command line would be back to guessing
    // what a pane is for, which is what `pane_mode` exists to end.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let original = a_terminal(&mut client, &workspace.id, "one").await;

    let list = layout_call(
        &mut client,
        "layout.split",
        &workspace.id,
        farcooler_protocol::v1::LayoutUpdate {
            target: Some(original.id.clone()),
            side: farcooler_protocol::v1::SplitSide::Right as i32,
            command_preset: "changes".into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(list.items.len(), 1, "the diff joins the layout, it does not open its own");
    assert_eq!(list.items[0].panes.len(), 2, "two panes: the terminal and the diff");

    let mut terminals = request("terminal.list");
    terminals.target_resource_id = Some(workspace.id.clone());
    let result = client.call(terminals).await.expect("terminal.list");
    let Some(result::Value::TerminalList(terminals)) = result.value else { panic!("wrong result") };
    let changes = terminals
        .items
        .iter()
        .find(|t| t.id != original.id)
        .expect("the pane the split created");
    assert_eq!(
        changes.pane_mode,
        farcooler_protocol::v1::PaneMode::Changes as i32,
        "the preset it was created with is what the pane IS"
    );
    assert_eq!(
        changes.state(),
        farcooler_protocol::v1::TerminalState::Running,
        "something is holding the rectangle, or tmux would have collapsed it \
         (running: {:?}, exit: {:?}) — a changes pane runs this channel's CLI by \
         name, so a machine without it on PATH gets a pane that dies instantly",
        changes.current_command,
        changes.exit_status,
    );

    // And it is not a posture that can be switched off, in either direction:
    // there is no TUI underneath a diff to go back to.
    let mut switch = request("terminal.set_pane_mode");
    switch.payload = Some(request::Payload::SetPaneMode(farcooler_protocol::v1::SetPaneMode {
        terminal_id: changes.id.clone(),
        pane_mode: farcooler_protocol::v1::PaneMode::Terminal as i32,
        force: false,
    }));
    assert!(
        client.call(switch).await.is_err(),
        "switching a changes pane to a shell leaves a pane every client still calls Changes"
    );

    let mut into = request("terminal.set_pane_mode");
    into.payload = Some(request::Payload::SetPaneMode(farcooler_protocol::v1::SetPaneMode {
        terminal_id: original.id.clone(),
        pane_mode: farcooler_protocol::v1::PaneMode::Changes as i32,
        force: false,
    }));
    assert!(
        client.call(into).await.is_err(),
        "a diff is a pane you open, not a mode that respawns whatever was running"
    );
}

/// The real case `hiding_does_not_consult_terminal_state`
/// (`crates/daemon/src/service.rs`) cannot manufacture: a terminal whose
/// derived state is genuinely `Running`, which needs a live tmux pane behind
/// it rather than just a `TerminalIntent::Running` row with nothing backing
/// it. `a_terminal` spawns a real pane, so this is the scenario the old
/// `archive_workspace` guard (`== TerminalState::Running`) actually refused,
/// and hiding must allow it unconditionally.
#[tokio::test]
async fn hiding_succeeds_while_a_terminal_is_genuinely_running() {
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let terminal = a_terminal(&mut client, &workspace.id, "agent").await;

    let mut list = request("terminal.list");
    list.target_resource_id = Some(workspace.id.clone());
    let result = client.call(list).await.expect("terminal.list");
    let Some(result::Value::TerminalList(terminals)) = result.value else { panic!("wrong result") };
    let live = terminals.items.iter().find(|t| t.id == terminal.id).expect("its own terminal");
    assert_eq!(
        live.state(),
        farcooler_protocol::v1::TerminalState::Running,
        "this test needs a genuinely running terminal to mean anything: {live:?}"
    );

    let mut hide = request("workspace.hide");
    hide.target_resource_id = Some(workspace.id.clone());
    let result =
        client.call(hide).await.expect("workspace.hide must not refuse a running terminal");
    let Some(result::Value::Workspace(hidden)) = result.value else { panic!("wrong result") };
    assert_eq!(hidden.state(), farcooler_protocol::v1::WorkspaceState::Hidden);

    let mut unhide = request("workspace.unhide");
    unhide.target_resource_id = Some(workspace.id.clone());
    let result = client.call(unhide).await.expect("workspace.unhide");
    let Some(result::Value::Workspace(back)) = result.value else { panic!("wrong result") };
    assert_eq!(
        back.state(),
        farcooler_protocol::v1::WorkspaceState::Active,
        "the terminal is still running, so unhiding must not hide that"
    );
}

#[tokio::test]
async fn an_agent_subscribe_from_a_cursor_is_accepted() {
    // A client attaches to a PANE, not to a session: subscribing before any
    // agent has ever run there must be accepted, with nothing to replay,
    // rather than refused.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let workspace = a_workspace(&mut client, dir.path()).await;
    let terminal = a_terminal(&mut client, &workspace.id, "claude").await;

    let mut req = request("terminal.agent_subscribe");
    req.payload = Some(request::Payload::AgentSubscribe(farcooler_protocol::v1::AgentSubscribe {
        epoch: 0,
        terminal_id: terminal.id.clone(),
        from_seq: 0,
    }));
    let result = client.call(req).await;
    assert!(result.is_ok(), "subscribe must be accepted even before a session exists: {result:?}");

    let Some(result::Value::AgentEventBatch(batch)) = result.unwrap().value else {
        panic!("wrong result")
    };
    assert!(batch.events.is_empty(), "nothing has happened on this pane yet");
}

#[tokio::test]
async fn switching_a_panes_mode_tells_every_client_and_not_just_the_caller() {
    // A pane-mode change alters nothing the watcher samples — same activity,
    // same command, same liveness — so the runtime poll has nothing to notice
    // and never announces on its own. The RPC reply reaches only the client
    // that asked.
    //
    // Without an explicit announce, every OTHER client kept rendering the pane
    // in its old mode until something unrelated happened to it, or until a
    // human found "Reload Fleet". A pane switched to agent mode from the CLI
    // sat there as a terminal while its agent talked into a view nobody was
    // showing.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let dir = h._dir.path().to_path_buf();
    let workspace = a_workspace(&mut client, &dir).await;
    let terminal = a_terminal(&mut client, &workspace.id, "pane").await;

    // Subscribed AFTER the setup above, so the fleet traffic that creating a
    // workspace and a terminal legitimately produces cannot be mistaken for
    // the announce this test is about.
    let mut events = h.watcher.subscribe();

    let mut req = request("terminal.set_pane_mode");
    req.payload = Some(request::Payload::SetPaneMode(farcooler_protocol::v1::SetPaneMode {
        terminal_id: terminal.id.clone(),
        // Terminal rather than Agent: switching TO agent demands a pane
        // already running one, and this is about the announce, not about what
        // a mode change is allowed to do.
        pane_mode: farcooler_protocol::v1::PaneMode::Terminal as i32,
        force: false,
    }));
    client.call(req).await.expect("set_pane_mode");

    let announced = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if matches!(
                        event.payload,
                        Some(farcooler_protocol::v1::event::Payload::FleetChanged(_))
                    ) {
                        return true;
                    }
                }
                Err(_) => return false,
            }
        }
    })
    .await;

    assert_eq!(
        announced,
        Ok(true),
        "a pane-mode change reached only the caller; every other client stayed wrong"
    );
}

/// A minimal but real PNG: an 8-byte signature and enough filler to be worth
/// chunking. Nothing decodes it — the daemon sniffs its header and the test
/// compares its bytes.
fn png(len: usize) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.resize(len, 0x5a);
    b
}

#[tokio::test]
async fn an_image_pasted_in_chunks_lands_as_one_file_and_is_typed_into_the_pane() {
    // The whole feature end to end: bytes cross a real socket in several
    // chunks, become one file on the host, and the pane is told where it is.
    // A unit test can prove the assembly; only this can prove that a client
    // pasting an image ends up with an agent able to open it.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let (_repo_dir, repository) = registered_repository(&mut client).await;
    let ws = create_workspace(&mut client, repository, "look at this", "feat/look", "shell").await;
    let terminal = terminals(&mut client)
        .await
        .into_iter()
        .find(|t| t.workspace_id == ws.id)
        .expect("the workspace opened a terminal");

    // Three chunks, so the offset bookkeeping is exercised rather than assumed.
    let image = png(300);
    let transfer = farcooler_protocol::ids::new_id();
    let mut stored = 0u64;
    let mut landed = None;
    for chunk in image.chunks(128) {
        let mut req = request("terminal.paste_file");
        req.target_resource_id = Some(terminal.id.clone());
        req.payload = Some(request::Payload::TerminalFilePut(
            farcooler_protocol::v1::TerminalFilePut {
                terminal_id: terminal.id.clone(),
                transfer_id: transfer.clone(),
                mime: "image/png".into(),
                name: "shot.png".into(),
                total_size: image.len() as u64,
                offset: stored,
                chunk: bytes::Bytes::copy_from_slice(chunk),
            },
        ));
        let result = client.call(req).await.expect("terminal.paste_file");
        let Some(result::Value::TerminalFilePut(r)) = result.value else { panic!("wrong result") };
        stored = r.stored;
        landed = r.path.or(landed);
    }

    let path = landed.expect("the last chunk named the file");
    assert_eq!(stored, image.len() as u64);
    assert_eq!(std::fs::read(&path).expect("the file exists"), image, "the bytes survived chunking");
    assert!(
        path.starts_with(h._dir.path().to_str().expect("utf-8")),
        "a paste belongs to the daemon's own directory, not the user's real one: {path}"
    );

    // And the pane was told. The shell echoes what was typed into it, so the
    // path appears on the screen without anything being run.
    let quoted = farcooler_daemon::pastes::quote_for_paste(&path);
    let seen = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let mut req = request("terminal.screen");
            req.target_resource_id = Some(terminal.id.clone());
            req.payload = Some(request::Payload::TerminalScreenRequest(
                farcooler_protocol::v1::TerminalScreenRequest { known_revision: 0 },
            ));
            if let Ok(result) = client.call(req).await {
                if let Some(result::Value::TerminalScreen(s)) = result.value {
                    let text = String::from_utf8_lossy(&s.contents).to_string();
                    // The shell may wrap the line, so compare on the filename
                    // rather than the whole path.
                    let name = std::path::Path::new(&path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .expect("a name");
                    if text.contains(name) {
                        return true;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    assert_eq!(seen, Ok(true), "the path was never typed into the pane (quoted: {quoted})");
}

#[tokio::test]
async fn a_paste_whose_offset_does_not_match_is_refused_over_the_wire() {
    // The unit test proves the rule; this proves it survives the dispatch
    // layer as a refusal rather than a dropped connection.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let (_repo_dir, repository) = registered_repository(&mut client).await;
    let ws = create_workspace(&mut client, repository, "look", "feat/look2", "shell").await;
    let terminal = terminals(&mut client)
        .await
        .into_iter()
        .find(|t| t.workspace_id == ws.id)
        .expect("terminal");

    let image = png(300);
    let transfer = farcooler_protocol::ids::new_id();
    let mut first = request("terminal.paste_file");
    first.target_resource_id = Some(terminal.id.clone());
    first.payload = Some(request::Payload::TerminalFilePut(
        farcooler_protocol::v1::TerminalFilePut {
            terminal_id: terminal.id.clone(),
            transfer_id: transfer.clone(),
            mime: "image/png".into(),
            name: "shot.png".into(),
            total_size: image.len() as u64,
            offset: 0,
            chunk: bytes::Bytes::copy_from_slice(&image[..128]),
        },
    ));
    client.call(first).await.expect("the first chunk is accepted");

    // A gap. Accepting it would produce a file that sniffs as a PNG and
    // decodes to garbage.
    let mut gap = request("terminal.paste_file");
    gap.target_resource_id = Some(terminal.id.clone());
    gap.payload = Some(request::Payload::TerminalFilePut(
        farcooler_protocol::v1::TerminalFilePut {
            terminal_id: terminal.id.clone(),
            transfer_id: transfer.clone(),
            mime: "image/png".into(),
            name: "shot.png".into(),
            total_size: image.len() as u64,
            offset: 256,
            chunk: bytes::Bytes::copy_from_slice(&image[256..]),
        },
    ));
    let err = client.call(gap).await.expect_err("refused");
    assert!(
        matches!(err, ClientError::Daemon { code, .. } if code == ErrorCode::InvalidArgument as i32),
        "expected a specific refusal, got {err:?}"
    );
}

#[tokio::test]
async fn the_handshake_says_what_this_machine_can_do() {
    // The whole drift mechanism: a client learns what a machine supports BY
    // NAME, in the handshake, before its first request. Nothing here compares
    // version strings, which is the point — `PROTOCOL_VERSION` is reserved for
    // a framing break and is expected never to move.
    let h = start(Scope::HostAdmin).await;
    let client = connect(&h).await;

    let advertised = &client.server_hello().capabilities;
    assert!(!advertised.is_empty(), "a daemon that advertises nothing can be asked for nothing");
    for floor in
        [farcooler_protocol::capability::WORKSPACES, farcooler_protocol::capability::TERMINALS]
    {
        assert!(advertised.iter().any(|c| c == floor), "{floor} is the floor; every daemon has it");
    }
}

#[tokio::test]
async fn the_two_capability_lists_cannot_disagree() {
    // `ServerHello` and `daemon.version` answer the same question, so they are
    // built from the same table. A second copy is the drift the table exists to
    // prevent.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;
    let hello = client.server_hello().capabilities.clone();

    let value = client.call(request("daemon.version")).await.expect("version");
    let Some(farcooler_protocol::v1::result::Value::DaemonVersion(v)) = value.value else {
        panic!("expected a daemon version");
    };
    assert_eq!(hello, v.capabilities);
}

#[tokio::test]
async fn a_capability_this_machine_lacks_is_refused_before_anything_runs() {
    // The case negotiation alone cannot catch: a NEW FIELD on an existing
    // payload, which an older daemon drops as an unknown proto3 field while
    // doing the old thing. The client names the capability that field belongs
    // to, and the refusal replaces the silence.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let mut req = request("workspace.list");
    req.required_capabilities = vec!["time-travel".into()];

    match client.call(req).await {
        Err(ClientError::Daemon { code, retryable, .. }) => {
            assert_eq!(code, ErrorCode::CapabilityUnsupported as i32);
            assert!(!retryable, "no amount of retrying updates the other side");
        }
        other => panic!("expected a capability refusal, got {other:?}"),
    }

    // And the connection survives it, so a client can degrade and carry on
    // rather than reconnecting.
    assert!(client.call(request("daemon.version")).await.is_ok());
}

#[tokio::test]
async fn a_capability_this_machine_has_is_not_refused() {
    // The other half: naming a capability that IS advertised must change
    // nothing. A check that refused everything would pass the test above while
    // breaking every real call.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    let mut req = request("workspace.list");
    req.required_capabilities = vec![farcooler_protocol::capability::WORKSPACES.into()];
    assert!(client.call(req).await.is_ok());
}

#[tokio::test]
async fn a_method_this_daemon_never_heard_of_says_so_precisely() {
    // Not NOT_FOUND, which this used to be. To a newer app asking for a feature
    // this build predates, "no such method" and "no such workspace" were the
    // same code — so it could neither dim the control nor say anything a person
    // could act on.
    let h = start(Scope::HostAdmin).await;
    let mut client = connect(&h).await;

    match client.call(request("workspace.teleport")).await {
        Err(ClientError::Daemon { code, message, .. }) => {
            assert_eq!(code, ErrorCode::CapabilityUnsupported as i32);
            assert!(
                message.to_lowercase().contains("older"),
                "the message has to tell a person what to do: {message}"
            );
        }
        other => panic!("expected a capability refusal, got {other:?}"),
    }
}
