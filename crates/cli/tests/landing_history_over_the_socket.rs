//! The CLI reads engine groups and old landing history through the real daemon dispatch.
#[path = "../../daemon/tests/support/in_process.rs"]
mod in_process;

use farcooler_protocol::v1::Scope;
use farcooler_store::{models::Actor, testing::{exec_for_tests, old_landing, old_landing_lander}};
use farcooler_workflow::{Archived, ArchivedGroup, Phase, State};
use in_process::*;
use serde_json::Value;

async fn run_landing(h: &Harness, repo: &Repo, args: &[&str]) -> std::process::Output {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .env("FARCOOLER_HOME", h.socket.parent().unwrap())
        .env_remove("FARCOOLER_WORKSPACE")
        .env_remove("FARCOOLER_ACTOR")
        .args(["landing", "--repo", &repo.id.to_string(), "--workspace", &repo.workspace.to_string()])
        .args(args)
        .output().await.unwrap()
}

async fn landing(h: &Harness, repo: &Repo, args: &[&str]) -> Value {
    let output = run_landing(h, repo, &[&["--json"], args].concat()).await;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn list_and_show_distinguish_engine_groups() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let mut state = State::empty(repo.workspace.to_string(), 1);
    state.revision = 2;
    let mut keys = Vec::new();
    for (n, sha) in [(1, "abcdef1"), (2, "abcdef2")] {
        let task = store.create_task(repo.workspace, &format!("Engine landing {n}"), Actor::Manager).unwrap();
        state.archive.insert(format!("group-{n:020}-landing"), Archived {
            phase: Phase::Done,
            candidate: Some(sha.into()),
            group: Some(ArchivedGroup {
                name: "landing".into(),
                tickets: vec![task.key.clone()],
                pushed: Some(sha.into()),
                since_ms: now_millis(),
            }),
            start_load: None,
            last_run: None,
            history: Vec::new(),
            closed: None,
        });
        exec_for_tests(store, &format!(
            "INSERT INTO workflow_owned_items(workspace, item, task_id) VALUES ('{}', '{}', X'{}')",
            repo.workspace, task.key, task.id.simple(),
        ));
        keys.push(task.key);
    }
    let json = serde_json::to_string(&state).unwrap().replace('\'', "''");
    exec_for_tests(store, &format!(
        "INSERT INTO workflow_states(workspace, revision, epoch, state) VALUES ('{}', {}, 1, '{json}')",
        repo.workspace, state.revision,
    ));
    std::os::unix::fs::symlink(&h.socket, h.socket.with_file_name("farcoolerd.sock")).unwrap();

    let list = landing(&h, &repo, &["list"]).await;
    let [first, second] = list["landings"].as_array().unwrap().as_slice() else { panic!("{list}") };
    assert_ne!(first["id"], second["id"]);
    assert_ne!(first["tickets"], second["tickets"]);
    let text = run_landing(&h, &repo, &["list"]).await;
    assert!(text.status.success(), "{}", String::from_utf8_lossy(&text.stderr));
    let text = String::from_utf8(text.stdout).unwrap();
    for listed in [first, second] {
        assert_eq!(listed["number"], 0);
        let key = listed["tickets"][0]["key"].as_str().unwrap();
        assert!(keys.iter().any(|expected| expected == key));
        let id = listed["id"].as_str().unwrap();
        let mut shown = landing(&h, &repo, &["show", id]).await;
        assert_eq!(shown.as_object_mut().unwrap().remove("events"), Some(serde_json::json!([])));
        assert_eq!(&shown, listed);
        assert!(text.lines().any(|line| line.contains(id) && line.contains(key)), "{text}");
    }
    let ambiguous = run_landing(&h, &repo, &["show", "0"]).await;
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("More than one engine landing"));
}

#[tokio::test]
async fn list_and_show_read_stored_history() {
    let h = start_legacy(Scope::Read).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Old landing", Actor::Manager).unwrap();
    let landing_id = old_landing(&h.service.store, repo.workspace, 373, "pushed", Some("abcdef1"), &[task.id], now_millis());
    let agent = h.service.store.record_agent(repo.workspace, &farcooler_store::agents::NewAgent {
        role: farcooler_store::agents::AgentRole::Lander,
        place: farcooler_store::agents::AgentPlace::Subagent { id: "history-lander".into(), parent: None },
        harness: "claude".into(), session_id: None, session_cwd: None, model: None, title: None,
    }).unwrap();
    old_landing_lander(&h.service.store, landing_id, agent.id);
    h.service.store.record_turn(&farcooler_store::usage::NewTurn {
        key: "claude-log:agent:history-lander".into(), terminal_id: None, worktree_id: None, repository_id: None,
        workspace_id: None, task_id: None, harness: "claude".into(), surface: farcooler_store::usage::Surface::Terminal,
        started_at: None, ended_at: now_millis(), active_ms: None, usage: "reported",
        models: vec![farcooler_store::usage::TurnModel::priced(Some("model".into()),
            farcooler_core::usage::TokenCounts { input: 1000, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, fast: false }, Some(2000))],
        kind: farcooler_store::usage::TurnKind::Subagent,
    }).unwrap();
    let home = h.socket.parent().unwrap();
    std::os::unix::fs::symlink(&h.socket, home.join("farcoolerd.sock")).unwrap();
    for args in [&["list"][..], &["show", "373"][..]] {
        let value = landing(&h, &repo, args).await;
        let landing = if args[0] == "list" { &value["landings"][0] } else { &value };
        assert_eq!(landing["number"], 373);
        assert_eq!(landing["pushed_sha"], "abcdef1");
        assert_eq!(landing["phase"], "landed");
        assert_eq!(landing["summary"], "Old landing");
        assert_eq!(landing["tickets"][0]["key"], task.key);
        assert_eq!(landing["spend"]["input_tokens"], 1000);
        assert_eq!(landing["spend"]["runs"], 1);
        if args[0] == "show" { assert!(landing["events"].is_array()); }
    }
}
