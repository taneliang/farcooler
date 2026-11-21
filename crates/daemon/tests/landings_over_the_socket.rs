//! The old landings (ov-309, ov-872) as a client meets them since the engine
//! took over landing (ov-960): over a real socket, through the real dispatch
//! table and scope check. `plan.get` excludes old landings.

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
    let p = payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(workspace), include_closed: true, ..Default::default() });
    match call(link, "plan.get", BOARD_PLAN, p).await.expect("plan.get") {
        result::Value::Plan(p) => p,
        other => panic!("wrong result: {other:?}"),
    }
}

/// Landing 373, left pushed and waiting on CI when the engine took over,
/// remains stored but does not appear in `plan.get`.
#[tokio::test]
async fn a_landing_left_waiting_for_ci_stays_out_of_the_plan() {
    let h = start_legacy(Scope::Read).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Bring the CI chip back", Actor::Manager).unwrap();
    let now = farcooler_store::testing::now_millis();
    old_landing(&h.service.store, repo.workspace, 373, "pushed", Some("abcdef1"), &[task.id], now - 60_000);
    let mut link = connect(&h).await;
    let read = plan(&mut link, repo.workspace).await;
    assert!(read.landings.is_empty());
    let stored = h.service.store.landings(repo.workspace).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!((stored[0].number, stored[0].pushed_sha.as_deref()), (373, Some("abcdef1")));
}
