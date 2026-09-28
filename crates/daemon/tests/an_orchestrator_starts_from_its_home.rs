//! `workspace.start_orchestrator` over a real socket, on a real tmux server,
//! with a stand-in in place of every agent. And, since only this binary can
//! start one, how the layout verbs treat an orchestrator's window.
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
    /// The daemon's service, for the board writes this client's scope
    /// doesn't reach.
    service: Arc<Service>,
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
    // The daemon's library, without cfg(test): stub every agent launch and
    // refuse a real one (`agent_program`).
    farcooler_daemon::service::stub_agents_in_this_process();
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
    let held = service.clone();
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
    Harness { dir, socket, tmux_socket, repo, repository, service: held }
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
    start_orchestrator_reading(client, workspace, harness, replace, "").await
}

/// `start_orchestrator`, naming the task whose handoff it reads first.
async fn start_orchestrator_reading(
    client: &mut SocketClient,
    workspace: bytes::Bytes,
    harness: &str,
    replace: bool,
    handoff_task: &str,
) -> Result<farcooler_protocol::v1::Terminal, ClientError> {
    let mut r = request("workspace.start_orchestrator");
    r.target_resource_id = Some(workspace);
    r.payload = Some(request::Payload::WorkspaceStartOrchestrator(
        farcooler_protocol::v1::WorkspaceStartOrchestrator {
            harness: harness.into(),
            replace,
            handoff_task: handoff_task.into(),
        },
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
/// at the resolved main checkout for its files and for its project settings,
/// told its charter and that it's the manager.
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
    // The repository's own hooks, allowlist and `.mcp.json`, which
    // `--add-dir` doesn't carry.
    assert_eq!(after("--project-config-root"), Some(resolved(&repo).as_str()), "{args:?}");
    let settings = h.dir.path().join("state").join(format!("orchestrator-{id}.json"));
    assert_eq!(after("--settings"), Some(settings.to_str().unwrap()), "{args:?}");
    assert!(after("--plugin-dir").is_some(), "the manager skill: {args:?}");
    assert_eq!(value(&record, "charter"), [home.join("charter.md").to_str().unwrap()]);
    assert_eq!(value(&record, "actor"), ["manager"]);
    assert_eq!(value(&record, "claude_md"), ["1"]);
    // Its first turn runs the manager skill: the prompt is the last argument.
    assert_eq!(args.last(), Some(&"/farcooler:manager"), "{args:?}");
}

/// Every harness starts on its own spelling of the manager skill, as its
/// last argument, and `handoff_task` names the task to read first.
#[tokio::test]
async fn each_orchestrator_starts_on_the_manager_skill() {
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let main = main_workspace(&h, &mut client).await;
    let workspace = uuid::Uuid::from_slice(&main.id).unwrap();
    let task = h.service.store.create_task(workspace, "the handoff", farcooler_store::models::Actor::User).unwrap();
    let id = workspace_id(&main);
    let key = task.key.as_str();
    let cases = [
        ("claude", "", "claude", "/farcooler:manager".to_string()),
        ("codex", "", "codex", "$farcooler-manager".to_string()),
        ("cursor", "", "cursor-agent", "/manager".to_string()),
        ("codex", key, "codex", format!("$farcooler-manager Read the handoff note on {key} first.")),
    ];
    for (n, (harness, read, program, prompt)) in cases.iter().enumerate() {
        start_orchestrator_reading(&mut client, main.id.clone(), harness, n > 0, read).await.expect("started");
        let record = records(&h, &id, n + 1).await.remove(n);
        let args = value(&record, "arg");
        assert_eq!(args.first(), Some(program), "{args:?}");
        assert_eq!(args.last(), Some(&prompt.as_str()), "{harness} {read:?}: {args:?}");
    }
}

/// A handoff task that isn't on the workspace's own board is refused in
/// words, and the live orchestrator is left running.
#[tokio::test]
async fn a_handoff_task_off_the_workspaces_board_is_refused() {
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let main = main_workspace(&h, &mut client).await;
    let first = start_orchestrator(&mut client, main.id.clone(), "claude", false).await.expect("first");
    records(&h, &workspace_id(&main), 1).await;
    let other = h.service.store.create_workspace(h.repository, "Billing", "bil").unwrap();
    let elsewhere = h.service.store.create_task(other.id, "not Main's", farcooler_store::models::Actor::User).unwrap();

    for key in ["NOPE-9", elsewhere.key.as_str()] {
        match start_orchestrator_reading(&mut client, main.id.clone(), "claude", true, key).await {
            Err(ClientError::Daemon { code, what, message, .. }) => {
                assert_eq!(code, ErrorCode::InvalidArgument as i32);
                assert_eq!(what, "handoff_task", "{key}");
                assert_eq!(message, "That task isn't on this workspace's board.");
            }
            other => panic!("{key} must be refused: {other:?}"),
        }
    }
    let live = h.service.store.get_terminal(uuid::Uuid::from_slice(&first.id).unwrap()).expect("still there");
    assert_eq!(live.role, farcooler_store::models::TerminalRole::Orchestrator);
    assert_eq!(live.intent, farcooler_protocol::v1::TerminalIntent::Running);
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

// ---------------------------------------------------------------------------
// An orchestrator's window beside the checkout's
//
// Every orchestrator's window is tagged with the main checkout's worktree
// (`start_orchestrator` opens it in `main.id`), so `layout.list` for the
// checkout lists it, and a layout verb that names no window acts on the one
// tmux calls active. The CLI or an agent focusing the orchestrator makes that
// the orchestrator's. A client showing the checkout's own window says which
// one it means with `group_id`, and these tests hold the daemon to it.
// ---------------------------------------------------------------------------

/// The repository's main checkout, where every orchestrator's window is.
async fn main_checkout(client: &mut SocketClient) -> farcooler_protocol::v1::Worktree {
    let Some(result::Value::WorktreeList(list)) =
        client.call(request("worktree.list")).await.expect("worktree.list").value
    else {
        panic!("wrong result")
    };
    list.items.into_iter().find(|w| w.is_main_checkout).expect("the main checkout")
}

async fn layout_call(
    client: &mut SocketClient,
    method: &str,
    worktree: &bytes::Bytes,
    update: farcooler_protocol::v1::LayoutUpdate,
) -> farcooler_protocol::v1::PaneGroupList {
    let mut r = request(method);
    r.target_resource_id = Some(worktree.clone());
    r.payload = Some(request::Payload::LayoutUpdate(update));
    let Some(result::Value::PaneGroupList(list)) =
        client.call(r).await.unwrap_or_else(|e| panic!("{method}: {e:?}")).value
    else {
        panic!("{method} returned the wrong resource")
    };
    list
}

/// The window holding `terminal`.
fn window_of<'a>(
    list: &'a farcooler_protocol::v1::PaneGroupList,
    terminal: &[u8],
) -> &'a farcooler_protocol::v1::PaneGroup {
    list.items
        .iter()
        .find(|g| g.panes.iter().any(|p| p.terminal_id == terminal))
        .unwrap_or_else(|| panic!("no window holds that terminal: {list:?}"))
}

/// The window with id `id`.
fn window<'a>(list: &'a farcooler_protocol::v1::PaneGroupList, id: &str) -> &'a farcooler_protocol::v1::PaneGroup {
    list.items.iter().find(|g| g.id == id).unwrap_or_else(|| panic!("no window {id}: {list:?}"))
}

/// Main's orchestrator, and beside it in the main checkout a window of two
/// shells, with tmux's active window the orchestrator's: what focusing the
/// orchestrator from the CLI leaves behind.
struct Scene {
    workspace: farcooler_protocol::v1::Workspace,
    /// The main checkout's worktree id.
    main: bytes::Bytes,
    orchestrator: bytes::Bytes,
    /// The checkout's own window, the one its row shows.
    checkout: String,
    /// The orchestrator's window.
    orchestrators: String,
}

impl Scene {
    /// A verb naming the checkout's window.
    fn named(&self, update: farcooler_protocol::v1::LayoutUpdate) -> farcooler_protocol::v1::LayoutUpdate {
        farcooler_protocol::v1::LayoutUpdate { group_id: self.checkout.clone(), ..update }
    }

    /// Focus the orchestrator, as `farcooler layout focus` from an agent does.
    async fn focus_the_orchestrator(&self, client: &mut SocketClient) -> farcooler_protocol::v1::PaneGroupList {
        let list = layout_call(
            client,
            "layout.focus",
            &self.main,
            farcooler_protocol::v1::LayoutUpdate { focus: Some(self.orchestrator.clone()), ..Default::default() },
        )
        .await;
        assert!(window(&list, &self.orchestrators).active, "tmux calls the orchestrator's window active");
        assert!(!window(&list, &self.checkout).active);
        list
    }

    /// Nothing about the orchestrator's window moved: one pane, the
    /// orchestrator's, not zoomed, and still the one tmux calls active.
    fn untouched(&self, list: &farcooler_protocol::v1::PaneGroupList, name: &str, verb: &str) {
        let w = window(list, &self.orchestrators);
        assert_eq!(w.panes.len(), 1, "{verb}: {list:?}");
        assert_eq!(w.panes[0].terminal_id, self.orchestrator, "{verb}");
        assert!(!w.panes[0].zoomed, "{verb}");
        assert_eq!(w.name, name, "{verb}");
        assert!(w.active, "{verb} moved tmux off the orchestrator's window");
    }
}

async fn scene(h: &Harness, client: &mut SocketClient) -> Scene {
    let workspace = main_workspace(h, client).await;
    let orchestrator = start_orchestrator(client, workspace.id.clone(), "claude", false).await.expect("started").id;
    records(h, &workspace_id(&workspace), 1).await;
    let main = main_checkout(client).await.id;

    let mut create = request("terminal.create");
    create.target_resource_id = Some(main.clone());
    create.payload = Some(request::Payload::TerminalCreate(farcooler_protocol::v1::TerminalCreate {
        title: "shell".into(),
        command_preset: "shell".into(),
        join_active_group: false,
        prompt: None,
        task_key: None,
    }));
    let Some(result::Value::Terminal(shell)) = client.call(create).await.expect("terminal.create").value else {
        panic!("wrong result")
    };
    let list = layout_call(
        client,
        "layout.split",
        &main,
        farcooler_protocol::v1::LayoutUpdate {
            target: Some(shell.id.clone()),
            side: farcooler_protocol::v1::SplitSide::Right as i32,
            command_preset: "shell".into(),
            ..Default::default()
        },
    )
    .await;
    let checkout = window_of(&list, &shell.id).id.clone();
    let orchestrators = window_of(&list, &orchestrator).id.clone();
    assert_ne!(checkout, orchestrators, "the orchestrator has a window of its own");
    assert_eq!(window(&list, &checkout).panes.len(), 2);

    let scene = Scene { workspace, main, orchestrator, checkout, orchestrators };
    // A size stated for both, so geometry is tmux's arithmetic rather than
    // whatever the session happened to have.
    for id in [&scene.checkout, &scene.orchestrators] {
        let update = farcooler_protocol::v1::LayoutUpdate {
            group_id: id.clone(),
            columns: Some(120),
            rows: Some(40),
            ..Default::default()
        };
        layout_call(client, "layout.viewport", &scene.main, update).await;
    }
    scene.focus_the_orchestrator(client).await;
    scene
}

/// Zoom, a preset, `cycle`, the viewport, a rename and ⌃B o/; each act on
/// the window they name, with the orchestrator's the one tmux calls active.
#[tokio::test]
async fn a_verb_naming_the_checkouts_window_leaves_the_orchestrators_alone() {
    use farcooler_protocol::v1::{LayoutPreset, LayoutUpdate};
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let s = scene(&h, &mut client).await;
    let before = s.focus_the_orchestrator(&mut client).await;
    let name = window(&before, &s.orchestrators).name.clone();

    let list = layout_call(&mut client, "layout.zoom", &s.main, s.named(LayoutUpdate::default())).await;
    assert!(window(&list, &s.checkout).panes.iter().any(|p| p.zoomed && p.focused), "zoom: {list:?}");
    s.untouched(&list, &name, "zoom");
    let unzoom = LayoutUpdate { unzoom: true, ..Default::default() };
    let list = layout_call(&mut client, "layout.zoom", &s.main, s.named(unzoom)).await;
    assert!(!window(&list, &s.checkout).panes.iter().any(|p| p.zoomed), "unzoom");
    s.untouched(&list, &name, "unzoom");

    let stacked = LayoutUpdate { preset: Some(LayoutPreset::EvenVertical as i32), ..Default::default() };
    let list = layout_call(&mut client, "layout.preset", &s.main, s.named(stacked)).await;
    let panes = &window(&list, &s.checkout).panes;
    assert!(panes.iter().all(|p| p.left == 0) && panes[0].top != panes[1].top, "stacked: {panes:?}");
    s.untouched(&list, &name, "preset");

    let was = window(&list, &s.checkout).layout.clone();
    let list = layout_call(&mut client, "layout.cycle", &s.main, s.named(LayoutUpdate::default())).await;
    assert_ne!(window(&list, &s.checkout).layout, was, "cycle rearranged the checkout's panes");
    s.untouched(&list, &name, "cycle");

    let sized = LayoutUpdate { columns: Some(100), rows: Some(30), ..Default::default() };
    let list = layout_call(&mut client, "layout.viewport", &s.main, s.named(sized)).await;
    let (c, o) = (window(&list, &s.checkout), window(&list, &s.orchestrators));
    assert_eq!((c.columns, c.rows), (100, 30), "the checkout's window takes the viewport");
    assert_eq!((o.columns, o.rows), (120, 40), "and the orchestrator's keeps its own");
    s.untouched(&list, &name, "viewport");

    let renamed = LayoutUpdate { name: "shells".into(), ..Default::default() };
    let list = layout_call(&mut client, "layout.rename", &s.main, s.named(renamed)).await;
    assert_eq!(window(&list, &s.checkout).name, "shells");
    s.untouched(&list, &name, "rename");

    // ⌃B o and ⌃B ; move focus within the named window, which is the one
    // verb here that is meant to bring its window forward.
    for step in [1, -1] {
        let was = window(&s.focus_the_orchestrator(&mut client).await, &s.checkout)
            .panes
            .iter()
            .position(|p| p.focused);
        let stepped = LayoutUpdate { step: Some(step), ..Default::default() };
        let list = layout_call(&mut client, "layout.focus", &s.main, s.named(stepped)).await;
        let c = window(&list, &s.checkout);
        assert!(c.active, "step {step}: the checkout's window comes forward");
        assert_ne!(c.panes.iter().position(|p| p.focused), was, "step {step}: focus moved within it");
        assert!(!window(&list, &s.orchestrators).active);
    }
    s.focus_the_orchestrator(&mut client).await;
    let second = LayoutUpdate { pane: Some(2), ..Default::default() };
    let list = layout_call(&mut client, "layout.focus", &s.main, s.named(second)).await;
    let c = window(&list, &s.checkout);
    assert!(c.active && c.panes[1].focused, "pane 2 is counted in the checkout's window: {c:?}");
}

/// A split or a break with no pane named starts from the focused pane of the
/// window named, not of the one tmux calls active.
#[tokio::test]
async fn a_split_or_break_in_a_named_window_starts_from_its_focused_pane() {
    use farcooler_protocol::v1::{LayoutUpdate, SplitSide};
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let s = scene(&h, &mut client).await;
    let name = window(&s.focus_the_orchestrator(&mut client).await, &s.orchestrators).name.clone();

    let split = LayoutUpdate { side: SplitSide::Bottom as i32, command_preset: "shell".into(), ..Default::default() };
    let list = layout_call(&mut client, "layout.split", &s.main, s.named(split)).await;
    assert_eq!(window(&list, &s.checkout).panes.len(), 3, "split the checkout's window: {list:?}");
    assert_eq!(window(&list, &s.orchestrators).panes.len(), 1, "not the orchestrator's");

    let list = s.focus_the_orchestrator(&mut client).await;
    let focused =
        window(&list, &s.checkout).panes.iter().find(|p| p.focused).expect("a focused pane").terminal_id.clone();
    let list = layout_call(&mut client, "layout.break", &s.main, s.named(LayoutUpdate::default())).await;
    assert_eq!(window(&list, &s.checkout).panes.len(), 2, "broke out of the checkout's window: {list:?}");
    let out = window_of(&list, &focused);
    assert!(out.id != s.checkout && out.id != s.orchestrators && out.panes.len() == 1, "its focused pane, alone");
    let o = window(&list, &s.orchestrators);
    assert_eq!((o.panes.len(), &o.panes[0].terminal_id, &o.name), (1, &s.orchestrator, &name));
}

/// A daemon started again on the same home and tmux server still finds the
/// orchestrator: its window is still among the checkout's, its seat is still
/// taken, and a verb naming the checkout's window still leaves it alone.
#[tokio::test]
async fn a_restarted_daemon_still_finds_the_orchestrator() {
    use farcooler_protocol::v1::LayoutPreset;
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let s = scene(&h, &mut client).await;

    let again = Service::open_in(h.dir.path().join("state")).await.expect("service");
    assert_eq!(again.tmux.socket(), h.tmux_socket, "the same tmux server");
    let main = uuid::Uuid::from_slice(&s.main).unwrap();
    let orchestrator = uuid::Uuid::from_slice(&s.orchestrator).unwrap();
    let layouts = again.layout(main).await.expect("layout");
    let held = |id: &str| layouts.iter().find(|l| l.window.window_id == id).map(|l| l.panes.len());
    assert_eq!(held(&s.orchestrators), Some(1), "the orchestrator's window is still the checkout's");
    assert_eq!(held(&s.checkout), Some(2));
    assert!(layouts.iter().any(|l| l.panes.iter().any(|p| p.terminal_id == orchestrator)));

    let workspace = uuid::Uuid::from_slice(&s.workspace.id).unwrap();
    match again.start_orchestrator(workspace, "claude", false, None).await {
        Err(farcooler_core::DomainError::InvalidArgument { what: "orchestrator_taken" }) => {}
        other => panic!("the live orchestrator's seat must still be taken: {other:?}"),
    }

    let after = again.layout_preset(main, Some(&s.checkout), LayoutPreset::EvenVertical).await.expect("preset");
    let c = after.iter().find(|l| l.window.window_id == s.checkout).unwrap();
    assert!(c.panes.iter().all(|p| p.left == 0), "the checkout's window was stacked: {c:?}");
    let o = after.iter().find(|l| l.window.window_id == s.orchestrators).unwrap();
    assert!(o.window.active && o.panes.len() == 1, "and the orchestrator's left alone");
}

/// A second window of shells in the main checkout, which `terminal.create`
/// opens as a window of its own.
async fn another_shell_window(client: &mut SocketClient, main: &bytes::Bytes, join: bool) -> bytes::Bytes {
    let mut create = request("terminal.create");
    create.target_resource_id = Some(main.clone());
    create.payload = Some(request::Payload::TerminalCreate(farcooler_protocol::v1::TerminalCreate {
        title: "more".into(),
        command_preset: "shell".into(),
        join_active_group: join,
        prompt: None,
        task_key: None,
    }));
    let Some(result::Value::Terminal(t)) = client.call(create).await.expect("terminal.create").value else {
        panic!("wrong result")
    };
    t.id
}

/// What `farcooler layout zoom <main>` sends, and every other verb that
/// names no window, after an agent has focused the orchestrator: the
/// checkout's own windows answer, and the orchestrator's is never "active"
/// for them. It's reached only by naming it.
#[tokio::test]
async fn a_verb_naming_no_window_never_reaches_the_orchestrators() {
    use farcooler_protocol::v1::{LayoutPreset, LayoutUpdate, SplitSide};
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let s = scene(&h, &mut client).await;
    let name = window(&s.focus_the_orchestrator(&mut client).await, &s.orchestrators).name.clone();

    let list = layout_call(&mut client, "layout.zoom", &s.main, LayoutUpdate::default()).await;
    assert!(window(&list, &s.checkout).panes.iter().any(|p| p.zoomed && p.focused), "zoom: {list:?}");
    s.untouched(&list, &name, "zoom");
    let unzoom = LayoutUpdate { unzoom: true, ..Default::default() };
    let list = layout_call(&mut client, "layout.zoom", &s.main, unzoom).await;
    assert!(!window(&list, &s.checkout).panes.iter().any(|p| p.zoomed), "unzoom");
    s.untouched(&list, &name, "unzoom");

    let stacked = LayoutUpdate { preset: Some(LayoutPreset::EvenVertical as i32), ..Default::default() };
    let list = layout_call(&mut client, "layout.preset", &s.main, stacked).await;
    assert!(window(&list, &s.checkout).panes.iter().all(|p| p.left == 0), "preset: {list:?}");
    s.untouched(&list, &name, "preset");

    let was = window(&list, &s.checkout).layout.clone();
    let list = layout_call(&mut client, "layout.cycle", &s.main, LayoutUpdate::default()).await;
    assert_ne!(window(&list, &s.checkout).layout, was, "cycle");
    s.untouched(&list, &name, "cycle");

    let split = LayoutUpdate { side: SplitSide::Right as i32, command_preset: "shell".into(), ..Default::default() };
    let list = layout_call(&mut client, "layout.split", &s.main, split).await;
    assert_eq!(window(&list, &s.checkout).panes.len(), 3, "split: {list:?}");
    assert_eq!(window(&list, &s.orchestrators).panes.len(), 1, "split");

    s.focus_the_orchestrator(&mut client).await;
    let next = LayoutUpdate { step: Some(1), ..Default::default() };
    let list = layout_call(&mut client, "layout.focus", &s.main, next).await;
    assert!(window(&list, &s.checkout).active, "⌃B o lands in the checkout's window: {list:?}");

    // `terminal.create --tile` joins the checkout's window too.
    s.focus_the_orchestrator(&mut client).await;
    let joined = another_shell_window(&mut client, &s.main, true).await;
    let list = s.focus_the_orchestrator(&mut client).await;
    assert_eq!(window_of(&list, &joined).id, s.checkout, "joined the checkout's window: {list:?}");
    assert_eq!(window(&list, &s.orchestrators).panes.len(), 1);

    // `layout select --next` and `--prev` walk the checkout's windows and
    // step over the orchestrator's.
    let more = another_shell_window(&mut client, &s.main, false).await;
    let mores = window_of(&s.focus_the_orchestrator(&mut client).await, &more).id.clone();
    let mut seen = Vec::new();
    for step in [1, 1, 1, -1, -1] {
        let stepped = LayoutUpdate { step: Some(step), ..Default::default() };
        let list = layout_call(&mut client, "layout.group.select", &s.main, stepped).await;
        let shown = list.items.iter().find(|g| g.active).expect("an active window").id.clone();
        assert_ne!(shown, s.orchestrators, "select {step:+} landed on the orchestrator's window");
        seen.push(shown);
    }
    assert!(seen.contains(&s.checkout) && seen.contains(&mores), "{seen:?}");
}

/// tmux's active window in another worktree leaves the checkout with none
/// of its own active, and what it falls back to is its own first window,
/// not the orchestrator's, which is older.
#[tokio::test]
async fn the_checkouts_fallback_is_never_the_orchestrators_window() {
    use farcooler_protocol::v1::LayoutUpdate;
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let s = scene(&h, &mut client).await;
    let name = window(&s.focus_the_orchestrator(&mut client).await, &s.orchestrators).name.clone();

    let mut create = request("worktree.create");
    create.target_resource_id = Some(bytes::Bytes::copy_from_slice(h.repository.as_bytes()));
    create.payload = Some(request::Payload::WorktreeCreate(farcooler_protocol::v1::WorktreeCreate {
        task_name: "elsewhere".into(),
        branch: "feat/elsewhere".into(),
        base_revision: "HEAD".into(),
        terminal_preset: "shell".into(),
        adopt_existing: false,
        fork_only: false,
        workspace_id: None,
    }));
    let Some(result::Value::Worktree(other)) = client.call(create).await.expect("worktree.create").value else {
        panic!("wrong result")
    };
    let theirs = layout_call(&mut client, "layout.list", &other.id, LayoutUpdate::default()).await;
    let focus = LayoutUpdate { focus: Some(theirs.items[0].panes[0].terminal_id.clone()), ..Default::default() };
    layout_call(&mut client, "layout.focus", &other.id, focus).await;

    let list = layout_call(&mut client, "layout.zoom", &s.main, LayoutUpdate::default()).await;
    assert!(list.items.iter().all(|g| !g.active), "tmux's active window is the other worktree's: {list:?}");
    assert!(window(&list, &s.checkout).panes.iter().any(|p| p.zoomed), "zoomed the checkout's: {list:?}");
    let o = window(&list, &s.orchestrators);
    let index = |id: &str| id.trim_start_matches('@').parse::<u32>().unwrap();
    assert!(index(&o.id) < index(&s.checkout), "the orchestrator's window is the older one");
    assert_eq!((o.panes.len(), o.panes[0].zoomed, &o.name), (1, false, &name));
}

/// A worktree, made over the wire with a shell in its first window.
async fn a_worktree(h: &Harness, client: &mut SocketClient, task: &str) -> farcooler_protocol::v1::Worktree {
    let mut create = request("worktree.create");
    create.target_resource_id = Some(bytes::Bytes::copy_from_slice(h.repository.as_bytes()));
    create.payload = Some(request::Payload::WorktreeCreate(farcooler_protocol::v1::WorktreeCreate {
        task_name: task.into(),
        branch: format!("feat/{task}"),
        base_revision: "HEAD".into(),
        terminal_preset: "shell".into(),
        adopt_existing: false,
        fork_only: false,
        workspace_id: None,
    }));
    let Some(result::Value::Worktree(w)) = client.call(create).await.expect("worktree.create").value else {
        panic!("wrong result")
    };
    w
}

/// In a plain worktree none of whose windows is active, `layout select
/// --next` starts from the first, the one the fallback shows, so it moves to
/// the second. With one layout it does nothing, and tmux's active window
/// stays in the other worktree.
#[tokio::test]
async fn select_next_with_none_active_starts_from_the_fallback() {
    use farcooler_protocol::v1::LayoutUpdate;
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let two = a_worktree(&h, &mut client, "two").await;
    let second = another_shell_window(&mut client, &two.id, false).await;
    let one = a_worktree(&h, &mut client, "one").await;
    let elsewhere = a_worktree(&h, &mut client, "elsewhere").await;
    let theirs = layout_call(&mut client, "layout.list", &elsewhere.id, LayoutUpdate::default()).await;
    let focus = LayoutUpdate { focus: Some(theirs.items[0].panes[0].terminal_id.clone()), ..Default::default() };
    layout_call(&mut client, "layout.focus", &elsewhere.id, focus).await;

    let next = LayoutUpdate { step: Some(1), ..Default::default() };
    let list = layout_call(&mut client, "layout.group.select", &one.id, next.clone()).await;
    assert!(list.items.len() == 1 && !list.items[0].active, "one layout steps nowhere: {list:?}");
    let theirs = layout_call(&mut client, "layout.list", &elsewhere.id, LayoutUpdate::default()).await;
    assert!(theirs.items[0].active, "and tmux's active window stays where it was");

    let before = layout_call(&mut client, "layout.list", &two.id, LayoutUpdate::default()).await;
    assert!(before.items.len() == 2 && before.items.iter().all(|g| !g.active), "{before:?}");
    let list = layout_call(&mut client, "layout.group.select", &two.id, next).await;
    assert!(window_of(&list, &second).active, "--next shows the second, not the first again: {list:?}");
}

/// A pane named outright is zoomed even when the worktree has no default
/// layout: here the main checkout's only window is the orchestrator's.
#[tokio::test]
async fn zooming_a_named_pane_needs_no_default_layout() {
    use farcooler_protocol::v1::LayoutUpdate;
    let h = start().await;
    let mut client = Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect");
    let workspace = main_workspace(&h, &mut client).await;
    let orchestrator =
        start_orchestrator(&mut client, workspace.id.clone(), "claude", false).await.expect("started").id;
    records(&h, &workspace_id(&workspace), 1).await;
    let main = main_checkout(&mut client).await.id;

    for off in [false, true] {
        let zoom = LayoutUpdate { zoom: Some(orchestrator.clone()), unzoom: off, ..Default::default() };
        let mut r = request("layout.zoom");
        r.target_resource_id = Some(main.clone());
        r.payload = Some(request::Payload::LayoutUpdate(zoom));
        client.call(r).await.unwrap_or_else(|e| panic!("zoom, off {off}: {e:?}"));
    }
    // With nothing named there is nothing to act on, and it says so.
    let mut r = request("layout.zoom");
    r.target_resource_id = Some(main.clone());
    r.payload = Some(request::Payload::LayoutUpdate(LayoutUpdate::default()));
    assert!(client.call(r).await.is_err(), "no default layout for the checkout");
}
