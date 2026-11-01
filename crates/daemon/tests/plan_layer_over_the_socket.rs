//! The plan layer (ov-268, ov-872) as a client meets it: themes, the line,
//! the plan's order and agents, over a real socket, through the real dispatch
//! table and scope check.
//!
//! Each test is the one that goes red when the call it is about is removed:
//! the announce, the scope, the worker an agent becomes, the drill.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, event, request as payload, result};
use farcooler_store::models::{Actor, NoteKind, TaskStatus};
use farcooler_store::ticket_lines::NewTicketLine;
use farcooler_transport::{ClientError, request};
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

async fn call(link: &mut Link, method: &str, p: payload::Payload) -> Result<result::Value, ClientError> {
    let mut r = request(method);
    r.required_capabilities =
        vec![farcooler_protocol::capability::BOARD_PLAN.into(), farcooler_protocol::capability::BOARD_LINE.into()];
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

/// `line.set` for `task`, as `manager`, with what `fill` writes.
fn line_set(task: Uuid, fill: impl FnOnce(&mut pb::TicketLineSet)) -> payload::Payload {
    let mut p = pb::TicketLineSet { task_id: id(task), actor: "manager".into(), ..Default::default() };
    fill(&mut p);
    payload::Payload::LineSet(p)
}

async fn line(link: &mut Link, p: payload::Payload) -> pb::TicketLine {
    match call(link, "line.set", p).await.expect("line.set") {
        result::Value::TicketLine(l) => l,
        other => panic!("wrong result: {other:?}"),
    }
}

fn order(workspace: Uuid, tasks: &[Uuid]) -> payload::Payload {
    payload::Payload::LineOrder(pb::LineOrder {
        workspace_id: id(workspace),
        task_ids: tasks.iter().map(|t| id(*t)).collect(),
        actor: "manager".into(),
    })
}

/// A Claude subagent recorded at work on `tasks`.
async fn record(link: &mut Link, workspace: Uuid, role: pb::AgentRole, subagent: &str, tasks: &[Uuid]) -> pb::Agent {
    let p = payload::Payload::AgentRecord(pb::AgentRecord {
        workspace_id: id(workspace),
        role: role as i32,
        harness: "claude".into(),
        subagent_id: subagent.into(),
        model: Some("sonnet".into()),
        task_ids: tasks.iter().map(|t| id(*t)).collect(),
        actor: "manager".into(),
        ..Default::default()
    });
    match call(link, "agent.record", p).await.expect("agent.record") {
        result::Value::Agent(a) => a,
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

fn refusal(r: Result<result::Value, ClientError>) -> (i32, String) {
    match r {
        Err(ClientError::Daemon { code, what, .. }) => (code, what),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A theme, a ticket on the line and the plan, written and read back; a write
/// announces `plan_changed` and never `task_changed`.
#[tokio::test]
async fn a_theme_a_ticket_and_the_plan_round_trip_and_announce() {
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

    let waiting = line(
        &mut a,
        line_set(task.id, |p| {
            p.reason = Some("Because.".into());
            p.branch = Some("ov-1/jump".into());
            p.worktree_path = Some("/repos/ny/.claude/worktrees/x".into());
        }),
    )
    .await;
    assert_eq!(waiting.stage, pb::TicketStage::Waiting as i32, "a ticket goes on the line waiting");
    assert_eq!((waiting.reason.as_str(), waiting.branch.as_str()), ("Because.", "ov-1/jump"));
    let result::Value::Plan(p) = call(&mut a, "line.order", order(repo.workspace, &[task.id])).await.expect("line.order") else {
        panic!("wrong result")
    };
    assert_eq!(p.lines[0].plan_rank, Some(1), "the answer is the new plan");

    let read = plan(&mut connect(&h).await, repo.workspace).await;
    assert_eq!(read.themes.len(), 1);
    assert_eq!(read.lines.len(), 1);
    assert_eq!(read.lines[0].task_id, id(task.id));
    assert_eq!(read.lines[0].plan_rank, Some(1));
    assert_eq!(read.lines[0].worktree_path, "/repos/ny/.claude/worktrees/x", "host_admin sees the path");
    assert_eq!(read.cards[0].key, task.key);

    // Its timeline says what happened, over `plan.events`.
    let p = payload::Payload::PlanEvents(pb::PlanEventsRequest {
        subject: Some(pb::plan_events_request::Subject::TaskId(id(task.id))),
        since_ms: 0,
    });
    let result::Value::PlanEventList(list) = call(&mut a, "plan.events", p).await.expect("plan.events") else { panic!() };
    let kinds: Vec<_> = list.events.iter().map(|e| e.kind.as_str()).collect();
    assert!(kinds.contains(&"stage") && kinds.contains(&"plan"), "{kinds:?}");

    let (plans, tasks) = events(&mut listener, Duration::from_millis(600)).await;
    assert_eq!(plans.len(), 3, "one per write");
    assert!(plans.iter().all(|p| p.workspace_id == id(repo.workspace) && p.actor == "manager"));
    assert_eq!(tasks, 0, "no layer write moves a task");
}

/// An agent recorded over the wire works its tickets, which go on the line;
/// a subagent is a worker on each, so the runner reads its spend; `agent.work`
/// starts and ends its work on one, and `agent.end` ends it and the rest.
#[tokio::test]
async fn an_agent_is_recorded_works_its_tickets_and_ends() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let first = store.create_task(repo.workspace, "Mac: first", Actor::User).unwrap();
    let second = store.create_task(repo.workspace, "Mac: second", Actor::User).unwrap();
    let mut a = connect(&h).await;

    let builder = record(&mut a, repo.workspace, pb::AgentRole::Builder, "agent-1", &[first.id]).await;
    assert_eq!((builder.role, builder.harness.as_str(), builder.model.as_str()), (pb::AgentRole::Builder as i32, "claude", "sonnet"));
    assert_eq!(builder.subagent_id, "agent-1");
    assert_eq!(builder.tickets.len(), 1);
    assert_eq!((builder.tickets[0].task_id.clone(), builder.tickets[0].role), (id(first.id), pb::WorkRole::Build as i32));
    assert_eq!(store.ticket_line(first.id).unwrap().stage, farcooler_store::ticket_lines::Stage::Waiting, "on the line");
    let workers = store.workers_for(first.id).unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0].agent_id, "agent-1");
    assert!(store.workers_for(second.id).unwrap().is_empty());

    // Recording the same subagent again fills it in rather than adding one.
    let again = record(&mut a, repo.workspace, pb::AgentRole::Builder, "agent-1", &[]).await;
    assert_eq!(again.id, builder.id);

    let work = |task: Uuid, role: pb::WorkRole, verdict: Option<&str>| {
        payload::Payload::AgentWork(pb::AgentWork {
            agent_id: builder.id.clone(),
            task_id: id(task),
            role: role as i32,
            verdict: verdict.map(str::to_string),
            actor: "manager".into(),
        })
    };
    let result::Value::Agent(two) = call(&mut a, "agent.work", work(second.id, pb::WorkRole::Build, None)).await.unwrap() else {
        panic!()
    };
    assert_eq!(two.tickets.len(), 2, "it works both now: {:?}", two.tickets);
    assert!(store.ticket_line(second.id).is_ok(), "work puts the ticket on the line");
    let (code, what) = refusal(call(&mut a, "agent.work", work(second.id, pb::WorkRole::Review, None)).await);
    assert_eq!((code, what.as_str()), (ErrorCode::InvalidArgument as i32, "role"), "a builder doesn't review");
    let result::Value::Agent(one) =
        call(&mut a, "agent.work", work(first.id, pb::WorkRole::Build, Some("finished"))).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(one.tickets.len(), 1);
    assert_eq!(one.tickets[0].task_id, id(second.id), "the finished one is no longer worked");
    let (_, what) = refusal(call(&mut a, "agent.work", work(first.id, pb::WorkRole::Build, Some("bored"))).await);
    assert_eq!(what, "verdict");

    let end = |verdict: &str| {
        payload::Payload::AgentEnd(pb::AgentEnd { agent_id: builder.id.clone(), verdict: verdict.into(), actor: "manager".into() })
    };
    let (_, what) = refusal(call(&mut a, "agent.end", end("bored")).await);
    assert_eq!(what, "verdict");
    let result::Value::Agent(_) = call(&mut a, "agent.end", end("finished")).await.unwrap() else { panic!() };
    let work_on_second = store.work_on(second.id).unwrap();
    assert!(work_on_second.iter().all(|w| w.ended_at.is_some()), "its open work ended with it: {work_on_second:?}");
    let (_, what) = refusal(call(&mut a, "agent.work", work(first.id, pb::WorkRole::Build, None)).await);
    assert_eq!(what, "agent_ended");
    let read = plan(&mut a, repo.workspace).await;
    assert!(read.agents.iter().all(|x| x.id != builder.id), "an ended agent leaves the plan");
}

/// Refusals arrive as the words the CLI maps to sentences.
#[tokio::test]
async fn refusals_name_what_was_wrong() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    let mut a = connect(&h).await;
    let to_landed = line_set(task.id, |p| p.stage = Some(pb::TicketStage::Landed as i32));
    let (code, what) = refusal(call(&mut a, "line.set", to_landed).await);
    assert_eq!((code, what.as_str()), (ErrorCode::InvalidArgument as i32, "stage"), "waiting doesn't jump to landed");
    let building = line_set(task.id, |p| p.stage = Some(pb::TicketStage::Building as i32));
    line(&mut a, building).await;
    let (_, what) = refusal(call(&mut a, "line.order", order(repo.workspace, &[task.id])).await);
    assert_eq!(what, "not_waiting", "only a waiting ticket has a place in the plan");
    let (_, what) = refusal(call(&mut a, "line.set", line_set(task.id, |p| p.stage = Some(99))).await);
    assert_eq!(what, "stage");
    let as_runner = line_set(task.id, |p| {
        p.reason = Some("x".into());
        p.actor = "runner".into();
    });
    assert!(matches!(call(&mut a, "line.set", as_runner).await, Err(ClientError::Daemon { .. })), "nobody speaks as the runner");
    let neither = payload::Payload::AgentRecord(pb::AgentRecord {
        workspace_id: id(repo.workspace),
        role: pb::AgentRole::Builder as i32,
        harness: "claude".into(),
        actor: "manager".into(),
        ..Default::default()
    });
    let (_, what) = refusal(call(&mut a, "agent.record", neither).await);
    assert_eq!(what, "place", "an agent is a pane or a subagent");
}

/// Reading the plan is a read, and the path stays home; writing it is control.
#[tokio::test]
async fn a_read_client_reads_the_plan_without_the_path_and_cannot_write_it() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let task = store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    store.put_on_line(task.id, &NewTicketLine { worktree_path: "/secret".into(), ..Default::default() }, Actor::Manager).unwrap();
    let mut link = connect(&h).await;
    let read = plan(&mut link, repo.workspace).await;
    assert_eq!(read.lines[0].worktree_path, "");

    for (method, p) in [
        ("line.order", order(repo.workspace, &[])),
        ("line.set", line_set(task.id, |p| p.reason = Some("x".into()))),
        ("agent.end", payload::Payload::AgentEnd(pb::AgentEnd { agent_id: id(Uuid::now_v7()), verdict: "finished".into(), actor: String::new() })),
    ] {
        let (code, _) = refusal(call(&mut link, method, p).await);
        assert_eq!(code, ErrorCode::ScopeDenied as i32, "{method}");
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
    store.add_note(b.id, NoteKind::Progress, Actor::Manager, "Dispatched to a builder.", serde_json::json!({})).unwrap();

    let mut link = connect(&h).await;
    record(&mut link, repo.workspace, pb::AgentRole::Builder, "agent-1", &[a.id, b.id]).await;
    let to_building = line_set(b.id, |p| p.stage = Some(pb::TicketStage::Building as i32));
    call(&mut link, "line.set", to_building).await.unwrap();
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

/// A ticket that leaves waiting goes out on the wire without its place in the
/// plan, and no line or agent write moves its card.
#[tokio::test]
async fn a_ticket_that_leaves_waiting_is_sent_without_a_rank() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;
    line(&mut a, line_set(task.id, |_| {})).await;
    call(&mut a, "line.order", order(repo.workspace, &[task.id])).await.unwrap();
    assert_eq!(plan(&mut a, repo.workspace).await.lines[0].plan_rank, Some(1));

    let moved = line(&mut a, line_set(task.id, |p| p.stage = Some(pb::TicketStage::Building as i32))).await;
    assert_eq!(moved.plan_rank, None, "a building ticket isn't 2nd in the plan");
    assert_eq!(plan(&mut a, repo.workspace).await.lines[0].plan_rank, None);
    record(&mut a, repo.workspace, pb::AgentRole::Builder, "agent-1", &[task.id]).await;
    let (plans, tasks) = events(&mut listener, Duration::from_millis(600)).await;
    assert_eq!(plans.len(), 4, "one per write");
    // The line's writes leave the card alone; a builder set to work on a card
    // that hadn't started starts it, as a worker recorded on it always has.
    assert_eq!(tasks, 1, "the builder starts its backlog card, and nothing else moves the board");
    assert_eq!(h.service.store.get_task(task.id).unwrap().status, farcooler_store::models::TaskStatus::InProgress);
}

/// Lanes and trains are gone: their methods are refused as retired, so an app
/// from before is told the feature went rather than to update the runner.
#[tokio::test]
async fn a_lane_method_is_refused_as_retired() {
    let h = start(Scope::Control).await;
    let mut a = connect(&h).await;
    for method in ["lane.create", "lane.update", "plan.set", "train.start", "train.set"] {
        let mut r = request(method);
        r.required_capabilities = vec![farcooler_protocol::capability::BOARD_PLAN.into()];
        match a.call(r).await {
            Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::MethodRetired as i32, "{method}"),
            other => panic!("{method}: expected a refusal, got {other:?}"),
        }
    }
}
