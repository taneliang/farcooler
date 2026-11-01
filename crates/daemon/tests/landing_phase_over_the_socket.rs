//! A landing's phase and phase_since (ov-503), and the collecting landing
//! waiting behind the one ahead (ov-520), as a client meets them: over a real
//! socket in `plan.get` and in the answer to a landing write.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_protocol::capability::{BOARD_LINE, BOARD_PLAN};
use farcooler_protocol::v1::{self as pb, Scope, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_store::ticket_lines::{LineUpdate, NewTicketLine, Stage};
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

fn move_to(h: &Harness, task: Uuid, stage: Stage) {
    h.service.store.update_line(task, &LineUpdate { stage: Some(stage), ..Default::default() }, Actor::Manager).unwrap();
}

/// A ticket that has passed review: it is in the landing stage.
fn ready_ticket(h: &Harness, workspace: Uuid, title: &str) -> Uuid {
    let task = h.service.store.create_task(workspace, title, Actor::User).unwrap().id;
    h.service.store.put_on_line(task, &NewTicketLine::default(), Actor::Manager).unwrap();
    for stage in [Stage::Building, Stage::Reviewing, Stage::Landing] {
        move_to(h, task, stage);
    }
    task
}

/// Main's CI is red, so the collecting landing collects and doesn't leave on
/// its own: a landing can be started by hand from it.
fn hold(h: &Harness, workspace: Uuid) {
    let read = farcooler_store::board_ci::CiRead {
        subject: farcooler_store::board_ci::MAIN_SUBJECT.into(),
        sha: "0000000".into(),
        status: farcooler_store::board_ci::CiStatus::Failed,
        url: String::new(),
        jobs: vec![],
        fetched_at: 0,
        changed_at: 0,
        asked_at: 0,
    };
    h.service.store.record_ci(workspace, &read).unwrap();
}

async fn start_landing(link: &mut Link, workspace: Uuid, tickets: &[Uuid]) -> pb::Landing {
    let p = payload::Payload::LandingStart(pb::LandingStart {
        workspace_id: id(workspace),
        route: pb::LandingRoute::Push as i32,
        base: "origin/main".into(),
        task_ids: tickets.iter().map(|t| id(*t)).collect(),
        actor: "manager".into(),
        ..Default::default()
    });
    match call(link, "landing.start", BOARD_LINE, p).await.expect("landing.start") {
        result::Value::Landing(l) => l,
        other => panic!("wrong result: {other:?}"),
    }
}

async fn plan(link: &mut Link, workspace: Uuid) -> pb::Plan {
    let p = payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(workspace), include_closed: true });
    match call(link, "plan.get", BOARD_PLAN, p).await.expect("plan.get") {
        result::Value::Plan(p) => p,
        other => panic!("wrong result: {other:?}"),
    }
}

/// The collecting landing says it is waiting for the earlier landing over the
/// socket, and a ticket sent back for a fix comes off the landing that was
/// checking it: the landing is never held for it.
#[tokio::test]
async fn the_collecting_landing_waits_for_the_previous_one_and_a_ticket_sent_back_comes_off() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    hold(&h, repo.workspace);
    let ready = ready_ticket(&h, repo.workspace, "Ready one");
    let mut a = connect(&h).await;

    // The ticket joined the collecting landing when it passed review, and it
    // waits on main; starting one by hand adopts it.
    let read = plan(&mut a, repo.workspace).await;
    let waiting = read.landings.iter().find(|l| l.collecting).expect("the collecting landing");
    assert_eq!(waiting.phase, pb::LandingPhase::WaitingForMain as i32);
    let started = start_landing(&mut a, repo.workspace, &[ready]).await;
    assert_eq!(started.id, waiting.id, "the collecting landing, adopted");
    assert_eq!(started.task_ids, vec![id(ready)]);
    assert!(!started.collecting, "started by hand, it stops collecting");
    assert_eq!(started.phase, pb::LandingPhase::Integrating as i32, "a write answers with the phase too");
    let gating = pb::LandingSet {
        landing_id: started.id.clone(),
        state: pb::LandingState::Gating as i32,
        actor: "manager".into(),
        ..Default::default()
    };
    let checking = match call(&mut a, "landing.set", BOARD_LINE, payload::Payload::LandingSet(gating)).await.expect("landing.set") {
        result::Value::Landing(l) => l,
        other => panic!("wrong result: {other:?}"),
    };
    assert_eq!(checking.phase, pb::LandingPhase::GatingLocally as i32);

    // The next ticket that passes review joins the landing collecting behind it.
    let next = ready_ticket(&h, repo.workspace, "Next one");
    let read = plan(&mut a, repo.workspace).await;
    let behind = read.landings.iter().find(|l| l.collecting).expect("a collecting landing");
    assert_ne!(behind.id, checking.id);
    assert_eq!(behind.state, pb::LandingState::Open as i32);
    assert_eq!(behind.phase, pb::LandingPhase::WaitingForPreviousLanding as i32);
    assert_eq!(pb::LandingPhase::WaitingForPreviousLanding as i32, 10, "the number the wire reserved");
    assert_eq!(behind.task_ids, vec![id(next)]);

    // A ticket sent back for a fix comes off the landing that was checking it.
    move_to(&h, ready, Stage::Building);
    let read = plan(&mut a, repo.workspace).await;
    let landing = read.landings.iter().find(|l| l.id == checking.id).unwrap();
    assert!(landing.task_ids.is_empty(), "the ticket is off the landing: {landing:?}");
    assert!(read.now_ms - landing.phase_since < 60 * 60_000);
}
