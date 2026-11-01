//! Landings (ov-309, ov-872) as a client meets them: over a real socket,
//! through the real dispatch table and scope check, and read back in
//! `plan.get`.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::capability::{BOARD_LINE, BOARD_PLAN};
use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, event, request as payload, result};
use farcooler_store::landings::{NewLanding, Route};
use farcooler_store::models::Actor;
use farcooler_store::ticket_lines::{LineUpdate, Stage};
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

/// A ticket on the line moved through `stages`, as the store writes it.
fn moved(h: &Harness, task: Uuid, stages: &[Stage]) {
    let store = &h.service.store;
    store.ensure_on_line(task, Actor::Manager).unwrap();
    for stage in stages {
        store.update_line(task, &LineUpdate { stage: Some(*stage), ..Default::default() }, Actor::Manager).unwrap();
    }
}

/// A ticket through review and waiting to land: the only kind a landing takes.
fn ready(h: &Harness, task: Uuid) {
    moved(h, task, &[Stage::Building, Stage::Reviewing, Stage::Landing]);
}

fn start_landing(workspace: Uuid, tickets: &[Uuid]) -> pb::LandingStart {
    pb::LandingStart {
        workspace_id: id(workspace),
        route: pb::LandingRoute::Push as i32,
        base: "origin/main".into(),
        task_ids: tickets.iter().map(|t| id(*t)).collect(),
        actor: "manager".into(),
        ..Default::default()
    }
}

fn set(landing: &pb::Landing, state: pb::LandingState, sha: Option<&str>) -> payload::Payload {
    payload::Payload::LandingSet(pb::LandingSet {
        landing_id: landing.id.clone(),
        state: state as i32,
        sha: sha.map(str::to_string),
        actor: "manager".into(),
        ..Default::default()
    })
}

async fn landing(link: &mut Link, p: payload::Payload, method: &str) -> pb::Landing {
    match call(link, method, BOARD_LINE, p).await.expect(method) {
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

/// The `plan_changed` and `task_changed` events on `listener` within `window`.
async fn events(listener: &mut Link, window: Duration) -> (usize, usize) {
    let (mut plans, mut tasks) = (0, 0);
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        match e.payload {
            Some(event::Payload::PlanChanged(_)) => plans += 1,
            Some(event::Payload::TaskChanged(_)) => tasks += 1,
            _ => {}
        }
    }
    (plans, tasks)
}

fn to(task: Uuid, stage: pb::TicketStage) -> payload::Payload {
    payload::Payload::LineSet(pb::TicketLineSet {
        task_id: id(task),
        stage: Some(stage as i32),
        actor: "manager".into(),
        ..Default::default()
    })
}

/// A ticket that passes review over `line.set` joins the collecting landing
/// in the same write, and with nothing ahead of it the landing leaves for its
/// checks; the next one waits on the fresh collecting landing behind it, and
/// comes off it when it's sent back for a fix.
#[tokio::test]
async fn a_reviewed_ticket_joins_a_landing_over_line_set() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let first = h.service.store.create_task(repo.workspace, "Mac: first", Actor::Manager).unwrap();
    let second = h.service.store.create_task(repo.workspace, "Mac: second", Actor::Manager).unwrap();
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;

    let mut last = None;
    for stage in [pb::TicketStage::Building, pb::TicketStage::Reviewing, pb::TicketStage::Landing] {
        let answer = call(&mut a, "line.set", BOARD_LINE, to(first.id, stage)).await.expect("line.set");
        let result::Value::TicketLine(line) = answer else { panic!("wrong result") };
        assert_eq!(line.stage, stage as i32);
        last = Some(line);
    }
    assert_eq!(last.unwrap().landing, Some(1), "the answer says which landing it went on");

    let read = plan(&mut a, repo.workspace).await;
    assert_eq!(read.landings.len(), 1, "the empty collecting landing behind it isn't drawn: {:?}", read.landings);
    let gone = &read.landings[0];
    assert_eq!((gone.number, gone.state, gone.collecting), (1, pb::LandingState::Gating as i32, false), "nothing ahead: it left");
    assert_eq!(gone.task_ids, vec![id(first.id)]);
    assert_eq!((gone.title.as_str(), gone.summary.as_str()), ("Landing 1", "First"));

    for stage in [pb::TicketStage::Building, pb::TicketStage::Reviewing, pb::TicketStage::Landing] {
        call(&mut a, "line.set", BOARD_LINE, to(second.id, stage)).await.expect("line.set");
    }
    let read = plan(&mut a, repo.workspace).await;
    let behind = read.landings.iter().find(|l| l.number == 2).expect("the second waits on the next landing");
    assert_eq!((behind.state, behind.collecting), (pb::LandingState::Open as i32, true));
    assert_eq!(behind.task_ids, vec![id(second.id)]);
    assert_eq!(behind.phase, pb::LandingPhase::WaitingForPreviousLanding as i32);

    // A ticket sent back for a fix comes off a landing whose SHA isn't out.
    call(&mut a, "line.set", BOARD_LINE, to(second.id, pb::TicketStage::Building)).await.expect("line.set");
    let read = plan(&mut a, repo.workspace).await;
    let line = read.lines.iter().find(|l| l.task_id == id(second.id)).unwrap();
    assert_eq!(line.landing, None, "it came off");

    let (plans, tasks) = events(&mut listener, Duration::from_millis(400)).await;
    assert_eq!(plans, 7, "every write announces once");
    assert_eq!(tasks, 0, "no line write moves a card");
}

/// Started by hand, pushed, and read back in the plan with its ticket on it
/// and the ticket saying so; each write announces `plan_changed` once and
/// never `task_changed`.
#[tokio::test]
async fn a_landing_round_trips_through_the_plan_and_announces() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let first = h.service.store.create_task(repo.workspace, "Mac: first", Actor::Manager).unwrap();
    let second = h.service.store.create_task(repo.workspace, "Mac: second", Actor::Manager).unwrap();
    // The first leaves for its checks on Landing 1, and the second waits on
    // the collecting Landing 2 behind it.
    ready(&h, first.id);
    ready(&h, second.id);
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;

    let started =
        landing(&mut a, payload::Payload::LandingStart(start_landing(repo.workspace, &[second.id])), "landing.start").await;
    assert_eq!(started.number, 2, "it adopted the collecting landing, and its number");
    assert_eq!((started.state, started.collecting), (pb::LandingState::Open as i32, false));
    assert_eq!((started.route, started.base.as_str()), (pb::LandingRoute::Push as i32, "origin/main"));
    assert_eq!(started.task_ids, vec![id(second.id)]);
    assert_eq!(started.ci_subject, "");
    let pushed = landing(&mut a, set(&started, pb::LandingState::Unspecified, Some("1A1B3275")), "landing.set").await;
    assert_eq!((pushed.state, pushed.pushed_sha.as_deref()), (pb::LandingState::Pushed as i32, Some("1a1b3275")));
    assert_eq!(pushed.ci_subject, "sha:1a1b3275");

    let read = plan(&mut a, repo.workspace).await;
    assert_eq!(read.landings.len(), 2, "the two live ones; the empty collecting one isn't drawn: {:?}", read.landings);
    let mine = read.landings.iter().find(|l| l.number == 2).unwrap();
    assert_eq!(mine, &pushed, "the answer to a write is what a read says");
    let line = read.lines.iter().find(|l| l.task_id == id(second.id)).unwrap();
    assert_eq!(line.landing, Some(2), "the ticket says it's on the landing");
    assert!(read.ci.is_empty(), "nothing read yet");

    let (plans, tasks) = events(&mut listener, Duration::from_millis(400)).await;
    assert_eq!((plans, tasks), (2, 0));
}

/// Refusals arrive as the words the CLI maps to sentences.
#[tokio::test]
async fn refusals_name_what_was_wrong() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    let refused = |r: Result<result::Value, ClientError>| match r {
        Err(ClientError::Daemon { code, what, .. }) => {
            assert_eq!(code, ErrorCode::InvalidArgument as i32);
            what
        }
        other => panic!("expected a refusal, got {other:?}"),
    };
    let start = payload::Payload::LandingStart;
    let waiting = h.service.store.create_task(repo.workspace, "Later", Actor::Manager).unwrap();
    h.service.store.ensure_on_line(waiting.id, Actor::Manager).unwrap();
    let p = start(start_landing(repo.workspace, &[waiting.id]));
    assert_eq!(refused(call(&mut a, "landing.start", BOARD_LINE, p).await), "not_ready");
    let in_review = h.service.store.create_task(repo.workspace, "Nearly", Actor::Manager).unwrap();
    moved(&h, in_review.id, &[Stage::Building, Stage::Reviewing]);
    let p = start(start_landing(repo.workspace, &[in_review.id]));
    assert_eq!(refused(call(&mut a, "landing.start", BOARD_LINE, p).await), "ticket_in_review");
    let mut bad_route = start_landing(repo.workspace, &[]);
    bad_route.route = 99;
    assert_eq!(refused(call(&mut a, "landing.start", BOARD_LINE, start(bad_route)).await), "route");
    let mut long = start_landing(repo.workspace, &[]);
    long.title = "x".repeat(81);
    assert_eq!(refused(call(&mut a, "landing.start", BOARD_LINE, start(long)).await), "title");

    let l = landing(&mut a, start(start_landing(repo.workspace, &[])), "landing.start").await;
    assert_eq!(refused(call(&mut a, "landing.set", BOARD_LINE, set(&l, pb::LandingState::Green, None)).await), "sha");
    let bad_sha = set(&l, pb::LandingState::Unspecified, Some("nope"));
    assert_eq!(refused(call(&mut a, "landing.set", BOARD_LINE, bad_sha).await), "sha");
    landing(&mut a, set(&l, pb::LandingState::Dropped, None), "landing.set").await;
    let moved_on = call(&mut a, "landing.set", BOARD_LINE, set(&l, pb::LandingState::Gating, None)).await;
    assert_eq!(refused(moved_on), "landing_settled");
    let bad = pb::LandingSet { landing_id: l.id.clone(), state: 99, actor: "manager".into(), ..Default::default() };
    assert_eq!(refused(call(&mut a, "landing.set", BOARD_LINE, payload::Payload::LandingSet(bad)).await), "state");
    let mut as_runner = start_landing(repo.workspace, &[]);
    as_runner.actor = "runner".into();
    assert!(
        matches!(call(&mut a, "landing.start", BOARD_LINE, start(as_runner)).await, Err(ClientError::Daemon { .. })),
        "nobody speaks as the runner"
    );
}

/// A read client sees landings in the plan and can't write one.
#[tokio::test]
async fn a_read_client_reads_landings_and_cannot_write_them() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let new = NewLanding { route: Route::Push, base: String::new(), title: String::new(), lander: None, collecting: false };
    h.service.store.start_landing(repo.workspace, &new, &[], Actor::Manager).unwrap();
    let mut link = connect(&h).await;
    assert_eq!(plan(&mut link, repo.workspace).await.landings.len(), 1);
    let p = payload::Payload::LandingStart(start_landing(repo.workspace, &[]));
    match call(&mut link, "landing.start", BOARD_LINE, p).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32),
        other => panic!("expected a scope denial, got {other:?}"),
    }
}

/// A landing's title and lander cross the socket (ov-461, ov-872): the answer
/// to a write is what a read says, "Landing 1" is read off the number and the
/// summary off its tickets' titles without their area, and an agent that
/// doesn't land is refused as its lander.
#[tokio::test]
async fn a_landing_carries_its_lander_and_title() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let card = h.service.store.create_task(repo.workspace, "Plan: tickets read as sentences", Actor::Manager).unwrap();
    let mut a = connect(&h).await;
    let record = |role: pb::AgentRole, subagent: &str| {
        payload::Payload::AgentRecord(pb::AgentRecord {
            workspace_id: id(repo.workspace),
            role: role as i32,
            harness: "claude".into(),
            subagent_id: subagent.into(),
            model: Some("opus".into()),
            actor: "manager".into(),
            ..Default::default()
        })
    };
    let agent = |v: result::Value| match v {
        result::Value::Agent(a) => a,
        other => panic!("wrong result: {other:?}"),
    };
    let lander = agent(call(&mut a, "agent.record", BOARD_LINE, record(pb::AgentRole::Lander, "l1")).await.unwrap());
    let builder = agent(call(&mut a, "agent.record", BOARD_LINE, record(pb::AgentRole::Builder, "b1")).await.unwrap());
    ready(&h, card.id);
    // The ticket left on Landing 1 by itself; this one is started by hand.
    let mut by_hand = start_landing(repo.workspace, &[]);
    by_hand.lander_id = Some(builder.id.clone());
    match call(&mut a, "landing.start", BOARD_LINE, payload::Payload::LandingStart(by_hand.clone())).await {
        Err(ClientError::Daemon { what, .. }) => assert_eq!(what, "lander", "a builder doesn't land"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    by_hand.lander_id = Some(lander.id.clone());
    by_hand.title = "Phones catch up".into();
    let started = landing(&mut a, payload::Payload::LandingStart(by_hand), "landing.start").await;
    assert_eq!(started.title, "Phones catch up");
    assert_eq!(started.lander_id, Some(lander.id.clone()));
    assert_eq!(started.spend.unwrap().unmeasured_agents, 1, "no turn read: not reported");

    let read = plan(&mut a, repo.workspace).await;
    let left = read.landings.iter().find(|l| l.number == 1).expect("the landing that left by itself");
    assert_eq!((left.title.as_str(), left.summary.as_str()), ("Landing 1", "Tickets read as sentences"));
    let mine = read.landings.iter().find(|l| l.id == started.id).unwrap();
    assert_eq!(mine, &started);
}

/// `landing.start` with `run` (ov-872): the charter's `## Landing` section
/// decides, so a board with none, or one that doesn't read, is refused; only
/// a person says land; and a run that starts is handed to the landing job,
/// whose steps reach the landing's timeline (here, a fetch that can't work:
/// the repository is only in the store).
#[tokio::test]
async fn a_run_reads_the_charter_and_hands_the_landing_to_the_job() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    let refused = |r: Result<result::Value, ClientError>| match r {
        Err(ClientError::Daemon { what, .. }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    };
    let task = h.service.store.create_task(repo.workspace, "Ready", Actor::Manager).unwrap();
    ready(&h, task.id);
    let run = |land: bool, actor: &str| {
        payload::Payload::LandingStart(pb::LandingStart { run: true, land, actor: actor.into(), ..start_landing(repo.workspace, &[task.id]) })
    };
    assert_eq!(refused(call(&mut a, "landing.start", BOARD_LINE, run(false, "manager")).await), "no_landing_rules");
    let charter = farcooler_daemon::workspace_home::charter_path(h.service.root_dir(), repo.workspace);
    std::fs::create_dir_all(charter.parent().unwrap()).unwrap();
    std::fs::write(&charter, "## Landing\n\nPush it.\n").unwrap();
    assert_eq!(refused(call(&mut a, "landing.start", BOARD_LINE, run(false, "manager")).await), "charter_landing");
    std::fs::write(&charter, "## Landing\n- Route: push\n- Gate: true\n- CI: none\n").unwrap();
    assert_eq!(refused(call(&mut a, "landing.start", BOARD_LINE, run(true, "manager")).await), "land_is_yours");

    let l = landing(&mut a, run(false, "manager"), "landing.start").await;
    assert_eq!(l.task_ids, vec![id(task.id)]);
    let landing_id = Uuid::from_slice(&l.id).unwrap();
    let subject = farcooler_store::ticket_lines::LineSubject::Landing(landing_id);
    let mut stopped = None;
    for _ in 0..100 {
        let events = h.service.store.line_events(subject, 0).unwrap();
        stopped = events.into_iter().find(|e| e.body.starts_with("Stopped."));
        if stopped.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let stopped = stopped.expect("the job ran and stopped");
    assert!(stopped.body.contains("Couldn't fetch origin"), "{}", stopped.body);
}
