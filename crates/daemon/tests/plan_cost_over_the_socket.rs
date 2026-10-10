//! Cost on the plan (ov-307) as a client meets it: a budget written through
//! the two updates that already exist, and the trend, the week and the
//! comparison read back from `plan.get`, through the real dispatch table.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_core::usage::TokenCounts;
use farcooler_protocol::v1::{self as pb, Scope, event, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_store::usage::{NewTurn, Surface, TurnKind, TurnModel};
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

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// A budget is set, read back on the theme and the lane, flagged by the
/// numbers it carries beside the spend, announced, and taken away by zero.
#[tokio::test]
async fn a_budget_is_set_over_the_wire_and_read_back_with_the_trend_and_the_week() {
    let h = start(Scope::HostAdmin).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;

    let result::Value::BoardThemeView(theme) = call(
        &mut a,
        "board_theme.create",
        payload::Payload::BoardThemeCreate(pb::BoardThemeCreate {
            workspace_id: id(repo.workspace),
            name: "Cost".into(),
            outcome: "Spend is visible.".into(),
            task_ids: vec![id(task.id)],
            actor: "manager".into(),
        }),
    )
    .await
    .expect("board_theme.create") else {
        panic!("wrong result")
    };
    let theme_id = theme.theme.unwrap().id;
    let result::Value::Lane(lane) = call(
        &mut a,
        "lane.create",
        payload::Payload::LaneCreate(pb::LaneCreate {
            workspace_id: id(repo.workspace),
            name: "cost-lane".into(),
            cards: vec![pb::LaneCard { task_id: id(task.id), slice: String::new(), stage: None }],
            agent: Some(pb::LaneAgentRecord {
                harness: "claude".into(),
                agent_id: "a1".into(),
                role: pb::LaneAgentRole::Build as i32,
                model: None,
                ended: false,
            }),
            actor: "manager".into(),
            ..Default::default()
        }),
    )
    .await
    .expect("lane.create") else {
        panic!("wrong result")
    };
    h.service
        .store
        .record_turn(&NewTurn {
            key: "claude-log:agent:a1".into(),
            terminal_id: None,
            // Filed where the session ran, as the daemon files a turn: the
            // week counts this project's repository and its worktrees.
            worktree_id: Some(repo.worktree),
            repository_id: Some(repo.id),
            workspace_id: None,
            task_id: None,
            harness: "claude".into(),
            surface: Surface::Terminal,
            started_at: None,
            ended_at: now_ms(),
            active_ms: None,
            usage: "reported",
            models: vec![TurnModel::priced(
                Some("claude-opus-5".into()),
                TokenCounts { input: 1_000, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0 },
                Some(2_000),
            )],
            kind: TurnKind::Subagent,
        })
        .unwrap();
    // Drain what the setup announced, so the budget's own is the one counted.
    while tokio::time::timeout(Duration::from_millis(300), listener.next_event()).await.is_ok() {}

    let set = |budget| {
        payload::Payload::BoardThemeUpdate(pb::BoardThemeUpdate {
            theme_id: theme_id.clone(),
            budget_tokens: Some(budget),
            actor: "manager".into(),
            ..Default::default()
        })
    };
    let result::Value::BoardThemeView(view) = call(&mut a, "board_theme.update", set(600)).await.unwrap() else {
        panic!("wrong result")
    };
    assert_eq!(view.budget_tokens, Some(600), "the answer carries it");
    let lane_set = payload::Payload::LaneUpdate(pb::LaneUpdate {
        lane_id: lane.id.clone(),
        budget_tokens: Some(5_000),
        actor: "manager".into(),
        ..Default::default()
    });
    let result::Value::Lane(l) = call(&mut a, "lane.update", lane_set).await.unwrap() else { panic!("wrong result") };
    assert_eq!(l.budget_tokens, Some(5_000));

    let read = plan(&mut connect(&h).await, repo.workspace).await;
    let theme = &read.themes[0];
    assert_eq!(theme.budget_tokens, Some(600));
    let spent = theme.spend.unwrap();
    assert!(spent.input_tokens > 600, "past its budget: {spent:?}");
    assert_eq!(theme.trend_tokens.len(), 7);
    assert_eq!(theme.trend_tokens.iter().sum::<u64>(), 1_000, "today's turn, in the trend");
    assert_eq!(*theme.trend_tokens.last().unwrap(), 1_000, "today is last");
    assert_eq!(read.lanes[0].budget_tokens, Some(5_000));
    let cost = read.cost.expect("the runner sends cost");
    assert_eq!(cost.week_tokens, 1_000);
    assert!(cost.compare.is_empty() && cost.compare_held_back == 0, "no finished card yet");

    let mut plans = 0;
    while let Ok(Ok(e)) = tokio::time::timeout(Duration::from_millis(400), listener.next_event()).await {
        if matches!(e.payload, Some(event::Payload::PlanChanged(_))) {
            plans += 1;
        }
    }
    assert_eq!(plans, 2, "each budget write announces plan_changed once");

    call(&mut a, "board_theme.update", set(0)).await.unwrap();
    assert_eq!(plan(&mut a, repo.workspace).await.themes[0].budget_tokens, None, "zero takes it away");
}
