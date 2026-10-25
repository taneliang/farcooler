//! Who is pushed about an item (ov-633): a member's own, out loud on their
//! phone and quietly on the owner's once the relay takes it; the owner's and
//! nobody's, as before; and never nobody (review M2).

use std::sync::Arc;

use farcooler_protocol::v1::AgentActivity;
use farcooler_store::claude_logins::TerminalLogin;
use farcooler_store::models::Actor;
use farcooler_store::people::Directory;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedReceiver;
use uuid::Uuid;

use super::super::{Quoted, Tapped, Watcher, Who, agent_notice};
use super::Class;
use crate::push::Pairing;
use crate::service::Service;

/// A runner with a Main workspace and one pane in its checkout, and Alice as
/// a member with the relay account `acct-alice`.
async fn a_runner() -> (crate::test_support::ScratchDir, Arc<Service>, Uuid, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let checkout = rows.iter().find(|w| w.is_main_checkout).unwrap();
    let pane = svc.store.create_terminal_for_test(checkout.id, main.id);
    let directory = Directory { account: "acct-alice", email: "alice@example.com" };
    svc.store.set_member_in("alice", "Alice Chen", &["alice-phone".into()], directory).unwrap();
    (dir, svc, main.id, pane)
}

/// Launch `pane` on `handle`'s login, as `claude_logins` records it.
fn launched_for(svc: &Service, pane: Uuid, handle: &str) {
    let login_id = svc.store.ensure_member_login(handle).unwrap();
    let login = TerminalLogin { handle: handle.into(), login_id, config_dir: format!("/r/claude/{handle}"), shared: false };
    svc.store.set_terminal_login(pane, &login).unwrap();
}

fn blocked_notice(watcher: &Watcher, pane: Uuid) {
    let who = Who { label: "claude".into(), workspace: Some("Main".into()), orchestrator: false };
    let quoted = Quoted { worktree: "main", question: Some("Which one?"), said: None };
    let notice = agent_notice(AgentActivity::Blocked, &who, quoted, false, None).unwrap();
    watcher.push_notice_alerting(pane, who, notice, true);
}

/// Every notice the watcher sends in the next moment.
async fn heard(taps: &mut UnboundedReceiver<Tapped>) -> Vec<Tapped> {
    let mut out = Vec::new();
    while let Ok(Some(t)) = tokio::time::timeout(std::time::Duration::from_millis(500), taps.recv()).await {
        out.push(t);
    }
    out
}

/// A relay on a local port, paired with `svc`: it answers the member route
/// with `member_status` and `member_body`, and everything else with 200, and
/// hands each request over as its first line and its body.
async fn a_relay(svc: &Service, member_status: &'static str, member_body: &'static str) -> UnboundedReceiver<(String, serde_json::Value)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    Pairing { relay: format!("http://{}", listener.local_addr().unwrap()), token: "t".into() }
        .save_in(svc.root_dir())
        .unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut seen = Vec::new();
                let mut buf = [0u8; 8192];
                loop {
                    let Ok(n) = socket.read(&mut buf).await else { return };
                    if n == 0 {
                        return;
                    }
                    seen.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&seen).to_string();
                    let Some(end) = text.find("\r\n\r\n") else { continue };
                    let length = text[..end]
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if seen.len() < end + 4 + length {
                        continue;
                    }
                    let line = text.lines().next().unwrap_or_default().to_string();
                    let (status, body) = if line.contains("/v1/notify/member") { (member_status, member_body) } else { ("200 OK", "{}") };
                    let reply = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(reply.as_bytes()).await;
                    let body = serde_json::from_slice(&seen[end + 4..end + 4 + length]).unwrap_or_default();
                    let _ = tx.send((line, body));
                    return;
                }
            });
        }
    });
    rx
}

/// What the relay was sent in the next moment.
async fn sent_to(relay: &mut UnboundedReceiver<(String, serde_json::Value)>) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    while let Ok(Some(r)) = tokio::time::timeout(std::time::Duration::from_secs(2), relay.recv()).await {
        out.push(r);
    }
    out
}

fn member_requests(sent: &[(String, serde_json::Value)]) -> Vec<&serde_json::Value> {
    sent.iter().filter(|(line, _)| line.starts_with("POST /v1/notify/member ")).map(|(_, b)| b).collect()
}

/// The owner's agent notice, as the relay got it.
fn owners_notice(sent: &[(String, serde_json::Value)]) -> &serde_json::Value {
    &sent.iter().find(|(line, b)| line.starts_with("POST /v1/notify ") && b["status"] == "blocked").expect("the owner's notice went").1
}

#[tokio::test]
async fn a_members_blocked_agent_pushes_her_account_and_only_moves_the_owners_card() {
    let (_dir, svc, _, pane) = a_runner().await;
    launched_for(&svc, pane, "alice");
    let mut relay = a_relay(&svc, "200 OK", r#"{"ok":true,"sent":true}"#).await;
    let watcher = Watcher::new(svc.clone());
    blocked_notice(&watcher, pane);
    let sent = sent_to(&mut relay).await;
    let member = member_requests(&sent);
    assert_eq!(member.len(), 1, "{sent:#?}");
    assert_eq!((&member[0]["account"], &member[0]["kind"], &member[0]["class"]), (&"acct-alice".into(), &"needs_you".into(), &"blocked".into()));
    // What her tap opens (ov-705): the pane, and no task.
    assert_eq!((&member[0]["terminal"], member[0].get("task")), (&pane.to_string().into(), None), "{sent:#?}");
    assert_eq!(owners_notice(&sent)["alert"], false, "the owner's phone doesn't buzz about Alice's agent: {sent:#?}");
}

/// M2: a relay older than `needs_you` answers 400, and the owner is buzzed
/// instead of nobody.
#[tokio::test]
async fn a_relay_that_refuses_the_members_push_leaves_the_owner_buzzed() {
    let (_dir, svc, _, pane) = a_runner().await;
    launched_for(&svc, pane, "alice");
    let mut relay = a_relay(&svc, "400 Bad Request", r#"{"error":"kind"}"#).await;
    let watcher = Watcher::new(svc.clone());
    blocked_notice(&watcher, pane);
    let sent = sent_to(&mut relay).await;
    assert_eq!(member_requests(&sent).len(), 1, "the member push was tried: {sent:#?}");
    assert!(owners_notice(&sent).get("alert").is_none(), "the owner alerts as before: {sent:#?}");
}

/// M2, last round: a relay that took the push but sent it nowhere (no
/// install, every install keeping the class off, a refusal) says
/// `"sent": false`, and one older than `sent` says nothing: either way the
/// owner is buzzed.
#[tokio::test]
async fn a_push_the_relay_sent_nowhere_leaves_the_owner_buzzed() {
    for body in [r#"{"ok":true,"sent":false}"#, r#"{"ok":true}"#] {
        let (_dir, svc, _, pane) = a_runner().await;
        launched_for(&svc, pane, "alice");
        let mut relay = a_relay(&svc, "200 OK", body).await;
        let watcher = Watcher::new(svc.clone());
        blocked_notice(&watcher, pane);
        let sent = sent_to(&mut relay).await;
        assert_eq!(member_requests(&sent).len(), 1, "{body}: {sent:#?}");
        assert!(owners_notice(&sent).get("alert").is_none(), "{body}: the owner alerts: {sent:#?}");
    }
}

/// M2: a member added by hand has no relay account, so the owner is buzzed.
#[tokio::test]
async fn a_member_with_no_relay_account_leaves_the_owner_buzzed() {
    let (_dir, svc, _, pane) = a_runner().await;
    svc.store.set_member("bob", "Bob Okafor", &[]).unwrap();
    launched_for(&svc, pane, "bob");
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    blocked_notice(&watcher, pane);
    let sent = heard(&mut taps).await;
    assert!(sent.iter().all(|t| t.kind != Some("member")), "{sent:#?}");
    assert!(sent.iter().any(|t| t.kind.is_none() && !t.quiet), "{sent:#?}");
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
    svc.store.set_member("bob", "Bob Okafor", &[]).unwrap();
    launched_for(&svc, pane, "bob");
    svc.store.remove_member("bob").unwrap();
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    blocked_notice(&watcher, pane);
    let sent = heard(&mut taps).await;
    assert!(sent.iter().all(|t| t.kind != Some("member")), "{sent:#?}");
    assert!(sent.iter().any(|t| t.kind.is_none() && !t.quiet), "nobody's goes to the owner: {sent:#?}");
}

#[tokio::test]
async fn a_solo_owner_is_pushed_as_before() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let pane = svc.store.create_terminal_for_test(rows.iter().find(|w| w.is_main_checkout).unwrap().id, main.id);
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    blocked_notice(&watcher, pane);
    let sent = heard(&mut taps).await;
    assert!(sent.iter().any(|t| t.kind.is_none() && !t.quiet), "{sent:#?}");
    assert!(sent.iter().all(|t| t.kind != Some("member")));
}

/// A task on Alice's lane: the lane opened for it, on her login.
fn alices_task(svc: &Service, workspace: Uuid) -> farcooler_store::models::Task {
    lane_task(svc, workspace, "alice")
}

/// A task on `handle`'s lane: the lane opened for it, on their login.
fn lane_task(svc: &Service, workspace: Uuid, handle: &str) -> farcooler_store::models::Task {
    let task = svc.store.create_task(workspace, "Pick a PDF library", Actor::User).unwrap();
    let checkout = svc.store.list_worktrees_for_repository(task.repository_id).unwrap();
    let lane = svc
        .store
        .create_terminal_for_task(checkout[0].id, "agent", "claude", farcooler_protocol::v1::TerminalIntent::Running, 80, 24, Some(task.id))
        .unwrap();
    launched_for(svc, lane.id, handle);
    task
}

/// `handle` moves `task` to In Review from their phone, as the gate files it.
fn moved_to_review_by(svc: &Service, watcher: &Watcher, task: Uuid, handle: &str) {
    let to = farcooler_protocol::v1::TaskSetStatus {
        task_id: crate::wire::id_bytes(task),
        status: farcooler_protocol::v1::TaskStatus::InReview as i32,
        actor: format!("member:{handle}"),
    };
    crate::task_ops::set_status(svc, watcher, &to).unwrap();
}

/// ov-712: Alice moving her own lane's card to In Review is no news to her:
/// her phone isn't pushed, and the owner is told as for anyone's move. The
/// runner's `notice` event names her, so her own Mac doesn't post it either.
/// Her move of Bob's card still pushes Bob.
#[tokio::test]
async fn a_members_own_move_pushes_everyone_but_her() {
    let (_dir, svc, workspace, _) = a_runner().await;
    let bob = Directory { account: "acct-bob", email: "bob@example.com" };
    svc.store.set_member_in("bob", "Bob Okafor", &["bob-phone".into()], bob).unwrap();
    let hers = lane_task(&svc, workspace, "alice");
    let his = lane_task(&svc, workspace, "bob");
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    let mut events = watcher.subscribe();
    tokio::time::pause();

    moved_to_review_by(&svc, &watcher, hers.id, "alice");
    let sent = heard_until_quiet(&mut taps).await;
    assert!(sent.iter().all(|t| t.kind != Some("member")), "Alice isn't pushed about her own move: {sent:#?}");
    let notice = sent.iter().find(|t| t.kind == Some("task")).expect("the owner's notice went");
    assert!(!notice.quiet, "the owner is told: {sent:#?}");
    assert_eq!(notice_author(&mut events).await, "alice", "the notice names its author");

    moved_to_review_by(&svc, &watcher, his.id, "alice");
    let sent = heard_until_quiet(&mut taps).await;
    let member: Vec<_> = sent.iter().filter(|t| t.kind == Some("member")).collect();
    assert_eq!(member.len(), 1, "{sent:#?}");
    assert_eq!((member[0].title.as_str(), member[0].level), ("acct-bob", Some("review")), "Bob is pushed about his card");
}

/// The orchestrator's move names nobody: every device is told as before.
#[tokio::test]
async fn the_orchestrators_move_names_no_author() {
    let (_dir, svc, workspace, _) = a_runner().await;
    let hers = alices_task(&svc, workspace);
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    let mut events = watcher.subscribe();
    tokio::time::pause();
    let to = farcooler_protocol::v1::TaskSetStatus {
        task_id: crate::wire::id_bytes(hers.id),
        status: farcooler_protocol::v1::TaskStatus::InReview as i32,
        actor: "manager".into(),
    };
    crate::task_ops::set_status(&svc, &watcher, &to).unwrap();
    let sent = heard_until_quiet(&mut taps).await;
    assert_eq!(sent.iter().filter(|t| t.kind == Some("member")).count(), 1, "Alice is pushed: {sent:#?}");
    assert_eq!(notice_author(&mut events).await, "");
}

/// Every notice tapped until the task composer's windows have long closed.
async fn heard_until_quiet(taps: &mut UnboundedReceiver<Tapped>) -> Vec<Tapped> {
    let mut out = Vec::new();
    while let Ok(Some(t)) = tokio::time::timeout(super::super::task_notice::AT_MOST * 3, taps.recv()).await {
        out.push(t);
    }
    out
}

/// The author on the next `notice` event.
async fn notice_author(events: &mut tokio::sync::broadcast::Receiver<farcooler_protocol::v1::Event>) -> String {
    loop {
        let event = tokio::time::timeout(super::super::task_notice::AT_MOST * 3, events.recv()).await.expect("a notice").unwrap();
        if let Some(farcooler_protocol::v1::event::Payload::Notice(n)) = event.payload {
            return n.author;
        }
    }
}

#[tokio::test]
async fn a_decision_on_a_members_lane_pushes_her_account_and_quiets_the_owners_thread() {
    let (_dir, svc, workspace, _) = a_runner().await;
    let task = alices_task(&svc, workspace);
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    assert!(!watcher.route_task_notice(&task, crate::needs_you::concern::whose_task(&svc.store, &task), Class::Decision, None).settle(&watcher, None).await, "the owner's thread goes quietly");
    let sent = heard(&mut taps).await;
    let member: Vec<_> = sent.iter().filter(|t| t.kind == Some("member")).collect();
    assert_eq!(member.len(), 1, "{sent:#?}");
    assert_eq!((member[0].title.as_str(), member[0].event, member[0].level), ("acct-alice", Some("needs_you"), Some("decision")));
    // A task finishing isn't waiting on anybody: no member push, and the
    // owner isn't buzzed about Alice's work either.
    assert!(!watcher.route_task_notice(&task, crate::needs_you::concern::whose_task(&svc.store, &task), Class::Done, None).settle(&watcher, None).await);
    assert!(heard(&mut taps).await.is_empty());
}

#[tokio::test]
async fn a_task_with_no_agent_is_nobodys_and_alerts_the_owner() {
    let (_dir, svc, workspace, _) = a_runner().await;
    let task = svc.store.create_task(workspace, "Pick a PDF library", Actor::User).unwrap();
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    assert!(watcher.route_task_notice(&task, crate::needs_you::concern::whose_task(&svc.store, &task), Class::Decision, None).settle(&watcher, None).await);
    assert!(heard(&mut taps).await.is_empty());
}

#[test]
fn a_needs_you_push_names_its_kind_and_class_on_the_wire() {
    let kind = crate::push::MemberKind::NeedsYou("review");
    assert_eq!((kind.word(), kind.class()), ("needs_you", Some("review")));
    assert_eq!((crate::push::MemberKind::Login.word(), crate::push::MemberKind::Login.class()), ("login", None));
}

/// ov-740 review F4: a notice's panes are read only for a member's item
/// that waits on them: never for the owner's or nobody's, for news, or for
/// a member's own write.
#[tokio::test]
async fn the_panes_are_read_only_for_a_members_waiting_item() {
    use crate::needs_you::concern::Whose;
    let (_dir, svc, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let never = || -> Vec<Uuid> { panic!("a notice's panes read when nothing needs them") };
    let waiting = || Some(("decision", "t".to_string(), crate::push::Opens::Task("OV-1".into())));
    watcher.route(None, waiting(), None, never);
    watcher.route(Some(Whose::Owner), waiting(), None, never);
    watcher.route(Some(Whose::Member("alice".into())), None, None, never);
    watcher.route(Some(Whose::Member("alice".into())), waiting(), Some("alice"), never);
    let mut asked = false;
    watcher.route(Some(Whose::Member("alice".into())), waiting(), None, || {
        asked = true;
        vec![]
    });
    assert!(asked, "her waiting item asks");
}
