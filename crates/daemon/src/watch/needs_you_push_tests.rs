//! Pushes and the needs-you rollup: the title leads with the workspace, a
//! decision pushes, and the lock screen's count moves when nothing else
//! would carry it (spec §7). Its own file to keep `watch.rs` inside its size
//! budget.

use super::*;
use farcooler_agent::event::{AgentEvent, PermissionOption};
use farcooler_store::models::{Actor, TaskStatus};

fn blocked() -> Quoted<'static> {
    Quoted { worktree: "fc-3-webhooks", question: None, said: None }
}

#[test]
fn a_blocked_agents_notice_leads_with_its_workspace() {
    let who = Who { label: "claude".into(), workspace: Some("Billing".into()), orchestrator: false };
    let notice = agent_notice(AgentActivity::Blocked, &who, blocked(), false, None).unwrap();
    assert_eq!(notice.title, "Billing · claude needs you");
    // With no workspace, what it always said.
    let bare = Who { workspace: None, ..who };
    assert_eq!(agent_notice(AgentActivity::Blocked, &bare, blocked(), false, None).unwrap().title, "claude needs you");
}

#[test]
fn an_orchestrators_notice_is_its_workspace_alone() {
    let who = Who { label: "claude".into(), workspace: Some("Billing".into()), orchestrator: true };
    let notice = agent_notice(AgentActivity::Blocked, &who, blocked(), false, None).unwrap();
    assert_eq!(notice.title, "Billing Orchestrator needs you");
}

/// A service with a Main workspace and one claude pane in its checkout.
async fn a_runner() -> (crate::test_support::ScratchDir, Arc<Service>, Uuid, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let checkout = rows.iter().find(|w| w.is_main_checkout).unwrap();
    let pane = svc.store.create_terminal_for_test(checkout.id, main.id);
    (dir, svc, main.id, pane)
}

fn ask(id: &str) -> AgentEvent {
    AgentEvent::Permission {
        id: id.into(),
        tool_call: String::new(),
        options: vec![PermissionOption { id: "allow".into(), name: "Allow touch x".into(), kind: "allow_once".into() }],
    }
}

/// The next notice the watcher sends, waiting out any debounce.
async fn next(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) -> Option<Tapped> {
    tokio::time::timeout(std::time::Duration::from_secs(30), taps.recv()).await.ok().flatten()
}

#[tokio::test]
async fn a_decision_notice_has_a_task_and_no_terminal() {
    let (_dir, svc, workspace, _) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    let task = svc.store.create_task(workspace, "Pick a PDF library", Actor::User).unwrap();
    svc.store
        .add_note(task.id, farcooler_store::models::NoteKind::Question, Actor::Manager, "Which?", serde_json::json!({}))
        .unwrap();
    crate::task_ops::set_status(
        &svc,
        &watcher,
        &farcooler_protocol::v1::TaskSetStatus {
            task_id: crate::wire::id_bytes(task.id),
            status: farcooler_protocol::v1::TaskStatus::NeedsDecision as i32,
            actor: "manager".into(),
        },
    )
    .unwrap();
    // Now as its task's notice (ov-94): `kind: "task"`, class decision.
    let sent = next(&mut taps).await.expect("a decision pushes");
    // Only `kind: "task"`: the legacy decision kind is gone (ov-108).
    assert_eq!((sent.kind, sent.event), (Some("task"), Some("decision")));
    assert!(sent.notice_id.as_deref().is_some_and(|id| id.starts_with("t:")));
    assert_eq!(sent.terminal, None, "a decision is about a task, not a pane");
    assert_eq!(sent.task.as_deref(), Some(task.key.as_str()));
    assert_eq!(sent.title, format!("{} Pick a PDF library", task.key));
    assert_eq!(sent.subtitle, "Needs your decision · Which?");
    assert_eq!(sent.needs_you, Some(1), "the decision itself is counted");
}

#[tokio::test]
async fn answering_a_chat_ask_sends_a_count_notice() {
    use farcooler_agent::link::{ShimMessage, encode_line};
    use farcooler_transport::Handler;
    use tokio::io::AsyncWriteExt;
    let (_dir, svc, _, pane) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    // The ask comes from a shim, as a chat's does: a socket the supervisor
    // listens on, in a directory short enough to bind.
    let sockets = tempfile::Builder::new().prefix("fcw").tempdir_in("/tmp").unwrap();
    svc.agents().ensure_listening(sockets.path(), pane);
    let path = crate::agent_supervisor::socket_path(sockets.path(), pane);
    let mut shim = loop {
        if let Ok(stream) = tokio::net::UnixStream::connect(&path).await {
            break stream;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    let events = vec![farcooler_agent::event::Sequenced { seq: 0, event: ask("chat-1") }];
    shim.write_all(encode_line(&ShimMessage::Events { events }).unwrap().as_bytes()).await.unwrap();
    while svc.agents().open_permission(pane).is_none() {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    watcher
        .observe_for_tests(pane, crate::needs_you::Observation {
            activity: AgentActivity::Blocked,
            state_since: now_millis(),
            command: "claude".into(),
            ..Default::default()
        })
        .await;
    tokio::time::pause();
    let asked = next(&mut taps).await.expect("the ask moved the count");
    assert_eq!((asked.kind, asked.needs_you), (Some("count"), Some(1)));

    // Answered the way a phone answers, with nothing recorded by hand: the
    // answer's own `Resolved` is what announces the change. The agent is
    // then put back to work, as the next sample would, so the count the
    // notice carries is 0.
    let rpc = crate::rpc::RpcFactory::new(
        svc.clone(),
        watcher.clone(),
        Arc::new(tokio::sync::Notify::new()),
        farcooler_transport::Peer { client_id: None, scope: farcooler_protocol::v1::Scope::Control },
    );
    let answer = farcooler_protocol::v1::Request {
        method: "terminal.agent_answer".into(),
        payload: Some(farcooler_protocol::v1::request::Payload::AgentAnswer(farcooler_protocol::v1::AgentAnswer {
            terminal_id: crate::wire::id_bytes(pane),
            request_id: "chat-1".into(),
            option_id: "allow".into(), answers: Default::default(),
        })),
        ..Default::default()
    };
    rpc.handle(answer).await;
    assert!(svc.agents().open_permission(pane).is_none(), "the answer left the ask open");
    watcher
        .observe_for_tests(pane, crate::needs_you::Observation {
            activity: AgentActivity::Working,
            state_since: now_millis(),
            command: "claude".into(),
            ..Default::default()
        })
        .await;
    let answered = next(&mut taps).await.expect("the answer moved the count");
    assert_eq!((answered.kind, answered.needs_you, answered.terminal), (Some("count"), Some(0), None));
}

/// The relay refreshes the card on every count notice, so a burst of
/// changes is one notice, trailing, and a count it already has is none.
#[tokio::test]
async fn a_burst_of_changes_sends_one_count_notice_and_never_a_repeat() {
    let (_dir, svc, workspace, pane) = a_runner().await;
    let checkout = svc.store.get_terminal(pane).unwrap().worktree_id;
    let mut panes = vec![pane];
    for _ in 0..3 {
        panes.push(svc.store.create_terminal_for_test(checkout, workspace));
    }
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    tokio::time::pause();
    // Four asks on four panes, a second apart: the count goes 1, 2, 3, 4.
    for (n, pane) in panes.iter().enumerate() {
        watcher
            .observe_for_tests(*pane, crate::needs_you::Observation {
                activity: AgentActivity::Blocked,
                state_since: now_millis(),
                command: "claude".into(),
                ..Default::default()
            })
            .await;
        svc.agents().record(*pane, vec![ask(&format!("chat-{n}"))], &|_, _| {});
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
    let first = next(&mut taps).await.expect("a count notice");
    assert_eq!(first.needs_you, Some(4), "the burst's last count, not its first");
    assert_eq!(next(&mut taps).await, None, "one notice for the whole burst");
    // A change that leaves the count where it was says nothing.
    watcher.announce_needs_you();
    assert_eq!(next(&mut taps).await, None, "the relay already has this count");
}

/// A runner nobody paired, and no test tapping, sends nothing and so
/// gathers no count for it: a working agent's card refresh every ten
/// seconds would otherwise cost a whole gather each.
#[tokio::test]
async fn an_unpaired_runner_gathers_no_count() {
    let (_dir, svc, _, pane) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    tokio::time::pause();
    watcher.push_if_paired(pane, AgentActivity::Blocked, "claude", blocked(), false, None);
    watcher.announce_needs_you();
    // The clock is paused, so these waits are exact. The count notice is
    // scheduled once the debounce has passed, and runs (its flag goes
    // down, just before it would gather) one interval later: both seen,
    // so the zero below is a timer that ran and gathered nothing, not one
    // that had not fired yet.
    let pending = || watcher.count_pending.load(std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(NEEDS_YOU_DEBOUNCE + std::time::Duration::from_millis(10)).await;
    assert!(pending(), "no count notice was scheduled, so nothing here was tested");
    tokio::time::sleep(COUNT_NOTICE_EVERY).await;
    assert!(!pending(), "the count notice never ran");
    assert_eq!(watcher.counts_gathered.load(std::sync::atomic::Ordering::SeqCst), 0);
}

/// A count is the relay's only once a notice carrying it landed: a push
/// that failed leaves the same count to be sent again.
#[tokio::test]
async fn a_count_that_never_landed_is_sent_again() {
    let (_dir, svc, _, pane) = a_runner().await;
    // Paired with a relay nothing answers on.
    crate::push::Pairing { relay: "http://127.0.0.1:9".into(), token: "t".into() }
        .save_in(svc.root_dir())
        .unwrap();
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    let blocked_now = crate::needs_you::Observation {
        activity: AgentActivity::Blocked,
        state_since: now_millis(),
        command: "claude".into(),
        ..Default::default()
    };
    watcher.observe_for_tests(pane, blocked_now).await;
    watcher.announce_needs_you();
    let first = next(&mut taps).await.expect("a count notice");
    assert_eq!(first.needs_you, Some(1));
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    watcher.announce_needs_you();
    let again = next(&mut taps).await.expect("the same count, since the first never landed");
    assert_eq!(again.needs_you, Some(1));
}

/// What reaches the relay names this runner by its install id, whichever
/// token it holds: that is how the relay keys its count per runner rather
/// than per label, since every Mac pairs as "This Mac".
#[tokio::test]
async fn a_notice_names_the_runner_by_its_install_id() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, svc, _, pane) = a_runner().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    crate::push::Pairing { relay: format!("http://{}", listener.local_addr().unwrap()), token: "t".into() }
        .save_in(svc.root_dir())
        .unwrap();
    let relay = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        // Headers, then as many body bytes as they promise.
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            seen.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&seen).to_string();
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                if seen.len() >= end + 4 + length {
                    socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}").await.unwrap();
                    return serde_json::from_slice::<serde_json::Value>(&seen[end + 4..end + 4 + length]).unwrap();
                }
            }
            if n == 0 {
                panic!("the relay's socket closed before a whole request");
            }
        }
    });
    let watcher = Watcher::new(svc.clone());
    watcher
        .observe_for_tests(pane, crate::needs_you::Observation {
            activity: AgentActivity::Blocked,
            state_since: now_millis(),
            command: "claude".into(),
            ..Default::default()
        })
        .await;
    watcher.announce_needs_you();
    let body = tokio::time::timeout(std::time::Duration::from_secs(30), relay).await.unwrap().unwrap();
    assert_eq!(body["kind"], "count", "{body}");
    assert_eq!(body["install"], svc.install_id(), "{body}");
}

/// A paired runner beats at `/v1/heartbeat`, naming itself by its install
/// id and promising the next beat: the phone's widget judges its silence
/// by that promise (ov-53).
#[tokio::test]
async fn a_paired_runner_beats_and_names_itself() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, svc, _, _pane) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    assert!(!watcher.beat().await, "an unpaired runner has nobody to beat to");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    crate::push::Pairing { relay: format!("http://{}", listener.local_addr().unwrap()), token: "t".into() }
        .save_in(svc.root_dir())
        .unwrap();
    let relay = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            seen.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&seen).to_string();
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                if seen.len() >= end + 4 + length {
                    socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}").await.unwrap();
                    let line = text.lines().next().unwrap_or_default().to_string();
                    let body = serde_json::from_slice::<serde_json::Value>(&seen[end + 4..end + 4 + length]).unwrap();
                    return (line, body);
                }
            }
            if n == 0 {
                panic!("the relay's socket closed before a whole request");
            }
        }
    });
    assert!(watcher.beat().await, "the beat landed");
    let (line, body) = tokio::time::timeout(std::time::Duration::from_secs(30), relay).await.unwrap().unwrap();
    assert!(line.starts_with("POST /v1/heartbeat "), "{line}");
    assert_eq!(body["install"], svc.install_id(), "{body}");
    assert_eq!(body["beatEvery"], crate::push::BEAT_EVERY.as_secs(), "{body}");
}

#[tokio::test]
async fn the_push_count_is_the_lists_length() {
    let (_dir, svc, workspace, pane) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    for (title, status) in [("Review me", TaskStatus::InReview), ("Decide me", TaskStatus::NeedsDecision)] {
        let task = svc.store.create_task(workspace, title, Actor::User).unwrap();
        svc.store.set_task_status(task.id, status, Actor::Manager).unwrap();
    }
    watcher
        .observe_for_tests(pane, crate::needs_you::Observation {
            activity: AgentActivity::Blocked,
            state_since: now_millis(),
            command: "claude".into(),
            ..Default::default()
        })
        .await;
    watcher.push_if_paired(pane, AgentActivity::Blocked, "claude", blocked(), false, None);
    let sent = next(&mut taps).await.expect("the blocked agent's notice");
    let listed = crate::needs_you::assemble(
        &crate::needs_you::gather(&svc, &watcher).await.unwrap(),
        std::time::SystemTime::now(),
    );
    assert_eq!(sent.needs_you, Some(listed.len() as u32));
    assert_eq!(sent.needs_you, Some(3), "a block, a decision and a review; one pane");
    assert_eq!(sent.title, "Main · claude needs you");
}

// -- ov-57: the held ask on a blocked pane's card --

/// The next `kind:"ask"` notice, skipping the count notices a change to
/// the needs-you list also sends.
async fn next_ask(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) -> Option<Tapped> {
    loop {
        let tapped = next(taps).await?;
        if tapped.kind == Some("ask") {
            return Some(tapped);
        }
    }
}

/// Every notice sent from now until the watcher goes quiet (with the
/// clock paused, until nothing is left to wake).
async fn drain(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) -> Vec<Tapped> {
    let mut all = Vec::new();
    while let Some(tapped) = next(taps).await {
        all.push(tapped);
    }
    all
}

/// The next agent notice (no kind).
async fn next_agent(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) -> Option<Tapped> {
    loop {
        let tapped = next(taps).await?;
        if tapped.kind.is_none() {
            return Some(tapped);
        }
    }
}

/// Wait until the last landed notice about `pane` has been noted: blocked
/// (`true`) or not.
async fn noted(watcher: &Watcher, pane: Uuid, blocked: bool) {
    for _ in 0..500 {
        if watcher.asks_told.lock().unwrap().contains_key(&pane) == blocked {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the landed notice was never noted as blocked={blocked}");
}

fn working() -> Quoted<'static> {
    Quoted { worktree: "", question: Some("Reading files"), said: None }
}

/// A blocked pane, as the watcher has told the relay about it.
async fn a_blocked_pane() -> (crate::test_support::ScratchDir, Arc<Service>, Uuid, Arc<Watcher>, tokio::sync::mpsc::UnboundedReceiver<Tapped>) {
    let (dir, svc, _, pane) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let taps = watcher.tap_notices();
    watcher
        .observe_for_tests(pane, crate::needs_you::Observation {
            activity: AgentActivity::Blocked,
            state_since: now_millis(),
            command: "claude".into(),
            ..Default::default()
        })
        .await;
    (dir, svc, pane, watcher, taps)
}

/// The ask is usually offered before the blocked notice goes out, and
/// rides on it. When it comes after, one silent `kind:"ask"` carries it,
/// and only one.
#[tokio::test]
async fn an_ask_offered_after_blocked_is_sent_once() {
    let (_dir, svc, pane, watcher, mut taps) = a_blocked_pane().await;
    tokio::time::pause();
    watcher.push_if_paired(pane, AgentActivity::Blocked, "claude", blocked(), false, None);
    let sent = next_agent(&mut taps).await.expect("the blocked notice");
    assert_eq!(sent.ask, None, "nothing was offered yet");
    noted(&watcher, pane, true).await;

    let asks = svc.hooks().asks().clone();
    let (id, _rx) = asks.hold_for(pane, Some("Bash"), std::time::Duration::from_secs(60));
    assert!(asks.offer(pane, &id, ask(&id)));
    let told = next_ask(&mut taps).await.expect("the offer is told");
    assert_eq!((told.terminal, told.ask.as_deref()), (Some(pane), Some(id.as_str())));
    assert_eq!(told.title, "", "an ask notice is silent: no title to alert with");
    watcher.sync_asks(None, false).await;
    assert_eq!(next_ask(&mut taps).await, None, "once");
}

/// claude's dialog outlives the 60 s hold. When the hold runs out, the
/// card is told the ask is gone, silently, and the pane stays blocked.
#[tokio::test]
async fn a_hold_that_runs_out_sends_an_ask_clear_without_an_alert() {
    let (_dir, svc, pane, watcher, mut taps) = a_blocked_pane().await;
    let asks = svc.hooks().asks().clone();
    let (id, _rx) = asks.hold_for(pane, Some("Bash"), std::time::Duration::from_secs(60));
    assert!(asks.offer(pane, &id, ask(&id)));
    tokio::time::pause();
    watcher.push_if_paired(pane, AgentActivity::Blocked, "claude", blocked(), false, None);
    let sent = next_agent(&mut taps).await.expect("the blocked notice");
    assert_eq!(sent.ask.as_deref(), Some(id.as_str()), "the blocked notice carries its ask");
    noted(&watcher, pane, true).await;

    // What `serve` does when its hold's timer fires. The dialog stays up,
    // so no working notice follows, and after the grace the clear is owed.
    let ended = tokio::time::Instant::now();
    asks.withdraw(pane, &id);
    // Everything sent until the clear. The relay alerts on an agent
    // notice, never on `kind:"ask"`, so "without an alert" is: the clear
    // is an ask notice, and no agent notice went out beside it.
    let mut between = Vec::new();
    let cleared = loop {
        let tapped = next(&mut taps).await.expect("the hold running out is told");
        if tapped.kind == Some("ask") {
            break tapped;
        }
        between.push(tapped);
    };
    assert_eq!((cleared.terminal, cleared.ask), (Some(pane), None), "a clear: no ask");
    assert!(ended.elapsed() >= ASK_CLEAR_GRACE, "a clear waits out the grace: {:?}", ended.elapsed());
    between.extend(drain(&mut taps).await);
    assert!(between.iter().all(|t| t.kind.is_some()), "an agent notice went out: {between:?}");
    assert!(between.iter().all(|t| t.kind != Some("ask")), "the clear went out twice: {between:?}");
}

/// An answered ask is followed by the pane leaving Blocked, and that
/// working notice clears the ask on the relay itself. So the clear is not
/// sent as well: one card push for the answer, not two.
#[tokio::test]
async fn an_answered_ask_that_unblocks_sends_no_separate_clear() {
    let (_dir, svc, pane, watcher, mut taps) = a_blocked_pane().await;
    let asks = svc.hooks().asks().clone();
    let (id, _rx) = asks.hold(pane);
    assert!(asks.offer(pane, &id, ask(&id)));
    tokio::time::pause();
    watcher.push_if_paired(pane, AgentActivity::Blocked, "claude", blocked(), false, None);
    assert_eq!(next_agent(&mut taps).await.expect("blocked").ask.as_deref(), Some(id.as_str()));
    noted(&watcher, pane, true).await;

    // Answered: the ask ends, and a second later the pane is working.
    asks.withdraw(pane, &id);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    watcher.push_if_paired(pane, AgentActivity::Working, "claude", working(), false, None);
    noted(&watcher, pane, false).await;
    // Everything sent from the answer on, past the grace: the working
    // notice, maybe a count, and no ask notice.
    let sent = drain(&mut taps).await;
    let agent: Vec<_> = sent.iter().filter(|t| t.kind.is_none()).collect();
    assert_eq!(agent.len(), 1, "the working notice, once: {sent:?}");
    assert_eq!(agent[0].ask, None);
    assert!(sent.iter().all(|t| t.kind != Some("ask")), "the working notice already cleared it: {sent:?}");
}

/// The relay already has the ask the blocked notice carried, so a change
/// elsewhere, or a second look, sends nothing about this pane.
#[tokio::test]
async fn an_ask_is_not_resent_unchanged() {
    let (_dir, svc, pane, watcher, mut taps) = a_blocked_pane().await;
    let asks = svc.hooks().asks().clone();
    let (id, _rx) = asks.hold(pane);
    assert!(asks.offer(pane, &id, ask(&id)));
    tokio::time::pause();
    watcher.push_if_paired(pane, AgentActivity::Blocked, "claude", blocked(), false, None);
    assert_eq!(next_agent(&mut taps).await.expect("blocked").ask.as_deref(), Some(id.as_str()));
    noted(&watcher, pane, true).await;

    // Another pane's ask moves the ledger; this pane's has not moved.
    let other = Uuid::now_v7();
    let (elsewhere, _other_rx) = asks.hold(other);
    assert!(asks.offer(other, &elsewhere, ask(&elsewhere)));
    watcher.sync_asks(None, true).await;
    watcher.sync_asks(Some(pane), true).await;
    assert_eq!(next_ask(&mut taps).await, None, "nothing changed on this pane");
}

/// Once the relay is told the pane is working, its ask is not followed:
/// the card isn't blocked, so there is nothing to answer on it.
#[tokio::test]
async fn no_ask_notice_follows_a_working_notice() {
    let (_dir, svc, pane, watcher, mut taps) = a_blocked_pane().await;
    tokio::time::pause();
    watcher.push_if_paired(pane, AgentActivity::Blocked, "claude", blocked(), false, None);
    next_agent(&mut taps).await.expect("blocked");
    noted(&watcher, pane, true).await;
    watcher.push_if_paired(pane, AgentActivity::Working, "claude", working(), false, None);
    let sent = next_agent(&mut taps).await.expect("working");
    assert_eq!(sent.ask, None);
    noted(&watcher, pane, false).await;

    let asks = svc.hooks().asks().clone();
    let (id, _rx) = asks.hold(pane);
    assert!(asks.offer(pane, &id, ask(&id)));
    asks.withdraw(pane, &id);
    assert_eq!(next_ask(&mut taps).await, None, "no ask notice for a pane that is working");
}
