//! The old landings (ov-309, ov-872) as a client meets them since the engine
//! took over landing (ov-960): over a real socket, through the real dispatch
//! table and scope check. `plan.get` reads each old landing as history.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_protocol::capability::BOARD_PLAN;
use farcooler_protocol::v1::{self as pb, Scope, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_store::testing::old_landing;
use farcooler_transport::{ClientError, request};
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

async fn call(link: &mut Link, method: &str, capability: &str, p: payload::Payload) -> Result<result::Value, ClientError> {
    let mut r = request(method);
    r.required_capabilities = vec![capability.into()];
    r.payload = Some(p);
    Ok(link.call(r).await?.value.expect("a value"))
}

async fn plan(link: &mut Link, workspace: Uuid) -> pb::Plan {
    let p = payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(workspace), include_closed: true });
    match call(link, "plan.get", BOARD_PLAN, p).await.expect("plan.get") {
        result::Value::Plan(p) => p,
        other => panic!("wrong result: {other:?}"),
    }
}

/// Landing 373, left pushed and waiting on CI when the engine took over,
/// reads as landed history over `plan.get`, and a read client sees it and
/// still can't write.
#[tokio::test]
async fn a_landing_left_waiting_for_ci_reads_as_landed() {
    let h = start_legacy(Scope::Read).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Bring the CI chip back", Actor::Manager).unwrap();
    let now = farcooler_store::testing::now_millis();
    old_landing(&h.service.store, repo.workspace, 373, "pushed", Some("abcdef1"), &[task.id], now - 60_000);
    let mut link = connect(&h).await;
    let read = plan(&mut link, repo.workspace).await;
    let [l] = read.landings.as_slice() else { panic!("{:?}", read.landings) };
    assert_eq!((l.number, l.phase, l.phase_since), (373, pb::LandingPhase::Landed as i32, now - 60_000));
    assert_eq!((l.state, l.pushed_sha.as_deref()), (pb::LandingState::Pushed as i32, Some("abcdef1")), "stored as it was left");
    assert_eq!(l.summary, "Bring the CI chip back");
}

#[tokio::test]
async fn a_large_board_loads_through_the_released_plan_request() {
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
        workspace_id: id(repo.workspace), include_closed: false,
    });
    let result::Value::Plan(read) = call(&mut link, "plan.get", BOARD_PLAN, request).await.expect("large board plan.get") else {
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

}
