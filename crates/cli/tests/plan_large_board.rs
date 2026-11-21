//! Exercise the CLI and the released app request against a real, frame-limited RPC socket.
#[path = "../../daemon/tests/support/in_process.rs"]
mod in_process;

use farcooler_protocol::capability::BOARD_PLAN;
use farcooler_protocol::v1::{self as pb, Scope, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_store::testing::old_landing;
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
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
        let title = format!("Ticket {n}. {}", "Keep the current board usable while the completed work accumulates. ".repeat(3));
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
    for (n, cards) in tasks.chunks(15).enumerate() {
        old_landing(store, repo.workspace, n as u32 + 1, "landed", None, cards, now + n as i64);
    }

    let mut link = connect(&h).await;
    let request = payload::Payload::PlanGet(pb::PlanGetRequest {
        workspace_id: id(repo.workspace), include_closed: false, ..Default::default()
    });
    let mut get = farcooler_transport::request("plan.get");
    get.required_capabilities = vec![BOARD_PLAN.into()];
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
    assert_eq!(read.landings[0].number, 100);
    assert!(read.landings.len() < 100, "landing history must be bounded");
    assert!(read.cards.len() < 1500, "closed cards must be summarized");

    let session = farcooler_client::session::Session::connect_local(&h.socket).await.unwrap();
    let json = session.plan_with_workflow(repo.workspace).await.unwrap();
    assert_eq!(json["themes"][0]["counts"]["cancelled"], 250);
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
        if !output.status.success() {
            failures.push(format!("{args:?}: {}", String::from_utf8_lossy(&output.stderr)));
            continue;
        }
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        match i {
            0 => assert_eq!(response["themes"].as_array().unwrap().len(), 25),
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
            _ => unreachable!(),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
