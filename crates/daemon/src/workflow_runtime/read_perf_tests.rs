//! Scratch-only RPC profiling and latency checks for a board with retained engine history.
use super::{CodingHarness, CommandHandle, Definition, Phase, Runtime, StageKind, wait_until};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;
use farcooler_client::plan_pages::PlanPages;
use farcooler_protocol::v1::{self as pb, request, response, result, Scope};
use farcooler_transport::{Handler, Peer};
use std::sync::Arc;

async fn read_requests(service: &Arc<crate::service::Service>, handle: CommandHandle, board: Uuid) -> Vec<(String, Duration)> {
    let repo = service.store.get_workspace(board).unwrap().repository_id;
    let board_bytes = crate::wire::id_bytes(board);
    let rpc = crate::rpc::RpcFactory::new(service.clone(), crate::watch::Watcher::new(service.clone()),
        Arc::new(tokio::sync::Notify::new()), Peer { client_id: None, scope: Scope::Control })
        .with_workflow_fixture(handle);
    let mut times = Vec::new();
    let theme_list = Some(pb::plan_detail_read::Subject::ThemeName(String::new()));
    for (name, mut pages) in [
        ("plan", PlanPages::overview(board_bytes.clone())),
        ("plan theme list", PlanPages::detail(board_bytes.clone(), theme_list)),
    ] {
        let start = std::time::Instant::now();
        loop {
            let reply = rpc.handle(pb::Request {
                method: "plan.get".into(),
                payload: Some(request::Payload::PlanGet(pages.request())),
                ..Default::default()
            }).await;
            let Some(response::Outcome::Result(pb::Result { value: Some(result::Value::Plan(page)), .. })) = reply.outcome else {
                panic!("{name}: {:?}", reply.outcome);
            };
            if pages.push(page).unwrap().is_some() { break; }
        }
        let elapsed = start.elapsed();
        println!("{name}: {:.3} ms", elapsed.as_secs_f64() * 1000.0);
        times.push((name.to_owned(), elapsed));
    }

    let mut workflow = super::admission_tests::request(board, &farcooler_protocol::workflow_commands::Action::Get);
    workflow.accepts.push("workflow_summary_v1".into());
    let cases = [
        ("workflow get", workflow),
        ("task list", pb::Request {
            method: "task.list".into(),
            payload: Some(request::Payload::TaskList(pb::TaskListRequest {
                repository_id: crate::wire::id_bytes(repo), workspace_id: Some(board_bytes), page_after: Some(bytes::Bytes::new()),
                ..Default::default()
            })),
            ..Default::default()
        }),
        ("worktree list", pb::Request { method: "worktree.list".into(), ..Default::default() }),
    ];
    for (name, request) in cases {
        let start = std::time::Instant::now();
        let reply = rpc.handle(request).await;
        let elapsed = start.elapsed();
        assert!(matches!(reply.outcome, Some(response::Outcome::Result(_))), "{name}: {:?}", reply.outcome);
        println!("{name}: {:.3} ms", elapsed.as_secs_f64() * 1000.0);
        times.push((name.to_owned(), elapsed));
    }
    times
}

/// Use a disposable copy of the installed database in a scratch service root.
/// Deliberately leave the effect owner stopped: retained adapter paths and
/// remote bindings in the copy belong to the original installation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a disposable database copy under FARCOOLER_SCRATCH"]
async fn profile_copied_board() {
    let root = PathBuf::from(std::env::var_os("FARCOOLER_PROFILE_ROOT").unwrap()).canonicalize().unwrap();
    let scratch = PathBuf::from(std::env::var_os("FARCOOLER_SCRATCH").unwrap()).canonicalize().unwrap();
    assert!(root.starts_with(scratch) && root.join("farcooler.db").is_file());
    let svc = Arc::new(crate::service::Service::open_in(root.clone()).await.unwrap());
    let board = svc.store.workflow_workspaces().unwrap().remove(0).parse().unwrap();
    let runtime = Runtime::open_service(&root.join("workflow"), svc.store.clone()).unwrap();
    println!("scratch runner pid: {}", std::process::id());
    let seconds = std::env::var("FARCOOLER_PROFILE_SECONDS").ok().map(|value| value.parse().unwrap()).unwrap_or(0);
    let until = std::time::Instant::now() + Duration::from_secs(seconds);
    let handle = runtime.command_handle(&runtime.controller()).unwrap();
    loop {
        read_requests(&svc, handle.clone(), board).await;
        if std::time::Instant::now() >= until { break; }
    }
    if let Some(cli) = std::env::var_os("FARCOOLER_PROFILE_CLI") {
        profile_cli(&svc, handle, board, Path::new(&cli)).await;
    }
}

async fn profile_cli(
    svc: &Arc<crate::service::Service>, handle: CommandHandle, board: Uuid, cli: &Path,
) {
    use farcooler_transport::{HandshakeConfig, UnixListenerServer};
    use std::process::Stdio;

    assert!(cli.is_absolute() && cli.is_file());
    // The socket address must fit the short Unix socket path limit, even
    // when the copied database lives under the engine's long scratch path.
    let socket_root = tempfile::Builder::new().prefix("fc-1429-").tempdir_in("/tmp").unwrap();
    let server = UnixListenerServer::bind(socket_root.path().join("farcoolerd.sock")).unwrap();
    let svc = svc.clone();
    let watcher = crate::watch::Watcher::new(svc.clone());
    let stop = Arc::new(tokio::sync::Notify::new());
    let server = tokio::spawn(async move {
        server.serve(move |_| {
            let factory = crate::rpc::RpcFactory::new(svc.clone(), watcher.clone(), stop.clone(),
                Peer { client_id: None, scope: Scope::Control }).with_workflow_fixture(handle.clone());
            Some((HandshakeConfig { daemon_version: farcooler_protocol::BUILD.into() }, factory))
        }).await.unwrap();
    });
    let board = board.to_string();
    for (name, args) in [
        ("plan", vec!["plan", "--workspace", &board]),
        ("plan theme list", vec!["plan", "--workspace", &board, "theme", "list"]),
        ("workflow get", vec!["workflow", "get", &board, "--json"]),
        ("task list", vec!["task", "list", "--workspace", &board, "--json"]),
        ("worktree list", vec!["worktree", "list", "--json"]),
    ] {
        let start = std::time::Instant::now();
        let reply = tokio::process::Command::new(cli).env("FARCOOLER_HOME", socket_root.path())
            .env("FARCOOLER_WORKSPACE", &board).args(&args).stdout(Stdio::null()).kill_on_drop(true)
            .output().await.unwrap();
        assert!(reply.status.success(), "{name}: {}", String::from_utf8_lossy(&reply.stderr));
        println!("CLI {name}: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
    }
    server.abort();
    let _ = server.await;
}

// SQL seeds historical rows in one transaction, avoiding quadratic fixture
// setup via thousands of successive reducer snapshots and fsyncs.
fn seed_history(root: &Path, board: Uuid) {
    let conn = rusqlite::Connection::open(root.join("farcooler.db")).unwrap();
    conn.execute("CREATE TEMP TABLE board AS SELECT id, repository_id FROM workspaces WHERE id = ?1",
        [board.as_bytes()]).unwrap();
    conn.execute_batch("
        CREATE TEMP TABLE numbers AS
            WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i < 12000)
            SELECT i FROM n;
        BEGIN;
        INSERT INTO tasks (id,repository_id,workspace_id,key,title,status,status_since,created_at,resource_version)
            SELECT CAST(printf('t%015d',i) AS BLOB), repository_id, id, 'perf-' || i,
                'Performance fixture ticket ' || i,'todo',0,0,1 FROM numbers,board WHERE i <= 1500;
        INSERT INTO ticket_lines(task_id,stage,stage_since,created_at,resource_version)
            SELECT id,'waiting',0,0,1 FROM tasks;
        INSERT INTO board_themes(id,workspace_id,name,state,ordinal,created_at,resource_version)
            SELECT CAST(printf('h%015d',i) AS BLOB),id,'Theme ' || i,'active',i,0,1 FROM numbers,board WHERE i <= 25;
        INSERT INTO board_theme_tasks(theme_id,task_id,added_at)
            SELECT CAST(printf('h%015d',1+(i-1)%25) AS BLOB),CAST(printf('t%015d',i) AS BLOB),0 FROM numbers WHERE i <= 1500;
        INSERT INTO agents(id,workspace_id,role,kind,harness,terminal_id,started_at,ended_at)
            SELECT CAST(printf('a%015d',i) AS BLOB),id,'builder','pane','codex',
                CAST(printf('p%015d',i) AS BLOB),0,1 FROM numbers,board WHERE i <= 750;
        INSERT INTO ticket_work(agent_id,task_id,role,started_at,ended_at)
            SELECT CAST(printf('a%015d',i) AS BLOB),CAST(printf('t%015d',i) AS BLOB),'build',0,1 FROM numbers WHERE i <= 750;
        INSERT INTO agent_turns(id,turn_key,terminal_id,harness,surface,ended_at,usage)
            SELECT CAST(printf('s%015d',i) AS BLOB),'turn-' || i,
                CAST(printf('p%015d',1+(i-1)%750) AS BLOB),'codex','terminal',1,'reported' FROM numbers;
        INSERT INTO agent_turn_models(turn_id,model,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,cost_source)
            SELECT id,'fixture',100,200,0,0,'unknown' FROM agent_turns;
        INSERT INTO workflow_outbox(workspace,id,revision,payload,payload_hash,active,dispatch_state)
            SELECT workspace,'old-' || i,revision,json_set(payload, '$.id', 'old-' || i, '$.item', 'archived-' || i),
                payload_hash,0,'settled'
            FROM (SELECT * FROM workflow_outbox LIMIT 1),numbers WHERE i <= 4000;
        COMMIT;
    ").unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "wall-clock budget with four CPU-active builds and a landing check"]
async fn board_read_latency_with_engine_work() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let board = svc.store.ensure_main_workspace(repo).unwrap().id;
    let ws = board.to_string();
    let (load_dir, mut runtime) = super::coding_tests::configured(false, "pass").await;
    runtime.exec.store = svc.store.clone();
    runtime.set_effect_cap(4);
    runtime.places().work = 5;
    let worktree = svc.list_worktrees().unwrap().remove(0);
    svc.create_terminal(worktree.id, "Benchmark pane", "shell").await.unwrap();

    if let CodingHarness::Command(build) = &mut runtime.exec.coding.as_mut().unwrap().harness {
        build.timeout_ms = 45_000;
    }
    for check in &mut runtime.exec.coding.as_mut().unwrap().checks { check.timeout_ms = 45_000; }
    // Get the landing candidate past its build before arming the barriers.
    let definition = runtime.configured_definition().unwrap();
    super::concurrency_tests::enqueue_item(&mut runtime, &ws, "landing", definition.clone());
    runtime.pump(2).await.unwrap(); // provision
    runtime.pump(3).await.unwrap(); // build
    let state = runtime.state(&ws).unwrap().unwrap();
    let landing = &state.items["landing"];
    assert_eq!(landing.stage, 2, "{:?}", landing.failed_reason());
    assert_eq!(landing.definition.stages[landing.stage].kind, StageKind::Check);
    let definition = runtime.configured_definition().unwrap();
    for n in 0..4 {
        super::concurrency_tests::enqueue_item(&mut runtime, &ws, &format!("builder-{n}"), definition.clone());
    }
    runtime.pump(4).await.unwrap(); // one check and four provisions
    let state = runtime.state(&ws).unwrap().unwrap();
    for n in 0..4 {
        let item = &state.items[&format!("builder-{n}")];
        assert_eq!(item.stage, 1, "{:?}", item.failed_reason());
    }
    let landing = &state.items["landing"];
    assert_eq!(landing.definition.stages[landing.stage].kind, StageKind::Check);
    let script = load_dir.path().join("adapter.py");
    let original = std::fs::read_to_string(&script).unwrap();
    let load = "
if stage in ('build', 'check'):
    import hashlib, time
    activity = pathlib.Path(__file__).parent / ('active-' + stage + '-' + os.environ['FARCOOLER_WORKFLOW_RUN'])
    activity.touch()
    end = time.monotonic() + 35
    while time.monotonic() < end:
        hashlib.sha256(b'x' * 1048576).digest()
";
    std::fs::write(&script, original.replacen("if stage == 'build':", &format!("{load}\nif stage == 'build':"), 1)).unwrap();
    seed_history(svc.root_dir(), board);
    let mut state = runtime.state(&ws).unwrap().unwrap();
    let tracker = Definition::ticket_at("todo").unwrap();
    let mut item = state.items["landing"].clone();
    item.definition = tracker;
    item.pending = None;
    item.phase = Phase::WaitingApproval;
    for n in 1..=1500 { state.items.insert(format!("perf-{n}"), item.clone()); }
    let state_bytes = serde_json::to_vec(&state).unwrap().len();
    assert!(state_bytes >= 900_000, "engine fixture holds {state_bytes} bytes");
    runtime.store.workflow_state_for_tests(&state).unwrap();

    let handle = runtime.command_handle(&runtime.controller()).unwrap();
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    // This fixture stages fresh work in a running engine. Going through
    // startup would classify the five pending effects as interrupted runs.
    runtime.continuous = true;
    runtime.shutdown = Some(shutdown);
    let worker = tokio::spawn(async move { runtime.pump(super::now()).await });
    let ready = wait_until(tokio::time::Instant::now() + Duration::from_secs(30), Duration::from_millis(30), || {
        std::fs::read_dir(load_dir.path()).unwrap().flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("active-")).count() == 5
    }).await;
    if !ready {
        let state = svc.store.workflow_state(&ws).unwrap().unwrap();
        for (id, item) in state.items.iter().filter(|(id, _)| !id.starts_with("perf-")) {
            eprintln!("{id} {:?} stage {}: {:?}", item.phase, item.stage, item.failed_reason());
        }
        for path in std::fs::read_dir(load_dir.path().join("runtime/receipts")).unwrap().flatten().map(|e| e.path()) {
            if path.extension().is_some_and(|ext| ext == "output") {
                eprintln!("{}", std::fs::read_to_string(path).unwrap_or_default());
            }
        }
    }
    let calls = svc.tmux.calls_total();
    let samples = if ready { read_requests(&svc, handle, board).await } else { Vec::new() };
    stop.send_replace(true);
    worker.await.unwrap().unwrap();
    assert!(ready, "four builds and the landing check did not reach their barriers");
    for (name, elapsed) in samples {
        let limit = match name.as_str() {
            "workflow get" => Duration::from_secs(1),
            "task list" => continue,
            _ => Duration::from_secs(2),
        };
        assert!(elapsed < limit, "{name} took {elapsed:?} (budget {limit:?})");
    }
    assert_eq!(calls, svc.tmux.calls_total(), "board RPCs must reuse the inventory");
}


/// Seed an idle board with one held landing candidate and a blocked cleanup
/// intent. The actual runtime owns every probe once the census starts.
pub(crate) async fn idle_board_fixture(
    svc: &crate::service::Service, ws: &str, repository: &Path, base: String, tree_root: &Path, pushed_sha: &str,
) -> (tempfile::TempDir, Runtime) {
    use super::{adapters, CodingLanding, RunTree};
    use farcooler_protocol::workflow_config::AdapterConfiguration;
    use farcooler_workflow::scheduling::{Budget, StartPolicy};

    let (config_dir, mut fixture) = super::coding_tests::configured(false, "pass").await;
    let config = fixture.coding.as_mut().unwrap();
    config.repository = repository.to_path_buf();
    config.base = base.clone();
    config.landing_ref = "refs/heads/main".into();
    config.landing = Some(CodingLanding {
        job: "idle-landing".into(),
        repository: "https://github.com/example/idle-fixture.git".into(),
        target: config.landing_ref.clone(),
        config_digest: String::new(),
    });
    let config = AdapterConfiguration::Coding(config.clone());
    let mut definition = super::coding_verbs_tests::batched_push(&fixture, "any", 5_000);
    definition.start_policy = Some(StartPolicy {
        columns: ["todo".into()].into(), budget: Budget::Fixed(1), files: Default::default(),
        dependencies: Default::default(), rotate_models: false, auto_queue: true,
    });
    let mut runtime = Runtime::open_service(&svc.root_dir().join("workflow"), svc.store.clone()).unwrap();
    runtime.user_home = Some(config_dir.path().to_path_buf());
    runtime.host = Some(svc.worktree_host().unwrap());

    // Seed a waiting candidate with its immutable adapter. The mock remote
    // never answers, so it stays at rest.
    fixture.exec.store = svc.store.clone();
    super::concurrency_tests::enqueue_item(&mut fixture, ws, "waiting", definition.clone());
    let mut state = fixture.state(ws).unwrap().unwrap();
    let waiting = state.items.get_mut("waiting").unwrap();
    let bundle = farcooler_store::workflow_publication::configuration_pin(&definition, Some(&config));
    waiting.policy_pin = bundle.clone();
    waiting.stage = definition.stages.len() - 1;
    waiting.phase = Phase::WaitingJoin;
    waiting.candidate = Some(base.clone());
    waiting.pending = None;
    // A separate already-pushed group stays paused while its CI result is
    // retained. It names a SHA to the same watcher used for real landings.
    let mut pushed = waiting.clone();
    pushed.definition = definition.group_definition("landing").unwrap();
    pushed.pin = pushed.definition.pin();
    pushed.resource = definition.groups["landing"].exclusive.clone();
    pushed.run = "finished-ci".into();
    pushed.stage = pushed.definition.stages.len() - 1;
    pushed.phase = Phase::WaitingApproval;
    pushed.candidate = Some(pushed_sha.into());
    pushed.group_of = Some(farcooler_workflow::GroupOf { name: "landing".into(), pin: definition.pin() });
    let db = rusqlite::Connection::open(svc.root_dir().join("farcooler.db")).unwrap();
    db.execute("INSERT OR IGNORE INTO workflow_definitions(pin,definition) VALUES (?1,?2)",
        (&definition.pin(), serde_json::to_string(&definition).unwrap())).unwrap();
    db.execute("INSERT INTO workflow_bundles(pin,definition_pin,adapter) VALUES (?1,?2,?3)",
        (&bundle, &definition.pin(), serde_json::to_string(&config).unwrap())).unwrap();
    drop(db);
    assert!(runtime.item_configuration(ws, "waiting", waiting).unwrap().is_some());
    state.items.insert("group-finished-ci".into(), pushed);
    svc.store.workflow_state_for_tests(&state).unwrap();

    // A finished build left folders with open files. Seed its durable intent
    // and let the real dispatcher do preflight and retry the blocked sweep.
    let tree = tree_root.canonicalize().unwrap().join("finished");
    adapters::git(repository, &["worktree", "add", "-b", "finished", tree.to_str().unwrap()]).await.unwrap();
    let folders: Vec<String> = (0..16).map(|n| format!("output-{n:02}")).collect();
    for folder in &folders {
        std::fs::create_dir(tree.join(folder)).unwrap();
        std::fs::write(tree.join(folder).join("compiled"), b"fixture").unwrap();
    }
    svc.store.workflow_cleanup_enqueue(&farcooler_workflow::cleanup::CleanupEffect {
        id: "idle-cleanup".into(), workspace: ws.into(), revision: state.revision,
        item: "finished".into(), run: "finished-run".into(), restart: None,
        tree: Some(RunTree { name: "finished".into(), branch: "finished".into(), path: tree.to_string_lossy().into_owned(), base, adopted: false }),
        folders, uncertain: false, directory: None,
    }).unwrap();
    (config_dir, runtime)
}
