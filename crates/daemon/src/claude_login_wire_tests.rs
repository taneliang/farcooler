//! A pane's wire record names whose Claude login it runs on (ov-597).

use farcooler_protocol::v1::{self as pb, Request, response};
use farcooler_transport::Handler as _;

use super::*;
use crate::claude_logins::tests::{create, factory, runner};

/// `terminal.list` as the owner's connection, the way an app reads it.
async fn listed(svc: &std::sync::Arc<Service>, id: uuid::Uuid) -> pb::Terminal {
    let req = Request { method: "terminal.list".into(), ..Default::default() };
    let outcome = factory(svc, None, None).handle(req).await.outcome;
    let Some(response::Outcome::Result(r)) = outcome else { panic!("no list: {outcome:?}") };
    let Some(pb::result::Value::TerminalList(list)) = r.value else { panic!("not a list") };
    list.items.into_iter().find(|t| t.id.as_ref() == id.as_bytes()).expect("the pane is listed")
}

/// `term`'s login as a list would say it.
fn login_of(svc: &Service, term: &models::Terminal) -> Option<pb::ClaudeLogin> {
    svc.claude_logins_seen()?.label(term)
}

/// A runner where only the owner has a login says nothing, even with members
/// added: a member without a login of their own changes no pane's.
#[tokio::test]
async fn a_runner_with_only_the_owners_login_says_nothing() {
    let (_dir, svc, _main, checkout) = runner().await;
    let owners = create(&factory(&svc, None, None), checkout, "claude").await.expect("the owner's pane");
    assert_eq!(listed(&svc, owners).await.claude_login, None);
}

/// Once a member has a login, each pane names its person: the owner's as the
/// owner, a member's with the name people read, and a rep's as its member.
#[tokio::test]
async fn each_pane_names_whose_login_it_uses() {
    let (_dir, svc, main, checkout) = runner().await;
    let rep = svc.start_rep(main, "alice", "").await.expect("Alice's rep").terminal;
    let bobs = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    let owners = create(&factory(&svc, None, None), checkout, "claude").await.expect("the owner's pane");

    let alice = listed(&svc, rep.id).await.claude_login.expect("Alice's rep names a login");
    assert_eq!((alice.handle.as_str(), alice.name.as_str(), alice.owner), ("alice", "Alice Chen", false));
    let bob = listed(&svc, bobs).await.claude_login.expect("Bob's pane names a login");
    assert_eq!((bob.handle.as_str(), bob.name.as_str(), bob.owner), ("bob", "Bob", false));
    let owner = listed(&svc, owners).await.claude_login.expect("the owner's pane names a login");
    assert_eq!((owner.handle.as_str(), owner.name.as_str(), owner.owner), ("", "", true));
}

/// An orchestrator started afresh on the owner's login after its member lead
/// is removed (it comes back on the runner's own) reports the owner, not the
/// lead it was on before.
#[tokio::test]
async fn a_pane_reparented_to_the_owner_reports_the_owner() {
    let (_dir, svc, main, checkout) = runner().await;
    // Bob has a login, so the runner has more than one.
    create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let led = svc.start_orchestrator(main, "claude", true, None).await.expect("Bob leads");
    assert_eq!(login_of(&svc, &svc.store.get_terminal(led.id).unwrap()).map(|l| l.handle), Some("bob".into()));

    svc.store.set_workspace_lead(main, None).unwrap();
    let fresh = svc.start_orchestrator(main, "claude", true, None).await.expect("the owner leads");
    let owner = listed(&svc, fresh.id).await.claude_login.expect("names a login");
    assert!(owner.owner && owner.handle.is_empty(), "{owner:?}");
}

/// A pane whose member is gone names nobody rather than the handle's next
/// holder or a stale name.
#[tokio::test]
async fn a_removed_members_pane_names_nobody() {
    let (_dir, svc, _main, checkout) = runner().await;
    let bobs = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    svc.store.remove_member("bob").unwrap();
    assert_eq!(login_of(&svc, &svc.store.get_terminal(bobs).unwrap()), None);
}

/// A codex, web or changes pane reads no Claude login, so it names none; a
/// shell does (a `claude` typed there runs on it), and so does a claude pane.
#[tokio::test]
async fn only_a_pane_that_can_run_claude_names_a_login() {
    let (_dir, svc, _main, checkout) = runner().await;
    let bobs = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    let mut term = svc.store.get_terminal(bobs).unwrap();
    assert!(login_of(&svc, &term).is_some(), "claude");
    term.command_preset = "shell".into();
    assert!(login_of(&svc, &term).is_some(), "shell");
    term.command_preset = "codex".into();
    assert_eq!(login_of(&svc, &term), None, "codex");
    term.command_preset = "claude".into();
    term.pane_mode = models::PaneMode::Changes;
    assert_eq!(login_of(&svc, &term), None, "changes");
}

/// A handle removed and given to someone new does not put the new person's
/// name on the old pane: the pane's login id is the old person's.
#[tokio::test]
async fn a_reissued_handle_does_not_name_the_old_panes_login() {
    let (_dir, svc, _main, checkout) = runner().await;
    let bobs = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    svc.store.remove_member("bob").unwrap();
    svc.store.set_member("bob", "Bobbie Newcomer", &["phone-n".to_string()]).unwrap();
    svc.store.ensure_member_login("bob").unwrap();
    assert_eq!(login_of(&svc, &svc.store.get_terminal(bobs).unwrap()), None);
}
