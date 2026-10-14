//! A phone's people (ov-508): `member.list` through `dispatch`, a `Session`
//! and the daemon's own `RpcFactory`, in the shape the apps decode.

use farcooler_protocol::method::Method;
use farcooler_protocol::v1::Scope;
use serde_json::json;

use super::phone_path_tests::a_runner;
use super::{dispatch, route::route};
use crate::session::Session;

/// A member with a rep comes back named, with the rep's pane and board;
/// `member.list` is routed and its writes aren't. Goes red without the arm,
/// the route, or the rep's board.
#[tokio::test]
async fn a_phone_reads_who_is_on_the_runner() {
    let runner = a_runner(Scope::Control).await;
    let store = &runner.service.store;
    let worktree = store.list_all_worktrees().unwrap().remove(0);
    let pane = store.list_terminals_for_worktree(worktree.id).unwrap().remove(0);
    let board = store.ensure_main_workspace(worktree.repository_id).unwrap().id;
    store.set_member("alice", "Alice Chen", &[]).unwrap();
    store.bind_rep(pane.id, board, "alice").unwrap();
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    let list = dispatch(&session, "member.list", &json!({})).await.expect("member.list");
    assert_eq!(list["you"], "", "a local connection is the owner: {list}");
    assert_eq!(list["members"][0]["name"], "Alice Chen", "{list}");
    assert_eq!(list["members"][0]["rep_panes"], json!([{ "terminal": pane.id.to_string(), "workspace": board.to_string() }]));
    assert!(list["claude_account"].is_string(), "{list}");

    assert_eq!(route(Method::MemberList), Some("member.list"));
    for method in [Method::MemberSet, Method::MemberRemove, Method::RepStart] {
        assert_eq!(route(method), None, "{} has a phone route", method.name());
    }
}

/// A device enrolled only to look is told the account is shared, not whose.
#[tokio::test]
async fn a_read_only_phone_is_not_told_whose_account() {
    let runner = a_runner(Scope::Read).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");
    let list = dispatch(&session, "member.list", &json!({})).await.expect("member.list");
    assert_eq!(list["claude_account"], "", "{list}");
    assert_eq!(list["members"], json!([]));
}
