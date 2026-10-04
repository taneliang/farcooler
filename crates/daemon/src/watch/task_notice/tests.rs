//! The task notice composer, end to end through the board's own writes: what
//! a burst sends, what never sends, and what an agent on a task sends.

use std::sync::Arc;

use farcooler_protocol::v1 as pb;
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::models::{Actor, NoteKind, TaskStatus, TaskUpdate};

use super::*;
use crate::service::Service;
use crate::watch::{Observed, Quoted, Tapped, Watcher};
use farcooler_transport::Handler;

/// A runner with a Main workspace, its checkout, and one claude pane there.
async fn a_runner() -> (crate::test_support::ScratchDir, Arc<Service>, Uuid, Uuid, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let checkout = rows.iter().find(|w| w.is_main_checkout).unwrap().id;
    let pane = svc.store.create_terminal_for_test(checkout, main.id);
    (dir, svc, main.id, checkout, pane)
}

fn move_to(svc: &Service, watcher: &Watcher, task: Uuid, status: pb::TaskStatus, actor: &str) {
    crate::task_ops::set_status(
        svc,
        watcher,
        &pb::TaskSetStatus { task_id: crate::wire::id_bytes(task), status: status as i32, actor: actor.into() },
    )
    .unwrap();
}

fn note(svc: &Service, watcher: &Watcher, task: Uuid, kind: pb::TaskNoteKind, actor: &str, body: &str, extra: &str) {
    crate::task_ops::note(
        svc,
        watcher,
        &pb::TaskNoteAppend {
            task_id: crate::wire::id_bytes(task),
            kind: kind as i32,
            actor: actor.into(),
            body: body.into(),
            extra_json: extra.into(),
            ..Default::default()
        },
    )
    .unwrap();
}

/// Every task notice tapped until the composer has long gone quiet, in
/// order, whether it went as `kind: "task"` or as a legacy decision: both
/// carry a class. Count notices, which the same writes cause, are left out.
async fn task_notices(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) -> Vec<Tapped> {
    let mut heard = Vec::new();
    while let Ok(Some(tap)) = tokio::time::timeout(AT_MOST * 3, taps.recv()).await {
        if tap.event.is_some() {
            heard.push(tap);
        }
    }
    heard
}

#[test]
fn a_notice_id_names_the_runner_and_the_task_in_at_most_64_bytes() {
    let runner = "7537626f-0002-415e-1e11-000d48034210";
    let mut task = farcooler_store::models::Task {
        id: Uuid::now_v7(),
        key: "ov-90".into(),
        ..test_task()
    };
    assert_eq!(notice_id(runner, &task), format!("t:{runner}:ov-90"));
    assert_eq!(notice_id(runner, &task), notice_id(runner, &task), "stable");
    // A key long enough to pass APNs's limit is hashed, still per task.
    task.key = "averyveryverylongprefix-12345".into();
    let long = notice_id(runner, &task);
    assert!(long.len() <= 64, "{long}");
    assert!(long.starts_with("t:"), "{long}");
    assert_eq!(long, notice_id(runner, &task));
    let other = farcooler_store::models::Task { id: Uuid::now_v7(), ..task.clone() };
    assert_ne!(long, notice_id(runner, &other), "two tasks, two ids");
}

fn test_task() -> farcooler_store::models::Task {
    farcooler_store::models::Task {
        id: Uuid::nil(),
        key: String::new(),
        repository_id: Uuid::nil(),
        workspace_id: Uuid::nil(),
        title: String::new(),
        status: TaskStatus::Backlog,
        status_since: 0,
        intent: String::new(),
        acceptance: Vec::new(),
        constraints: Vec::new(),
        worktree_id: None,
        labels: Vec::new(),
        resource_version: 0,
        created_at: 0,
        updated_at: 0,
        wait: None,
    }
}

#[tokio::test]
async fn a_burst_of_moves_sends_one_notice_saying_where_it_ended() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let task = svc.store.create_task(workspace, "Wake the agent on an answer", Actor::User).unwrap();
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    move_to(&svc, &watcher, task.id, pb::TaskStatus::InProgress, "manager");
    tokio::time::advance(Duration::from_secs(1)).await;
    move_to(&svc, &watcher, task.id, pb::TaskStatus::InReview, "manager");
    tokio::time::advance(Duration::from_secs(1)).await;
    move_to(&svc, &watcher, task.id, pb::TaskStatus::Done, "manager");

    let heard = task_notices(&mut taps).await;
    assert_eq!(heard.len(), 1, "{heard:#?}");
    let done = &heard[0];
    assert_eq!(done.event, Some("done"));
    assert_eq!(done.level, Some("passive"));
    assert_eq!(done.subtitle, "Done");
    assert_eq!(done.title, format!("{} Wake the agent on an answer", task.key));
    assert_eq!(done.task.as_deref(), Some(task.key.as_str()));
    assert_eq!(done.terminal, None, "a task is not a pane");
    let runner = crate::service::stable_host_id(svc.install_id()).to_string();
    assert_eq!(done.notice_id, Some(notice_id(&runner, &task)));
}

#[tokio::test]
async fn a_steady_trickle_still_sends_once_the_window_has_been_open_ten_seconds() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let task = svc.store.create_task(workspace, "Trickle", Actor::User).unwrap();
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    move_to(&svc, &watcher, task.id, pb::TaskStatus::InReview, "manager");
    let task = svc.store.get_task(task.id).unwrap();
    // News every two seconds keeps the window from ever going quiet.
    let began = tokio::time::Instant::now();
    let mut sent = None;
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        watcher.task_event(&task, TaskEvent::Moved { to: TaskStatus::InReview }, Actor::Manager);
        while let Ok(tap) = taps.try_recv() {
            if tap.kind == Some("task") && sent.is_none() {
                sent = Some(began.elapsed());
            }
        }
    }
    let sent = sent.expect("the window closed at its cap");
    assert!(sent <= AT_MOST + Duration::from_secs(2), "{sent:?}");
}

#[tokio::test]
async fn a_decision_sends_at_once_with_its_question_and_options() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let task = svc.store.create_task(workspace, "Pick a PDF library", Actor::User).unwrap();
    svc.store
        .add_note(
            task.id,
            NoteKind::Question,
            Actor::Manager,
            "Which PDF library?",
            serde_json::json!({ "options": ["pdfkit", "pdf.js", "x".repeat(41), "qpdf", "mupdf"] }),
        )
        .unwrap();
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    let began = tokio::time::Instant::now();
    move_to(&svc, &watcher, task.id, pb::TaskStatus::NeedsDecision, "manager");
    let first = loop {
        let tap = taps.recv().await.expect("a notice");
        if tap.event.is_some() {
            break tap;
        }
    };
    // As a legacy decision, so a relay or an app older than task notices
    // still alerts and opens it, carrying the task notice's own fields.
    assert_eq!(first.kind, Some("decision"));
    let runner = crate::service::stable_host_id(svc.install_id()).to_string();
    assert_eq!(first.notice_id, Some(notice_id(&runner, &task)));
    assert!(began.elapsed() < QUIET_FOR, "waited {:?} on a decision", began.elapsed());
    assert_eq!(first.event, Some("decision"));
    assert_eq!(first.level, Some("time-sensitive"));
    assert_eq!(first.subtitle, "Needs your decision · Which PDF library?");
    assert_eq!(first.options, vec!["pdfkit", "pdf.js", "qpdf"]);
    assert_eq!(first.needs_you, Some(1), "the decision itself is counted");
    assert!(task_notices(&mut taps).await.is_empty(), "and only once");
}

#[tokio::test]
async fn a_decision_answered_inside_its_window_says_nothing() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let task = svc.store.create_task(workspace, "Pick", Actor::User).unwrap();
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    // In Review opens the window; the decision joins it rather than sending.
    move_to(&svc, &watcher, task.id, pb::TaskStatus::InReview, "manager");
    move_to(&svc, &watcher, task.id, pb::TaskStatus::NeedsDecision, "manager");
    note(&svc, &watcher, task.id, pb::TaskNoteKind::Answer, "user", "pdfkit", "");
    assert_eq!(task_notices(&mut taps).await, Vec::<Tapped>::new());
}

#[tokio::test]
async fn your_own_moves_are_never_news_and_the_managers_are() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let mine = svc.store.create_task(workspace, "Mine", Actor::User).unwrap();
    let theirs = svc.store.create_task(workspace, "Theirs", Actor::User).unwrap();
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    move_to(&svc, &watcher, mine.id, pb::TaskStatus::InReview, "user");
    move_to(&svc, &watcher, theirs.id, pb::TaskStatus::InReview, "manager");
    let heard = task_notices(&mut taps).await;
    assert_eq!(heard.iter().map(|t| t.task.clone().unwrap()).collect::<Vec<_>>(), vec![theirs.key.clone()]);
    assert_eq!(heard[0].event, Some("review"));
    assert_eq!(heard[0].subtitle, "Moved to In Review");
}

#[tokio::test]
async fn progress_notes_and_the_runners_own_writes_say_nothing() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let task = svc.store.create_task(workspace, "Quiet", Actor::User).unwrap();
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    note(&svc, &watcher, task.id, pb::TaskNoteKind::Progress, "manager", "Halfway", "");
    note(&svc, &watcher, task.id, pb::TaskNoteKind::Decision, "manager", "Use pdfkit", "");
    watcher.task_event(&task, TaskEvent::Moved { to: TaskStatus::InReview }, Actor::Runner);
    move_to(&svc, &watcher, task.id, pb::TaskStatus::InProgress, "manager");
    move_to(&svc, &watcher, task.id, pb::TaskStatus::Cancelled, "manager");
    assert_eq!(task_notices(&mut taps).await, Vec::<Tapped>::new());
}

#[tokio::test]
async fn blocked_on_a_task_that_is_done_says_nothing_and_on_an_open_one_says_so() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let task = svc.store.create_task(workspace, "Waiting", Actor::User).unwrap();
    let finished = svc.store.create_task(workspace, "Finished", Actor::User).unwrap();
    svc.store.set_task_status(finished.id, TaskStatus::Done, Actor::User).unwrap();
    let open = svc.store.create_task(workspace, "Open", Actor::User).unwrap();
    let block = |by: Uuid, reason: Option<&str>| {
        crate::task_ops::block(
            &svc,
            &watcher,
            &pb::TaskBlockSet {
                task_id: crate::wire::id_bytes(task.id),
                blocked_by: crate::wire::id_bytes(by),
                reason: reason.map(str::to_string),
                actor: "manager".into(),
                clear: false,
            },
        )
        .unwrap();
    };
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    block(finished.id, None);
    assert_eq!(task_notices(&mut taps).await, Vec::<Tapped>::new());
    block(open.id, Some("needs the schema"));
    let heard = task_notices(&mut taps).await;
    assert_eq!(heard.len(), 1, "{heard:#?}");
    assert_eq!(heard[0].event, Some("blocked"));
    assert_eq!(heard[0].subtitle, format!("Blocked on {} · needs the schema", open.key));
}

#[tokio::test]
async fn a_task_filed_by_the_manager_is_new() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let repository = svc.store.get_workspace(workspace).unwrap().repository_id;
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    let filed = |actor: &str, title: &str| {
        crate::task_ops::create(
            &svc,
            &watcher,
            &pb::TaskCreate {
                repository_id: crate::wire::id_bytes(repository),
                title: title.into(),
                actor: actor.into(),
                ..Default::default()
            },
        )
        .unwrap()
    };
    filed("user", "Mine");
    let theirs = filed("manager", "Theirs");
    let heard = task_notices(&mut taps).await;
    assert_eq!(heard.len(), 1, "{heard:#?}");
    assert_eq!(heard[0].task.as_deref(), Some(theirs.key.as_str()));
    assert_eq!(heard[0].event, Some("new"));
    assert_eq!(heard[0].subtitle, "New in Main · filed by the manager");
}

#[tokio::test]
async fn an_agent_on_a_task_is_told_about_through_the_task() {
    let (_dir, svc, workspace, checkout, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    // The pane's lane is this task's, and it is the only open one there. The
    // lane is not the main checkout, whose hand-opened agents are nobody's.
    let repo = svc.store.get_worktree(checkout).unwrap().repository_id;
    let checkout = svc.store.create_worktree(repo, "lane", "/tmp/fc-t/ov-112-lane", false).unwrap().id;
    let pane = svc.store.create_terminal_for_test(checkout, workspace);
    let task = svc.store.create_task(workspace, "Wake the agent", Actor::User).unwrap();
    let update = TaskUpdate {
        title: task.title.clone(),
        intent: String::new(),
        acceptance: Vec::new(),
        constraints: Vec::new(),
        labels: Vec::new(),
        worktree_id: Some(checkout),
    };
    let task = svc.store.update_task(task.id, task.resource_version, &update).unwrap();
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    let asking = Quoted { worktree: "main", question: Some("Create haiku.txt?"), said: None };
    watcher.announce_transition(pane, AgentActivity::Blocked, "claude", asking, false, None);

    let mut agent = None;
    let mut notice = None;
    while let Ok(Some(tap)) = tokio::time::timeout(AT_MOST * 3, taps.recv()).await {
        match tap.kind {
            None => agent = Some(tap),
            Some("task") => notice = Some(tap),
            _ => {}
        }
    }
    let agent = agent.expect("the agent's own notice, for its card");
    assert!(agent.quiet, "sent with alert: false");
    assert_eq!(agent.terminal, Some(pane));
    let notice = notice.expect("the task's notice");
    assert_eq!(notice.event, Some("decision"));
    assert_eq!(notice.subtitle, "claude needs you · Create haiku.txt?");
    assert_eq!(notice.task.as_deref(), Some(task.key.as_str()));
    assert!(notice.options.is_empty(), "never an agent's ask's options");
}

/// What `announce_transition` taps for one blocked agent, minus the counts.
async fn heard_from_a_blocked(watcher: &Watcher, pane: Uuid) -> Vec<Tapped> {
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    let asking = Quoted { worktree: "main", question: Some("Create haiku.txt?"), said: None };
    watcher.announce_transition(pane, AgentActivity::Blocked, "claude", asking, false, None);
    let mut heard = Vec::new();
    while let Ok(Some(tap)) = tokio::time::timeout(AT_MOST * 3, taps.recv()).await {
        if tap.kind != Some("count") {
            heard.push(tap);
        }
    }
    heard
}

#[tokio::test]
async fn an_agent_on_a_closed_task_notifies_as_itself() {
    let (_dir, svc, workspace, checkout, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let repo = svc.store.get_worktree(checkout).unwrap().repository_id;
    let lane = svc.store.create_worktree(repo, "lane", "/tmp/fc-t/ov-112-lane", false).unwrap().id;
    let task = svc.store.create_task(workspace, "Over", Actor::User).unwrap();
    let made = svc
        .store
        .create_terminal_for_task(lane, "agent", "claude", farcooler_protocol::v1::TerminalIntent::Running, 80, 24, Some(task.id))
        .unwrap();
    let pane = svc.store.set_terminal_workspace(made.id, workspace).unwrap().id;
    svc.store.set_task_status(task.id, TaskStatus::Done, Actor::User).unwrap();
    let heard = heard_from_a_blocked(&watcher, pane).await;
    assert_eq!(heard.len(), 1, "{heard:#?}");
    assert_eq!(heard[0].kind, None);
    assert!(!heard[0].quiet, "its own banner alerts: no task notice speaks for it");
}

#[tokio::test]
async fn a_hand_opened_agent_in_the_main_checkout_notifies_as_itself() {
    let (_dir, svc, workspace, checkout, pane) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    // The checkout's one open task is somebody else's dispatch.
    let task = svc.store.create_task(workspace, "Dispatched here", Actor::User).unwrap();
    let update = TaskUpdate {
        title: task.title.clone(),
        intent: String::new(),
        acceptance: Vec::new(),
        constraints: Vec::new(),
        labels: Vec::new(),
        worktree_id: Some(checkout),
    };
    svc.store.update_task(task.id, task.resource_version, &update).unwrap();
    let heard = heard_from_a_blocked(&watcher, pane).await;
    assert_eq!(heard.len(), 1, "{heard:#?}");
    assert!(!heard[0].quiet, "it alerts");
}

#[tokio::test]
async fn an_agent_with_no_task_notifies_as_it_always_has() {
    let (_dir, svc, _, _, pane) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    let asking = Quoted { worktree: "main", question: Some("Create haiku.txt?"), said: None };
    watcher.announce_transition(pane, AgentActivity::Blocked, "claude", asking, false, None);
    let mut heard = Vec::new();
    while let Ok(Some(tap)) = tokio::time::timeout(AT_MOST * 3, taps.recv()).await {
        if tap.kind != Some("count") {
            heard.push(tap);
        }
    }
    assert_eq!(heard.len(), 1, "{heard:#?}");
    assert_eq!(heard[0].kind, None);
    assert!(!heard[0].quiet, "it alerts");
    assert_eq!(heard[0].title, "Main · claude needs you");
}

#[tokio::test]
async fn a_restart_tells_nothing_it_already_said() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let task = svc.store.create_task(workspace, "Already", Actor::User).unwrap();
    svc.store.set_task_status(task.id, TaskStatus::InReview, Actor::Manager).unwrap();
    let other = svc.store.create_task(workspace, "Waiting", Actor::User).unwrap();
    svc.store.set_task_status(other.id, TaskStatus::NeedsDecision, Actor::Manager).unwrap();
    // A daemon starting over a board in this state.
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    assert_eq!(task_notices(&mut taps).await, Vec::<Tapped>::new());
}

#[tokio::test]
async fn the_notice_event_reaches_every_client_paired_or_not() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let task = svc.store.create_task(workspace, "Heard", Actor::User).unwrap();
    let mut events = watcher.subscribe();
    tokio::time::pause();
    move_to(&svc, &watcher, task.id, pb::TaskStatus::InReview, "manager");
    let notice = loop {
        let event = tokio::time::timeout(AT_MOST * 3, events.recv()).await.expect("an event").unwrap();
        if let Some(pb::event::Payload::Notice(notice)) = event.payload {
            break notice;
        }
    };
    assert_eq!(notice.event, "review");
    assert_eq!(notice.task_key, task.key);
    assert!(notice.notice_id.starts_with("t:"));
    assert_eq!(notice.title, format!("{} Heard", task.key));
    // Which board the key is on, so a click on it opens this task and not
    // another repository's under the same key (ov-106).
    assert_eq!(notice.repository_id.as_ref(), task.repository_id.as_bytes());
}

#[tokio::test]
async fn a_follow_up_question_in_the_same_status_is_news_and_a_repeat_is_not() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let task = svc.store.create_task(workspace, "Pick", Actor::User).unwrap();
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    note(&svc, &watcher, task.id, pb::TaskNoteKind::Question, "manager", "Which library?", "");
    move_to(&svc, &watcher, task.id, pb::TaskStatus::NeedsDecision, "manager");
    let first = task_notices(&mut taps).await;
    assert_eq!(first.len(), 1, "{first:#?}");
    note(&svc, &watcher, task.id, pb::TaskNoteKind::Answer, "user", "pdfkit", "");
    // Asked again, the task never having left Needs Decision.
    note(&svc, &watcher, task.id, pb::TaskNoteKind::Question, "manager", "Which version?", "");
    let second = task_notices(&mut taps).await;
    assert_eq!(second.len(), 1, "a follow-up question buzzes: {second:#?}");
    assert_eq!(second[0].subtitle, "Needs your decision · Which version?");
    // The same question, told again, is not.
    let task = svc.store.get_task(task.id).unwrap();
    watcher.task_event(&task, TaskEvent::Asked, Actor::Manager);
    assert_eq!(task_notices(&mut taps).await, Vec::<Tapped>::new());
}

#[tokio::test]
async fn a_second_blocker_in_the_same_status_is_news() {
    let (_dir, svc, workspace, _, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let task = svc.store.create_task(workspace, "Waiting", Actor::User).unwrap();
    let a = svc.store.create_task(workspace, "A", Actor::User).unwrap();
    let b = svc.store.create_task(workspace, "B", Actor::User).unwrap();
    let block = |by: Uuid| {
        crate::task_ops::block(
            &svc,
            &watcher,
            &pb::TaskBlockSet {
                task_id: crate::wire::id_bytes(task.id),
                blocked_by: crate::wire::id_bytes(by),
                reason: None,
                actor: "manager".into(),
                clear: false,
            },
        )
        .unwrap();
    };
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    block(a.id);
    assert_eq!(task_notices(&mut taps).await.len(), 1);
    block(b.id);
    let heard = task_notices(&mut taps).await;
    assert_eq!(heard.len(), 1, "{heard:#?}");
    assert_eq!(heard[0].subtitle, format!("Blocked on {}", b.key));
}

/// The same hand-opened agent, in a lane with one open task, read two ways: in
/// a `terminal.list` reply and in the `TerminalChanged` event the watcher
/// broadcasts. Both carry the id, because both finishers stamp it; a client
/// that lists and then watches must never be told two different answers.
#[tokio::test]
async fn the_list_and_the_event_agree_on_the_notice_task() {
    let (_dir, svc, workspace, checkout, _) = a_runner().await;
    let repo = svc.store.get_worktree(checkout).unwrap().repository_id;
    let lane = svc.store.create_worktree(repo, "lane", "/tmp/fc-t/ov-112-lane", false).unwrap().id;
    let task = svc.store.create_task(workspace, "On the lane", Actor::User).unwrap();
    let update = TaskUpdate {
        title: task.title.clone(),
        intent: String::new(),
        acceptance: Vec::new(),
        constraints: Vec::new(),
        labels: Vec::new(),
        worktree_id: Some(lane),
    };
    let task = svc.store.update_task(task.id, task.resource_version, &update).unwrap();
    // The main checkout has an open task of its own, so only the guard keeps
    // the bystander from taking it.
    let there = svc.store.create_task(workspace, "On the checkout", Actor::User).unwrap();
    let update = TaskUpdate { title: there.title.clone(), worktree_id: Some(checkout), ..update };
    svc.store.update_task(there.id, there.resource_version, &update).unwrap();
    let pane = svc.store.create_terminal_for_test(lane, workspace);
    let bystander = svc.store.create_terminal_for_test(checkout, workspace);
    let watcher = Watcher::new(svc.clone());
    let want = Some(bytes::Bytes::copy_from_slice(task.id.as_bytes()));

    // The list.
    let rpc = crate::rpc::RpcFactory::new(
        svc.clone(),
        watcher.clone(),
        Arc::new(tokio::sync::Notify::new()),
        farcooler_transport::Peer { client_id: None, scope: pb::Scope::Control },
    );
    let listed = rpc
        .handle(pb::Request {
            method: "terminal.list".into(),
            payload: Some(pb::request::Payload::Empty(pb::Empty {})),
            ..Default::default()
        })
        .await;
    let Some(pb::response::Outcome::Result(pb::Result { value: Some(pb::result::Value::TerminalList(list)) })) =
        listed.outcome
    else {
        panic!("a terminal list, got {listed:?}")
    };
    let by_id = |id: Uuid| list.items.iter().find(|t| t.id.as_ref() == id.as_bytes()).expect("listed").clone();
    assert_eq!(by_id(pane).notice_task_id, want, "the list");
    assert_eq!(by_id(bystander).notice_task_id, None, "a hand-opened agent in the main checkout notifies as itself");

    // The event.
    let mut events = watcher.subscribe();
    watcher.announce(pane, Observed::begin(AgentActivity::Working, 0), None).await;
    let Some(pb::event::Payload::TerminalChanged(changed)) = events.recv().await.unwrap().payload else {
        panic!("a terminal event")
    };
    assert_eq!(changed.notice_task_id, want, "the event");
}

/// An event announced for a transition says what that transition's notice did,
/// not a second reading of the board: a task closing between the two reads
/// would otherwise leave the runner folding and the Mac's event saying
/// nothing, or the reverse, which is silence.
#[tokio::test]
async fn an_event_for_a_transition_carries_the_notice_that_was_sent() {
    let (_dir, svc, workspace, checkout, _) = a_runner().await;
    let repo = svc.store.get_worktree(checkout).unwrap().repository_id;
    let lane = svc.store.create_worktree(repo, "lane", "/tmp/fc-t/ov-112-lane", false).unwrap().id;
    let task = svc.store.create_task(workspace, "On the lane", Actor::User).unwrap();
    let update = TaskUpdate {
        title: task.title.clone(),
        intent: String::new(),
        acceptance: Vec::new(),
        constraints: Vec::new(),
        labels: Vec::new(),
        worktree_id: Some(lane),
    };
    let task = svc.store.update_task(task.id, task.resource_version, &update).unwrap();
    let pane = svc.store.create_terminal_for_test(lane, workspace);
    let watcher = Watcher::new(svc.clone());
    let mut events = watcher.subscribe();
    // A fresh reading would say the task; the transition folded nothing.
    watcher.announce(pane, Observed::begin(AgentActivity::Blocked, 0), Some(None)).await;
    let Some(pb::event::Payload::TerminalChanged(changed)) = events.recv().await.unwrap().payload else {
        panic!("a terminal event")
    };
    assert_eq!(changed.notice_task_id, None, "the transition's own answer");
    watcher.announce(pane, Observed::begin(AgentActivity::Blocked, 0), Some(Some(task.id))).await;
    let Some(pb::event::Payload::TerminalChanged(changed)) = events.recv().await.unwrap().payload else {
        panic!("a terminal event")
    };
    assert_eq!(changed.notice_task_id, Some(bytes::Bytes::copy_from_slice(task.id.as_bytes())));
}
