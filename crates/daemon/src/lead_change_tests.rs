//! A lead change starts the orchestrator afresh on the new lead's login
//! (ov-585), on the fixture's real tmux server with the test stub in place
//! of claude.

use std::path::PathBuf;
use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_transport::Peer;

use crate::claude_logins::tests::{assert_on, assert_runners, create, dir_of, factory, launched, pid_of, runner};
use crate::claude_logins::{MIN_MEMBER_CLAUDE, Version};
use crate::people::{Person, talks_to_orchestrator};
use crate::service::Service;
use crate::workspace_ops::set_settings;

use super::*;

/// Hand `workspace`'s lead to `handle` ("owner" for the owner) the way a
/// runner admin does: the answer, or the refusal.
async fn hand_lead(svc: &Arc<Service>, workspace: Uuid, handle: &str) -> Result<pb::Workspace> {
    let watcher = crate::watch::Watcher::new(svc.clone());
    let req = pb::WorkspaceSetSettings { lead: Some(handle.into()), ..Default::default() };
    set_settings(svc, &watcher, workspace, &req, Scope::HostAdmin, crate::workspace_ops::LOCAL).await
}

/// `workspace`'s live orchestrator.
async fn live(svc: &Service, workspace: Uuid) -> Option<models::Terminal> {
    svc.orchestrator_seat(workspace).await.unwrap().0
}

/// The charter `terminal`'s launch hands it in `FARCOOLER_CHARTER`, read
/// from where that names.
async fn charter_read(svc: &Service, terminal: Uuid) -> String {
    let command = launched(svc, terminal).await;
    let word = format!("{}=", farcooler_core::pane_env::CHARTER);
    let at = command.find(&word).expect("a charter in the launch") + word.len();
    let path: String = command[at..].trim_start_matches(['\'', '\\']).chars().take_while(|c| !matches!(c, '\'' | '\\' | ' ')).collect();
    std::fs::read_to_string(path).unwrap()
}

/// The memory directory `workspace`'s orchestrator settings name.
fn memory_of(svc: &Service, workspace: Uuid) -> PathBuf {
    let settings = svc.root_dir().join(format!("orchestrator-{workspace}.json"));
    let settings: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
    PathBuf::from(settings["autoMemoryDirectory"].as_str().expect("a memory directory"))
}

/// Handing the lead on stops the orchestrator and starts a new one on the
/// new lead's login, each time: owner to Bob, Bob to Alice, and back to the
/// owner, whose launch sets no directory. The charter it reads is the same
/// file. Its memory (review 1, M4): the owner-led one reads the owner's
/// repository memory; a member-led one reads the workspace's own, which
/// starts empty, never shows the owner's notes, and carries from Bob to
/// Alice. The owner still talks to it (R-54).
#[tokio::test]
async fn a_lead_change_starts_the_orchestrator_afresh_on_the_new_leads_login() {
    let (_dir, svc, main, _checkout) = runner().await;
    let first = svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    assert_runners(&launched(&svc, first.id).await);
    std::fs::write(crate::workspace_home::charter_path(svc.root_dir(), main), "Ship small.").unwrap();
    assert_eq!(charter_read(&svc, first.id).await, "Ship small.");
    let owners_memory = memory_of(&svc, main);
    assert!(owners_memory.starts_with(svc.agent_home().unwrap()), "the owner's own: {}", owners_memory.display());
    std::fs::create_dir_all(&owners_memory).unwrap();
    std::fs::write(owners_memory.join("owner.md"), "the owner's notes").unwrap();

    hand_lead(&svc, main, "bob").await.expect("Bob leads");
    let bobs = live(&svc, main).await.expect("an orchestrator");
    assert_ne!(bobs.id, first.id, "a new conversation");
    assert!(svc.store.get_terminal(first.id).is_err(), "the old one is gone");
    assert_on(&launched(&svc, bobs.id).await, &dir_of(&svc, "bob"));
    assert_eq!(charter_read(&svc, bobs.id).await, "Ship small.", "the same charter");
    let memory = memory_of(&svc, main);
    assert_eq!(memory, workspace_memory(svc.root_dir(), main), "the workspace's own memory");
    assert_eq!(std::fs::read_dir(&memory).unwrap().count(), 0, "empty the first time: none of the owner's notes");
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(std::fs::metadata(&memory).unwrap().permissions().mode() & 0o777, 0o700);
    std::fs::write(memory.join("board.md"), "Bob's notes").unwrap();
    assert!(talks_to_orchestrator(&svc.store, &Person::Owner, main).unwrap(), "R-54: the owner still gets through");

    hand_lead(&svc, main, "alice").await.expect("Alice leads");
    let alices = live(&svc, main).await.expect("an orchestrator");
    assert_ne!(alices.id, bobs.id);
    assert_on(&launched(&svc, alices.id).await, &dir_of(&svc, "alice"));
    assert_eq!(charter_read(&svc, alices.id).await, "Ship small.");
    assert_eq!(std::fs::read_to_string(memory_of(&svc, main).join("board.md")).unwrap(), "Bob's notes", "carried to Alice");
    assert!(!memory_of(&svc, main).join("owner.md").exists());

    hand_lead(&svc, main, "alice").await.expect("the same lead again");
    assert_eq!(live(&svc, main).await.map(|t| t.id), Some(alices.id), "no change, no restart");

    hand_lead(&svc, main, "owner").await.expect("the owner takes it back");
    let owners = live(&svc, main).await.expect("an orchestrator");
    assert_ne!(owners.id, alices.id);
    assert_runners(&launched(&svc, owners.id).await);
    assert_eq!(svc.store.terminal_login(owners.id).unwrap(), None);
    assert_eq!(memory_of(&svc, main), owners_memory, "the owner's own again");
}

/// Lanes the fresh orchestrator opens are on its new lead's login.
#[tokio::test]
async fn the_fresh_orchestrators_lanes_are_on_the_new_leads_login() {
    let (_dir, svc, main, checkout) = runner().await;
    svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    hand_lead(&svc, main, "bob").await.expect("Bob leads");
    let bobs = live(&svc, main).await.expect("an orchestrator");
    let in_orchestrator = factory(&svc, None, Some(pid_of(&svc, bobs.id).await));
    let lane = create(&in_orchestrator, checkout, "claude").await.expect("a lane");
    assert_on(&launched(&svc, lane).await, &dir_of(&svc, "bob"));
    assert_eq!(svc.store.terminal_login(lane).unwrap().map(|l| l.handle), Some("bob".to_string()));
}

/// A restart of an orchestrator whose lead changed under it starts afresh
/// on the lead's login instead of being refused: one that's down is
/// replaced and its record goes; one that's up is replaced too. On the
/// lead's own login, a restart is a plain restart of the same pane.
#[tokio::test]
async fn a_restart_on_an_old_leads_login_starts_afresh() {
    let (_dir, svc, main, _checkout) = runner().await;
    let first = svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    svc.stop_terminal(first.id).await.unwrap();
    let fresh = svc.restart_terminal(first.id).await.expect("started afresh");
    assert_ne!(fresh.id, first.id);
    assert!(svc.store.get_terminal(first.id).is_err(), "the old record is gone");
    assert_on(&launched(&svc, fresh.id).await, &dir_of(&svc, "bob"));
    assert_eq!(live(&svc, main).await.map(|t| t.id), Some(fresh.id));

    svc.stop_terminal(fresh.id).await.unwrap();
    let again = svc.restart_terminal(fresh.id).await.expect("restarted");
    assert_eq!(again.id, fresh.id, "on its lead's own login, the same pane");
    assert_on(&launched(&svc, fresh.id).await, &dir_of(&svc, "bob"));

    svc.store.set_workspace_lead(main, Some("alice")).unwrap();
    let running = svc.restart_terminal(fresh.id).await.expect("a running one, started afresh");
    assert_ne!(running.id, fresh.id);
    assert_on(&launched(&svc, running.id).await, &dir_of(&svc, "alice"));
    let terminals = svc.store.unended_orchestrators(main).unwrap();
    assert_eq!(terminals.iter().map(|t| t.id).collect::<Vec<_>>(), vec![running.id], "one orchestrator");
}

/// Removing the member who leads a workspace hands the lead to the owner,
/// and the orchestrator starts afresh on the owner's login: nothing stays on
/// the removed member's.
#[tokio::test]
async fn removing_a_lead_starts_the_orchestrator_on_the_owners_login() {
    use farcooler_transport::Handler as _;
    let (_dir, svc, main, _checkout) = runner().await;
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    svc.store.set_workspace_lead(main, Some("carol")).unwrap();
    let carols = svc.start_orchestrator(main, "claude", false, None).await.expect("Carol leads");
    assert_on(&launched(&svc, carols.id).await, &dir_of(&svc, "carol"));

    let peer = Peer { client_id: None, scope: Scope::HostAdmin };
    let admin = crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer);
    let remove = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: "carol".into() })),
        ..Default::default()
    };
    assert!(matches!(admin.handle(remove).await.outcome, Some(response::Outcome::Result(_))), "removed");
    let owners = live(&svc, main).await.expect("an orchestrator");
    assert_ne!(owners.id, carols.id);
    assert!(svc.store.get_terminal(carols.id).is_err());
    assert_runners(&launched(&svc, owners.id).await);
}

/// A lead whose login can't run a fresh orchestrator (a claude too old to
/// key logins by directory) is refused before anything is written: the lead
/// and the orchestrator stay as they were.
#[tokio::test]
async fn a_lead_whose_claude_is_too_old_is_refused_and_nothing_changes() {
    let (_dir, svc, main, _checkout) = runner().await;
    let first = svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    *svc.claude_logins.claimed.lock().unwrap() = Some(Some(Version(1, 0, 7)));
    let refused = hand_lead(&svc, main, "bob").await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "claude_too_old" })), "{refused:?}");
    assert_eq!(svc.store.workspace_lead(main).unwrap(), None, "the owner still leads");
    assert_eq!(live(&svc, main).await.map(|t| t.id), Some(first.id), "the same orchestrator, still up");

    // A replace on a member lead checks the new login before stopping the
    // live one too.
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let replaced = svc.start_orchestrator(main, "claude", true, None).await;
    assert!(matches!(replaced, Err(DomainError::Conflict { what: "claude_too_old" })), "{replaced:?}");
    assert_eq!(live(&svc, main).await.map(|t| t.id), Some(first.id), "not stopped for a start that can't happen");

    *svc.claude_logins.claimed.lock().unwrap() = Some(Some(MIN_MEMBER_CLAUDE));
    hand_lead(&svc, main, "bob").await.expect("Bob leads");
    assert_on(&launched(&svc, live(&svc, main).await.unwrap().id).await, &dir_of(&svc, "bob"));
}

/// The claude version is asked in the orchestrator's main checkout, its
/// worktree, on its start and on a relaunch; and in a lane's own worktree on
/// a lane's relaunch (ov-583 review: untested).
#[tokio::test]
async fn the_version_is_asked_where_the_pane_runs_on_start_and_relaunch() {
    let (_dir, svc, main, checkout) = runner().await;
    let main_checkout = PathBuf::from(svc.store.get_worktree(checkout).unwrap().worktree_path);
    let asked = |svc: &Service| std::mem::take(&mut *svc.claude_logins.asked_in.lock().unwrap());
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    asked(&svc);
    let orchestrator = svc.start_orchestrator(main, "claude", false, None).await.expect("Bob leads");
    assert_eq!(asked(&svc), vec![main_checkout.clone()], "orchestrator start");
    svc.stop_terminal(orchestrator.id).await.unwrap();
    svc.restart_terminal(orchestrator.id).await.expect("restarted");
    assert_eq!(asked(&svc), vec![main_checkout.clone()], "orchestrator relaunch");

    let repo = svc.store.get_worktree(checkout).unwrap().repository_id;
    let lane_tree = svc.create_worktree(repo, "lane", "lane", "HEAD").await.expect("a lane's worktree");
    let lane = create(&factory(&svc, Some("phone-b"), None), lane_tree.id, "claude").await.expect("Bob's lane");
    asked(&svc);
    svc.stop_terminal(lane).await.unwrap();
    svc.restart_terminal(lane).await.expect("restarted");
    assert_eq!(asked(&svc), vec![PathBuf::from(&lane_tree.worktree_path)], "lane relaunch");
}

/// A lead change while a start is in flight (review 1, M1): the start has
/// decided its login (the owner's) and isn't seated yet. The lead change
/// waits for it, then replaces it, so the orchestrator ends on Bob's login.
#[tokio::test]
async fn a_lead_change_waits_for_a_start_in_flight() {
    let (_dir, svc, main, _checkout) = runner().await;
    let pause = svc.orchestrator_locks.pause_at(main);
    let starting = tokio::spawn({
        let svc = svc.clone();
        async move { svc.start_orchestrator(main, "claude", false, None).await }
    });
    pause.0.notified().await;
    let mut handing = tokio::spawn({
        let svc = svc.clone();
        async move { hand_lead(&svc, main, "bob").await }
    });
    // Go on once the lead change is queued behind the start, or (without
    // the lock) has finished.
    let handed_early = tokio::select! {
        () = svc.orchestrator_locks.waiting.notified() => None,
        handed = &mut handing => Some(handed.unwrap()),
    };
    pause.1.notify_one();
    let _ = starting.await.unwrap();
    let handed = match handed_early {
        Some(handed) => handed,
        None => handing.await.unwrap(),
    };
    handed.expect("Bob leads");
    let now = live(&svc, main).await.expect("an orchestrator");
    assert_on(&launched(&svc, now.id).await, &dir_of(&svc, "bob"));
    assert_eq!(svc.store.unended_orchestrators(main).unwrap().len(), 1);
}

/// The backstop: a start that finds the lead changed once it's seated (a
/// lead written without the workspace's lock) is refused `lead_changed`,
/// and leaves nothing on the old login.
#[tokio::test]
async fn a_start_seated_after_the_lead_changed_is_refused() {
    let (_dir, svc, main, _checkout) = runner().await;
    let pause = svc.orchestrator_locks.pause_at(main);
    let starting = tokio::spawn({
        let svc = svc.clone();
        async move { svc.start_orchestrator(main, "claude", false, None).await }
    });
    pause.0.notified().await;
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    pause.1.notify_one();
    let refused = starting.await.unwrap();
    assert!(matches!(refused, Err(DomainError::Conflict { what: "lead_changed" })), "{refused:?}");
    assert!(svc.store.unended_orchestrators(main).unwrap().is_empty(), "nothing on the owner's login");
}

/// Whether `events` holds a layout for `worktree`.
fn layout_published(events: &mut tokio::sync::broadcast::Receiver<pb::Event>, worktree: Uuid) -> bool {
    let mut seen = false;
    loop {
        match events.try_recv() {
            Ok(event) => {
                if let Some(pb::event::Payload::LayoutChanged(l)) = event.payload {
                    seen |= l.worktree_id == crate::wire::id_bytes(worktree);
                }
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(_) => return seen,
        }
    }
}

/// `terminal.restart` that starts an orchestrator afresh, and
/// `member.remove` of a lead, each publish the new window's layout: the Mac
/// reads layouts only from these events (review 1, M2).
#[tokio::test]
async fn a_restart_and_a_leads_removal_publish_the_new_window() {
    use farcooler_transport::Handler as _;
    let (_dir, svc, main, checkout) = runner().await;
    let watcher = crate::watch::Watcher::new(svc.clone());
    let admin = crate::rpc::RpcFactory::new(svc.clone(), watcher.clone(), Arc::new(tokio::sync::Notify::new()), Peer { client_id: None, scope: Scope::HostAdmin });
    let first = svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    svc.store.set_workspace_lead(main, Some("carol")).unwrap();
    let mut events = watcher.subscribe();
    let restart = Request { method: "terminal.restart".into(), target_resource_id: Some(crate::wire::id_bytes(first.id)), ..Default::default() };
    assert!(matches!(admin.handle(restart).await.outcome, Some(response::Outcome::Result(_))), "restarted");
    assert!(layout_published(&mut events, checkout), "a restart's layout");
    assert_ne!(live(&svc, main).await.unwrap().id, first.id, "started afresh");

    let remove = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: "carol".into() })),
        ..Default::default()
    };
    assert!(matches!(admin.handle(remove).await.outcome, Some(response::Outcome::Result(_))), "removed");
    assert!(layout_published(&mut events, checkout), "a removal's layout");
}

/// Make `workspace`'s home a file, so an orchestrator start there fails
/// after the lead change has passed its checks.
fn break_home(svc: &Service, workspace: Uuid) {
    let home = crate::workspace_home::home(svc.root_dir(), workspace);
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::write(&home, "not a directory").unwrap();
}

/// A lead change whose orchestrator can't start afresh once the lead is
/// written answers with the workspace as written, and says why in
/// `orchestrator_not_restarted`, never as a refusal (review 1, M3). The old
/// orchestrator is stopped, not left on the old lead's login.
#[tokio::test]
async fn a_written_lead_whose_restart_fails_answers_success_and_says_so() {
    let (_dir, svc, main, _checkout) = runner().await;
    let first = svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    break_home(&svc, main);
    let answer = hand_lead(&svc, main, "bob").await.expect("the lead is written");
    assert_eq!(answer.lead.as_deref(), Some("bob"));
    assert_eq!(answer.orchestrator_not_restarted.as_deref(), Some("orchestrator_home"));
    assert_eq!(svc.store.workspace_lead(main).unwrap().as_deref(), Some("bob"));
    assert!(live(&svc, main).await.is_none(), "the old one is stopped");
    assert!(svc.store.get_terminal(first.id).is_ok(), "kept, to start afresh from");

    let fine = hand_lead(&svc, main, "bob").await.expect("again");
    assert_eq!(fine.orchestrator_not_restarted, None, "nothing live to restart");
}

#[derive(Clone, Default)]
struct Sink(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
    type Writer = Sink;
    fn make_writer(&'a self) -> Sink {
        self.clone()
    }
}

/// A removed lead's workspace whose orchestrator can't start afresh is
/// logged, by the error's word, and its old orchestrator stopped.
#[tokio::test]
async fn following_a_removed_lead_logs_a_failed_restart() {
    let (_dir, svc, main, _checkout) = runner().await;
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    svc.store.set_workspace_lead(main, Some("carol")).unwrap();
    let carols = svc.start_orchestrator(main, "claude", false, None).await.expect("Carol leads");
    let led = svc.led_by("carol").unwrap();
    assert_eq!(led, vec![main]);
    svc.store.remove_member("carol").unwrap();
    break_home(&svc, main);
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).finish();
    let started = {
        let _logging = tracing::subscriber::set_default(subscriber);
        svc.follow_leads(&led).await
    };
    assert!(started.is_empty());
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("couldn't start an orchestrator afresh") && logged.contains("orchestrator_home"), "{logged}");
    assert!(live(&svc, main).await.is_none(), "Carol's is stopped");
    assert!(svc.store.get_terminal(carols.id).is_ok());
}

/// A handle removed and given to someone new is another login (review 1,
/// L5): the old Bob's orchestrator, restarted once the new Bob leads,
/// starts afresh on the new Bob's directory rather than being refused.
#[tokio::test]
async fn a_new_holder_of_the_leads_handle_gets_a_fresh_orchestrator() {
    let (_dir, svc, main, _checkout) = runner().await;
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let old = svc.start_orchestrator(main, "claude", false, None).await.expect("Bob leads");
    let old_dir = dir_of(&svc, "bob");
    svc.stop_terminal(old.id).await.unwrap();
    svc.store.remove_member("bob").unwrap();
    svc.store.set_member("bob", "Another Bob", &["phone-b2".to_string()]).unwrap();
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let fresh = svc.restart_terminal(old.id).await.expect("started afresh");
    assert_ne!(fresh.id, old.id);
    let new_dir = dir_of(&svc, "bob");
    assert_ne!(new_dir, old_dir);
    assert_on(&launched(&svc, fresh.id).await, &new_dir);
}

/// `relaunch_dir` still refuses an orchestrator on an old lead's login,
/// for any caller that doesn't start it afresh first (review 1, L4).
#[tokio::test]
async fn relaunch_dir_refuses_an_old_leads_login() {
    let (_dir, svc, main, checkout) = runner().await;
    let first = svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let path = PathBuf::from(svc.store.get_worktree(checkout).unwrap().worktree_path);
    let refused = svc.relaunch_dir(&svc.store.get_terminal(first.id).unwrap(), &path).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "lead_changed" })), "{refused:?}");
}

/// A relaunch that starts an orchestrator afresh holds the workspace's
/// lock too: a lead change made while it's in flight waits, then replaces
/// it, so it ends on the newest lead's login.
#[tokio::test]
async fn a_lead_change_waits_for_a_relaunch_in_flight() {
    let (_dir, svc, main, _checkout) = runner().await;
    let first = svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    svc.stop_terminal(first.id).await.unwrap();
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let pause = svc.orchestrator_locks.pause_at(main);
    let restarting = tokio::spawn({
        let svc = svc.clone();
        async move { svc.restart_terminal(first.id).await }
    });
    pause.0.notified().await;
    let mut handing = tokio::spawn({
        let svc = svc.clone();
        async move { hand_lead(&svc, main, "alice").await }
    });
    let handed_early = tokio::select! {
        () = svc.orchestrator_locks.waiting.notified() => None,
        handed = &mut handing => Some(handed.unwrap()),
    };
    pause.1.notify_one();
    let _ = restarting.await.unwrap();
    let handed = match handed_early {
        Some(handed) => handed,
        None => handing.await.unwrap(),
    };
    handed.expect("Alice leads");
    let now = live(&svc, main).await.expect("an orchestrator");
    assert_on(&launched(&svc, now.id).await, &dir_of(&svc, "alice"));
}
