//! The cutover's boundary matrix. At every durable boundary a real process runs
//! the real stage and is killed by the kernel, then the runner is reopened
//! through the real `farcoolerd` main, `--stream` and `--stdio`, `Service` and
//! `Store`. Nothing here constructs a crash state.
//!
//! SIGKILL leaves the page cache alone, so this proves what a crashed process
//! leaves behind, not that an `fsync` reached the disk.
//!
//! The helper is this test binary run again: `cutover_helper_entry` does nothing
//! until `HELPER` names a stage.
mod common;
use std::collections::BTreeMap;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use farcooler_core::DomainError;
use farcooler_daemon::{cutover_admin, service::Service};
use farcooler_protocol::v1::{self as pb, request};
use farcooler_store::engine_cutover::{boundary, startup, transfer};
use farcooler_store::{Actor, Store};
use farcooler_transport::{Client, ClientError};
use tokio::process::Command;

const HELPER: &str = "FARCOOLER_TEST_CUTOVER_HELPER";
const PERSON: &str = "FARCOOLER_TEST_CUTOVER_PERSON";
const ANSWER: &str = "FARCOOLER_TEST_CUTOVER_ANSWER";
const ATTEMPT: &str = "matrix";
const REFUSED: i32 = 3;

/// Run one stage in this process, then leave without the harness's report.
#[test]
fn cutover_helper_entry() {
    let Ok(op) = std::env::var(HELPER) else { return };
    let root = PathBuf::from(std::env::var_os("FARCOOLER_HOME").unwrap());
    let db = cutover_admin::database(&root);
    let done = match op.as_str() {
        "hold" => transfer::hold(&db, ATTEMPT).map(|()| serde_json::Value::Null).map_err(|e| e.to_string()),
        "prepare" => cutover_admin::prepare(&root, ATTEMPT).map_err(|e| e.to_string()),
        "commit" => cutover_admin::commit(&root).map_err(|e| e.to_string()),
        "verify" => cutover_admin::verify(&root).map_err(|e| e.to_string()),
        "release" => cutover_admin::release(&root).map_err(|e| e.to_string()),
        "finish" => cutover_admin::finish(&root).map_err(|e| e.to_string()),
        "recover" => cutover_admin::recover(&root).map_err(|e| e.to_string()),
        other => panic!("no stage {other}"),
    };
    match done {
        Ok(_) => std::process::exit(0),
        Err(why) => { eprintln!("refused: {why}"); std::process::exit(REFUSED) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Source { Raw60, Legacy }

/// One runner under a short path, with everything a daemon reaches outside it.
struct Runner { _dir: tempfile::TempDir, home: PathBuf, tmux: PathBuf, claude: PathBuf, original: Vec<u8> }
impl Runner {
    fn new(source: Source) -> Self {
        let dir = tempfile::Builder::new().prefix("fc963").tempdir_in("/tmp").unwrap();
        let home = dir.path().join("h");
        let (tmux, claude) = (dir.path().join("t"), dir.path().join("c"));
        for path in [&home, &tmux, &claude] { std::fs::create_dir(path).unwrap(); }
        let db = cutover_admin::database(&home);
        match source {
            Source::Raw60 => farcooler_store::testing::write_source60(&db),
            Source::Legacy => {
                let store = Store::open(&db).unwrap();
                let host = uuid::Uuid::now_v7();
                let root = store.create_repository_root(host, "resources/synthetic", 0).unwrap();
                let repository = store.create_repository(host, root.id, "Synthetic", "resources/synthetic/.git", "").unwrap();
                let workspace = store.ensure_main_workspace(repository.id).unwrap();
                store.create_task(workspace.id, "Retained synthetic history", Actor::User).unwrap();
            }
        }
        let original = std::fs::read(&db).unwrap();
        Self { _dir: dir, home, tmux, claude, original }
    }
    fn db(&self) -> PathBuf { cutover_admin::database(&self.home) }
    fn artifacts(&self) -> PathBuf { startup::artifact_dir(&self.db()) }
    fn socket(&self) -> PathBuf { self.home.join("farcoolerd.sock") }
    fn daemon(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_farcoolerd"));
        self.sandbox(&mut command);
        command.stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
        command
    }
    fn sandbox(&self, command: &mut Command) {
        command.env("FARCOOLER_HOME", &self.home).env("FARCOOLER_CONFIG", self.home.join("config.toml")).env("CLAUDE_CONFIG_DIR", &self.claude)
            .env("TMUX_TMPDIR", &self.tmux).env("FARCOOLER_TEST_STUB_AGENTS", "1").env_remove(boundary::KILL_AT);
    }
    /// Run a stage in its own process. `Some(signal)` when it died of one.
    async fn stage(&self, op: &str, kill_at: Option<&str>) -> Result<i32, i32> {
        let mut command = Command::new(std::env::current_exe().unwrap());
        self.sandbox(&mut command);
        command.args(["--exact", "cutover_helper_entry", "--nocapture", "--test-threads=1"]).env(HELPER, op).stdout(Stdio::null());
        if let Some(name) = kill_at { command.env(boundary::KILL_AT, name); }
        let status = tokio::time::timeout(Duration::from_secs(120), command.status()).await.expect("the helper ended").unwrap();
        match status.signal() { Some(signal) => Err(signal), None => Ok(status.code().unwrap()) }
    }
    async fn must(&self, op: &str) { assert_eq!(self.stage(op, None).await, Ok(0), "{op} on {:?}", self.home); }
    fn phase(&self) -> Option<String> { self.settled().phase }
    /// What a reader finds, from a copy, so reading can't roll back the original.
    fn settled(&self) -> State {
        let scratch = tempfile::tempdir().unwrap();
        let copy = scratch.path().join("x.db");
        std::fs::copy(self.db(), &copy).unwrap();
        let journal = format!("{}-journal", self.db().display());
        if Path::new(&journal).exists() { std::fs::copy(&journal, scratch.path().join("x.db-journal")).unwrap(); }
        let conn = rusqlite::Connection::open(&copy).unwrap();
        let meta = |key: &str| conn.query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get::<_, String>(0)).ok();
        let (schema, floor) = (meta("schema_version"), meta("compatible_down_to"));
        let phase = conn.query_row("SELECT phase FROM engine_cutover_authority", [], |r| r.get(0)).ok();
        let frozen = conn.query_row("SELECT id FROM engine_cutover_attempts WHERE id LIKE 'frozen:%'", [], |r| r.get(0)).ok();
        drop(conn);
        State { schema, floor, phase, frozen, bytes: std::fs::read(&copy).unwrap() }
    }
    /// Every path under the home and its bytes, apart from what merely starting
    /// a process leaves: its log, its locks, the owner lock and its socket.
    fn tree(&self) -> BTreeMap<String, Vec<u8>> {
        fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap().map(Result::unwrap) {
                let path = entry.path();
                let name = path.strip_prefix(base).unwrap().display().to_string();
                if name == "logs" || name.ends_with(".lock") || name.ends_with(".owners") || name.ends_with(".sock") { continue; }
                if entry.file_type().unwrap().is_dir() { out.insert(format!("{name}/"), Vec::new()); walk(&path, base, out); }
                else { out.insert(name, std::fs::read(&path).unwrap()); }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.home, &self.home, &mut out);
        out
    }
}
struct State { schema: Option<String>, floor: Option<String>, phase: Option<String>, frozen: Option<String>, bytes: Vec<u8> }

fn host() -> pb::Request { pb::Request { method: "host.get".into(), ..Default::default() } }
async fn answering(socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while tokio::net::UnixStream::connect(socket).await.is_err() {
        assert!(Instant::now() < deadline, "nothing answered on {}", socket.display());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
fn terminate(child: &tokio::process::Child) {
    // SAFETY: a signal to a child this test still holds, so its pid is its own.
    unsafe { libc::kill(child.id().unwrap() as i32, libc::SIGTERM); }
}
/// Real main, on a held runner: up, and answering nothing but recovery.
async fn main_serves_only_recovery(runner: &Runner) {
    let mut main = runner.daemon().spawn().unwrap();
    answering(&runner.socket()).await;
    let mut client = Client::connect(&runner.socket(), "matrix", "0").await.expect("recovery completes a hello");
    let refused = client.call(host()).await.expect_err("no ordinary method is served");
    assert!(matches!(&refused, ClientError::Daemon { .. }), "{refused:?}");
    drop(client);
    terminate(&main);
    let status = tokio::time::timeout(Duration::from_secs(20), main.wait()).await.expect("recovery leaves when told").unwrap();
    assert_eq!(status.code(), Some(0));
}
/// Real main, serving as usual.
async fn main_serves_ordinarily(runner: &Runner) {
    let _reaper = common::TmuxReaper::new(&runner.home);
    let mut main = runner.daemon().spawn().unwrap();
    answering(&runner.socket()).await;
    let mut client = Client::connect(&runner.socket(), "matrix", "0").await.expect("a hello");
    client.call(host()).await.expect("ordinary service");
    drop(client);
    terminate(&main);
    let _ = tokio::time::timeout(Duration::from_secs(20), main.wait()).await;
}
async fn stream_is_refused(runner: &Runner) {
    let mut command = runner.daemon();
    command.args(["--stream", &uuid::Uuid::now_v7().to_string()]).stdout(Stdio::piped());
    let output = tokio::time::timeout(Duration::from_secs(30), command.output()).await.expect("a refused stream exits").unwrap();
    assert!(!output.status.success() && output.stdout.is_empty(), "{output:?}");
}
async fn stdio_tells_the_device(runner: &Runner) {
    let mut command = runner.daemon();
    command.arg("--stdio").stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let (read, write) = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let told = tokio::time::timeout(Duration::from_secs(30), Client::over(read, write, "matrix", "0")).await.expect("stdio answers").err().expect("a refusal");
    assert!(matches!(&told, ClientError::Daemon { message, .. } if *message == DomainError::CutoverHeld.to_string()), "{told:?}");
}
async fn in_process_seams_refuse(runner: &Runner) {
    assert!(matches!(Service::open_in(runner.home.clone()).await.err(), Some(DomainError::CutoverHeld)));
    assert!(matches!(Store::open(runner.db()).err(), Some(DomainError::CutoverHeld)));
}

/// What one boundary leaves behind, relative to the artifact directory.
#[derive(Default)]
struct Row { has: &'static [&'static str], lacks: &'static [&'static str], torn: &'static [&'static str], upgraded: bool, phase: Option<&'static str>, ordinary: bool }
fn row(name: &str, source: Source) -> Row {
    let quiesced = Row { upgraded: true, phase: Some("quiesced"), ..Row::default() };
    let committed = Row { upgraded: true, phase: Some("committed"), ..Row::default() };
    let released = Row { upgraded: true, phase: Some("released"), ..Row::default() };
    match name {
        "hold.before" => Row { ordinary: true, ..Row::default() },
        "hold.torn" => Row { torn: &["hold.json"], ..Row::default() },
        "hold.after" => Row { has: &["hold.json"], ..Row::default() },
        "begin.before-baseline-db" => Row { lacks: &["baseline.db"], ..Row::default() },
        "begin.after-baseline-db" => Row { has: &["baseline.db"], lacks: &["baseline.json"], ..Row::default() },
        "begin.torn-baseline" => Row { has: &["baseline.db"], torn: &["baseline.json"], lacks: &["intent.json"], ..Row::default() },
        "begin.before-intent" => Row { has: &["baseline.db", "baseline.json"], lacks: &["intent.json"], ..Row::default() },
        "begin.torn-intent" => Row { has: &["baseline.db", "baseline.json"], torn: &["intent.json"], ..Row::default() },
        "begin.after-intent" | "begin.in-upgrade" => Row { has: &["baseline.db", "baseline.json", "intent.json"], ..Row::default() },
        "begin.after-upgrade" => Row { has: &["baseline.db", "baseline.json", "intent.json"], lacks: &["closed-1"], ..quiesced },
        "commit.before-copy" => Row { has: &["closed-1"], lacks: &["closed-1/db"], ..quiesced },
        "commit.after-copy" => Row { has: &["closed-1/db"], lacks: &["closed-1/SNAPSHOT.json"], ..quiesced },
        "commit.torn-snapshot" => Row { has: &["closed-1/db"], torn: &["closed-1/SNAPSHOT.json"], ..quiesced },
        "commit.after-snapshot" => Row { has: &["closed-1/db", "closed-1/SNAPSHOT.json"], lacks: &["journal"], ..quiesced },
        "commit.after-restore" => Row { has: &["closed-1/SNAPSHOT.json"], lacks: &["closed-1/restore-check", "journal"], ..quiesced },
        "journal.torn" => Row { torn: &["journal/backed_up-closed-1.json"], ..quiesced },
        "commit.after-permit" | "commit.in-mapping" => Row { has: &["journal/backed_up-closed-1.json"], lacks: &["journal/committed.json"], ..quiesced },
        "commit.after-mapping" => Row { has: &["journal/backed_up-closed-1.json"], lacks: &["journal/committed.json"], ..committed },
        "commit.after-journal" => Row { has: &["journal/committed.json"], lacks: &["journal/verified.json"], ..committed },
        "verify.after" => Row { lacks: &["journal/verified.json"], ..committed },
        "release.in-transaction" => Row { has: &["journal/verified.json"], lacks: &["journal/released.json"], ..committed },
        "release.after" => Row { lacks: &["journal/released.json"], ..released },
        "finish.before-rename" => Row { has: &["journal/resolved.json"], ..released },
        "finish.after-rename" => Row { ordinary: source == Source::Legacy, ..released },
        "recover.before-rename" => Row { has: &["restore.db"], ..quiesced },
        "recover.after-rename" => Row { has: &["baseline.db"], ..Row::default() },
        "recover.after-retire" => Row { ordinary: source == Source::Legacy, ..Row::default() },
        other => panic!("no row for {other}"),
    }
}
fn stage_of(name: &str) -> &'static str {
    match name.split('.').next().unwrap() {
        "hold" => "hold", "begin" => "prepare", "commit" | "journal" => "commit", "verify" => "verify",
        "release" => "release", "finish" => "finish", "recover" => "recover", other => panic!("no stage for {other}"),
    }
}
/// The stages before the one that dies at `name`.
fn before(name: &str, source: Source) -> Vec<&'static str> {
    let order = ["hold", "prepare", "commit", "verify", "release", "finish"];
    let stage = stage_of(name);
    let run: Vec<&str> = if stage == "recover" { vec!["hold", "prepare"] } else { order.iter().copied().take_while(|s| *s != stage).collect() };
    run.into_iter().filter(|s| source == Source::Legacy || *s != "hold").collect()
}
fn committed_already(name: &str) -> bool {
    matches!(stage_of(name), "verify" | "release" | "finish") || matches!(name, "commit.after-mapping" | "commit.after-journal")
}

async fn crash(source: Source, name: &'static str) -> Runner {
    let runner = Runner::new(source);
    for stage in before(name, source) { runner.must(stage).await; }
    assert_eq!(runner.stage(stage_of(name), Some(name)).await, Err(libc::SIGKILL), "{name} was never reached: a helper that finishes proves nothing");
    runner
}
fn check_row(runner: &Runner, name: &str, source: Source) {
    let row = row(name, source);
    let (found, dir) = (runner.settled(), runner.artifacts());
    for file in row.has { assert!(dir.join(file).exists(), "{name}: {file} must exist"); }
    for file in row.lacks { assert!(!dir.join(file).exists(), "{name}: {file} must not exist yet"); }
    for file in row.torn {
        assert!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(dir.join(file)).unwrap()).is_err(), "{name}: {file} must be torn");
    }
    // Present is not enough: a file that exists at its boundary must already be whole.
    if row.has.contains(&"baseline.db") || (found.frozen.is_some() && dir.exists()) {
        assert!(std::fs::read(dir.join("baseline.db")).unwrap() == runner.original, "{name}: the baseline is the exact source");
    }
    if row.has.contains(&"closed-1/db") {
        assert!(std::fs::read(dir.join("closed-1/db")).unwrap() == found.bytes, "{name}: the closed backup is the frozen file");
    }
    let phase = row.phase.or(if source == Source::Legacy { Some("legacy") } else { None });
    assert_eq!(found.phase.as_deref(), phase, "{name}");
    if row.upgraded {
        let current = found.schema.clone().unwrap();
        assert_eq!(found.floor.as_deref(), Some(current.as_str()), "{name}: upgraded means the refusing floor, atomically");
        assert_ne!(Some(current.as_str()), if source == Source::Raw60 { Some("60") } else { None });
        assert_eq!(found.frozen.as_deref(), Some("frozen:matrix"), "{name}: upgraded means the bound attempt, atomically");
    } else {
        assert!(found.bytes == runner.original, "{name}: nothing upgraded means the exact source, after any rollback");
    }
}
/// Every seam, on a runner one of them must not change.
async fn every_seam_is_held(runner: &Runner, name: &str) {
    let before = runner.tree();
    assert!(!before.contains_key("install-id"));
    in_process_seams_refuse(runner).await;
    stream_is_refused(runner).await;
    stdio_tells_the_device(runner).await;
    main_serves_only_recovery(runner).await;
    let after = runner.tree();
    let changed: Vec<_> = before.keys().chain(after.keys()).filter(|k| before.get(*k) != after.get(*k)).collect();
    assert!(changed.is_empty(), "{name}: a refused start changed {changed:?}");
    assert_eq!(std::fs::read_dir(&runner.tmux).unwrap().count(), 0, "{name}: a refused start reached tmux");
}
/// The way back from a crash, by fresh proof.
async fn back(runner: &Runner, source: Source, name: &str) {
    let settled = runner.settled().bytes;
    let answer = runner.stage("recover", None).await;
    if committed_already(name) {
        assert_eq!(answer, Ok(REFUSED), "{name}: committed recovers forward only");
        assert!(runner.settled().bytes == settled, "{name}: a refused Recover moves nothing");
        if row(name, source).ordinary { main_serves_ordinarily(runner).await; }
        return;
    }
    let nothing_held = matches!(name, "hold.before" | "recover.after-retire");
    assert_eq!(answer, Ok(if nothing_held { REFUSED } else { 0 }), "{name}");
    assert!(std::fs::read(runner.db()).unwrap() == runner.original, "{name}: Recover returns the exact original bytes");
    assert!(!runner.artifacts().exists(), "{name}");
    if source == Source::Legacy { main_serves_ordinarily(runner).await; } else { main_serves_only_recovery(runner).await; }
}
/// The way on: whatever state the crash left, drive it to ordinary service.
async fn forward(runner: &Runner, source: Source, name: &str) {
    for _ in 0..12 {
        let exists = runner.artifacts().exists();
        match runner.phase().as_deref() {
            None | Some("legacy") if exists => {
                if runner.stage("prepare", None).await != Ok(0) {
                    runner.must("recover").await;
                    if source == Source::Legacy { runner.must("hold").await; }
                    runner.must("prepare").await;
                }
            }
            None | Some("legacy") => { if source == Source::Legacy { runner.must("hold").await; } runner.must("prepare").await; }
            Some("quiesced") => runner.must("commit").await,
            // A repeat of Commit finishes its journal and changes nothing else.
            Some("committed") => { runner.must("commit").await; runner.must("verify").await; runner.must("release").await; }
            Some("released") if exists => runner.must("finish").await,
            Some("released") => break,
            other => panic!("{name}: {other:?}"),
        }
    }
    assert_eq!(runner.phase().as_deref(), Some("released"), "{name}");
    let parent = runner.home.clone();
    let kept: Vec<_> = std::fs::read_dir(parent).unwrap().map(|e| e.unwrap().path()).filter(|p| p.display().to_string().contains("-resolved-")).collect();
    assert_eq!(kept.len(), 1, "{name}");
    assert!(kept[0].join("baseline.db").is_file() && kept[0].join("journal/resolved.json").is_file(), "{name}");
    assert!(std::fs::read_dir(&kept[0]).unwrap().any(|e| {
        let path = e.unwrap().path();
        path.file_name().unwrap().to_string_lossy().starts_with("closed-") && path.join("db").is_file() && path.join("SNAPSHOT.json").is_file()
    }), "{name}: finished with a closed backup");
    if source == Source::Legacy { main_serves_ordinarily(runner).await; }
}

/// `CUTOVER_ONLY` narrows a run for `reports/ov-963/mutants.py`. Each case stands
/// alone, and each red one is named, so a mutant can be held to its boundary.
async fn whole_matrix(source: Source) {
    let only = std::env::var("CUTOVER_ONLY").ok();
    let wanted = |name: &str| only.as_deref().is_none_or(|list| list.split(',').any(|n| n == name));
    let room = Arc::new(tokio::sync::Semaphore::new(4));
    let mut cases = Vec::new();
    for &name in boundary::ALL {
        if !wanted(name) || (source == Source::Raw60 && stage_of(name) == "hold") { continue; }
        let room = room.clone();
        cases.push((name, tokio::spawn(async move {
            let _seat = room.acquire().await.unwrap();
            let runner = crash(source, name).await;
            check_row(&runner, name, source);
            // `back` starts main itself on a runner that should be ordinary.
            if !row(name, source).ordinary && !matches!(name, "finish.after-rename" | "recover.after-retire") { every_seam_is_held(&runner, name).await; }
            back(&runner, source, name).await;
            let runner = crash(source, name).await;
            forward(&runner, source, name).await;
        })));
    }
    assert!(!cases.is_empty());
    let mut red = Vec::new();
    for (name, case) in cases {
        if case.await.is_err() { eprintln!("BOUNDARY-RED {name}"); red.push(name); }
    }
    assert!(red.is_empty(), "red at {red:?}");
}
#[tokio::test(flavor = "multi_thread")]
async fn a_raw_schema60_runner_survives_a_crash_at_every_boundary() { whole_matrix(Source::Raw60).await; }
#[tokio::test(flavor = "multi_thread")]
async fn a_current_runner_survives_a_crash_at_every_boundary() { whole_matrix(Source::Legacy).await; }

/// One `workflow.cutover` over the real socket, from a process that really is a
/// person to the recovery seat, and the reply.
#[test]
fn cutover_person_entry() {
    let Ok(op) = std::env::var(PERSON) else { return };
    let socket = PathBuf::from(std::env::var_os("FARCOOLER_HOME").unwrap()).join("farcoolerd.sock");
    let answer = PathBuf::from(std::env::var_os(ANSWER).unwrap());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let reply = runtime.block_on(async {
        let mut client = Client::connect(&socket, "person", "0").await.map_err(|e| format!("{e:?}"))?;
        let request = pb::Request { method: farcooler_protocol::workflow_cutover::METHOD.into(),
            payload: Some(request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json: op })), ..Default::default() };
        client.call(request).await.map(|r| format!("{r:?}")).map_err(|e| format!("{e:?}"))
    });
    std::fs::write(answer.with_extension("tmp"), match &reply { Ok(text) => format!("ok {text}"), Err(text) => format!("err {text}") }).unwrap();
    std::fs::rename(answer.with_extension("tmp"), answer).unwrap();
    std::process::exit(0);
}
/// An orphan with a terminal of its own, outside every pane however this test
/// was started: what a person in a plain terminal looks like to the walk.
async fn as_a_person(runner: &Runner, json: &str) -> Result<String, String> {
    let answer = runner.home.parent().unwrap().join("answer");
    let _ = std::fs::remove_file(&answer);
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "cutover_person_entry", "--nocapture", "--test-threads=1"]).env(PERSON, json).env(ANSWER, &answer)
        .env("FARCOOLER_HOME", &runner.home).env("FARCOOLER_CONFIG", runner.home.join("config.toml")).stdout(Stdio::null()).stderr(Stdio::null());
    // SAFETY: the closure runs between fork and exec and calls only what that allows.
    unsafe { command.pre_exec(|| {
        match libc::fork() { -1 => return Err(std::io::Error::last_os_error()), 0 => {} _ => libc::_exit(0) }
        libc::setsid();
        let (mut master, mut slave) = (0, 0);
        if libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) != 0 { return Err(std::io::Error::last_os_error()); }
        libc::ioctl(slave, libc::TIOCSCTTY as _, 0);
        Ok(())
    }); }
    command.status().unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(text) = std::fs::read_to_string(&answer) {
            return match text.split_once(' ') { Some(("ok", rest)) => Ok(rest.into()), Some((_, rest)) => Err(rest.into()), None => Err(text) };
        }
        assert!(Instant::now() < deadline, "the person never answered {json}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn a_person_takes_a_real_runner_from_prepare_to_ordinary_service() {
    let runner = Runner::new(Source::Legacy);
    let _reaper = common::TmuxReaper::new(&runner.home);
    let tasks = |runner: &Runner| rusqlite::Connection::open(runner.db()).unwrap().query_row("SELECT count(*) FROM tasks", [], |r| r.get::<_, i64>(0)).unwrap();
    let mut live = runner.daemon().spawn().unwrap();
    answering(&runner.socket()).await;
    as_a_person(&runner, r#"{"op":"Prepare","attempt":"real"}"#).await.expect("the live daemon holds and stops");
    let status = tokio::time::timeout(Duration::from_secs(20), live.wait()).await.expect("it stops").unwrap();
    assert!(status.success(), "{status:?}");
    assert!(runner.artifacts().join("hold.json").is_file());

    let mut recovery = runner.daemon().spawn().unwrap();
    answering(&runner.socket()).await;
    for op in [r#"{"op":"Prepare","attempt":"real"}"#, r#"{"op":"Commit"}"#, r#"{"op":"Verify"}"#, r#"{"op":"Release"}"#] {
        as_a_person(&runner, op).await.unwrap_or_else(|e| panic!("{op}: {e}"));
        assert_eq!(recovery.try_wait().unwrap(), None, "recovery stays up through {op}");
    }
    as_a_person(&runner, r#"{"op":"Finish"}"#).await.expect("finish");
    let status = tokio::time::timeout(Duration::from_secs(20), recovery.wait()).await.expect("recovery ends with the attempt").unwrap();
    assert_eq!(status.code(), Some(farcooler_daemon::rpc::recovery::RESTART_EXIT), "a supervisor restarts only a failure");
    assert!(!runner.artifacts().exists());
    assert_eq!(runner.phase().as_deref(), Some("released"));
    assert_eq!(tasks(&runner), 1, "the history came through");
    main_serves_ordinarily(&runner).await;
}
/// A process from before the owner lock, holding a real connection across
/// Recover: it must be named and stop Recover, or every write it makes after
/// the restore goes to an inode no one will ever read.
#[tokio::test(flavor = "multi_thread")]
async fn an_old_build_cannot_lose_its_writes_across_recover() {
    let runner = Runner::new(Source::Legacy);
    runner.must("hold").await;
    runner.must("prepare").await;
    let mut old = std::process::Command::new("/usr/bin/python3")
        .args(["-I", "-c", "import sqlite3,sys\nc=sqlite3.connect(sys.argv[1],isolation_level=None)\nc.execute('SELECT count(*) FROM meta').fetchall()\nprint('open',flush=True)\nsys.stdin.readline()\nc.execute(\"INSERT INTO meta VALUES('old-build-write','kept')\")\n"])
        .arg(runner.db()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    { use std::io::BufRead; let mut line = String::new(); std::io::BufReader::new(old.stdout.as_mut().unwrap()).read_line(&mut line).unwrap(); assert_eq!(line.trim(), "open"); }
    let refused = runner.stage("recover", None).await;
    { use std::io::Write; writeln!(old.stdin.as_mut().unwrap()).unwrap(); }
    old.wait().unwrap();
    let kept = rusqlite::Connection::open(runner.db()).unwrap().query_row("SELECT count(*) FROM meta WHERE key='old-build-write'", [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!((refused, kept), (Ok(REFUSED), 1), "Recover waits, and the old build's write is still in the file a reader opens");
}
