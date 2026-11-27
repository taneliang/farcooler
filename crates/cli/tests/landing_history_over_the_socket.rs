//! The CLI reads engine groups and old landing history through the real daemon dispatch.
#[path = "../../daemon/tests/support/in_process.rs"]
mod in_process;

use farcooler_protocol::v1::Scope;
use farcooler_store::{models::Actor, testing::{exec_for_tests, old_landing, old_landing_lander}};
use farcooler_workflow::{Archived, ArchivedGroup, Definition, Phase, Progression, StageKind, State};
use in_process::*;
use serde_json::{Value, json};

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
    let h = start_legacy(Scope::Read).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let historical = store.create_task(repo.workspace, "Historical landing", Actor::Manager).unwrap();
    let historical_id = old_landing(store, repo.workspace, 373, "landed", Some("abcdef0"), &[historical.id], now_millis());
    let mut state = State::empty(repo.workspace.to_string(), 1);
    state.revision = 3;
    let archived = store.create_task(repo.workspace, "Archived landing", Actor::Manager).unwrap();
    let live = store.create_task(repo.workspace, "Live landing", Actor::Manager).unwrap();
    let landed = |key: &str, sha: &str| Archived {
        phase: Phase::Done,
        candidate: Some(sha.into()),
        group: Some(ArchivedGroup {
            name: "landing".into(),
            tickets: vec![key.into()],
            pushed: Some(sha.into()),
            since_ms: now_millis(),
        }),
        start_load: None,
        last_run: None,
        history: Vec::new(),
        closed: None,
    };
    state.archive.insert("group-00000000000000000001-landing".into(), landed(&archived.key, "abcdef1"));
    let mut definition = Definition::tracker("orchestrator");
    definition.schema = 2;
    definition.source = "group/landing".into();
    definition.stages[0].kind = StageKind::AwaitBase;
    definition.stages[0].progression = Progression::Automatic;
    definition.validate_group().unwrap();
    let live_group = "group-00000000000000000002-landing";
    state.items.insert(live_group.into(), serde_json::from_value(json!({
        "pin": definition.pin(),
        "definition": definition,
        "policy_pin": "fixture",
        "dependencies": [],
        "resource": "ref:origin:main",
        "stage": 0,
        "iteration": 0,
        "attempt": 0,
        "fence": 1,
        "phase": Phase::Blocked,
        "run": "fixture",
        "group_of": { "name": "landing", "pin": "fixture" },
        "members": [{ "item": live.key, "candidate": "abcdef2" }],
        "since_ms": now_millis(),
    })).unwrap());
    for task in [&archived, &live] {
        exec_for_tests(store, &format!(
            "INSERT INTO workflow_owned_items(workspace, item, task_id) VALUES ('{}', '{}', X'{}')",
            repo.workspace, task.key, task.id.simple(),
        ));
    }
    let json = serde_json::to_string(&state).unwrap().replace('\'', "''");
    exec_for_tests(store, &format!(
        "INSERT INTO workflow_states(workspace, revision, epoch, state) VALUES ('{}', {}, 1, '{json}')",
        repo.workspace, state.revision,
    ));
    std::os::unix::fs::symlink(&h.socket, h.socket.with_file_name("farcoolerd.sock")).unwrap();

    let plan = store.plan(repo.workspace, i64::MIN).unwrap();
    assert_eq!(plan.landings.len(), 2);
    assert!(plan.landings.iter().all(|view| view.landing.number == 0));

    let list = landing(&h, &repo, &["list"]).await;
    let [first, second, history] = list["landings"].as_array().unwrap().as_slice() else { panic!("{list}") };
    assert_eq!(first["title"], "Landing");
    assert_eq!(second["title"], first["title"]);
    assert_ne!(first["id"], second["id"]);
    assert_eq!(history["number"], 373);
    assert_eq!(history["id"], historical_id.to_string());
    let mut shown_history = landing(&h, &repo, &["show", "373"]).await;
    assert!(shown_history.as_object_mut().unwrap().remove("events").unwrap().is_array());
    assert_eq!(&shown_history, history);
    let listed_for = |key: &str| [first, second].into_iter().find(|l| l["tickets"][0]["key"] == key).unwrap();
    let text = run_landing(&h, &repo, &["list"]).await;
    assert!(text.status.success(), "{}", String::from_utf8_lossy(&text.stderr));
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(!text.contains("Landing 0"), "{text}");
    for (key, pushed_sha) in [(&archived.key, json!("abcdef1")), (&live.key, Value::Null)] {
        let listed = listed_for(key);
        assert_eq!(listed["number"], 0);
        assert_eq!(listed["pushed_sha"], pushed_sha);
        let id = listed["id"].as_str().unwrap();
        let mut shown = landing(&h, &repo, &["show", id]).await;
        assert_eq!(shown.as_object_mut().unwrap().remove("events"), Some(json!([])));
        assert_eq!(&shown, listed);
        let listed_line = text.lines().find(|line| line.contains(id) && line.contains(key)).unwrap();
        let output = run_landing(&h, &repo, &["show", id]).await;
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let details = String::from_utf8(output.stdout).unwrap();
        assert_eq!(details.lines().next(), Some(listed_line));
    }
    let missing = run_landing(&h, &repo, &["show", "00000000-0000-0000-0000-000000000000"]).await;
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("No engine landing here has ID"));
    let ambiguous = run_landing(&h, &repo, &["show", "0"]).await;
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("More than one engine landing"));

    state.items.remove(live_group).unwrap();
    state.archive.insert(live_group.into(), landed(&live.key, "abcdef2"));
    state.revision += 1;
    let json = serde_json::to_string(&state).unwrap().replace('\'', "''");
    exec_for_tests(store, &format!(
        "UPDATE workflow_states SET revision = {}, state = '{json}' WHERE workspace = '{}'",
        state.revision, repo.workspace,
    ));
    let archived_list = landing(&h, &repo, &["list"]).await;
    let archived_groups = archived_list["landings"].as_array().unwrap();
    assert_eq!(archived_groups.len(), 3);
    for (key, pushed_sha) in [(&archived.key, "abcdef1"), (&live.key, "abcdef2")] {
        let id = listed_for(key)["id"].as_str().unwrap();
        let listed = archived_groups.iter().find(|l| l["id"] == id).unwrap();
        let mut shown = landing(&h, &repo, &["show", id]).await;
        assert_eq!(shown.as_object_mut().unwrap().remove("events"), Some(json!([])));
        assert_eq!(&shown, listed);
        assert_eq!(shown["tickets"][0]["key"], *key);
        assert_eq!(shown["pushed_sha"], pushed_sha);
        assert_eq!(shown["phase"], "landed");
    }
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
