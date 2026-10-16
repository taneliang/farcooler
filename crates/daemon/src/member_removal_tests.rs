//! Removing a member wipes their Claude login (ov-588): on the fixture's
//! real tmux server with the test stub in place of claude, and the recording
//! `Keychain` in place of `security`, so no test touches a real Keychain.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use farcooler_core::DomainError;
use farcooler_protocol::v1::{Request, Scope, request, response};
use farcooler_transport::Peer;

use super::*;
use crate::claude_logins::tests::{create, dir_of, factory, runner};

/// A login id as the store makes them.
const ID: &str = "0199f0a1b2c37d5e8f90a1b2c3d4e5f6";

/// claude's name for a directory's entry: the first 8 hex digits of the
/// sha256 of the string as passed (ov-582), checked against a hash worked
/// out by hand. The call deletes by exact service, whatever the account
/// (review 1, M2), and never asks for the secret.
#[test]
fn the_keychain_entry_is_named_as_claude_names_it() {
    let dir = format!("/Users/alice/.farcooler/claude/{ID}");
    assert_eq!(keychain_service(&dir), "Claude Code-credentials-eaeb298d");
    assert_eq!(keychain_service("/Users/alice/.claude"), "Claude Code-credentials-5ac5e874");
    assert_eq!(delete_args("Claude Code-credentials-eaeb298d"), ["delete-generic-password", "-s", "Claude Code-credentials-eaeb298d"]);
}

/// Only strings shaped like this login's directory are deleted for, each
/// once; never the owner's `CLAUDE_CONFIG_DIR`'s or another member's, and
/// never one another login's directory hashes to as well. (The owner's
/// unsuffixed entry can't be named at all: `keychain_service` always adds
/// the hash; review 1, L3.)
#[test]
fn only_this_logins_entries_are_targeted() {
    let mine = format!("/r/claude/{ID}");
    let other = "/r/claude/0199f0a1b2c37d5e8f90a1b2c3d4e5f7".to_string();
    let strings = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let candidates = strings(&[&mine, &mine, "/Users/o/.claude", &other, &format!("r/claude/{ID}"), &format!("/r/x/{ID}"), "", "/r/claude"]);
    assert_eq!(keychain_targets(ID, &candidates, &[]), [keychain_service(&mine)]);
    assert_eq!(keychain_targets(ID, &strings(&[&mine]), &strings(&["/Users/o/.claude", &mine])), Vec::<String>::new(), "someone else's too");
    assert_eq!(keychain_targets("not-an-id", &strings(&["/r/claude/not-an-id"]), &[]), Vec::<String>::new());
}

/// The directory is deleted by its exact name, and only when it and the
/// member root are real directories: a link to someone else's is refused and
/// what it points at is left, as is anything a link inside it points at.
#[test]
fn only_the_logins_own_directory_is_deleted() {
    let scratch = farcooler_core::scratch::dir("member-removal-dir");
    let runtime = scratch.join("runtime");
    let elsewhere = scratch.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join(".credentials.json"), "someone else's").unwrap();
    assert!(!delete_login_dir(&runtime, ID).unwrap(), "no root yet");
    assert_eq!(delete_login_dir(&runtime, "../elsewhere").unwrap_err().kind(), std::io::ErrorKind::InvalidInput);

    let root = member_root(&runtime);
    std::fs::create_dir_all(&root).unwrap();
    assert!(!delete_login_dir(&runtime, ID).unwrap(), "no directory yet");
    std::os::unix::fs::symlink(&elsewhere, root.join(ID)).unwrap();
    assert!(delete_login_dir(&runtime, ID).is_err(), "a link is refused");
    assert!(elsewhere.join(".credentials.json").exists());
    std::fs::remove_file(root.join(ID)).unwrap();

    let dir = root.join(ID);
    std::fs::create_dir_all(dir.join("projects")).unwrap();
    std::fs::write(dir.join(".credentials.json"), "theirs").unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.join("projects").join("link")).unwrap();
    assert!(delete_login_dir(&runtime, ID).unwrap());
    assert!(!dir.exists());
    assert!(elsewhere.join(".credentials.json").exists(), "a link inside isn't followed");

    std::fs::remove_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &root).unwrap();
    std::fs::create_dir_all(elsewhere.join(ID)).unwrap();
    assert!(delete_login_dir(&runtime, ID).is_err(), "a root that's a link is refused");
    assert!(elsewhere.join(ID).exists());
    let _ = std::fs::remove_dir_all(&scratch);
}

/// `member.remove` as a runner admin: the answer's `removed`.
async fn remove(svc: &Arc<Service>, handle: &str) -> pb::MemberRemoval {
    try_remove(svc, handle).await.unwrap_or_else(|e| panic!("not removed: {e}"))
}

/// `member.remove`, or the refusal's word.
async fn try_remove(svc: &Arc<Service>, handle: &str) -> std::result::Result<pb::MemberRemoval, String> {
    use farcooler_transport::Handler as _;
    let peer = Peer { client_id: None, scope: Scope::HostAdmin };
    let admin = crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer);
    let req = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: handle.into() })),
        ..Default::default()
    };
    match admin.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => match r.value {
            Some(pb::result::Value::MemberList(list)) => Ok(list.removed.expect("a removal says what it did")),
            other => panic!("not a member list: {other:?}"),
        },
        Some(response::Outcome::Error(e)) => Err(farcooler_core::error::word_for(e.code).to_string()),
        other => panic!("no outcome: {other:?}"),
    }
}

fn intent(svc: &Service, id: Uuid) -> TerminalIntent {
    svc.store.get_terminal(id).unwrap().intent
}

fn deleted(svc: &Service) -> Vec<String> {
    std::mem::take(&mut *svc.claude_logins.keychain.deleted.lock().unwrap())
}

/// Removing Alice stops every pane on her login (her rep and a pane she
/// opened from her phone), deletes her Keychain entry by its exact name and
/// her directory, and says so. Bob's and the owner's panes keep running,
/// Bob's directory stays, and nothing of the owner's is deleted. Her pane
/// can't be relaunched.
#[tokio::test]
async fn removing_a_member_stops_their_panes_and_wipes_their_login() {
    let (_dir, svc, main, checkout) = runner().await;
    let rep = svc.start_rep(main, "alice", "").await.expect("Alice's rep").terminal.id;
    let hers = create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    let bobs = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    let owners = create(&factory(&svc, None, None), checkout, "claude").await.expect("the owner's pane");
    let alice = dir_of(&svc, "alice");
    let bob = dir_of(&svc, "bob");
    std::fs::write(Path::new(&alice).join(".credentials.json"), "{}").unwrap();
    deleted(&svc);
    // What her entry's name must not be shared with: the owner's own
    // directory and Bob's, never hers.
    let runner = svc.claude_logins.runner.clone().unwrap().into_os_string().into_string().unwrap();
    let id = svc.store.member_login("alice").unwrap().unwrap();
    let theirs = svc.other_login_dirs(&id, &svc.store.terminal_logins().unwrap());
    assert!(theirs.contains(&runner) && theirs.contains(&bob) && !theirs.contains(&alice), "{theirs:?}");

    // One entry per account claude used, whatever the daemon's `$USER`.
    let service = keychain_service(&alice);
    svc.claude_logins.keychain.entries.lock().unwrap().insert(service.clone(), 2);

    let removed = remove(&svc, "alice").await;
    assert_eq!(removed, pb::MemberRemoval { panes_stopped: 2, had_login: true, failed: vec![] });
    assert!(svc.store.get_terminal(rep).is_err(), "her rep is gone");
    assert_eq!(intent(&svc, hers), TerminalIntent::Stopped, "her pane stopped");
    assert_eq!((intent(&svc, bobs), intent(&svc, owners)), (TerminalIntent::Running, TerminalIntent::Running));
    assert!(!Path::new(&alice).exists(), "her directory is gone");
    assert!(Path::new(&bob).is_dir(), "Bob's stays");

    let calls = deleted(&svc);
    assert_eq!(calls, [service.clone(), service.clone(), service.clone()], "both her entries, until none is left, and nothing else");
    assert_eq!(svc.claude_logins.keychain.entries.lock().unwrap()[&service], 0);
    assert!(calls.iter().all(|s| s != "Claude Code-credentials" && *s != keychain_service(&bob)));
    assert!(calls.iter().all(|s| *s != keychain_service(&runner)), "never the owner's");

    let refused = svc.restart_terminal(hers).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "login_gone" })), "{refused:?}");
    assert!(!Path::new(&alice).exists(), "the refused relaunch made nothing");
}

/// A Keychain entry that can't be deleted is said, by step; the directory is
/// still deleted and the member still removed, and nothing relaunches on the
/// login. A member who never had a login has nothing to wipe and touches no
/// Keychain.
#[tokio::test]
async fn a_step_that_fails_is_said_and_the_member_is_still_removed() {
    let (_dir, svc, _main, checkout) = runner().await;
    let hers = create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    let alice = dir_of(&svc, "alice");
    svc.claude_logins.keychain.failing.store(true, Ordering::SeqCst);
    deleted(&svc);

    let removed = remove(&svc, "alice").await;
    assert_eq!(removed, pb::MemberRemoval { panes_stopped: 1, had_login: true, failed: vec!["keychain".into()] });
    assert!(svc.store.member("alice").is_err(), "removed anyway");
    assert!(!Path::new(&alice).exists());
    assert!(matches!(svc.restart_terminal(hers).await, Err(DomainError::Conflict { what: "login_gone" })));

    let removed = remove(&svc, "bob").await;
    assert_eq!(removed, pb::MemberRemoval { panes_stopped: 0, had_login: false, failed: vec![] });
    assert_eq!(deleted(&svc).len(), 1, "only Alice's");
}

/// A service that keeps having entries is deleted at most `MAX_ENTRIES`
/// times, then reported (review 1, M2).
#[tokio::test]
async fn a_keychain_that_never_runs_out_is_reported() {
    let (_dir, svc, _main, checkout) = runner().await;
    create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    let service = keychain_service(&dir_of(&svc, "alice"));
    svc.claude_logins.keychain.entries.lock().unwrap().insert(service.clone(), 100);
    deleted(&svc);
    let removed = remove(&svc, "alice").await;
    assert_eq!(removed.failed, ["keychain"]);
    assert_eq!(deleted(&svc), vec![service; MAX_ENTRIES]);
}

/// A pane on her login recorded after the removal first looked (a launch
/// that passed its member check before the row went) is stopped by the next
/// round, counted, and her login wiped again after it (review 1, M1).
#[tokio::test]
async fn a_pane_that_raced_the_removal_is_stopped_and_counted() {
    let (_dir, svc, _main, checkout) = runner().await;
    let hers = create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    let recorded = svc.store.terminal_login(hers).unwrap().unwrap();
    let raced = Arc::new(std::sync::Mutex::new(None));
    let (store_svc, slot) = (svc.clone(), raced.clone());
    *svc.claude_logins.keychain.at_first_delete.lock().unwrap() = Some(Box::new(move || {
        let t = store_svc.store.create_terminal(checkout, "Agent", "claude", TerminalIntent::Running, 120, 40).unwrap();
        store_svc.store.set_terminal_login(t.id, &recorded).unwrap();
        std::fs::create_dir_all(&recorded.config_dir).unwrap();
        *slot.lock().unwrap() = Some(t.id);
    }));
    let alice = dir_of(&svc, "alice");
    deleted(&svc);

    let removed = remove(&svc, "alice").await;
    let raced = raced.lock().unwrap().expect("the hook ran");
    assert_eq!(intent(&svc, raced), TerminalIntent::Stopped, "the raced pane stopped");
    assert_eq!(removed, pb::MemberRemoval { panes_stopped: 2, had_login: true, failed: vec![] });
    assert_eq!(deleted(&svc).len(), 2, "wiped again after it");
    assert!(!Path::new(&alice).exists(), "the directory it made is gone too");
}

/// A member root swapped for a link refuses the Keychain step, so no
/// entry is deleted for a directory that's somewhere else (review 1, L2).
#[tokio::test]
async fn a_member_root_that_is_a_link_deletes_no_keychain_entry() {
    let (_dir, svc, _main, checkout) = runner().await;
    create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    let root = member_root(svc.root_dir());
    let moved = svc.root_dir().join("claude-moved");
    std::fs::rename(&root, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &root).unwrap();
    deleted(&svc);
    let removed = remove(&svc, "alice").await;
    assert_eq!(removed.failed, ["keychain", "login_dir"]);
    assert_eq!(deleted(&svc), Vec::<String>::new());
}

/// Run again for someone already removed, the removal wipes what's left of
/// their login: a pane still running on it, its entry and its directory
/// (review 1, L1). For a handle with nothing left, it's refused as before.
#[tokio::test]
async fn removing_again_finishes_the_wipe() {
    let (_dir, svc, _main, checkout) = runner().await;
    let hers = create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    let recorded = svc.store.terminal_login(hers).unwrap().unwrap();
    remove(&svc, "alice").await;
    // A pane that couldn't be stopped the first time, and what it wrote.
    let left = svc.store.create_terminal(checkout, "Agent", "claude", TerminalIntent::Running, 120, 40).unwrap().id;
    svc.store.set_terminal_login(left, &recorded).unwrap();
    std::fs::create_dir_all(&recorded.config_dir).unwrap();
    deleted(&svc);

    let removed = remove(&svc, "alice").await;
    assert_eq!(removed, pb::MemberRemoval { panes_stopped: 1, had_login: true, failed: vec![] });
    assert_eq!(intent(&svc, left), TerminalIntent::Stopped);
    assert!(!Path::new(&recorded.config_dir).exists());
    assert_eq!(deleted(&svc), [keychain_service(&recorded.config_dir)]);
    assert_eq!(try_remove(&svc, "nobody").await, Err("not-found".to_string()));
}

/// A relaunch that passed its login check, then waited on the version check
/// while the member was removed, is refused once the wait is over (review 2,
/// M1 leftover): the login is wiped, and the pane would re-make it.
#[tokio::test]
async fn a_member_removed_during_the_version_check_is_refused() {
    let (_dir, svc, _main, checkout) = runner().await;
    let hers = create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    svc.stop_terminal(hers).await.unwrap();
    let remover = svc.clone();
    *svc.claude_logins.at_version_check.lock().unwrap() = Some(Box::new(move || remover.store.remove_member("alice").unwrap()));
    let term = svc.store.get_terminal(hers).unwrap();
    let cwd = svc.store.get_worktree(checkout).unwrap().worktree_path;
    let refused = svc.relaunch_dir(&term, Path::new(&cwd)).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "login_gone" })), "{refused:?}");
}

/// A member removed after the launch's login check, just before its pane is
/// spawned, is refused there: a relaunch and a new pane alike, and the new
/// one is recorded as never started (review 2).
#[tokio::test]
async fn a_member_removed_just_before_the_spawn_is_refused() {
    let (_dir, svc, _main, checkout) = runner().await;
    let hers = create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    svc.stop_terminal(hers).await.unwrap();
    let remover = svc.clone();
    *svc.claude_logins.before_spawn.lock().unwrap() = Some(Box::new(move || remover.store.remove_member("alice").unwrap()));
    let refused = svc.restart_terminal(hers).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "login_gone" })), "{refused:?}");

    let bob = svc.store.member_login("bob").unwrap();
    assert_eq!(bob, None, "Bob has no pane yet");
    let remover = svc.clone();
    *svc.claude_logins.before_spawn.lock().unwrap() = Some(Box::new(move || remover.store.remove_member("bob").unwrap()));
    let refused = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await;
    assert_eq!(refused, Err("resource-conflict:login_gone".to_string()));
    let bobs = svc.store.terminal_logins().unwrap().into_iter().find(|(_, l)| l.handle == "bob").expect("recorded").0;
    assert_eq!(intent(&svc, bobs), TerminalIntent::Failed);
}

/// Removing again never wipes a login some member holds now, even through a
/// pane row that names the removed handle (review 2, Low 2).
#[tokio::test]
async fn removing_again_never_reaches_a_current_members_login() {
    let (_dir, svc, _main, checkout) = runner().await;
    let bobs = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    let bob = svc.store.terminal_login(bobs).unwrap().unwrap();
    remove(&svc, "alice").await;
    let stray = svc.store.create_terminal(checkout, "Agent", "claude", TerminalIntent::Running, 120, 40).unwrap().id;
    svc.store.set_terminal_login(stray, &farcooler_store::claude_logins::TerminalLogin { handle: "alice".into(), ..bob.clone() }).unwrap();
    deleted(&svc);
    assert_eq!(try_remove(&svc, "alice").await, Err("not-found".to_string()));
    assert!(Path::new(&bob.config_dir).is_dir(), "Bob's login stays");
    assert_eq!(intent(&svc, bobs), TerminalIntent::Running);
    assert_eq!(deleted(&svc), Vec::<String>::new());
}
