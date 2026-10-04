//! The plan layer (ov-268) as a client meets it: over a real socket, through
//! the real dispatch table and scope check.
//!
//! Each test is the one that goes red when the call it is about is removed:
//! the announce, the scope, the worker the lane's agent becomes, the drill.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, event, request as payload, result};
use farcooler_store::models::{Actor, NoteKind, TaskStatus};
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

async fn make_lane(link: &mut Link, workspace: Uuid, name: &str, cards: &[Uuid], agent: Option<&str>) -> pb::Lane {
    let p = payload::Payload::LaneCreate(pb::LaneCreate {
        workspace_id: id(workspace),
        name: name.into(),
        reason: "Because.".into(),
        cards: cards.iter().map(|c| pb::LaneCard { task_id: id(*c), slice: String::new() }).collect(),
        worktree_path: "/repos/ny/.claude/worktrees/x".into(),
        branch: "x".into(),
        harness: "claude".into(),
        model: "sonnet".into(),
        agent: agent.map(|a| pb::LaneAgentRecord {
            harness: "claude".into(),
            agent_id: a.into(),
            role: pb::LaneAgentRole::Build as i32,
            model: None,
            ended: false,
        }),
        actor: "manager".into(),
        ..Default::default()
    });
    match call(link, "lane.create", p).await.expect("lane.create") {
        result::Value::Lane(l) => l,
        other => panic!("wrong result: {other:?}"),
    }
}

/// The `plan_changed` and `task_changed` events on `listener` within `window`.
async fn events(listener: &mut Link, window: Duration) -> (Vec<pb::PlanChanged>, usize) {
    let (mut plans, mut tasks) = (vec![], 0);
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        match e.payload {
            Some(event::Payload::PlanChanged(p)) => plans.push(p),
            Some(event::Payload::TaskChanged(_)) => tasks += 1,
            _ => {}
        }
    }
    (plans, tasks)
}

/// A theme, a lane and the plan, written and read back; a write announces
/// `plan_changed` and never `task_changed`.
#[tokio::test]
async fn a_theme_a_lane_and_the_plan_round_trip_and_announce() {
    let h = start(Scope::HostAdmin).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;

    let theme = call(
        &mut a,
        "board_theme.create",
        payload::Payload::BoardThemeCreate(pb::BoardThemeCreate {
            workspace_id: id(repo.workspace),
            name: "Visual language".into(),
            outcome: "One app.".into(),
            task_ids: vec![id(task.id)],
            actor: "manager".into(),
        }),
    )
    .await
    .expect("board_theme.create");
    let result::Value::BoardThemeView(view) = theme else { panic!("wrong result") };
    assert_eq!(view.counts.unwrap().backlog, 1);

    let queued = make_lane(&mut a, repo.workspace, "mac-fu3", &[task.id], None).await;
    assert_eq!(queued.state, pb::LaneState::Queued as i32);
    let set = payload::Payload::PlanSet(pb::PlanSet {
        workspace_id: id(repo.workspace),
        lane_ids: vec![queued.id.clone()],
        actor: "manager".into(),
    });
    let result::Value::Plan(p) = call(&mut a, "plan.set", set).await.expect("plan.set") else { panic!("wrong result") };
    assert_eq!(p.order, std::slice::from_ref(&queued.id));

    let read = plan(&mut connect(&h).await, repo.workspace).await;
    assert_eq!(read.themes.len(), 1);
    assert_eq!(read.lanes[0].plan_rank, Some(1));
    assert_eq!(read.lanes[0].worktree_path, "/repos/ny/.claude/worktrees/x", "host_admin sees the path");
    assert_eq!(read.cards[0].key, task.key);
    assert_eq!((read.coverage[0].live, read.coverage[0].landed), (1, 0));

    let (plans, tasks) = events(&mut listener, Duration::from_millis(600)).await;
    assert_eq!(plans.len(), 3, "one per write");
    assert!(plans.iter().all(|p| p.workspace_id == id(repo.workspace) && p.actor == "manager"));
    assert_eq!(tasks, 0, "no layer write moves a task");
}

/// A lane's agent is recorded as a worker on its first open card, so the
/// runner reads its spend, and a closed card is skipped.
#[tokio::test]
async fn a_lane_s_agent_becomes_a_worker_on_its_first_open_card() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let closed = store.create_task(repo.workspace, "Mac: done already", Actor::User).unwrap();
    store.set_task_status(closed.id, TaskStatus::Done, Actor::Manager).unwrap();
    let open = store.create_task(repo.workspace, "Mac: open", Actor::User).unwrap();
    let mut a = connect(&h).await;

    let lane = make_lane(&mut a, repo.workspace, "mac-ux", &[closed.id, open.id], Some("agent-1")).await;
    assert_eq!(lane.state, pb::LaneState::Building as i32);
    assert!(store.workers_for(closed.id).unwrap().is_empty());
    let workers = store.workers_for(open.id).unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0].agent_id, "agent-1");
    assert_eq!(store.get_task(open.id).unwrap().status, TaskStatus::InProgress, "ov-213: a worker means it started");
    assert_eq!(lane.worktree_path, "", "a control client is not shown the path");

    // A reviewer arrives with the move to review, in one write.
    let p = payload::Payload::LaneUpdate(pb::LaneUpdate {
        lane_id: lane.id.clone(),
        state: Some(pb::LaneState::Review as i32),
        agent: Some(pb::LaneAgentRecord {
            harness: "claude".into(),
            agent_id: "agent-2".into(),
            role: pb::LaneAgentRole::Review as i32,
            model: Some("opus".into()),
            ended: false,
        }),
        actor: "manager".into(),
        ..Default::default()
    });
    let result::Value::Lane(moved) = call(&mut a, "lane.update", p).await.expect("lane.update") else { panic!() };
    assert_eq!(moved.state, pb::LaneState::Review as i32);
    assert_eq!(moved.agents.len(), 2);
    assert_eq!(store.workers_for(open.id).unwrap().len(), 2);
}

/// Refusals arrive as the words the CLI maps to sentences.
#[tokio::test]
async fn refusals_name_what_was_wrong() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    let lane = make_lane(&mut a, repo.workspace, "mac-ux", &[], None).await;
    let p = payload::Payload::LaneUpdate(pb::LaneUpdate {
        lane_id: lane.id.clone(),
        state: Some(pb::LaneState::Landed as i32),
        actor: "manager".into(),
        ..Default::default()
    });
    match call(&mut a, "lane.update", p).await {
        Err(ClientError::Daemon { code, what, .. }) => {
            assert_eq!(code, ErrorCode::InvalidArgument as i32);
            assert_eq!(what, "lane_state");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    let p = payload::Payload::LaneUpdate(pb::LaneUpdate {
        lane_id: lane.id,
        reason: Some("x".into()),
        actor: "runner".into(),
        ..Default::default()
    });
    assert!(matches!(call(&mut a, "lane.update", p).await, Err(ClientError::Daemon { .. })), "nobody speaks as the runner");
}

/// Reading the plan is a read, and the path stays home; writing it is control.
#[tokio::test]
async fn a_read_client_reads_the_plan_without_the_path_and_cannot_write_it() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    h.service
        .store
        .create_lane(
            repo.workspace,
            &farcooler_store::plan::NewLane { name: "l".into(), worktree_path: "/secret".into(), ..Default::default() },
            &[],
            None,
            Actor::Manager,
        )
        .unwrap();
    let mut link = connect(&h).await;
    let read = plan(&mut link, repo.workspace).await;
    assert_eq!(read.lanes[0].worktree_path, "");

    let p = payload::Payload::PlanSet(pb::PlanSet { workspace_id: id(repo.workspace), lane_ids: vec![], actor: String::new() });
    match call(&mut link, "plan.set", p).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32),
        other => panic!("expected a scope denial, got {other:?}"),
    }
}

/// Everything a board client reads, as bytes.
async fn board_bytes(link: &mut Link, repo: &Repo) -> Vec<String> {
    let mut out = vec![];
    let mut list = request("task.list");
    list.payload = Some(payload::Payload::TaskList(pb::TaskListRequest {
        repository_id: id(repo.id),
        workspace_id: Some(id(repo.workspace)),
        ..Default::default()
    }));
    let got = link.call(list).await.expect("task.list").value.expect("a value");
    let result::Value::TaskList(mut tasks) = got else { panic!() };
    // The read marks carry the runner's first-look clock, which is written once
    // and is not the board's.
    tasks.reads = None;
    out.push(format!("{tasks:#?}"));
    for t in &tasks.items {
        let mut get = request("task.get");
        get.payload = Some(payload::Payload::TaskGet(pb::TaskGetRequest { task_id: t.id.clone(), ..Default::default() }));
        let value = link.call(get).await.expect("task.get").value.expect("a value");
        let result::Value::TaskDetail(detail) = value else { panic!() };
        out.push(format!("{detail:#?}"));
    }
    let value = link.call(request("needs_you.list")).await.expect("needs_you.list").value.expect("a value");
    let result::Value::NeedsYouList(needs) = value else { panic!() };
    out.push(format!("{needs:#?}"));
    let mut rep = request("report.get");
    rep.payload = Some(payload::Payload::ReportRequest(pb::ReportRequest {
        since: 0,
        until: now_millis() + 3_600_000,
        ..Default::default()
    }));
    let value = link.call(rep).await.expect("report.get").value.expect("a value");
    let result::Value::Report(report) = value else { panic!() };
    // What the report counts, not the clock it was cut at: its durations and
    // `generated_at` move with the runner's time between the two reads.
    let json: serde_json::Value = serde_json::from_str(&report.report_json).expect("the runner's JSON");
    let waits: Vec<_> = json["notable"]["longest_waits"].as_array().unwrap().iter().map(|w| w["key"].clone()).collect();
    let t = &json["totals"];
    out.push(format!(
        "{} {} {} {} {} {waits:?}",
        t["created"], t["completed"], t["canceled"], t["decisions"]["asked"], t["acceptance"]
    ));
    out
}

/// The removal drill (ov-268, section 8), over the wire: with the layer's
/// tables dropped, `task.list`, `task.get`, `needs_you.list` and `report.get`
/// answer the same bytes, and the board's writes still work.
#[tokio::test]
async fn the_boards_wire_reads_are_the_same_bytes_without_the_layer() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let a = store.create_task(repo.workspace, "Mac: first", Actor::User).unwrap();
    let b = store.create_task(repo.workspace, "Mac: second", Actor::User).unwrap();
    store.set_task_status(a.id, TaskStatus::InProgress, Actor::Manager).unwrap();
    store.add_note(a.id, NoteKind::Question, Actor::Manager, "Which one?", serde_json::json!({})).unwrap();
    store.set_task_status(a.id, TaskStatus::NeedsDecision, Actor::Manager).unwrap();
    store.add_note(b.id, NoteKind::Progress, Actor::Manager, "Dispatched in the mac-ux lane.", serde_json::json!({})).unwrap();

    let mut link = connect(&h).await;
    make_lane(&mut link, repo.workspace, "mac-ux", &[a.id, b.id], Some("agent-1")).await;
    let call_theme = payload::Payload::BoardThemeCreate(pb::BoardThemeCreate {
        workspace_id: id(repo.workspace),
        name: "T".into(),
        outcome: "o".into(),
        task_ids: vec![id(a.id), id(b.id)],
        actor: "manager".into(),
    });
    call(&mut link, "board_theme.create", call_theme).await.unwrap();

    let before = board_bytes(&mut link, &repo).await;
    farcooler_store::testing::drop_plan_layer(store);
    let after = board_bytes(&mut link, &repo).await;
    assert_eq!(before.len(), after.len());
    for (i, (x, y)) in before.iter().zip(&after).enumerate() {
        assert_eq!(x, y, "board read {i} changed when the layer went");
    }

    let mut create = request("task.create");
    create.target_resource_id = Some(id(repo.id));
    create.payload = Some(payload::Payload::TaskCreate(pb::TaskCreate {
        repository_id: id(repo.id),
        workspace_id: Some(id(repo.workspace)),
        title: "CLI: after".into(),
        ..Default::default()
    }));
    link.call(create).await.expect("task.create still works without the layer");
}
