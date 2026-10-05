//! A theme's `last_moved_at` (ov-331) as a client meets it: filled by the
//! runner from the newest of its story, its lanes, its rulings and its cards,
//! read back from `plan.get` through the real dispatch table.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, Scope, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_transport::{ClientError, request};
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

async fn call(link: &mut Link, method: &str, p: payload::Payload) -> Result<result::Value, ClientError> {
    let mut r = request(method);
    r.required_capabilities = vec![farcooler_protocol::capability::BOARD_PLAN.into()];
    r.payload = Some(p);
    Ok(link.call(r).await?.value.expect("a value"))
}

async fn plan(link: &mut Link, workspace: Uuid) -> pb::Plan {
    let p = payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(workspace), include_closed: false });
    match call(link, "plan.get", p).await.expect("plan.get") {
        result::Value::Plan(p) => p,
        other => panic!("wrong result: {other:?}"),
    }
}

async fn theme(link: &mut Link, workspace: Uuid, name: &str, task: Uuid) {
    call(
        link,
        "board_theme.create",
        payload::Payload::BoardThemeCreate(pb::BoardThemeCreate {
            workspace_id: id(workspace),
            name: name.into(),
            outcome: String::new(),
            task_ids: vec![id(task)],
            actor: "manager".into(),
        }),
    )
    .await
    .expect("board_theme.create");
}

/// A theme with a lane on its card moved when the lane did; one with only a
/// card moved when the card did; and a lane on another theme's card moves
/// neither.
#[tokio::test]
async fn a_theme_says_when_it_last_moved_by_its_lanes_and_its_cards() {
    let h = start(Scope::HostAdmin).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    let lonely = h.service.store.create_task(repo.workspace, "Mac: alone", Actor::User).unwrap();
    theme(&mut a, repo.workspace, "Alone", lonely.id).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let busy = h.service.store.create_task(repo.workspace, "Mac: busy", Actor::User).unwrap();
    theme(&mut a, repo.workspace, "Busy", busy.id).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    call(
        &mut a,
        "lane.create",
        payload::Payload::LaneCreate(pb::LaneCreate {
            workspace_id: id(repo.workspace),
            name: "busy-lane".into(),
            cards: vec![pb::LaneCard { task_id: id(busy.id), slice: String::new(), stage: None }],
            actor: "manager".into(),
            ..Default::default()
        }),
    )
    .await
    .expect("lane.create");

    let read = plan(&mut connect(&h).await, repo.workspace).await;
    let by_name = |name: &str| read.themes.iter().find(|v| v.theme.as_ref().unwrap().name == name).unwrap();
    let alone = by_name("Alone").last_moved_at.expect("the runner says");
    let busy_view = by_name("Busy");
    let busy_at = busy_view.last_moved_at.expect("the runner says");
    let lane = read.lanes.iter().find(|l| l.name == "busy-lane").unwrap();
    assert_eq!(busy_at, lane.state_since, "the lane moved after the card, so it's the lane's time");
    assert!(alone >= lonely.status_since, "a theme with no lane moved when its card did: {alone}");
    assert!(alone < busy_at, "the lane on Busy's card moved Busy and not Alone: {alone} against {busy_at}");
}
