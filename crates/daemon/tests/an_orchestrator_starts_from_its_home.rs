//! `workspace.start_orchestrator` over a real socket, on a real tmux server,
//! with a stand-in in place of every agent.
//!
//! Its own binary because of the stand-in. This is the daemon's library
//! built without `cfg(test)`, so an agent launch here runs whatever
//! `FARCOOLER_STAND_IN_AGENT` names, or the real `claude` when nothing does.
//! That variable is read once per process and set here before the first
//! launch, which `rpc_over_socket.rs`, a binary of many unrelated tests,
//! can't promise.
//!
//! The stand-in writes down what it was started with, which is what a real
//! harness would have been started with: its directory, its arguments and
//! the variables the orchestrator recipe exports. Then it waits, like the
//! unit tests' stub.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use farcooler_daemon::{rpc::RpcFactory, service::Service};
use farcooler_protocol::v1::{ErrorCode, Scope, request, result};
use farcooler_transport::{Client, ClientError, HandshakeConfig, Peer, UnixListenerServer, request};

type SocketClient = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

/// The stand-in, one program for every test here, since the daemon reads the
/// variable once.
///
/// **One file at one fixed path** (`stand_in_dir`), the same bytes every
/// run. A process-long `TempDir` in a `static` is never dropped, so each run
/// used to leave a directory behind in `$TMPDIR`. It's written to a private
/// name and renamed, so a run reads the whole file even with another run of
/// this binary writing it. It writes its records into the orchestrator's
/// home, which each test's `Harness` removes.
static STAND_IN: LazyLock<PathBuf> = LazyLock::new(|| {
    let dir = &stand_in_dir();
    std::fs::create_dir_all(dir).unwrap();
    let script = dir.join("an-orchestrator-stand-in");
    let private = dir.join(format!("an-orchestrator-stand-in.{}", std::process::id()));
    std::fs::write(
        &private,
        "#!/bin/sh\n\
         out=\"$(dirname \"$FARCOOLER_CHARTER\")/record-$$\"\n\
         {\n\
           echo \"cwd=$(pwd -P)\"\n\
           for a in \"$@\"; do echo \"arg=$a\"; done\n\
           echo \"charter=$FARCOOLER_CHARTER\"\n\
           echo \"actor=$FARCOOLER_ACTOR\"\n\
           echo \"claude_md=$CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD\"\n\
         } > \"$out.tmp\" && mv \"$out.tmp\" \"$out\"\n\
         exec /bin/sleep 600\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&private, &script).unwrap();
    let program = script.to_str().unwrap();
    assert!(plain(&script), "the daemon runs `false` for a stand-in path it can't use: {program}");
    // SAFETY: `set_var` is sound only while no other thread reads the
    // environment. Every test here forces this before it starts a daemon,
    // runtime threads or a process, and the first to arrive sets it while the
    // others wait on the `LazyLock`.
    unsafe { std::env::set_var("FARCOOLER_STAND_IN_AGENT", program) };
    script
});

/// Whether the daemon takes `path` as a stand-in: letters, digits and
/// `/._-` only (`stand_in_program`).
fn plain(path: &Path) -> bool {
    path.to_str().is_some_and(|p| p.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)))
}

/// Where the stand-in lives: the build's scratch directory
/// (`CARGO_TARGET_TMPDIR`, `target/tmp`), or, when that path isn't one the
/// daemon takes (a checkout under `~/My Projects`), a directory of this
/// user's own in the temporary directory, by a fixed name. Fixed either way,
/// so a run reuses the file the last one wrote rather than leaving its own.
///
/// The fallback is refused if it's a link or someone else's: in a shared
/// `/tmp` another account could make it first.
fn stand_in_dir() -> PathBuf {
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    if plain(&target) {
        return target;
    }
    // SAFETY: `getuid` has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let dir = std::env::temp_dir().join(format!("farcooler-orchestrator-stand-in-{uid}"));
    let _ = std::fs::create_dir(&dir);
    let meta = std::fs::symlink_metadata(&dir).unwrap();
    use std::os::unix::fs::MetadataExt;
    assert!(meta.is_dir() && meta.uid() == uid, "{} isn't this user's own directory", dir.display());
    dir
}

/// A daemon on a private socket, database and tmux server, as in
/// `rpc_over_socket.rs`, with one repository registered and a client at
/// `control`, the scope `workspace.start_orchestrator` needs.
struct Harness {
    dir: tempfile::TempDir,
    socket: PathBuf,
    tmux_socket: String,
    /// The main checkout, as registered.
    repo: PathBuf,
    repository: uuid::Uuid,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", &self.tmux_socket, "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

async fn start() -> Harness {
    LazyLock::force(&STAND_IN);
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("farcoolerd.sock");
    let service = Arc::new(Service::open_in(dir.path().join("state")).await.expect("service"));

    // Registered here rather than over the socket: adding a root is
    // `host_admin`, and the client below is `control`.
    let repo = dir.path().join("repos").join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    service.add_root(repo.parent().unwrap()).await.expect("root");
    let repository = service.register_repository(&repo).await.expect("registered").id;

    let tmux_socket = service.tmux.socket().to_string();
    let server = UnixListenerServer::bind(&socket).expect("bind");
    let watcher = farcooler_daemon::watch::Watcher::new(service.clone());
    tokio::spawn(async move {
        let _ = server
            .serve(move |_| {
                Some((
                    HandshakeConfig { daemon_version: "test".into() },
                    RpcFactory::new(
                        service.clone(),
                        watcher.clone(),
                        Arc::new(tokio::sync::Notify::new()),
                        Peer { client_id: None, scope: Scope::Control },
                    ),
                ))
            })
            .await;
    });
    tokio::task::yield_now().await;
    Harness { dir, socket, tmux_socket, repo, repository }
}

/// The repository's Main workspace, as a client lists it.
async fn main_workspace(h: &Harness, client: &mut SocketClient) -> farcooler_protocol::v1::Workspace {
    let mut list = request("workspace.list");
    list.target_resource_id = Some(bytes::Bytes::copy_from_slice(h.repository.as_bytes()));
    let Some(result::Value::WorkspaceList(list)) = client.call(list).await.expect("workspace.list").value else {
        panic!("wrong result")
    };
    list.items.into_iter().find(|w| w.is_main).expect("a Main")
}

async fn start_orchestrator(
    client: &mut SocketClient,
    workspace: bytes::Bytes,
    harness: &str,
    replace: bool,
) -> Result<farcooler_protocol::v1::Terminal, ClientError> {
    let mut r = request("workspace.start_orchestrator");
    r.target_resource_id = Some(workspace);
    r.payload = Some(request::Payload::WorkspaceStartOrchestrator(
        farcooler_protocol::v1::WorkspaceStartOrchestrator { harness: harness.into(), replace },
    ));
    match client.call(r).await?.value {
        Some(result::Value::Terminal(t)) => Ok(t),
        other => panic!("wrong result: {other:?}"),
    }
}

/// `workspace`'s home in `h`'s daemon, where the stand-in writes.
fn home(h: &Harness, workspace: &str) -> PathBuf {
    h.dir.path().join("state").join("workspaces").join(workspace)
}

/// What the stand-in wrote for each launch in `workspace`, oldest first,
/// waiting up to ten seconds for there to be `count`.
async fn records(h: &Harness, workspace: &str, count: usize) -> Vec<Vec<(String, String)>> {
    for _ in 0..100 {
        let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(home(h, workspace))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let name = p.file_name().unwrap().to_string_lossy();
                name.starts_with("record-") && !name.ends_with(".tmp")
            })
            .map(|p| (std::fs::metadata(&p).unwrap().modified().unwrap(), p))
            .collect();
        if found.len() >= count {
            found.sort();
            return found
                .into_iter()
                .map(|(_, p)| {
                    std::fs::read_to_string(p)
                        .unwrap()
                        .lines()
                        .filter_map(|l| l.split_once('='))
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect()
                })
                .collect();
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the stand-in never wrote {count} record(s) for {workspace}");
}

fn value<'a>(record: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    record.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
}

fn workspace_id(w: &farcooler_protocol::v1::Workspace) -> String {
    uuid::Uuid::from_slice(&w.id).unwrap().to_string()
}

#[tokio::test]
async fn a_second_orchestrator_is_refused_unless_replacing() {
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let main = main_workspace(&h, &mut client).await;

    let first = start_orchestrator(&mut client, main.id.clone(), "claude", false).await.expect("first");
    assert_eq!(first.role, farcooler_protocol::v1::TerminalRole::Orchestrator as i32);
    assert_eq!(first.workspace_id.as_deref(), Some(main.id.as_ref()));
    // Launched, before anything replaces it: a pane stopped this early can
    // be killed before its shell reaches the stand-in.
    records(&h, &workspace_id(&main), 1).await;

    match start_orchestrator(&mut client, main.id.clone(), "claude", false).await {
        Err(ClientError::Daemon { code, what, message, .. }) => {
            assert_eq!(code, ErrorCode::InvalidArgument as i32);
            assert_eq!(what, "orchestrator_taken");
            assert_eq!(message, "This workspace already has an orchestrator running.");
        }
        other => panic!("a second orchestrator must be refused: {other:?}"),
    }

    let second = start_orchestrator(&mut client, main.id.clone(), "claude", true).await.expect("replaced");
    assert_ne!(first.id, second.id);
    assert_eq!(second.role, farcooler_protocol::v1::TerminalRole::Orchestrator as i32);
    // The replacement really launched, rather than only being recorded.
    records(&h, &workspace_id(&main), 2).await;
}

/// The pane a real claude would have been started in: the home, pointed back
/// at the resolved main checkout, told its charter and that it's the manager.
#[tokio::test]
async fn a_claude_orchestrator_is_started_from_its_home_with_the_recipe() {
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let main = main_workspace(&h, &mut client).await;
    let repo = h.repo.clone();
    start_orchestrator(&mut client, main.id.clone(), "claude", false).await.expect("started");

    let id = workspace_id(&main);
    let record = records(&h, &id, 1).await.remove(0);
    let home = home(&h, &id);
    let resolved = |p: &Path| p.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(value(&record, "cwd"), [resolved(&home)]);
    let args = value(&record, "arg");
    assert_eq!(args[0], "claude");
    let after = |flag: &str| args.iter().position(|a| *a == flag).map(|i| args[i + 1]);
    assert_eq!(after("--add-dir"), Some(resolved(&repo).as_str()), "{args:?}");
    let settings = h.dir.path().join("state").join(format!("orchestrator-{id}.json"));
    assert_eq!(after("--settings"), Some(settings.to_str().unwrap()), "{args:?}");
    assert!(after("--plugin-dir").is_some(), "the manager skill: {args:?}");
    assert_eq!(value(&record, "charter"), [home.join("charter.md").to_str().unwrap()]);
    assert_eq!(value(&record, "actor"), ["manager"]);
    assert_eq!(value(&record, "claude_md"), ["1"]);
}

/// The stand-in is one file at a fixed path, not one per run, and it's in
/// the build's scratch directory whenever the daemon can take that path.
/// What it records goes into a home the test removes.
#[tokio::test]
async fn the_stand_in_is_one_file_not_one_per_run() {
    let h = start().await;
    assert_eq!(stand_in_dir(), stand_in_dir(), "fixed, not made fresh");
    assert_eq!(*STAND_IN, stand_in_dir().join("an-orchestrator-stand-in"));
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
    if plain(target) {
        assert_eq!(STAND_IN.parent(), Some(target));
    }

    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let main = main_workspace(&h, &mut client).await;
    start_orchestrator(&mut client, main.id.clone(), "claude", false).await.expect("started");
    records(&h, &workspace_id(&main), 1).await;
    assert!(home(&h, &workspace_id(&main)).starts_with(h.dir.path()), "removed with the test's directory");
}
