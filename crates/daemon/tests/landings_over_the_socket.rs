//! The old landings (ov-309, ov-872) as a client meets them since the engine
//! took over landing (ov-960): over a real socket, through the real dispatch
//! table and scope check. Both old writes are refused with `engine_lands`, and
//! `plan.get` reads each old landing as history.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_protocol::capability::{BOARD_LINE, BOARD_PLAN};
use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, request as payload, result};
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

/// Every shape of the two old writes, a record, a run, a move and a SHA, is
/// refused with `engine_lands`, and the landing stays as it was left.
#[tokio::test]
async fn the_old_landing_writes_say_the_engine_lands() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Bring the CI chip back", Actor::Manager).unwrap();
    let now = farcooler_store::testing::now_millis();
    let landing = old_landing(&h.service.store, repo.workspace, 373, "pushed", Some("abcdef1"), &[task.id], now);
    let mut a = connect(&h).await;
    let start = |run: bool, land: bool| {
        payload::Payload::LandingStart(pb::LandingStart {
            workspace_id: id(repo.workspace),
            task_ids: vec![id(task.id)],
            actor: "manager".into(),
            run,
            land,
            ..Default::default()
        })
    };
    let set = |p: pb::LandingSet| payload::Payload::LandingSet(pb::LandingSet { landing_id: id(landing), actor: "manager".into(), ..p });
    let writes = [
        ("landing.start", start(false, false)),
        ("landing.start", start(true, false)),
        ("landing.start", start(true, true)),
        ("landing.set", set(pb::LandingSet { sha: Some("abcdef2".into()), ..Default::default() })),
        ("landing.set", set(pb::LandingSet { state: pb::LandingState::Landed as i32, ..Default::default() })),
        ("landing.set", set(pb::LandingSet { remove_task_ids: vec![id(task.id)], ..Default::default() })),
    ];
    for (method, p) in writes {
        match call(&mut a, method, BOARD_LINE, p).await {
            Err(ClientError::Daemon { code, what, .. }) => {
                assert_eq!((code, what.as_str()), (ErrorCode::InvalidArgument as i32, "engine_lands"), "{method}");
            }
            other => panic!("{method}: expected engine_lands, got {other:?}"),
        }
    }
    let kept = h.service.store.landing(landing).unwrap();
    assert_eq!((kept.state.as_str(), kept.pushed_sha.as_deref(), kept.tickets.len()), ("pushed", Some("abcdef1"), 1));
}

/// Landing 373, left pushed and waiting on CI when the engine took over,
/// reads as landed history over `plan.get`, and a read client sees it and
/// still can't write.
#[tokio::test]
async fn a_landing_left_waiting_for_ci_reads_as_landed() {
    let h = start(Scope::Read).await;
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
    let p = payload::Payload::LandingStart(pb::LandingStart { workspace_id: id(repo.workspace), ..Default::default() });
    match call(&mut link, "landing.start", BOARD_LINE, p).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32),
        other => panic!("expected a scope denial, got {other:?}"),
    }
}
