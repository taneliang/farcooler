//! `report.get` as a client meets it: over a real socket, through the real
//! dispatch table and scope check, against a board the real store wrote.
//!
//! `report::tests` pin the arithmetic with every time chosen by hand. These
//! pin what those cannot: that `gather` reads what the store wrote (the
//! `StatusChange` notes' `to`, questions and answers, acceptance), that a
//! read-scoped client may ask, and what the runner refuses.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_daemon::report::Report;
use farcooler_protocol::v1::{ErrorCode, ReportRequest, Scope, request as payload, result};
use farcooler_store::models::{AcceptanceItem, Actor, NoteKind, TaskStatus, TaskUpdate};
use farcooler_transport::{ClientError, request};
use in_process::*;
use uuid::Uuid;

/// Three tasks: one done with two of its three lines ticked after a person
/// answered its question, one canceled, one still in progress. And a second
/// workspace with one new task.
fn seed(h: &Harness) -> (Repo, Uuid) {
    let repo = a_repository(h);
    let store = &h.service.store;

    let done = store.create_task(repo.workspace, "Mac: the jumpbar jumps anywhere", Actor::User).unwrap();
    let lines = [true, true, false]
        .map(|met| AcceptanceItem { id: Uuid::now_v7(), text: "a line".into(), met })
        .to_vec();
    store
        .update_task(
            done.id,
            done.resource_version,
            &TaskUpdate {
                title: done.title.clone(),
                intent: String::new(),
                acceptance: lines,
                constraints: Vec::new(),
                labels: vec!["found".into()],
                worktree_id: None,
            },
        )
        .unwrap();
    store.set_task_status(done.id, TaskStatus::InProgress, Actor::Manager).unwrap();
    store.add_note(done.id, NoteKind::Question, Actor::Manager, "Which one?", serde_json::json!({})).unwrap();
    store.set_task_status(done.id, TaskStatus::NeedsDecision, Actor::Manager).unwrap();
    store.add_note(done.id, NoteKind::Answer, Actor::User, "The first.", serde_json::json!({})).unwrap();
    store.set_task_status(done.id, TaskStatus::InProgress, Actor::Manager).unwrap();
    store.set_task_status(done.id, TaskStatus::Done, Actor::Manager).unwrap();

    let canceled = store.create_task(repo.workspace, "Relay: a rollup", Actor::User).unwrap();
    store.set_task_status(canceled.id, TaskStatus::Cancelled, Actor::Manager).unwrap();

    let working = store.create_task(repo.workspace, "Daemon: still going", Actor::User).unwrap();
    store.set_task_status(working.id, TaskStatus::InProgress, Actor::Manager).unwrap();

    let billing = store.create_workspace(repo.id, "Billing", "bil").unwrap();
    store.create_task(billing.id, "CLI: a new task", Actor::User).unwrap();
    (repo, billing.id)
}

fn asking(since: i64, until: i64) -> ReportRequest {
    ReportRequest { since, until, repository_id: None, workspace_id: None, utc_offset_minutes: 0 }
}

async fn report(link: &mut Link, ask: ReportRequest) -> Result<Report, ClientError> {
    let mut r = request("report.get");
    r.payload = Some(payload::Payload::ReportRequest(ask));
    let result = link.call(r).await?;
    let Some(result::Value::Report(r)) = result.value else { panic!("wrong result: {result:?}") };
    Ok(serde_json::from_str(&r.report_json).expect("the runner's JSON is a Report"))
}

fn a_minute_either_side() -> ReportRequest {
    asking(now_millis() - 60_000, now_millis() + 60_000)
}

#[tokio::test]
async fn a_read_client_gets_the_boards_numbers() {
    let h = start(Scope::Read).await;
    seed(&h);
    let r = report(&mut connect(&h).await, a_minute_either_side()).await.expect("report.get");
    let t = &r.totals;
    assert_eq!((t.created, t.completed, t.canceled), (4, 1, 1));
    assert_eq!((t.decisions.asked, t.decisions.answered, t.decisions.answered_by_you), (1, 1, 1));
    assert_eq!((t.needs_you.times, t.needs_you.cleared), (1, 1));
    assert_eq!((t.acceptance.met, t.acceptance.total), (2, 3));
    assert!(t.time_to_done.is_some());
    assert_eq!(r.scope.kind, "runner");
    let workspaces: Vec<_> = r.by_workspace.iter().map(|g| (g.name.as_str(), g.tally.created)).collect();
    assert_eq!(workspaces, [("Main", 3), ("Billing", 1)]);
    assert_eq!(r.by_label.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["found"]);
    assert_eq!(r.notable.canceled.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Relay: a rollup"]);
    assert!(t.usage.is_none(), "nothing records usage yet");
}

#[tokio::test]
async fn a_workspace_narrows_it() {
    let h = start(Scope::Read).await;
    let (_, billing) = seed(&h);
    let mut ask = a_minute_either_side();
    ask.workspace_id = Some(billing.as_bytes().to_vec().into());
    let r = report(&mut connect(&h).await, ask).await.expect("report.get");
    assert_eq!((r.totals.created, r.totals.completed), (1, 0));
    assert_eq!((r.scope.kind.as_str(), r.scope.name.as_deref(), r.scope.repository.as_deref()), ("workspace", Some("Billing"), Some("repo")));
}

#[tokio::test]
async fn a_period_before_the_board_is_empty() {
    let h = start(Scope::Read).await;
    seed(&h);
    let r = report(&mut connect(&h).await, asking(0, now_millis() - 3_600_000)).await.expect("report.get");
    assert_eq!(r.totals, Default::default());
    assert!(r.by_workspace.is_empty());
}

#[tokio::test]
async fn what_the_runner_refuses() {
    let h = start(Scope::Read).await;
    let (repo, billing) = seed(&h);
    let mut link = connect(&h).await;
    let code = |e: ClientError| match e {
        ClientError::Daemon { code, what, .. } => (ErrorCode::try_from(code).unwrap(), what),
        other => panic!("{other:?}"),
    };

    let now = now_millis();
    let backward = report(&mut link, asking(now, now - 1)).await.expect_err("backward");
    assert_eq!(code(backward), (ErrorCode::InvalidArgument, "until".to_string()));

    let mut both = a_minute_either_side();
    both.repository_id = Some(repo.id.as_bytes().to_vec().into());
    both.workspace_id = Some(billing.as_bytes().to_vec().into());
    let both = report(&mut link, both).await.expect_err("both");
    assert_eq!(code(both).0, ErrorCode::InvalidArgument);

    let mut elsewhere = a_minute_either_side();
    elsewhere.repository_id = Some(Uuid::now_v7().as_bytes().to_vec().into());
    let elsewhere = report(&mut link, elsewhere).await.expect_err("not here");
    assert_eq!(code(elsewhere).0, ErrorCode::NotFound);
}
