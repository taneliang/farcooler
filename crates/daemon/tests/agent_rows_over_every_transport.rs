//! `agent.rows` and `agent.rows_follow` (ov-366) against a real `farcoolerd`,
//! on the transports it serves, across a restart.
//!
//! - **The socket**: what the CLI speaks, and the Mac's FFI on its own runner.
//! - **`--stdio`, relayed into the running daemon**: what a phone's ssh
//!   session (and the Mac's, on another runner) lands on.
//! - **The FFI's JSON**, the phones' and the Mac's `core.call`, is the
//!   client crate's `ffi/rows_phone_tests.rs`, against this same binary.
//!
//! Each pages back through a claude pane's rows, follows a record appended to
//! its transcript, and after the daemon is killed and started again finds
//! the same rows by id, rebuilt from the file, in a new epoch that tells a
//! follower from before to page again.
//!
//! The daemon runs with `FARCOOLER_PROJECTOR=1`, and with `CLAUDE_CONFIG_DIR`
//! in the test's own directory: nothing here reads `~/.claude`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_protocol::v1::{self as pb, request, result, AgentRowChangeKind};
use farcooler_transport::{request as call_named, Client};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};

mod common;
use common::{listening_daemon_with_env, spawn, DaemonChild};

type Socket = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

fn env(dir: &Path) -> Vec<(&'static str, String)> {
    vec![("FARCOOLER_PROJECTOR", "1".into()), ("CLAUDE_CONFIG_DIR", dir.join("claude").to_string_lossy().into_owned())]
}

async fn daemon(dir: &Path) -> DaemonChild {
    let env = env(dir);
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    listening_daemon_with_env(dir, &env).await
}

async fn socket(dir: &Path) -> Socket {
    Client::connect(dir.join("farcoolerd.sock"), "test", "0.0.0").await.expect("socket handshake")
}

async fn call<R, W>(client: &Client<R, W>, method: &str, payload: request::Payload) -> result::Value
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut req = call_named(method);
    req.payload = Some(payload);
    let answer = client.call_with(req, Default::default()).await.unwrap_or_else(|e| panic!("{method}: {e:?}"));
    answer.value.unwrap_or_else(|| panic!("{method} returned nothing"))
}

/// A git repository under a root, and a worktree of it, through the socket.
async fn a_worktree(client: &mut Socket, dir: &Path) -> pb::Worktree {
    let repo = dir.join("repos").join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        assert!(std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap().success());
    }
    let root = call(client, "repository_root.add", request::Payload::RepositoryRootAdd(pb::RepositoryRootAdd {
        absolute_path: dir.join("repos").to_string_lossy().into_owned(),
        typed_confirmation: String::new(),
    }))
    .await;
    assert!(matches!(root, result::Value::RepositoryRoot(_)), "{root:?}");
    let result::Value::Repository(repository) = call(client, "repository.register", request::Payload::RepositoryRegister(pb::RepositoryRegister {
        relative_path: repo.to_string_lossy().into_owned(),
    }))
    .await
    else {
        panic!("register")
    };
    let mut create = call_named("worktree.create");
    create.target_resource_id = Some(repository.id.clone());
    create.payload = Some(request::Payload::WorktreeCreate(pb::WorktreeCreate {
        task_name: "rows".into(),
        branch: "feat/rows".into(),
        base_revision: "HEAD".into(),
        ..Default::default()
    }));
    let Some(result::Value::Worktree(worktree)) = client.call(create).await.expect("worktree.create").value else { panic!("worktree") };
    worktree
}

/// A claude pane (the stub runs in it), which declares its session id.
async fn a_claude_pane(client: &mut Socket, worktree: &pb::Worktree) -> pb::Terminal {
    let mut create = call_named("terminal.create");
    create.target_resource_id = Some(worktree.id.clone());
    create.payload = Some(request::Payload::TerminalCreate(pb::TerminalCreate {
        title: "claude".into(),
        command_preset: "claude".into(),
        join_active_group: false,
        prompt: None,
        task_key: None,
    }));
    let Some(result::Value::Terminal(terminal)) = client.call(create).await.expect("terminal.create").value else { panic!("terminal") };
    assert!(terminal.agent_session_id.is_some(), "a claude pane declares its session");
    terminal
}

fn prompt(id: &str, text: &str) -> serde_json::Value {
    json!({"type":"user","promptId":id,"promptSource":"typed","uuid":format!("u-{id}"),"timestamp":"2026-10-06T10:00:00Z","message":{"role":"user","content":text}})
}

fn reply(uuid: &str, text: &str) -> serde_json::Value {
    json!({"type":"assistant","uuid":uuid,"timestamp":"2026-10-06T10:00:01Z","message":{"content":[{"type":"text","text":text}],"stop_reason":"end_turn"}})
}

fn append(path: &Path, line: serde_json::Value) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    writeln!(file, "{line}").unwrap();
}

/// Where claude would write the pane's transcript: its config dir's project
/// directory for the worktree.
fn transcript(dir: &Path, worktree: &pb::Worktree, terminal: &pb::Terminal) -> PathBuf {
    let path = std::fs::canonicalize(worktree.worktree_path.as_deref().expect("host_admin sees paths")).unwrap();
    let slug = farcooler_core::session_log::claude_slug(&path.to_string_lossy());
    let project = dir.join("claude/projects").join(slug);
    std::fs::create_dir_all(&project).unwrap();
    project.join(format!("{}.jsonl", terminal.agent_session_id.as_deref().unwrap()))
}

async fn page<R, W>(client: &Client<R, W>, terminal: &pb::Terminal, before: Option<u64>, limit: u32) -> pb::AgentRowPage
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let payload = request::Payload::AgentRowsPage(pb::AgentRowsPage { terminal_id: terminal.id.clone(), before, limit });
    let result::Value::AgentRowPage(page) = call(client, "agent.rows", payload).await else { panic!("agent.rows") };
    page
}

async fn follow<R, W>(client: &Client<R, W>, terminal: &pb::Terminal, epoch: u64, after_rev: u64, wait_ms: u32) -> pb::AgentRowChanges
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let payload = request::Payload::AgentRowsFollow(pb::AgentRowsFollow { terminal_id: terminal.id.clone(), epoch, after_rev, wait_ms });
    let result::Value::AgentRowChanges(changes) = call(client, "agent.rows_follow", payload).await else { panic!("agent.rows_follow") };
    changes
}

fn ids(page: &pb::AgentRowPage) -> Vec<String> {
    page.rows.iter().map(|r| r.id.clone()).collect()
}

/// One transport's whole contract: page back, follow an append (with a
/// second call answered while the follow is held), and return the epoch.
async fn the_contract<R, W>(name: &str, client: &Client<R, W>, terminal: &pb::Terminal, path: &Path, n: u32) -> u64
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let newest = page(client, terminal, None, 2).await;
    assert_eq!(newest.rows.len(), 2, "{name}: {newest:?}");
    assert!(newest.more_before, "{name}");
    let older = page(client, terminal, Some(newest.rows[0].ord), 100).await;
    assert!(!older.rows.is_empty() && !older.more_before, "{name}: {older:?}");
    assert_eq!(older.rows.last().unwrap().ord + 1, newest.rows[0].ord, "{name}: pages abut");
    let row: serde_json::Value = serde_json::from_str(&older.rows[0].row_json).unwrap();
    assert_eq!(row["id"], "turn:p1", "{name}: the oldest row is the first prompt: {row}");
    assert_eq!(row["kind"]["Turn"]["prompt"], "Tidy the parser.", "{name}");

    let id = format!("p-{name}-{n}");
    let writer = {
        let (path, id) = (path.to_path_buf(), id.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            append(&path, prompt(&id, "And the lexer."));
            Instant::now()
        })
    };
    let held = follow(client, terminal, newest.epoch, newest.rev, 10_000);
    // The connection is not held up by a follow waiting on it.
    let meanwhile = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let started = Instant::now();
        let again = page(client, terminal, None, 1).await;
        assert_eq!(again.rows.len(), 1);
        started.elapsed()
    };
    let (changes, quick) = tokio::join!(held, meanwhile);
    let arrived = Instant::now();
    assert!(quick < Duration::from_secs(2), "{name}: a page waited behind the follow: {quick:?}");
    let written = writer.await.unwrap();
    assert!(!changes.reset, "{name}: {changes:?}");
    let insert = changes.changes.iter().find(|c| c.id == format!("turn:{id}")).unwrap_or_else(|| panic!("{name}: {changes:?}"));
    assert_eq!(insert.kind, AgentRowChangeKind::Insert as i32);
    assert!(insert.row.is_some());
    assert!(changes.rev > newest.rev);
    assert!(arrived.duration_since(written) < Duration::from_secs(2), "{name}: {:?}", arrived.duration_since(written));
    newest.epoch
}

#[tokio::test]
async fn rows_page_follow_and_survive_a_restart_on_every_transport() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = daemon(dir.path()).await;
    let mut client = socket(dir.path()).await;
    let worktree = a_worktree(&mut client, dir.path()).await;
    let terminal = a_claude_pane(&mut client, &worktree).await;
    let path = transcript(dir.path(), &worktree, &terminal);
    append(&path, prompt("p1", "Tidy the parser."));
    append(&path, reply("a1", "Tidied."));
    append(&path, prompt("p2", "Now the tests."));
    append(&path, reply("a2", "Green."));

    let socket_epoch = the_contract("socket", &client, &terminal, &path, 1).await;
    let (_relay, relayed) = spawn(dir.path()).await;
    let relayed_epoch = the_contract("stdio", &relayed, &terminal, &path, 1).await;
    assert_eq!(socket_epoch, relayed_epoch, "one projection, whichever way it is reached");
    let before = page(&client, &terminal, None, 100).await;
    drop((client, relayed));

    // The daemon goes; its tmux server and the pane stay, as in an upgrade.
    first.child.kill().await.unwrap();
    let _second = daemon(dir.path()).await;
    let client = socket(dir.path()).await;
    let after = page(&client, &terminal, None, 100).await;
    assert_eq!(ids(&after), ids(&before), "the same rows by id, rebuilt from the file");
    assert_ne!(after.epoch, before.epoch, "a new projection");
    let stale = follow(&client, &terminal, before.epoch, before.rev, 0).await;
    assert!(stale.reset, "a follower from before the restart pages again: {stale:?}");

    the_contract("socket after restart", &client, &terminal, &path, 2).await;
    let (_relay, relayed) = spawn(dir.path()).await;
    the_contract("stdio after restart", &relayed, &terminal, &path, 2).await;
}

/// Without `FARCOOLER_PROJECTOR=1` the methods are refused, and say what
/// they need.
#[tokio::test]
async fn without_the_flag_rows_are_refused_as_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = common::listening_daemon(dir.path()).await;
    let client = socket(dir.path()).await;
    let mut req = call_named("agent.rows");
    req.payload = Some(request::Payload::AgentRowsPage(pb::AgentRowsPage { terminal_id: uuid::Uuid::now_v7().as_bytes().to_vec().into(), before: None, limit: 0 }));
    match client.call_with(req, Default::default()).await {
        Err(farcooler_transport::ClientError::Daemon { code, .. }) => assert_eq!(code, pb::ErrorCode::CapabilityUnsupported as i32),
        other => panic!("{other:?}"),
    }
}

/// Measured, not asserted (`--ignored`): record-to-follow latency over 100
/// appends on the socket, and the cost of attaching to a 50,000-row session.
#[tokio::test]
#[ignore = "a measurement; run with --ignored --nocapture"]
async fn measure_follow_latency_and_attach_cost() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = daemon(dir.path()).await;
    let mut client = socket(dir.path()).await;
    let worktree = a_worktree(&mut client, dir.path()).await;
    let terminal = a_claude_pane(&mut client, &worktree).await;
    let path = transcript(dir.path(), &worktree, &terminal);
    append(&path, prompt("p0", "Start."));
    let first = page(&client, &terminal, None, 100).await;
    let (epoch, mut rev) = (first.epoch, first.rev);
    let mut lat = Vec::new();
    for n in 0..100 {
        let id = format!("m{n}");
        let writer = {
            let (path, id) = (path.clone(), id.clone());
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                append(&path, prompt(&id, "next"));
                Instant::now()
            })
        };
        let mut found = None;
        while found.is_none() {
            let changes = follow(&client, &terminal, epoch, rev, 5_000).await;
            rev = changes.rev.max(rev);
            if changes.changes.iter().any(|c| c.id == format!("turn:{id}")) {
                found = Some(Instant::now());
            }
        }
        lat.push(found.unwrap().duration_since(writer.await.unwrap()));
    }
    lat.sort();
    println!("record-to-follow over 100 appends: p50 {:?}, p95 {:?}, max {:?}", lat[49], lat[94], lat[99]);

    // 25,000 turns of a prompt and a reply: 50,000 rows.
    let other = a_claude_pane(&mut client, &worktree).await;
    let big = transcript(dir.path(), &worktree, &other);
    let mut text = String::new();
    for n in 0..25_000 {
        text.push_str(&format!("{}\n{}\n", prompt(&format!("b{n}"), "Another small change, please."), reply(&format!("r{n}"), "Done: one line moved.")));
    }
    std::fs::write(&big, &text).unwrap();
    let started = Instant::now();
    let attach = page(&client, &other, None, 0).await;
    let cold = started.elapsed();
    let started = Instant::now();
    let warm_page = page(&client, &other, None, 0).await;
    let warm = started.elapsed();
    let bytes: usize = warm_page.rows.iter().map(|r| r.row_json.len()).sum();
    println!(
        "attach to a 50,000-row session ({} MB): first page {:?} (rebuild), then {:?}; {} rows, {} bytes of rows",
        text.len() / 1_000_000,
        cold,
        warm,
        attach.rows.len(),
        bytes
    );
}
