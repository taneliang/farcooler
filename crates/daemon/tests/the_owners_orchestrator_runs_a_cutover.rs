//! ov-974: the owner's orchestrator runs the whole cutover from its own pane,
//! over the real socket, on a scratch runner: Prepare on the live daemon, which
//! stops into recovery, then Prepare through Finish against the recovery
//! daemon, which has no Store and admits the pane the live daemon pinned.
//!
//! Its own binary because of the stand-in, which the daemon reads once per
//! process (`an_orchestrator_starts_from_its_home.rs`). The stand-in is the
//! orchestrator's "agent": it waits for an ask file and runs this binary again
//! (`orchestrator_entry`) as its child, so every request comes from inside
//! the pane, as the orchestrator's own `farcooler workflow-cutover` would.
mod common;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::Client;
use tokio::process::Command;

const OP: &str = "FARCOOLER_TEST_OV974_OP";
const ANSWER: &str = "FARCOOLER_TEST_OV974_ANSWER";

/// One `workflow.cutover` over the real socket from wherever this runs, and
/// the reply written to `ANSWER`.
#[test]
fn orchestrator_entry() {
    let Ok(op) = std::env::var(OP) else { return };
    let socket = PathBuf::from(std::env::var_os("FARCOOLER_HOME").unwrap()).join("farcoolerd.sock");
    let answer = PathBuf::from(std::env::var_os(ANSWER).unwrap());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let reply = runtime.block_on(async {
        let mut client = Client::connect(&socket, "orchestrator", "0").await.map_err(|e| format!("{e:?}"))?;
        client.call(cutover(&op)).await.map(|r| format!("{r:?}")).map_err(|e| format!("{e:?}"))
    });
    std::fs::write(answer.with_extension("tmp"), match &reply { Ok(text) => format!("ok {text}"), Err(text) => format!("err {text}") }).unwrap();
    std::fs::rename(answer.with_extension("tmp"), answer).unwrap();
    std::process::exit(0);
}

fn cutover(json: &str) -> pb::Request {
    pb::Request { method: farcooler_protocol::workflow_cutover::METHOD.into(),
        payload: Some(request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json: json.into() })), ..Default::default() }
}

/// A scratch runner under a short, plain path (the stand-in's path must be
/// letters, digits and `/._-`), with one registered repository.
struct Runner { _dir: tempfile::TempDir, home: PathBuf, tmux: PathBuf, claude: PathBuf, talk: PathBuf, stand_in: PathBuf, workspace: uuid::Uuid }
impl Runner {
    async fn new() -> Self {
        let dir = tempfile::Builder::new().prefix("fc974").tempdir_in("/tmp").unwrap();
        let (home, tmux, claude, talk) = (dir.path().join("h"), dir.path().join("t"), dir.path().join("c"), dir.path().join("k"));
        for path in [&tmux, &claude, &talk] { std::fs::create_dir(path).unwrap(); }
        let repo = dir.path().join("r").join("demo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [vec!["init", "-q", "."], vec!["config", "user.email", "t@example.com"], vec!["config", "user.name", "t"],
            vec!["config", "commit.gpgsign", "false"], vec!["commit", "-q", "--allow-empty", "-m", "base"]] {
            assert!(std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap().success());
        }
        farcooler_daemon::service::stub_agents_in_this_process();
        let service = farcooler_daemon::service::Service::open_in(home.clone()).await.expect("service");
        assert_eq!(service.root_dir(), home);
        service.add_root(repo.parent().unwrap()).await.expect("root");
        let repository = service.register_repository(&repo).await.expect("registered").id;
        let workspace = service.store.ensure_main_workspace(repository).unwrap().id;
        drop(service);
        let stand_in = dir.path().join("agent");
        let exe = std::env::current_exe().unwrap();
        let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
        std::fs::write(&stand_in, format!(
            "#!/bin/sh\n\
             k={talk}\n\
             touch \"$k/ready\"\n\
             while :; do\n\
               if [ -f \"$k/ask\" ]; then\n\
                 op=$(cat \"$k/ask\"); rm -f \"$k/ask\"\n\
                 FARCOOLER_HOME={home} {OP}=\"$op\" {ANSWER}=\"$k/answer\" {exe} --exact orchestrator_entry --nocapture --test-threads=1 >/dev/null 2>&1\n\
               fi\n\
               sleep 0.05\n\
             done\n", talk = quote(&talk), home = quote(&home), exe = quote(&exe))).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(stand_in.to_str().unwrap().chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)), "{stand_in:?}");
        let runner = Self { _dir: dir, home, tmux, claude, talk, stand_in, workspace };
        released(&runner.db()).await;
        runner
    }
    fn db(&self) -> PathBuf { farcooler_daemon::cutover_admin::database(&self.home) }
    fn artifacts(&self) -> PathBuf { farcooler_store::engine_cutover::startup::artifact_dir(&self.db()) }
    fn socket(&self) -> PathBuf { self.home.join("farcoolerd.sock") }
    fn daemon(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_farcoolerd"));
        command.env("FARCOOLER_HOME", &self.home).env("FARCOOLER_CONFIG", self.home.join("config.toml")).env("CLAUDE_CONFIG_DIR", &self.claude)
            .env("TMUX_TMPDIR", &self.tmux).env("FARCOOLER_TEST_STUB_AGENTS", "1").env("FARCOOLER_STAND_IN_AGENT", &self.stand_in)
            .stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
        command
    }
    /// Ask from inside the orchestrator's pane, and the reply.
    async fn ask_from_the_pane(&self, json: &str) -> Result<String, String> {
        let answer = self.talk.join("answer");
        let _ = std::fs::remove_file(&answer);
        std::fs::write(self.talk.join("ask.tmp"), json).unwrap();
        std::fs::rename(self.talk.join("ask.tmp"), self.talk.join("ask")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if let Ok(text) = std::fs::read_to_string(&answer) {
                return match text.split_once(' ') { Some(("ok", rest)) => Ok(rest.into()), Some((_, rest)) => Err(rest.into()), None => Err(text) };
            }
            assert!(Instant::now() < deadline, "the pane never answered {json}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Until nothing holds the database: a dropped `Service`'s tasks hold it a while.
async fn released(db: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match farcooler_store::engine_cutover::JoinedOwners::acquire(db, "released") {
            Ok(free) => { drop(free); return; }
            Err(e) => assert!(Instant::now() < deadline, "the setup never released the database: {e}"),
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn answering(socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while tokio::net::UnixStream::connect(socket).await.is_err() {
        assert!(Instant::now() < deadline, "nothing answered on {}", socket.display());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owners_orchestrator_takes_a_real_runner_from_prepare_to_finish() {
    let runner = Runner::new().await;
    let _reaper = common::TmuxReaper::new(&runner.home);
    let mut live = runner.daemon().spawn().unwrap();
    answering(&runner.socket()).await;

    // The workspace's orchestrator, on the runner's own login: the owner's.
    let mut client = Client::connect(&runner.socket(), "setup", "0").await.expect("a hello");
    let mut start = farcooler_transport::request("workspace.start_orchestrator");
    start.target_resource_id = Some(bytes::Bytes::copy_from_slice(runner.workspace.as_bytes()));
    start.payload = Some(request::Payload::WorkspaceStartOrchestrator(pb::WorkspaceStartOrchestrator { harness: "claude".into(), replace: false, handoff_task: String::new() }));
    let Some(result::Value::Terminal(_)) = client.call(start).await.expect("the orchestrator starts").value else { panic!("not a terminal") };
    let deadline = Instant::now() + Duration::from_secs(90);
    while !runner.talk.join("ready").exists() {
        assert!(Instant::now() < deadline, "the stand-in never started in the pane");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The inventory reads the new pane before the caller is walked.
    client.call(farcooler_transport::request("host.get")).await.expect("host.get");
    drop(client);

    let held = runner.ask_from_the_pane(r#"{"op":"Prepare","attempt":"by-orchestrator"}"#).await.expect("the live daemon holds for the owner's orchestrator");
    assert!(held.contains("by-orchestrator"), "{held}");
    let status = tokio::time::timeout(Duration::from_secs(30), live.wait()).await.expect("it stops").unwrap();
    assert_eq!(status.code(), Some(farcooler_daemon::rpc::recovery::RESTART_EXIT), "{status:?}");
    assert!(runner.artifacts().join("hold.json").is_file());

    let mut recovery = runner.daemon().spawn().unwrap();
    answering(&runner.socket()).await;
    for op in [r#"{"op":"Prepare","attempt":"by-orchestrator"}"#, r#"{"op":"Commit"}"#, r#"{"op":"Verify"}"#, r#"{"op":"Release"}"#] {
        runner.ask_from_the_pane(op).await.unwrap_or_else(|e| panic!("{op}: {e}"));
        assert_eq!(recovery.try_wait().unwrap(), None, "recovery stays up through {op}");
    }
    let asked = std::fs::read_to_string(runner.artifacts().join("journal/commit-requested-1.json")).expect("the Commit names who asked");
    assert!(asked.contains("the owner's orchestrator") && asked.contains("acting for the owner") && asked.contains("pid "), "{asked}");
    runner.ask_from_the_pane(r#"{"op":"Finish"}"#).await.expect("finish");
    let status = tokio::time::timeout(Duration::from_secs(30), recovery.wait()).await.expect("recovery ends with the attempt").unwrap();
    assert_eq!(status.code(), Some(farcooler_daemon::rpc::recovery::RESTART_EXIT), "{status:?}");
    assert!(!runner.artifacts().exists());
    assert!(!runner.home.join(farcooler_daemon::cutover_delegate::FILE).exists(), "Finish forgets the pane it admitted");
}
