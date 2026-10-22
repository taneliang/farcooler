//! Who is pushed about an item (ov-633): a member's own, quietly on the
//! owner's phone and out loud on theirs; the owner's and nobody's, as before.

use std::sync::Arc;

use farcooler_protocol::v1::AgentActivity;
use farcooler_store::claude_logins::TerminalLogin;
use farcooler_store::models::Actor;
use uuid::Uuid;

use super::super::{Quoted, Tapped, Watcher, Who, agent_notice};
use super::Class;
use crate::service::Service;

/// A runner with a Main workspace and one pane in its checkout, and Alice as
/// a member.
async fn a_runner() -> (crate::test_support::ScratchDir, Arc<Service>, Uuid, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let checkout = rows.iter().find(|w| w.is_main_checkout).unwrap();
    let pane = svc.store.create_terminal_for_test(checkout.id, main.id);
    svc.store.set_member("alice", "Alice Chen", &["alice-phone".into()]).unwrap();
    (dir, svc, main.id, pane)
}

/// Launch `pane` on Alice's login, as `claude_logins` records it.
fn alices(svc: &Service, pane: Uuid) {
    let login_id = svc.store.ensure_member_login("alice").unwrap();
    let login = TerminalLogin { handle: "alice".into(), login_id, config_dir: "/r/claude/a".into(), shared: false };
    svc.store.set_terminal_login(pane, &login).unwrap();
}

fn blocked_notice(watcher: &Watcher, pane: Uuid) {
    let who = Who { label: "claude".into(), workspace: Some("Main".into()), orchestrator: false };
    let quoted = Quoted { worktree: "main", question: Some("Which one?"), said: None };
    let notice = agent_notice(AgentActivity::Blocked, &who, quoted, false, None).unwrap();
    watcher.push_notice_alerting(pane, who, notice, true);
}

/// Every notice the watcher sends in the next moment, by kind.
async fn heard(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) -> Vec<Tapped> {
    let mut out = Vec::new();
    while let Ok(Some(t)) = tokio::time::timeout(std::time::Duration::from_millis(500), taps.recv()).await {
        out.push(t);
    }
    out
}

#[tokio::test]
async fn a_members_blocked_agent_pushes_her_and_only_moves_the_owners_card() {
    let (_dir, svc, _, pane) = a_runner().await;
    alices(&svc, pane);
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    blocked_notice(&watcher, pane);
    let sent = heard(&mut taps).await;
    let member: Vec<_> = sent.iter().filter(|t| t.kind == Some("member")).collect();
    assert_eq!(member.len(), 1, "{sent:#?}");
    assert_eq!(member[0].title, "alice");
    let owners = sent.iter().find(|t| t.kind.is_none()).expect("the owner's card still moves");
    assert!(owners.quiet, "the owner's phone doesn't buzz about Alice's agent");
}

#[tokio::test]
async fn the_owners_agent_pushes_the_owner_as_before() {
    let (_dir, svc, _, pane) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    blocked_notice(&watcher, pane);
    let sent = heard(&mut taps).await;
    assert!(sent.iter().all(|t| t.kind != Some("member")), "{sent:#?}");
    assert!(sent.iter().any(|t| t.kind.is_none() && !t.quiet), "{sent:#?}");
}

#[tokio::test]
async fn a_removed_members_pane_is_nobodys_and_pushes_the_owner() {
    let (_dir, svc, _, pane) = a_runner().await;
    alices(&svc, pane);
    svc.store.set_member("bob", "Bob Okafor", &[]).unwrap();
    svc.store.remove_member("alice").unwrap();
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    blocked_notice(&watcher, pane);
    let sent = heard(&mut taps).await;
    assert!(sent.iter().all(|t| t.kind != Some("member")), "{sent:#?}");
    assert!(sent.iter().any(|t| t.kind.is_none() && !t.quiet), "nobody's goes to the owner: {sent:#?}");
}

#[tokio::test]
async fn a_solo_owner_is_pushed_as_before() {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let pane = svc.store.create_terminal_for_test(rows.iter().find(|w| w.is_main_checkout).unwrap().id, main.id);
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    blocked_notice(&watcher, pane);
    let sent = heard(&mut taps).await;
    assert!(sent.iter().any(|t| t.kind.is_none() && !t.quiet), "{sent:#?}");
    assert!(sent.iter().all(|t| t.kind != Some("member")));
    drop(dir);
}

#[tokio::test]
async fn a_decision_on_a_members_lane_pushes_her_and_quiets_the_owners_thread() {
    let (_dir, svc, workspace, _) = a_runner().await;
    let task = svc.store.create_task(workspace, "Pick a PDF library", Actor::User).unwrap();
    let checkout = svc.store.list_worktrees_for_repository(task.repository_id).unwrap();
    let lane = svc
        .store
        .create_terminal_for_task(checkout[0].id, "agent", "claude", farcooler_protocol::v1::TerminalIntent::Running, 80, 24, Some(task.id))
        .unwrap();
    alices(&svc, lane.id);
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    assert!(!watcher.route_task_notice(&task, Class::Decision), "the owner's thread goes quietly");
    let sent = heard(&mut taps).await;
    assert_eq!(sent.iter().filter(|t| t.kind == Some("member")).count(), 1, "{sent:#?}");
    // A task finishing isn't waiting on anybody: no member push.
    let _ = watcher.route_task_notice(&task, Class::Done);
    assert!(heard(&mut taps).await.is_empty());
}

#[tokio::test]
async fn a_task_with_no_agent_is_nobodys_and_alerts_the_owner() {
    let (_dir, svc, workspace, _) = a_runner().await;
    let task = svc.store.create_task(workspace, "Pick a PDF library", Actor::User).unwrap();
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    assert!(watcher.route_task_notice(&task, Class::Decision));
    assert!(heard(&mut taps).await.is_empty());
}
