//! Exercise the CLI and the released app request against a real, frame-limited RPC socket.
#[path = "../../daemon/tests/support/in_process.rs"]
mod in_process;

use farcooler_protocol::capability::BOARD_PLAN;
use farcooler_protocol::v1::{self as pb, Scope, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_store::testing::old_landing;
use in_process::*;
use uuid::Uuid;

const ARCHIVED_RUNS: usize = 1450;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

#[tokio::test]
async fn scoped_theme_commands_accept_a_full_uuid() {
    use farcooler_store::plan::NewTheme;

    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let card = store.create_task(repo.workspace, "A card to add", Actor::Manager).unwrap();
    let theme = store.create_theme(repo.workspace, &NewTheme {
        name: "A theme".into(), outcome: "Keep the board readable".into(),
    }, &[], Actor::Manager).unwrap();
    let uuid = theme.id.to_string();
    let commands = [
        vec!["show", &uuid],
        vec!["set", &uuid, "--story", "Updated by UUID"],
        vec!["cards", &uuid, "--add", &card.key],
    ];
    let mut failures = Vec::new();
    for args in commands {
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
            .env("FARCOOLER_HOME", h.socket.parent().unwrap())
            .env_remove("FARCOOLER_WORKSPACE")
            .env_remove("FARCOOLER_ACTOR")
            .args(["plan", "--json", "--repo", &repo.id.to_string()])
            .args(["--workspace", &repo.workspace.to_string(), "theme"])
            .args(&args)
            .output().await.unwrap();
        if !output.status.success() {
            failures.push(format!("{args:?}: {}", String::from_utf8_lossy(&output.stderr)));
            continue;
        }
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["id"], uuid);
        match args[0] {
            "set" => assert_eq!(response["story"], "Updated by UUID"),
            "cards" => assert_eq!(response["cards"][0]["key"], card.key),
            _ => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn a_large_board_loads_in_plan_overview_and_detail_commands() {
    use farcooler_store::models::TaskStatus;
    use farcooler_store::plan::{NewTheme, ThemeUpdate};
    use farcooler_store::ticket_lines::{LineUpdate, NewTicketLine, Stage};

    let h = start_legacy(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let now = farcooler_store::testing::now_millis();
    let mut tasks = Vec::new();
    for n in 0..1500 {
        let title = format!("Ticket {n}. {}", "Keep the current board usable while the completed work accumulates. ".repeat(4));
        let task = store.create_task(repo.workspace, title.trim_end(), Actor::Manager).unwrap();
        store.put_on_line(task.id, &NewTicketLine {
            reason: "Keep the evidence for the completed ticket available when reading its details. ".repeat(3),
            branch: format!("ticket-{n}/{}", "implementation-and-verification-".repeat(6)),
            ..Default::default()
        }, Actor::Manager).unwrap();
        if n >= 50 {
            store.update_line(task.id, &LineUpdate {
                stage: Some(Stage::Dropped), ..Default::default()
            }, Actor::Manager).unwrap();
            store.set_task_status(task.id, TaskStatus::Cancelled, Actor::Manager).unwrap();
        }
        tasks.push(task.id);
    }
    for n in 0..25 {
        let cards = if n == 0 { &tasks[..300] } else { &tasks[250 + n * 50..300 + n * 50] };
        let theme = store.create_theme(repo.workspace, &NewTheme {
            name: format!("Theme {n}"), outcome: "Keep the board readable".into(),
        }, cards, Actor::Manager).unwrap();
        store.update_theme(theme.id, &ThemeUpdate {
            story: Some("Now on the board".into()), next: Some("Next on the board".into()),
            ..Default::default()
        }, Actor::Manager).unwrap();
    }
    for (n, cards) in tasks.chunks(15).take(99).enumerate() {
        old_landing(store, repo.workspace, n as u32 + 1, "landed", None, cards, now + n as i64);
    }
    old_landing(store, repo.workspace, 100, "landed", None, &tasks[1200..], now + 99);
    // Enough more that the history outgrows one page.
    old_landing(store, repo.workspace, 101, "landed", None, &tasks[..300], now + 100);

    seed_large_workflow_archive(store, repo.workspace);
    h.service.start_workflows().unwrap();
    let mut link = connect(&h).await;
    let request = payload::Payload::PlanGet(pb::PlanGetRequest {
        workspace_id: id(repo.workspace), include_closed: false, ..Default::default()
    });
    let mut get = farcooler_transport::request("plan.get");
    assert!(link.server_hello().capabilities.iter().any(|cap| cap == BOARD_PLAN));
    get.payload = Some(request);
    let result::Value::Plan(read) = link.call(get).await.expect("large board plan.get").value.unwrap() else {
        panic!("wrong result");
    };
    assert_eq!(read.themes.len(), 25);
    let first = read.themes[0].theme.as_ref().unwrap();
    assert_eq!((&*first.story, &*first.next), ("Now on the board", "Next on the board"));
    assert_eq!(read.themes[0].counts.unwrap().cancelled, 250);
    assert_eq!(read.board_counts.unwrap().cancelled, 1450);
    assert!(read.lines.iter().any(|line| line.task_id == id(tasks[0])));
    assert!(read.landings.is_empty(), "stored landings aren't part of the Plan");
    assert!(read.cards.len() < 1500, "closed cards must be summarized");

    // The released app reads the unpaged board, then sends workflow.get with no view marker.
    let get_workflow = farcooler_client::plan_json::workflow_request(repo.workspace);
    let mut get_full = get_workflow.clone();
    get_full.accepts.push(farcooler_protocol::accepts::WORKFLOW_FULL.into());
    let legacy = farcooler_client::plan_json::with_workflow(&read, repo.workspace, Some(async {
        let reply = link.call(get_workflow).await?.value.unwrap();
        let result::Value::WorkflowReply(workflow) = &reply else {
            panic!("wrong legacy workflow reply");
        };
        let wire: serde_json::Value = serde_json::from_str(&workflow.json).unwrap();
        let kept = wire["state"]["archive"].as_object().unwrap().len();
        assert!(kept < ARCHIVED_RUNS);
        assert_eq!(kept + wire["archive_omitted"].as_u64().unwrap() as usize, ARCHIVED_RUNS);
        Ok(reply)
    })).await.expect("released app Plan load");
    assert_eq!(legacy["themes"][0]["counts"]["cancelled"], 250);
    assert_eq!(legacy["themes"][0]["story"], "Now on the board");
    assert_eq!(legacy["themes"][0]["next"], "Next on the board");
    assert_eq!(legacy["landings"], serde_json::json!([]));
    assert_eq!(legacy["workflow"]["runs"][0]["item"], "item");
    let result::Value::WorkflowReply(full) = link.call(get_full).await.unwrap().value.unwrap() else {
        panic!("wrong Engine workflow reply");
    };
    let full: serde_json::Value = serde_json::from_str(&full.json).unwrap();
    let engine = farcooler_client::engine_view::from_workflow_reply(
        repo.workspace, &full, Some(&read), farcooler_client::workflow_presentation::Style::Apple,
    ).expect("Engine loads with a history preview");
    assert_eq!(engine.workspace, repo.workspace.to_string());

    let cli = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .env("FARCOOLER_HOME", h.socket.parent().unwrap())
        .env_remove("FARCOOLER_WORKSPACE")
        .env_remove("FARCOOLER_ACTOR")
        .args(["plan", "engine", "--repo", &repo.id.to_string(),
            "--workspace", &repo.workspace.to_string(), "--json"])
        .output().await.unwrap();
    assert!(cli.status.success(), "{}", String::from_utf8_lossy(&cli.stderr));
    let cli: serde_json::Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(cli["workspace"], repo.workspace.to_string());
    assert_eq!(cli["revision"], full["state"]["revision"]);

    let session = farcooler_client::session::Session::connect_local(&h.socket).await.unwrap();
    let json = session.plan_with_workflow(repo.workspace).await.unwrap();
    assert_eq!(json["themes"][0]["counts"]["cancelled"], 250);
    assert!(json["workflow"].is_object());
    for style in ["apple", "material"] {
        let request = serde_json::json!({"schema": 1, "plan": json, "context": {"style": style}});
        let rendered = farcooler_client::plan_view::json(&request.to_string());
        let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert!(rendered["error"].is_null(), "{rendered}");
        assert_eq!(rendered["view"]["themes"]["tiles"].as_array().unwrap().len(), 25);
    }
    let waiting = store.get_task(tasks[0]).unwrap().key;
    let closed = store.get_task(tasks[300]).unwrap().key;
    let commands = [
        vec!["plan"],
        vec!["plan", "theme", "show", "Theme 0"],
        vec!["landing", "show", "1"],
        vec!["plan", "ticket", "show", &closed],
        vec!["plan", "ticket", "set", &waiting, "--reason", "Updated on the large board"],
        vec!["plan", "set", &waiting],
        vec!["landing", "show", "100"],
        vec!["landing", "list"],
    ];
    let mut failures = Vec::new();
    for (i, args) in commands.iter().enumerate() {
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
            .env("FARCOOLER_HOME", h.socket.parent().unwrap())
            .env_remove("FARCOOLER_WORKSPACE")
            .env_remove("FARCOOLER_ACTOR")
            .arg(args[0])
            .args(["--json", "--repo", &repo.id.to_string(), "--workspace", &repo.workspace.to_string()])
            .args(&args[1..])
            .output().await.unwrap();
        if i == 0 {
            h.service.stop_workflows().await.unwrap();
        }
        if !output.status.success() {
            failures.push(format!("{args:?}: {}", String::from_utf8_lossy(&output.stderr)));
            continue;
        }
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        match i {
            0 => {
                assert_eq!(response["themes"].as_array().unwrap().len(), 25);
                assert_eq!(response["workflow"]["runs"][0]["item"], "item");
            }
            1 => {
                let cards = response["cards"].as_array().unwrap();
                assert_eq!(cards.len(), 300);
                let last_card = store.get_task(tasks[299]).unwrap().key;
                assert_eq!(cards[299]["key"], last_card);
            }
            2 => assert_eq!(response["number"], 1),
            3 => assert_eq!(response["key"], closed),
            4 => assert_eq!(response["reason"], "Updated on the large board"),
            5 => assert_eq!(store.ticket_line(tasks[0]).unwrap().plan_rank, Some(1)),
            6 => {
                assert_eq!(response["number"], 100);
                assert!(response["summary"].as_str().unwrap().len() > 64 * 1024);
            }
            7 => {
                assert_eq!(response["landings"].as_array().unwrap().len(), 101);
            }
            _ => unreachable!(),
        }
    }
    // What `landing list` asks for.
    let mut get = farcooler_transport::request("plan.get");
    get.payload = Some(payload::Payload::PlanGet(pb::PlanGetRequest {
        workspace_id: id(repo.workspace), include_closed: true,
        detail: Some(pb::PlanDetailRead {
            subject: Some(pb::plan_detail_read::Subject::Landings(pb::PlanLandingQuery { number: None })),
            ..Default::default()
        }),
        ..Default::default()
    }));
    let result::Value::Plan(first) = connect(&h).await.call(get).await.unwrap().value.unwrap() else { panic!("wrong result") };
    assert!(first.next_offset.is_some(), "stored history is paged, so `landing list` above read several pages");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn a_large_live_board_keeps_its_lines_cards_and_active_landings() {
    use farcooler_store::plan::{NewTheme, ThemeUpdate};
    use farcooler_store::ticket_lines::NewTicketLine;

    let h = start_legacy(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let now = farcooler_store::testing::now_millis();
    let mut tasks = Vec::new();
    let mut keys = Vec::new();
    for n in 0..1500 {
        let title = format!("Ticket {n} {}", "長い見出し".repeat(30));
        let task = store.create_task(repo.workspace, title.trim_end(), Actor::Manager).unwrap();
        store.put_on_line(task.id, &NewTicketLine {
            reason: "r".repeat(280),
            branch: format!("ticket-{n}/{}", "b".repeat(200)),
            ..Default::default()
        }, Actor::Manager).unwrap();
        tasks.push(task.id);
        keys.push(task.key);
    }
    for (n, cards) in tasks.chunks(60).enumerate() {
        let theme = store.create_theme(repo.workspace, &NewTheme {
            name: format!("Theme {n}"), outcome: "Keep the board readable".into(),
        }, cards, Actor::Manager).unwrap();
        store.update_theme(theme.id, &ThemeUpdate {
            story: Some(live_story(n)), next: Some(format!("Next {n}")),
            ..Default::default()
        }, Actor::Manager).unwrap();
    }
    seed_active_landings(store, repo.workspace, &tasks, &keys, now);

    let session = farcooler_client::session::Session::connect_local(&h.socket).await.unwrap();
    let plan = session.plan(repo.workspace).await.unwrap();
    assert_eq!(plan.lines.len(), 1500);
    assert_eq!(plan.cards.len(), 1500);
    assert_eq!(plan.landings.len(), 100);
    assert!(plan.landings.iter().all(|landing| landing.phase() == pb::LandingPhase::ReadyToLand));
    assert_eq!(plan.themes.len(), 25);
    assert_eq!(plan.board_counts.unwrap().backlog, 1500);
    let card_ids: std::collections::HashSet<_> = plan.cards.iter().map(|card| &card.task_id).collect();
    assert_eq!(card_ids.len(), 1500);
    assert!(plan.lines.iter().all(|line| card_ids.contains(&line.task_id)));
    assert!(plan.landings.iter().flat_map(|landing| &landing.task_ids).all(|id| card_ids.contains(id)));
    for (n, theme) in plan.themes.iter().enumerate() {
        assert_eq!(theme.theme.as_ref().unwrap().story, live_story(n));
        assert_eq!(theme.theme.as_ref().unwrap().next, format!("Next {n}"));
        assert_eq!(theme.task_ids, tasks[n * 60..(n + 1) * 60].iter().copied().map(id).collect::<Vec<_>>());
    }

    let mut get = farcooler_transport::request("plan.get");
    get.payload = Some(payload::Payload::PlanGet(pb::PlanGetRequest {
        workspace_id: id(repo.workspace),
        ..Default::default()
    }));
    let result::Value::Plan(legacy) = connect(&h).await.call(get).await.unwrap().value.unwrap() else {
        panic!("wrong result for a released app's request");
    };
    assert!(legacy.overview_truncated);
    assert_eq!(legacy.themes.len(), 25);
    assert_eq!(legacy.board_counts.unwrap().backlog, 1500);
    for (n, view) in legacy.themes.iter().enumerate() {
        let theme = view.theme.as_ref().unwrap();
        assert_eq!(theme.story, live_story(n));
        assert_eq!(theme.next, format!("Next {n}"));
        assert_eq!(view.counts.unwrap().backlog, 60);
    }
    assert!(!legacy.lines.is_empty());
    let legacy_cards: std::collections::HashSet<_> = legacy.cards.iter().map(|card| &card.task_id).collect();
    assert!(plan.landings.iter().all(|landing| landing.task_ids.iter().any(|id| !legacy_cards.contains(id))));
    assert_eq!(legacy.landings.len(), 100);
    assert!(legacy.landings.iter().all(|landing| landing.phase() == pb::LandingPhase::ReadyToLand));
    assert!(legacy.lines.iter().all(|line| legacy_cards.contains(&line.task_id)));
    assert!(legacy.landings.iter().flat_map(|landing| &landing.task_ids).all(|id| legacy_cards.contains(id)));

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .env("FARCOOLER_HOME", h.socket.parent().unwrap())
        .env_remove("FARCOOLER_WORKSPACE")
        .env_remove("FARCOOLER_ACTOR")
        .args(["plan", "--json", "--repo", &repo.id.to_string(), "--workspace", &repo.workspace.to_string()])
        .output().await.unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let lines = json["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1500);
    assert!(lines.iter().all(|line| line["title"].as_str().is_some_and(|title| !title.is_empty())));
    assert_eq!(json["landings"].as_array().unwrap().len(), 100);
    assert_eq!(json["themes"][24]["next"], "Next 24");
}

fn live_story(n: usize) -> String {
    format!("Now {n}. {}", "計".repeat(3600))
}

fn seed_large_workflow_archive(store: &farcooler_store::Store, workspace: Uuid) {
    use farcooler_workflow::{Archived, Phase, RunRecord, State};
    use serde_json::{Value, json};

    let fixture: Value = serde_json::from_str(include_str!("../../client/src/testdata/workflow-presentation.json")).unwrap();
    let mut snapshot = fixture[0]["request"]["snapshot"].clone();
    snapshot["workspace"] = json!(workspace);
    let mut state: State = serde_json::from_value(snapshot).unwrap();
    let archived = Archived {
        start_load: None, closed: None, phase: Phase::Cancelled, candidate: None, group: None,
        last_run: Some(RunRecord {
            phase: Phase::Cancelled, run: "old-run".into(), stop_run: None, pin: "pin".into(),
            fence: 1, candidate: None, tree: None, models: Default::default(),
            reason: Some("build output ".repeat(100)), rework_history: Vec::new(),
            sent_back: Vec::new(), back_edges: Default::default(),
        }),
        history: Vec::new(),
    };
    state.archive = (0..ARCHIVED_RUNS).map(|n| (format!("archived-{n}"), archived.clone())).collect();
    let cap = farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES;
    assert!(serde_json::to_vec(&state.archive).unwrap().len() > cap);
    insert_workflow_state(store, &state);
}

fn insert_workflow_state(store: &farcooler_store::Store, state: &farcooler_workflow::State) {
    let (workspace, revision, epoch) = (&state.workspace, state.revision, state.epoch);
    let state = serde_json::to_string(state).unwrap().replace('\'', "''");
    farcooler_store::testing::exec_for_tests(store, &format!(
        "INSERT INTO workflow_states(workspace,revision,epoch,state) VALUES('{workspace}',{revision},{epoch},'{state}')"
    ));
}

fn seed_active_landings(store: &farcooler_store::Store, workspace: Uuid, tasks: &[Uuid], keys: &[String], now: i64) {
    use farcooler_workflow::{Item, State};
    use serde_json::{Value, json};

    let fixture: Value = serde_json::from_str(include_str!("../../client/src/testdata/workflow-presentation.json")).unwrap();
    let template = &fixture[0]["request"]["snapshot"]["items"]["item"];
    let mut state = State::empty(workspace.to_string(), 1);
    state.revision = 1;
    // Each group holds one of the last 100 cards in the Plan's ID order,
    // past the legacy card allowance.
    let mut sorted: Vec<_> = tasks.iter().zip(keys).collect();
    sorted.sort_unstable_by_key(|(task, _)| *task);
    let (early, late) = sorted.split_at(1400);
    for (n, (keys, last)) in early.chunks(14).zip(late).enumerate() {
        let mut group = template.clone();
        group["group_of"] = json!({"name": "landing", "pin": "fixture"});
        group["phase"] = json!("WaitingApproval");
        group["since_ms"] = json!(now + n as i64);
        group["members"] = json!(keys.iter().chain(std::iter::once(last)).map(|(_, key)| key)
            .map(|key| json!({"item": key, "candidate": "fixture"})).collect::<Vec<_>>());
        state.items.insert(format!("group-{n:020}-landing"), serde_json::from_value::<Item>(group).unwrap());
    }
    insert_workflow_state(store, &state);
    let values = tasks.iter().zip(keys).map(|(task, key)| format!("('{workspace}','{key}',X'{}')", task.simple()))
        .collect::<Vec<_>>().join(",");
    farcooler_store::testing::exec_for_tests(store, &format!(
        "INSERT INTO workflow_owned_items(workspace,item,task_id) VALUES {values}"
    ));
}
