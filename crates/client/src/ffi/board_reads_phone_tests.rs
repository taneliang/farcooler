//! A phone's read state, the whole way: `dispatch` with the JSON an app passes,
//! a `Session`, and the daemon's own `RpcFactory` at the scope the phone was
//! enrolled with (ov-113).

use farcooler_protocol::v1::{ErrorCode, Scope};
use farcooler_store::models::Actor;
use serde_json::json;

use super::dispatch;
use super::phone_path_tests::{Runner, a_runner};
use crate::session::{Session, SessionError};

/// The runner's one repository and its Main board, with a task on it.
fn a_board(runner: &Runner) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid) {
    let store = &runner.service.store;
    let repo = store.list_all_worktrees().unwrap().remove(0).repository_id;
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let task = store.create_task(main, "Mac: a jump", Actor::User).unwrap();
    (repo, main, task.id)
}

/// One phone opens a ticket and a board read on another carries the mark, in
/// the `reads` key beside `tasks`. Goes red when the `task.list` arm drops the
/// reads or `workspace.mark_read` has no arm.
#[tokio::test]
async fn a_phone_s_open_is_on_the_next_boards_reads() {
    let runner = a_runner(Scope::Control).await;
    let (repo, main, task) = a_board(&runner);
    let (a, b) = (
        Session::connect_local(&runner.socket).await.expect("connect"),
        Session::connect_local(&runner.socket).await.expect("connect"),
    );
    let at = crate::session::now_millis() - 1_000;

    let merged = dispatch(
        &a,
        "workspace.mark_read",
        &json!({ "workspace": main.to_string(), "opened": [{ "task_id": task.to_string(), "opened_ms": at }] }),
    )
    .await
    .expect("workspace.mark_read");
    assert_eq!(merged["opened"], json!([{ "task_id": task.to_string(), "opened_ms": at }]), "{merged}");

    let board = dispatch(&b, "task.list", &json!({ "repository": repo.to_string(), "workspace": main.to_string() }))
        .await
        .expect("task.list");
    assert_eq!(board["reads"], merged, "{board}");
    assert_eq!(board["tasks"].as_array().map(Vec::len), Some(1));

    // A read of every board in the repository carries no read state.
    let all = dispatch(&b, "task.list", &json!({ "repository": repo.to_string() })).await.expect("task.list");
    assert!(all.get("reads").is_none(), "{all}");
}

/// A read-scoped phone sees the board's state and cannot raise it.
#[tokio::test]
async fn a_read_scoped_phone_cannot_mark_a_board_read() {
    let runner = a_runner(Scope::Read).await;
    let (repo, main, task) = a_board(&runner);
    let session = Session::connect_local(&runner.socket).await.expect("connect");
    match dispatch(
        &session,
        "workspace.mark_read",
        &json!({ "workspace": main.to_string(), "opened": [{ "task_id": task.to_string(), "opened_ms": 5 }] }),
    )
    .await
    {
        Err(SessionError::Refused { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32),
        other => panic!("expected a scope denial, got {other:?}"),
    }
    let board = dispatch(&session, "task.list", &json!({ "repository": repo.to_string(), "workspace": main.to_string() }))
        .await
        .expect("task.list");
    assert_eq!(board["reads"]["opened"], json!([]), "{board}");
}
