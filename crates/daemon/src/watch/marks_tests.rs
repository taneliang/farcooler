//! Read, seen and watching are per person (ov-740), through the calls a
//! client makes: each test here goes red if a member's mark reaches the
//! owner's again, or the owner's a member's.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Event, Request, Scope, event, request, response, result};
use farcooler_store::claude_logins::TerminalLogin;
use farcooler_store::models::Actor;
use farcooler_store::people::Directory;
use farcooler_transport::{Handler as _, Peer};

use super::*;
use super::super::Tapped;
use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::wire::id_bytes;

const DONE: AgentActivity = AgentActivity::Done;
const IDLE: AgentActivity = AgentActivity::Idle;

/// A member's look ends their `Done` alone; the owner's look leaves a
/// member's `Done` standing until they look.
#[test]
fn a_look_ends_done_for_whoever_looked() {
    // The runner's Done since 100, which Bob opened at 150.
    assert_eq!(as_seen_by(DONE, Some(100), Some(150), None), (IDLE, Some(100)));
    // Opened before it began: still news to him.
    assert_eq!(as_seen_by(DONE, Some(100), Some(50), None), (DONE, Some(100)));
    assert_eq!(as_seen_by(DONE, Some(100), None, None), (DONE, Some(100)));
    // The owner's look at 300 left Idle: still that Done to Bob, unless he
    // opened it since it began.
    assert_eq!(as_seen_by(IDLE, Some(300), None, Some((100, 300))), (DONE, Some(100)));
    assert_eq!(as_seen_by(IDLE, Some(300), Some(120), Some((100, 300))), (IDLE, Some(300)));
    // The pane moved on since the owner's look: the agent, not anyone's look.
    assert_eq!(as_seen_by(IDLE, Some(400), None, Some((100, 300))), (IDLE, Some(400)));
    assert_eq!(as_seen_by(AgentActivity::Working, Some(300), None, Some((100, 300))), (AgentActivity::Working, Some(300)));
}

/// Only the owner's own screen silences the owner; a member's spares that
/// member; anyone's keeps automatic typing off a pane someone is at.
#[test]
fn whose_screen_silences_whom() {
    let (pane, now) = (Uuid::now_v7(), 1_000_000);
    let mut watched = HashMap::new();
    record_watch(&mut watched, "phone-b", Some("bob"), vec![pane], now);
    assert!(!owner_watching(&watched, pane, now), "Bob's screen isn't the owner's");
    assert!(member_watching(&watched, "bob", pane, now));
    assert!(!member_watching(&watched, "carol", pane, now));
    assert!(anyone_watching(&watched, pane, now), "someone is at the pane");
    record_watch(&mut watched, "-", None, vec![pane], now);
    assert!(owner_watching(&watched, pane, now));
    assert!(!member_watching(&watched, "carol", pane, now), "nor is the owner's Carol's");
}

/// A runner with the owner's Mac on the local socket, and Bob's and
/// Carol's phones, all on one watcher, with a pane in the checkout.
struct Team {
    _dir: crate::test_support::ScratchDir,
    svc: Arc<Service>,
    watcher: Arc<Watcher>,
    workspace: Uuid,
    pane: Uuid,
    owner: RpcFactory,
    bob: RpcFactory,
    carol: RpcFactory,
}

async fn team() -> Team {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap();
    let pane = svc.store.create_terminal_for_test(checkout.id, workspace);
    let directory = Directory { account: "acct-bob", email: "bob@example.com" };
    svc.store.set_member_in("bob", "Bob", &["phone-b".into()], directory).unwrap();
    svc.store.set_member("carol", "Carol", &["phone-c".into()]).unwrap();
    let watcher = Watcher::new(svc.clone());
    let on = |device: Option<&str>| {
        let peer = Peer { client_id: device.map(str::to_string), scope: Scope::Control };
        RpcFactory::new(svc.clone(), watcher.clone(), Arc::new(tokio::sync::Notify::new()), peer)
    };
    let (owner, bob, carol) = (on(None), on(Some("phone-b")), on(Some("phone-c")));
    Team { _dir: dir, svc, watcher, workspace, pane, owner, bob, carol }
}

async fn call(rpc: &RpcFactory, req: Request) -> Option<result::Value> {
    match rpc.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => r.value,
        other => panic!("refused: {other:?}"),
    }
}

fn mark(workspace: Uuid, opened: &[(Uuid, i64)], floor: Option<i64>) -> Request {
    Request {
        method: "workspace.mark_read".into(),
        payload: Some(request::Payload::WorkspaceMarkRead(pb::WorkspaceMarkRead {
            workspace_id: id_bytes(workspace),
            floor_ms: floor,
            opened: opened.iter().map(|(t, ms)| pb::TaskRead { task_id: id_bytes(*t), opened_ms: *ms }).collect(),
        })),
        ..Default::default()
    }
}

async fn reads(rpc: &RpcFactory, workspace: Uuid) -> pb::BoardReads {
    let list = Request {
        method: "task.list".into(),
        payload: Some(request::Payload::TaskList(pb::TaskListRequest { workspace_id: Some(id_bytes(workspace)), ..Default::default() })),
        ..Default::default()
    };
    match call(rpc, list).await {
        Some(result::Value::TaskList(l)) => l.reads.expect("a board's read state"),
        other => panic!("wrong result: {other:?}"),
    }
}

fn reads_event(of: pb::BoardReads) -> Event {
    Event { payload: Some(event::Payload::BoardReadsChanged(of)), ..Event::default() }
}

/// ov-740: Bob opening a card and marking his board read moves his Unread
/// alone: the owner's `task.list` and Carol's don't carry it, and his news
/// reaches only his devices. The owner's marks reach no member either.
#[tokio::test]
async fn a_members_read_marks_are_theirs() {
    let t = team().await;
    let task = t.svc.store.create_task(t.workspace, "Mac: a jump", Actor::User).unwrap().id;
    let before = reads(&t.owner, t.workspace).await;
    let at = crate::review::now_millis() - 1_000;

    let his = match call(&t.bob, mark(t.workspace, &[(task, at)], None)).await {
        Some(result::Value::BoardReads(r)) => r,
        other => panic!("wrong result: {other:?}"),
    };
    assert_eq!((his.member.as_str(), his.opened.len()), ("bob", 1));
    assert_eq!(reads(&t.owner, t.workspace).await, before, "the owner's Unread is untouched");
    assert!(reads(&t.carol, t.workspace).await.opened.is_empty(), "and Carol's");
    assert_eq!(reads(&t.bob, t.workspace).await.opened, his.opened, "his own devices read it");

    let news = reads_event(his.clone());
    assert!(t.bob.admits(&news), "his devices hear it");
    assert!(!t.owner.admits(&news), "the owner's don't");
    assert!(!t.carol.admits(&news), "nor Carol's");

    call(&t.owner, mark(t.workspace, &[], Some(at + 500))).await;
    let owners = reads(&t.owner, t.workspace).await;
    assert_eq!((owners.member.as_str(), owners.floor_ms), ("", at + 500));
    assert!(reads(&t.bob, t.workspace).await.floor_ms < at, "the owner's Mark All as Read isn't Bob's");
    assert!(!t.bob.admits(&reads_event(owners.clone())) && t.owner.admits(&reads_event(owners)));
}

async fn row(rpc: &RpcFactory, pane: Uuid) -> pb::Terminal {
    let list = Request { method: "terminal.list".into(), ..Default::default() };
    match call(rpc, list).await {
        Some(result::Value::TerminalList(l)) => l.items.into_iter().find(|r| r.id == id_bytes(pane)).expect("the pane"),
        other => panic!("wrong result: {other:?}"),
    }
}

fn seen(pane: Uuid) -> Request {
    Request { method: "terminal.seen".into(), target_resource_id: Some(id_bytes(pane)), ..Default::default() }
}

/// ov-740: Bob opening a finished pane ends `Done` for him alone, and the
/// owner opening it ends the owner's, not Carol's, in a list and in the
/// event the owner's look announces.
#[tokio::test]
async fn a_look_at_a_pane_is_its_persons() {
    let t = team().await;
    let finished = crate::review::now_millis() - 5_000;
    let done = crate::needs_you::Observation {
        activity: DONE,
        state_since: finished,
        blocked_question: None,
        turn_failed: false,
        command: "claude".into(),
        trust_prompt: false,
        trust_folder: None,
        not_started: None,
    };
    t.watcher.observe_for_tests(t.pane, done).await;

    call(&t.bob, seen(t.pane)).await;
    assert_eq!(row(&t.bob, t.pane).await.activity(), IDLE, "Bob has seen it");
    assert_eq!(row(&t.owner, t.pane).await.activity(), DONE, "the owner hasn't");
    assert_eq!(row(&t.carol, t.pane).await.activity(), DONE, "nor Carol");
    assert_eq!(t.watcher.activity(t.pane).await.0, DONE, "the runner's Done is the owner's");

    call(&t.owner, seen(t.pane)).await;
    let owners = row(&t.owner, t.pane).await;
    assert_eq!(owners.activity(), IDLE);
    assert_eq!(row(&t.carol, t.pane).await.activity(), DONE, "the owner's look isn't Carol's");
    assert_eq!(row(&t.bob, t.pane).await.activity(), IDLE);

    // The `terminal_changed` the owner's look announces, as each is told it.
    let event = || Event { payload: Some(event::Payload::TerminalChanged(owners.clone())), ..Event::default() };
    let told = |rpc: &RpcFactory| match rpc.shape(event()).payload {
        Some(event::Payload::TerminalChanged(r)) => (r.activity(), crate::wire::millis_of(&r.activity_changed_at)),
        other => panic!("{other:?}"),
    };
    assert_eq!(told(&t.owner).0, IDLE);
    assert_eq!(told(&t.carol), (DONE, Some(finished)), "Carol still reads the turn as finished");
    assert_eq!(told(&t.bob).0, IDLE);
}

fn watching(panes: &[Uuid]) -> Request {
    Request {
        method: "terminal.watching".into(),
        payload: Some(request::Payload::TerminalsWatched(pb::TerminalsWatched {
            terminal_ids: panes.iter().map(|p| id_bytes(*p)).collect(),
        })),
        ..Default::default()
    }
}

/// ov-740: Bob with a pane on screen doesn't silence the owner's push about
/// it; the owner's own screen does. And Bob, watching his own pane, isn't
/// pushed about it, while the owner's notice stays quiet as when he is.
#[tokio::test]
async fn a_members_screen_silences_their_push_and_not_the_owners() {
    let t = team().await;
    let login_id = t.svc.store.ensure_member_login("bob").unwrap();
    let login = TerminalLogin { handle: "bob".into(), login_id, config_dir: "/r/claude/bob".into(), shared: false };
    t.svc.store.set_terminal_login(t.pane, &login).unwrap();
    let mut taps = t.watcher.tap_notices();
    let member_pushes = |taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>| {
        std::iter::from_fn(|| taps.try_recv().ok()).filter(|tap| tap.kind == Some("member")).count()
    };

    let route = t.watcher.route_agent_notice(t.pane, "blocked", false, true);
    assert!(!route.settle(&t.watcher, None).await);
    assert_eq!(member_pushes(&mut taps), 1, "Bob, not watching, is pushed about his pane");

    call(&t.bob, watching(&[t.pane])).await;
    assert!(!t.watcher.is_watched(t.pane, now_millis()), "Bob's screen silences no push of the owner's");
    let route = t.watcher.route_agent_notice(t.pane, "blocked", false, true);
    assert!(!route.settle(&t.watcher, None).await, "the owner's notice stays quiet");
    assert_eq!(member_pushes(&mut taps), 0, "Bob, watching, isn't pushed");

    call(&t.owner, watching(&[t.pane])).await;
    assert!(t.watcher.is_watched(t.pane, now_millis()), "the owner's own screen does");
}

async fn inbox(rpc: &RpcFactory, worktree: Uuid) -> Option<bool> {
    match call(rpc, Request { method: "changes.inbox".into(), ..Default::default() }).await {
        Some(result::Value::ChangesInbox(i)) => {
            i.items.into_iter().find(|w| w.worktree_id == id_bytes(worktree)).map(|w| w.changed_since_reviewed)
        }
        other => panic!("wrong result: {other:?}"),
    }
}

/// ov-740: Bob reviewing a worktree clears "changed since you looked" for
/// him alone: the owner's inbox, and the review count the owner's lock
/// screen carries, still have it, and so does Carol's.
#[tokio::test]
async fn a_members_review_is_theirs() {
    let t = team().await;
    let repo = t.svc.store.get_workspace(t.workspace).unwrap().repository_id;
    let wt = t.svc.create_worktree(repo, "wt-one", "feature/one", "main").await.expect("a worktree");
    t.svc.store.set_review_base(wt.id, "main").unwrap();
    let diff = || t.svc.review_cache.set_counts_for_tests(wt.id, std::path::Path::new(&wt.worktree_path), crate::review::Counts::Known(1, 4, 0));
    diff();
    assert_eq!(inbox(&t.owner, wt.id).await, Some(true));
    assert_eq!(inbox(&t.bob, wt.id).await, Some(true));

    let read = Request {
        method: "changes.mark_read".into(),
        payload: Some(request::Payload::ChangesMarkRead(pb::ChangesMarkRead { worktree_id: id_bytes(wt.id), branch: wt.branch.clone() })),
        ..Default::default()
    };
    call(&t.bob, read.clone()).await;
    diff();
    assert_ne!(inbox(&t.bob, wt.id).await, Some(true), "Bob has reviewed it");
    assert_eq!(inbox(&t.owner, wt.id).await, Some(true), "the owner hasn't");
    assert_eq!(inbox(&t.carol, wt.id).await, Some(true), "nor Carol");
    assert_eq!(crate::review_ops::waiting(&t.svc).await, Some(1), "the owner's review count stands");

    call(&t.owner, read).await;
    diff();
    assert_ne!(inbox(&t.owner, wt.id).await, Some(true));
    assert_eq!(crate::review_ops::waiting(&t.svc).await, Some(0));
    assert_eq!(inbox(&t.carol, wt.id).await, Some(true), "the owner's review isn't Carol's");
}

fn done_since(at: i64) -> crate::needs_you::Observation {
    crate::needs_you::Observation {
        activity: DONE,
        state_since: at,
        blocked_question: None,
        turn_failed: false,
        command: "claude".into(),
        trust_prompt: false,
        trust_folder: None,
        not_started: None,
    }
}

/// ov-740 review F1: a runner with no members looks nobody up for a pane's
/// rows, even after the owner opened it from their phone: the owner's look
/// is written down only for a member to read.
#[tokio::test]
async fn a_runner_with_no_members_looks_nobody_up() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap();
    let pane = svc.store.create_terminal_for_test(checkout.id, workspace);
    let watcher = Watcher::new(svc.clone());
    let peer = Peer { client_id: Some("phone-o".into()), scope: Scope::Control };
    let phone = RpcFactory::new(svc.clone(), watcher.clone(), Arc::new(tokio::sync::Notify::new()), peer);
    watcher.observe_for_tests(pane, done_since(now_millis() - 5_000)).await;

    call(&phone, seen(pane)).await;
    assert_eq!(watcher.activity(pane).await.0, IDLE, "the owner's look ends Done");
    let no_lookup = || -> farcooler_core::Result<Person> { panic!("a person looked up on a runner with no members") };
    assert_eq!(watcher.viewer_for([pane], no_lookup), Person::Owner);
    let row = row(&phone, pane).await;
    let event = Event { payload: Some(event::Payload::TerminalChanged(row.clone())), ..Event::default() };
    assert_eq!(watcher.shape_event(event.clone(), no_lookup), event, "the row goes as it is");
}
