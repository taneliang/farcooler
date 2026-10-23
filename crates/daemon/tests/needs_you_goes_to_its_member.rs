//! `needs_you.list` on a runner with members, over the real socket (ov-633):
//! a member reads their own item first and another member's after it, marked
//! as someone else's, though the other's is a more urgent kind.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_agent_core::event::{AgentEvent, PermissionOption};
use farcooler_daemon::needs_you::Observation;
use farcooler_daemon::push::Pairing;
use farcooler_protocol::v1::{self as pb, AgentActivity, NeedsYouItem, NeedsYouKind, Scope, request as payload, result};
use farcooler_store::claude_logins::TerminalLogin;
use farcooler_store::models::{Actor, NoteKind};
use farcooler_store::people::Directory;
use farcooler_transport::{Peer, request};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use in_process::*;
use uuid::Uuid;

/// Alice and Bob, each with a device and a relay account.
fn members(h: &Harness) {
    let store = &h.service.store;
    let directory = |account| Directory { account, email: "" };
    store.set_member_in("alice", "Alice Chen-Montgomery", &["alice-phone".into()], directory("acct-alice")).unwrap();
    store.set_member_in("bob", "Bob Okafor-Lindqvist", &["bob-phone".into()], directory("acct-bob")).unwrap();
}

/// `members`, and a pane launched on each one's login.
fn two_members(h: &Harness) -> (Uuid, Uuid) {
    members(h);
    let repo = a_repository(h);
    let pane_of = |handle: &str| on_login(h, a_pane(h, repo.worktree, None), handle);
    (pane_of("alice"), pane_of("bob"))
}

/// Record `pane` as launched on `handle`'s login, as `claude_logins` does.
fn on_login(h: &Harness, pane: Uuid, handle: &str) -> Uuid {
    let store = &h.service.store;
    let login_id = store.ensure_member_login(handle).unwrap();
    let login = TerminalLogin { handle: handle.into(), login_id, config_dir: format!("/r/claude/{handle}"), shared: false };
    store.set_terminal_login(pane, &login).unwrap();
    pane
}

/// A relay on a local port, paired with the runner: 200 to everything, a
/// member push answered as sent, and each request handed over as its first
/// line and its body.
async fn a_relay(h: &Harness) -> tokio::sync::mpsc::UnboundedReceiver<(String, serde_json::Value)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    Pairing { relay: format!("http://{}", listener.local_addr().unwrap()), token: "t".into() }
        .save_in(h.service.root_dir())
        .unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut seen = Vec::new();
                let mut buf = [0u8; 8192];
                while let Ok(n) = socket.read(&mut buf).await {
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
                    let reply: &[u8] = if text.starts_with("POST /v1/notify/member ") {
                        b"HTTP/1.1 200 OK\r\ncontent-length: 23\r\nconnection: close\r\n\r\n{\"ok\":true,\"sent\":true}"
                    } else {
                        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}"
                    };
                    let _ = socket.write_all(reply).await;
                    let body = serde_json::from_slice(&seen[end + 4..end + 4 + length]).unwrap_or_default();
                    let _ = tx.send((text.lines().next().unwrap_or_default().to_string(), body));
                    return;
                }
            });
        }
    });
    rx
}

/// Alice's pane is blocked; Bob's holds a chat ask, which outranks a block.
async fn alice_blocked_and_bob_asking(h: &Harness, alices: Uuid, bobs: Uuid) {
    h.watcher
        .observe_for_tests(
            alices,
            Observation { activity: AgentActivity::Blocked, state_since: now_millis(), command: "claude".into(), ..Observation::default() },
        )
        .await;
    h.service.agents().record(
        bobs,
        vec![AgentEvent::Permission {
            id: "chat-1".into(),
            tool_call: "t1".into(),
            options: vec![PermissionOption { id: "allow".into(), name: "Allow touch x".into(), kind: "allow_once".into() }],
        }],
        &|_, _| {},
    );
    h.watcher
        .observe_for_tests(
            bobs,
            Observation { activity: AgentActivity::Blocked, state_since: now_millis(), command: "claude".into(), ..Observation::default() },
        )
        .await;
}

async fn needs_you(h: &Harness) -> Vec<NeedsYouItem> {
    let result = connect(h).await.call(request("needs_you.list")).await.expect("needs_you.list");
    let Some(result::Value::NeedsYouList(list)) = result.value else { panic!("wrong result: {result:?}") };
    list.items
}

fn terminal_of(item: &NeedsYouItem) -> Uuid {
    Uuid::from_slice(&item.terminal.as_ref().expect("a pane's item").id).unwrap()
}

#[tokio::test]
async fn a_members_own_item_comes_first_and_anothers_is_marked_as_theirs() {
    let h = start_as(Peer { client_id: Some("alice-phone".into()), scope: Scope::Control }).await;
    let (alices, bobs) = two_members(&h);
    alice_blocked_and_bob_asking(&h, alices, bobs).await;

    let items = needs_you(&h).await;
    assert_eq!(items.len(), 2, "{items:#?}");
    assert_eq!((terminal_of(&items[0]), items[0].kind()), (alices, NeedsYouKind::Blocked), "hers first: {items:#?}");
    let mine = items[0].concern.as_ref().expect("a member's item says whose");
    assert!(mine.yours);
    assert_eq!(mine.name, "Alice Chen-Montgomery");
    assert_eq!((terminal_of(&items[1]), items[1].kind()), (bobs, NeedsYouKind::Ask), "Bob's after, though an ask");
    let his = items[1].concern.as_ref().expect("another's item says whose");
    assert!(!his.yours, "Bob's is someone else's to Alice");
    assert_eq!((his.handle.as_str(), his.name.as_str()), ("bob", "Bob Okafor-Lindqvist"));
}

#[tokio::test]
async fn the_owner_reads_both_as_others_in_rank_order() {
    let h = start(Scope::Control).await;
    let (alices, bobs) = two_members(&h);
    alice_blocked_and_bob_asking(&h, alices, bobs).await;

    let items = needs_you(&h).await;
    assert_eq!(items.iter().map(terminal_of).collect::<Vec<_>>(), vec![bobs, alices], "rank order: {items:#?}");
    assert!(items.iter().all(|i| i.concern.as_ref().is_some_and(|c| !c.yours)), "{items:#?}");
}

#[tokio::test]
async fn a_solo_owners_items_carry_no_concern() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    h.watcher
        .observe_for_tests(
            pane,
            Observation { activity: AgentActivity::Blocked, state_since: now_millis(), command: "claude".into(), ..Observation::default() },
        )
        .await;
    let items = needs_you(&h).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].concern, None, "a runner with no members says nothing new");
}

/// M1 and L5 (ov-633 review): Bob's lane asks a decision and goes idle. Every
/// reader's list files it under Bob, and the push goes to Bob's relay account,
/// with the owner's task notice going quietly.
#[tokio::test]
async fn bobs_lanes_decision_is_his_on_every_list_and_is_pushed_to_him() {
    let h = start_as(Peer { client_id: Some("alice-phone".into()), scope: Scope::Control }).await;
    members(&h);
    let repo = a_repository(&h);
    let store = &h.service.store;
    let task = store.create_task(repo.workspace, "Pick a PDF library", Actor::User).unwrap();
    on_login(&h, a_pane(&h, repo.worktree, Some(task.id)), "bob");
    store.add_note(task.id, NoteKind::Question, Actor::Manager, "Which one?", serde_json::json!({})).unwrap();
    let mut relay = a_relay(&h).await;

    let mut moved = request("task.set_status");
    moved.payload = Some(payload::Payload::TaskSetStatus(pb::TaskSetStatus {
        task_id: bytes::Bytes::copy_from_slice(task.id.as_bytes()),
        status: pb::TaskStatus::NeedsDecision as i32,
        actor: "manager".into(),
    }));
    connect(&h).await.call(moved).await.expect("task.set_status");

    let items = needs_you(&h).await;
    let decision = items.iter().find(|i| i.kind() == NeedsYouKind::Decision).expect("the decision is listed");
    assert_eq!(decision.terminal, None, "an idle lane has no signal of its own");
    let concern = decision.concern.as_ref().expect("the decision is someone's");
    assert_eq!((concern.handle.as_str(), concern.yours), ("bob", false), "Bob's, to Alice");

    let mut sent = Vec::new();
    while let Ok(Some(r)) = tokio::time::timeout(std::time::Duration::from_secs(5), relay.recv()).await {
        sent.push(r);
        if sent.iter().any(|(l, _)| l.starts_with("POST /v1/notify ")) && sent.iter().any(|(l, _)| l.contains("/v1/notify/member")) {
            break;
        }
    }
    let member: Vec<_> = sent.iter().filter(|(l, _)| l.starts_with("POST /v1/notify/member ")).map(|(_, b)| b).collect();
    assert_eq!(member.len(), 1, "{sent:#?}");
    assert_eq!(member[0]["account"], "acct-bob", "{sent:#?}");
    assert_eq!((&member[0]["kind"], &member[0]["class"]), (&"needs_you".into(), &"decision".into()));
    let owners = sent.iter().find(|(l, b)| l.starts_with("POST /v1/notify ") && b["kind"] == "task").expect("the owner's thread moves");
    assert_eq!(owners.1["alert"], false, "the owner isn't buzzed about Bob's decision: {sent:#?}");
}
