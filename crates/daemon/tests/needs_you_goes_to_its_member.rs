//! `needs_you.list` on a runner with members, over the real socket (ov-633):
//! a member reads their own item first and another member's after it, marked
//! as someone else's, though the other's is a more urgent kind.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_agent_core::event::{AgentEvent, PermissionOption};
use farcooler_daemon::needs_you::Observation;
use farcooler_protocol::v1::{AgentActivity, NeedsYouItem, NeedsYouKind, Scope, result};
use farcooler_store::claude_logins::TerminalLogin;
use farcooler_transport::{Peer, request};
use in_process::*;
use uuid::Uuid;

/// Alice and Bob, each with a device, and a pane launched on each one's login.
fn two_members(h: &Harness) -> (Uuid, Uuid) {
    let store = &h.service.store;
    store.set_member("alice", "Alice Chen-Montgomery", &["alice-phone".into()]).unwrap();
    store.set_member("bob", "Bob Okafor-Lindqvist", &["bob-phone".into()]).unwrap();
    let repo = a_repository(h);
    let pane_of = |handle: &str| {
        let pane = a_pane(h, repo.worktree, None);
        let login_id = store.ensure_member_login(handle).unwrap();
        let login = TerminalLogin { handle: handle.into(), login_id, config_dir: format!("/r/claude/{handle}"), shared: false };
        store.set_terminal_login(pane, &login).unwrap();
        pane
    };
    (pane_of("alice"), pane_of("bob"))
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
