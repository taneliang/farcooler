//! The mobile client core, against a real daemon.
//!
//! This is the code iOS and Android will run, so it is worth proving against
//! the actual daemon binary rather than a stub: the JSON shapes a phone decodes
//! have to match what a host actually sends, and a mock would agree with
//! whatever this file believed on the day it was written.
//!
//! The transport here is the local socket rather than SSH, because SSH is not
//! what can go wrong at this layer — it is a byte pipe, and `ssh.rs` puts an
//! `AsyncRead`/`AsyncWrite` on either end of it. What is under test is
//! everything above that: the handshake, the calls, and the shapes.

use std::path::PathBuf;

use farcooler_client::session::Session;

/// Where cargo put `farcoolerd`.
///
/// `CARGO_BIN_EXE_*` only covers binaries in the same crate, and the daemon
/// lives in another one. The test executable sits in `target/<profile>/deps/`,
/// so the binary is two levels up — which is a cargo layout detail, but a
/// stable one, and the alternative is a dev-dependency cycle between these two
/// crates.
fn daemon_binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("test executable path");
    path.pop(); // deps/
    path.pop(); // <profile>/
    path.push("farcoolerd");
    assert!(
        path.is_file(),
        "no farcoolerd at {} — run `cargo build -p farcooler-daemon` first",
        path.display()
    );
    path
}

/// A daemon on a private socket with a private database.
struct Daemon {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    process: std::process::Child,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

async fn start() -> Daemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("farcoolerd.sock");

    let process = std::process::Command::new(daemon_binary())
        .env("FARCOOLER_HOME", dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn farcoolerd");

    // Wait for the socket rather than sleeping a fixed amount: a slow machine
    // would otherwise make this flaky and a fast one would waste the time.
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&socket).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    Daemon { _dir: dir, socket, process }
}

#[tokio::test]
async fn a_client_connects_and_learns_the_daemon_version() {
    let daemon = start().await;
    let session = Session::connect_local(&daemon.socket).await.expect("connect");
    assert!(!session.daemon_version().is_empty());
}

#[tokio::test]
async fn the_fleet_shape_is_the_one_a_phone_decodes() {
    // These key names are the app's contract. Changing one breaks a client that
    // cannot be updated at the same moment, which is the whole hazard of having
    // a phone in the picture.
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let fleet = session.fleet().await.expect("fleet");
    assert!(fleet.get("runtime_healthy").is_some_and(|v| v.is_boolean()));
    assert!(fleet.get("live_panes").is_some_and(|v| v.is_number()));
    assert!(fleet.get("workspaces").is_some_and(|v| v.is_array()));
}

#[tokio::test]
async fn a_workspace_created_through_the_client_comes_back_in_the_fleet() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    // A real repository, because workspace creation makes a real worktree.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }

    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    assert_eq!(repositories.len(), 1, "the repository must be visible to a second session");

    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let workspace = session
        .create_workspace(repository, "phone task", "feat/phone", "HEAD")
        .await
        .expect("create_workspace");
    assert_eq!(workspace.task_name, "phone task");

    let fleet = session.fleet().await.expect("fleet");
    let workspaces = fleet["workspaces"].as_array().unwrap();
    // Two: the one just created, plus the main checkout that registering the
    // repository adopts automatically.
    assert_eq!(workspaces.len(), 2);
    let created =
        workspaces.iter().find(|w| w["task"] == "phone task").expect("created workspace present");
    assert_eq!(created["branch"], "feat/phone");
    // Derived, never stored — and a fresh workspace with no terminals is ready.
    assert_eq!(created["state"], "ready");
    assert!(created["terminals"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn hiding_and_unhiding_round_trips_through_the_client() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let workspace = session
        .create_workspace(repository, "reversible", "feat/rev", "HEAD")
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&workspace.id);

    session.hide_workspace(id).await.expect("hide");
    let fleet = session.fleet().await.expect("fleet");
    let workspaces = fleet["workspaces"].as_array().unwrap();
    let reversible =
        workspaces.iter().find(|w| w["task"] == "reversible").expect("its own workspace present");
    assert_eq!(reversible["state"], "hidden");

    session.unhide_workspace(id).await.expect("unhide");
    let fleet = session.fleet().await.expect("fleet");
    let workspaces = fleet["workspaces"].as_array().unwrap();
    let reversible =
        workspaces.iter().find(|w| w["task"] == "reversible").expect("its own workspace present");
    assert_eq!(reversible["state"], "ready");
}

#[tokio::test]
async fn subscribing_to_a_terminal_with_no_agent_session_is_empty_not_an_error() {
    // A client attaches to a PANE, not to a session. A terminal that has never
    // been in agent mode must answer "nothing yet" rather than fail, or the
    // UI cannot open a chat view before the first turn.
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let workspace = session
        .create_workspace(repository, "agent test", "feat/agent-empty", "HEAD")
        .await
        .expect("create_workspace");
    let workspace_id = farcooler_client::session::uuid_of(&workspace.id);

    let terminal = session
        .create_terminal(workspace_id, "shell", "shell", false)
        .await
        .expect("create_terminal");
    let terminal_id = farcooler_client::session::uuid_of(&terminal.id);

    let batch = session.agent_subscribe(terminal_id, 0, 0).await.expect("subscribe succeeds");
    assert!(batch.events.is_empty());
}

#[tokio::test]
async fn a_failed_call_arrives_as_an_error_not_a_dropped_session() {
    // A phone on a train needs the session to survive a refusal; reconnecting
    // over SSH for every rejected request would be unusable.
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let missing = uuid::Uuid::now_v7();
    assert!(session.hide_workspace(missing).await.is_err());

    // Still usable.
    assert!(session.fleet().await.is_ok());
}

/// Add a root and register a repository, over a throwaway session.
async fn register_root_and_repository(
    socket: &std::path::Path,
    root: &std::path::Path,
    repo: &std::path::Path,
) {
    use farcooler_protocol::v1::request::Payload;

    let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    let (read, write) = stream.into_split();
    let mut client = farcooler_transport::Client::over(
        Box::new(read) as Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        Box::new(write) as Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        "test",
        "0.0.0",
    )
    .await
    .unwrap();

    let mut add = farcooler_transport::request("repository_root.add");
    add.payload = Some(Payload::RepositoryRootAdd(farcooler_protocol::v1::RepositoryRootAdd {
        absolute_path: root.to_string_lossy().into_owned(),
        typed_confirmation: String::new(),
    }));
    client.call(add).await.expect("root add");

    let mut register = farcooler_transport::request("repository.register");
    register.payload = Some(Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo.to_string_lossy().into_owned(),
        },
    ));
    client.call(register).await.expect("register");
}
