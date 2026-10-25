//! `caller_person` (ov-747): the one rule for whose a caller is, with the
//! walk from a pid to its pane given, so each case names the pane outright.

use std::sync::OnceLock;

use farcooler_protocol::v1::Scope;
use farcooler_store::claude_logins::TerminalLogin;

use super::*;

const LOCAL: Peer = Peer { client_id: None, scope: Scope::Control };

fn phone(id: &str) -> Peer {
    Peer { client_id: Some(id.into()), scope: Scope::Control }
}

fn no_walk(_: i32) -> Option<Uuid> {
    panic!("walked for a pane on a runner where no pane is anyone else's")
}

/// A runner's Main checkout, and a fresh pane in it.
async fn a_runner() -> (crate::test_support::ScratchDir, std::sync::Arc<Service>, impl Fn() -> Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap().id;
    let s = svc.clone();
    (dir, svc, move || s.store.create_terminal_for_test(checkout, workspace))
}

/// Launch `pane` on `handle`'s login, as `claude_logins` records it.
fn on_login(svc: &Service, pane: Uuid, handle: &str) {
    let login_id = svc.store.ensure_member_login(handle).unwrap();
    let login = TerminalLogin { handle: handle.into(), login_id, config_dir: format!("/r/claude/{handle}"), shared: false };
    svc.store.set_terminal_login(pane, &login).unwrap();
}

/// Whose a local caller in `pane` is (`None`: in no pane).
fn in_pane(svc: &Service, pane: Option<Uuid>) -> Person {
    caller_person_with(svc, &LOCAL, Some(4242), &OnceLock::new(), |_| pane).unwrap()
}

/// ov-740/743/744's solo-runner guard, kept: a runner where no pane can be
/// anyone else's (no rep, no pane on a member's login) walks no process
/// table and looks nobody up, members or none, a member's login or none. A
/// remote device is its person, unwalked.
#[tokio::test]
async fn a_runner_where_every_pane_is_the_owners_walks_nobody() {
    let (_dir, svc, _) = a_runner().await;
    let walked = OnceLock::new();
    assert_eq!(caller_person_with(&svc, &LOCAL, Some(4242), &walked, no_walk).unwrap(), Person::Owner);
    assert_eq!(caller_person_with(&svc, &LOCAL, None, &walked, no_walk).unwrap(), Person::Owner, "the in-process stdio session");
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.ensure_member_login("bob").unwrap();
    assert_eq!(caller_person_with(&svc, &LOCAL, Some(4242), &walked, no_walk).unwrap(), Person::Owner, "a member with a login, but no pane on it");
    assert_eq!(caller_person_with(&svc, &phone("phone-b"), Some(4242), &walked, no_walk).unwrap(), Person::Member("bob".into()));
    assert_eq!(walked.get(), None, "nothing kept, since nothing walked");
}

/// The rule: a rep's pane is its member's, on the runner's own login too; a
/// pane on a member's login is theirs; one on a removed member's login is
/// nobody's; the runner's own login's, no pane, or a pane gone since, the
/// owner's. And a remote device is its device's person even in a pane.
#[tokio::test]
async fn a_local_caller_is_whose_its_pane_is() {
    let (_dir, svc, pane) = a_runner().await;
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    let (rep, bobs, owners, carols) = (pane(), pane(), pane(), pane());
    let workspace = svc.store.get_terminal(rep).unwrap().workspace_id.unwrap();
    svc.store.bind_rep(rep, workspace, "bob").unwrap();
    on_login(&svc, bobs, "bob");
    on_login(&svc, carols, "carol");

    assert_eq!(in_pane(&svc, Some(rep)), Person::Member("bob".into()), "a rep's pane, on the runner's login");
    assert_eq!(in_pane(&svc, Some(bobs)), Person::Member("bob".into()), "Bob's own pane");
    assert_eq!(in_pane(&svc, Some(owners)), Person::Owner, "a pane on the runner's login");
    assert_eq!(in_pane(&svc, None), Person::Owner, "in no pane: the Mac app, the owner's terminal");
    assert_eq!(in_pane(&svc, Some(Uuid::now_v7())), Person::Owner, "a pane gone since the walk");
    let walked = OnceLock::new();
    let from_phone = caller_person_with(&svc, &phone("phone-b"), Some(4242), &walked, |_| panic!("a device isn't walked"));
    assert_eq!(from_phone.unwrap(), Person::Member("bob".into()));

    svc.store.remove_member("carol").unwrap();
    assert_eq!(in_pane(&svc, Some(carols)), Person::Released, "a removed member's pane is nobody's");
}

/// Review 743 finding 1, kept: a connection walks for its caller's pane
/// once, by its first call that asks; every later call reuses the pane, and
/// reads afresh whose it is.
#[tokio::test]
async fn a_connection_walks_for_its_pane_once() {
    let (_dir, svc, pane) = a_runner().await;
    let rep = pane();
    let workspace = svc.store.get_terminal(rep).unwrap().workspace_id.unwrap();
    svc.store.set_member("bob", "Bob", &[]).unwrap();
    svc.store.bind_rep(rep, workspace, "bob").unwrap();
    let walks = std::sync::atomic::AtomicUsize::new(0);
    let walk = |_: i32| {
        walks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(rep)
    };
    let walked = OnceLock::new();
    for _ in 0..3 {
        assert_eq!(caller_person_with(&svc, &LOCAL, Some(4242), &walked, walk).unwrap(), Person::Member("bob".into()));
    }
    assert_eq!(walks.load(std::sync::atomic::Ordering::SeqCst), 1, "one walk for the connection");
    svc.store.remove_member("bob").unwrap();
    assert_eq!(caller_person_with(&svc, &LOCAL, Some(4242), &walked, walk).unwrap(), Person::Owner, "its rep gone with Bob: at once");
}
