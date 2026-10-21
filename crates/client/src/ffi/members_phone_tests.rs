//! A phone's people (ov-508): `member.list` through `dispatch`, a `Session`
//! and the daemon's own `RpcFactory`, in the shape the apps decode.

use farcooler_protocol::method::Method;
use farcooler_protocol::v1::Scope;
use serde_json::json;

use super::phone_path_tests::{a_runner, a_runner_for};
use super::{dispatch, route::route};
use crate::session::Session;

/// Put `handle`'s Claude directory in the state `claude auth login` leaves
/// it, which `rep.start` asks for (ov-587).
fn signed_in(service: &farcooler_daemon::service::Service, handle: &str) {
    let id = service.store.ensure_member_login(handle).unwrap();
    let dir = farcooler_daemon::claude_logins::ensure_login_dir(service.root_dir(), &id).unwrap();
    std::fs::write(std::path::Path::new(&dir).join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"member@example.com"}}"#).unwrap();
}

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
    assert_eq!(list["claude_account"], "owner@example.com", "Control is told whose account: {list}");

    assert_eq!(route(Method::MemberList), Some("member.list"));
    for method in [Method::MemberSet, Method::MemberRemove] {
        assert_eq!(route(method), None, "{} has a phone route", method.name());
    }
}

/// A device enrolled only to look is told the account is shared, not whose.
#[tokio::test]
async fn a_read_only_phone_is_not_told_whose_account() {
    let runner = a_runner(Scope::Read).await;
    runner.service.store.set_member("alice", "Alice Chen", &[]).unwrap();
    let session = Session::connect_local(&runner.socket).await.expect("connect");
    let list = dispatch(&session, "member.list", &json!({})).await.expect("member.list");
    assert_eq!(list["claude_account"], "", "{list}");
    assert_eq!(list["members"][0]["name"], "Alice Chen", "the people are for every scope: {list}");
}

/// A phone starts its own rep through the bridge (ov-526): `rep.start` is
/// routed, `{workspace, handle}` reaches the runner, and the answer carries
/// the pane and the account. A call with no handle or no workspace never
/// leaves the phone, by its own complaint. Goes red without the route's arm,
/// the dispatch arm, or the session call.
#[tokio::test]
async fn a_phone_starts_its_own_rep() {
    assert_eq!(route(Method::RepStart), Some("rep.start"));
    let runner = a_runner(Scope::Control).await;
    let store = &runner.service.store;
    let worktree = store.list_all_worktrees().unwrap().remove(0);
    let board = store.ensure_main_workspace(worktree.repository_id).unwrap().id;
    store.set_member("alice", "Alice Chen", &[]).unwrap();
    signed_in(&runner.service, "alice");
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    let nothing = dispatch(&session, "rep.start", &json!({ "workspace": board.to_string() })).await;
    assert!(nothing.unwrap_err().to_string().contains("needs a handle"), "a start with no handle is refused before the wire");
    let nowhere = dispatch(&session, "rep.start", &json!({ "handle": "alice" })).await;
    assert!(nowhere.unwrap_err().to_string().contains("needs a workspace"), "a start with no workspace is refused before the wire");

    let started = dispatch(&session, "rep.start", &json!({ "workspace": board.to_string(), "handle": "alice" })).await;
    let started = started.expect("the owner's local connection starts anyone's rep");
    assert!(started["id"].as_str().is_some_and(|id| id.len() == 36), "{started}");
    assert_eq!(started["desk"].as_str().map(|k| k.is_empty()), Some(false), "{started}");
    assert_eq!(started["claude_account"], "owner@example.com", "{started}");

    let list = dispatch(&session, "member.list", &json!({})).await.expect("member.list");
    assert_eq!(list["members"][0]["rep_panes"][0]["terminal"], started["id"], "the rep is listed: {list}");
}

/// A member's phone starts its own rep and is refused anyone else's, the
/// runner's way (`scope-denied`), through the same bridge. Goes red if the
/// bridge named the wrong handle, or the runner stopped checking.
#[tokio::test]
async fn a_members_phone_starts_only_its_own_rep() {
    let runner = a_runner_for(Scope::Control, Some("phone-b")).await;
    let store = &runner.service.store;
    let worktree = store.list_all_worktrees().unwrap().remove(0);
    let board = store.ensure_main_workspace(worktree.repository_id).unwrap().id;
    store.set_member("alice", "Alice Chen", &[]).unwrap();
    store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    let list = dispatch(&session, "member.list", &json!({})).await.expect("member.list");
    assert_eq!(list["you"], "bob", "this connection is Bob's: {list}");

    let theirs = dispatch(&session, "rep.start", &json!({ "workspace": board.to_string(), "handle": "alice" })).await;
    let refused = theirs.expect_err("Bob starting Alice's rep");
    assert!(
        matches!(&refused, crate::session::SessionError::Refused { message, .. } if message == "scope denied: host_admin required"),
        "{refused:?}"
    );
    // Not signed in to Claude yet, where the runner's admins require their
    // own: asked to, with no rep started (ov-587, ov-652).
    store.set_member_login_policy("claude", true, "-", 0).unwrap();
    let unsigned = dispatch(&session, "rep.start", &json!({ "workspace": board.to_string(), "handle": "bob" })).await;
    let asked = unsigned.expect_err("Bob has no Claude login yet");
    assert!(asked.to_string().contains("Sign in to Claude"), "{asked:?}");
    signed_in(&runner.service, "bob");
    let own = dispatch(&session, "rep.start", &json!({ "workspace": board.to_string(), "handle": "bob" })).await;
    assert!(own.is_ok(), "Bob starts his own: {own:?}");
}

/// A runner admin's phone sets a switch through the bridge (ov-652):
/// `member.set_own_login` is routed, `{harness, on}` reaches the runner, and
/// the answer is the list with the switches. A `control` phone is told it
/// may not, and is refused; a call with no `on` never leaves the phone.
#[tokio::test]
async fn an_admins_phone_sets_members_own_logins() {
    assert_eq!(route(Method::MemberSetOwnLogin), Some("member.set_own_login"));
    let admin = a_runner(Scope::HostAdmin).await;
    let session = Session::connect_local(&admin.socket).await.expect("connect");
    let list = dispatch(&session, "member.list", &json!({})).await.expect("member.list");
    assert_eq!(list["own_logins"], json!({ "claude": false, "codex": false, "cursor": false, "may_change": true }), "off by default");
    let set = dispatch(&session, "member.set_own_login", &json!({ "harness": "cursor", "on": true })).await.expect("set");
    assert_eq!(set["own_logins"]["cursor"], true, "{set}");
    assert!(admin.service.store.member_login_policy().unwrap().cursor, "stored");
    let nothing = dispatch(&session, "member.set_own_login", &json!({ "harness": "codex" })).await;
    assert!(nothing.unwrap_err().to_string().contains("needs on"));

    let phone = a_runner(Scope::Control).await;
    let session = Session::connect_local(&phone.socket).await.expect("connect");
    let list = dispatch(&session, "member.list", &json!({})).await.expect("member.list");
    assert_eq!(list["own_logins"]["may_change"], false, "{list}");
    let refused = dispatch(&session, "member.set_own_login", &json!({ "harness": "codex", "on": true })).await;
    assert!(refused.is_err(), "{refused:?}");
    assert!(!phone.service.store.member_login_policy().unwrap().codex, "unchanged");
}
