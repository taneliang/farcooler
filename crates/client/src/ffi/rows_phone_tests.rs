//! A terminal's agent rows the way an app reads them (ov-366): `dispatch`
//! with the JSON an app passes, a `Session`, and a real `farcoolerd` run with
//! `FARCOOLER_PROJECTOR=1`, killed and started again. The daemon's own tests
//! cover the socket and the relayed stdio (`tests/agent_rows_over_every_
//! transport.rs`); this is the FFI's JSON on top, which iOS, Android and the
//! Mac decode.

use std::path::{Path, PathBuf};
use std::time::Duration;

use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::{Client, request as named};
use serde_json::{Value, json};

use super::dispatch;
use crate::session::Session;

/// `farcoolerd` beside this test binary's directory (see
/// `tests/against_a_real_daemon.rs`, which finds it the same way).
fn daemon_binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("test executable path");
    path.pop();
    path.pop();
    path.push("farcoolerd");
    assert!(path.is_file(), "no farcoolerd at {}: run `cargo build -p farcooler-daemon` first", path.display());
    path
}

struct Daemon {
    process: std::process::Child,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Reaps the tmux server of the runtime directory when the test ends,
/// however many daemons it ran there.
struct Reaper(PathBuf);

impl Drop for Reaper {
    fn drop(&mut self) {
        if let Ok(install) = std::fs::read_to_string(self.0.join("install-id")) {
            farcooler_tmux::reap_server(&format!("farcooler-{}", install.trim()));
        }
    }
}

async fn daemon(dir: &Path) -> Daemon {
    let process = std::process::Command::new(daemon_binary())
        .env("FARCOOLER_HOME", dir)
        .env("FARCOOLER_TEST_STUB_AGENTS", "1")
        .env("FARCOOLER_PROJECTOR", "1")
        .env("CLAUDE_CONFIG_DIR", dir.join("claude"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn farcoolerd");
    for _ in 0..200 {
        if tokio::net::UnixStream::connect(dir.join("farcoolerd.sock")).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Daemon { process }
}

/// A worktree and a claude pane in it, made over the socket, and the path
/// claude would write the pane's transcript to.
async fn a_claude_pane(dir: &Path) -> (String, PathBuf) {
    let mut client = Client::connect(dir.join("farcoolerd.sock"), "test", "0.0.0").await.expect("socket");
    let repo = dir.join("repos/demo");
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
    let call = |method: &str, target: Option<bytes::Bytes>, payload: request::Payload| {
        let mut req = named(method);
        req.target_resource_id = target;
        req.payload = Some(payload);
        req
    };
    let add = call("repository_root.add", None, request::Payload::RepositoryRootAdd(pb::RepositoryRootAdd {
        absolute_path: dir.join("repos").to_string_lossy().into_owned(),
        typed_confirmation: String::new(),
    }));
    client.call(add).await.expect("add root");
    let register = call("repository.register", None, request::Payload::RepositoryRegister(pb::RepositoryRegister { relative_path: repo.to_string_lossy().into_owned() }));
    let Some(result::Value::Repository(repository)) = client.call(register).await.expect("register").value else { panic!("register") };
    let create = call(
        "worktree.create",
        Some(repository.id.clone()),
        request::Payload::WorktreeCreate(pb::WorktreeCreate { task_name: "rows".into(), branch: "feat/rows".into(), base_revision: "HEAD".into(), ..Default::default() }),
    );
    let Some(result::Value::Worktree(worktree)) = client.call(create).await.expect("worktree.create").value else { panic!("worktree") };
    let pane = call(
        "terminal.create",
        Some(worktree.id.clone()),
        request::Payload::TerminalCreate(pb::TerminalCreate { title: "claude".into(), command_preset: "claude".into(), ..Default::default() }),
    );
    let Some(result::Value::Terminal(terminal)) = client.call(pane).await.expect("terminal.create").value else { panic!("terminal") };
    let path = std::fs::canonicalize(worktree.worktree_path.as_deref().unwrap()).unwrap();
    let project = dir.join("claude/projects").join(farcooler_core::session_log::claude_slug(&path.to_string_lossy()));
    std::fs::create_dir_all(&project).unwrap();
    let id = uuid::Uuid::from_slice(&terminal.id).unwrap().to_string();
    (id, project.join(format!("{}.jsonl", terminal.agent_session_id.unwrap())))
}

fn append(path: &Path, line: Value) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    writeln!(file, "{line}").unwrap();
}

fn prompt(id: &str, text: &str) -> Value {
    json!({"type":"user","promptId":id,"promptSource":"typed","uuid":format!("u-{id}"),"timestamp":"2026-10-06T10:00:00Z","message":{"content":text}})
}

fn ids(page: &Value) -> Vec<String> {
    page["rows"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn an_app_pages_follows_and_pages_again_after_a_restart() {
    std::fs::create_dir_all("/tmp/fc-phone").unwrap();
    let dir = tempfile::Builder::new().prefix("r").tempdir_in("/tmp/fc-phone").unwrap();
    let _reaper = Reaper(dir.path().to_path_buf());
    let first = daemon(dir.path()).await;
    let (terminal, path) = a_claude_pane(dir.path()).await;
    append(&path, prompt("p1", "Tidy the parser."));
    append(&path, json!({"type":"assistant","uuid":"a1","timestamp":"2026-10-06T10:00:01Z","message":{"content":[{"type":"text","text":"Tidied."}],"stop_reason":"end_turn"}}));

    let session = Session::connect_local(&dir.path().join("farcoolerd.sock")).await.expect("connect");
    let page = dispatch(&session, "agent.rows", &json!({ "terminal": terminal, "limit": 1 })).await.expect("agent.rows");
    assert_eq!(ids(&page), ["prose:a1:0"], "{page}");
    assert_eq!(page["moreBefore"], true);
    assert_eq!(page["rows"][0]["kind"]["Prose"]["text"], "Tidied.", "each row an object the app decodes: {page}");
    let older = dispatch(&session, "agent.rows", &json!({ "terminal": terminal, "before": page["rows"][0]["ord"] })).await.unwrap();
    assert_eq!(ids(&older), ["turn:p1"]);
    assert_eq!(older["moreBefore"], false);

    let writer = {
        let path = path.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            append(&path, prompt("p2", "And the lexer."));
        })
    };
    let ask = json!({ "terminal": terminal, "epoch": page["epoch"], "afterRev": page["rev"], "waitMs": 10_000 });
    let changes = dispatch(&session, "agent.rows_follow", &ask).await.expect("agent.rows_follow");
    writer.await.unwrap();
    assert_eq!(changes["reset"], false, "{changes}");
    let insert = changes["changes"].as_array().unwrap().iter().find(|c| c["id"] == "turn:p2").unwrap_or_else(|| panic!("{changes}"));
    assert_eq!(insert["kind"], "insert");
    assert_eq!(insert["row"]["kind"]["Turn"]["prompt"], "And the lexer.");
    let before = dispatch(&session, "agent.rows", &json!({ "terminal": terminal })).await.unwrap();
    drop(session);
    drop(first);

    let _second = daemon(dir.path()).await;
    let session = Session::connect_local(&dir.path().join("farcoolerd.sock")).await.expect("reconnect");
    let stale = json!({ "terminal": terminal, "epoch": before["epoch"], "afterRev": before["rev"], "waitMs": 0 });
    let stale = dispatch(&session, "agent.rows_follow", &stale).await.unwrap();
    assert_eq!(stale["reset"], true, "a follower from before the restart pages again: {stale}");
    let after = dispatch(&session, "agent.rows", &json!({ "terminal": terminal })).await.unwrap();
    assert_eq!(ids(&after), ids(&before), "the same rows by id, rebuilt from the file");
    assert_ne!(after["epoch"], before["epoch"]);
    assert!(matches!(dispatch(&session, "agent.rows", &json!({})).await, Err(crate::session::SessionError::Protocol(_))), "no terminal, no call");
}
