//! A chat pane moves to its TUI at its next idle (ov-376).

use std::time::Duration;

use farcooler_agent_core::event::{AgentEvent, EndReason, PermissionOption, QueuedPrompt, Role, Sequenced};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::*;

fn events(list: Vec<AgentEvent>) -> String {
    let events: Vec<Sequenced> =
        list.into_iter().enumerate().map(|(seq, event)| Sequenced { seq: seq as u64, event }).collect();
    format!("{}\n", serde_json::json!({ "kind": "events", "events": events }))
}

fn said(role: Role, text: &str) -> AgentEvent {
    AgentEvent::Message { role, text: text.into(), parent: None }
}

fn ended() -> AgentEvent {
    AgentEvent::TurnEnded { reason: EndReason::EndTurn }
}

const ESTABLISHED: &str = "{\"kind\":\"established\",\"session_id\":\"s-1\",\"available_modes\":[]}\n";

/// Short waits for what should happen, and a long one for what shouldn't.
fn quick() -> Timing {
    Timing {
        grace: Duration::from_secs(30),
        settle: Duration::from_millis(50),
        retry: Duration::from_millis(10),
        retry_max: Duration::from_millis(40),
    }
}

#[test]
fn a_turn_in_flight_holds_the_pane_and_its_end_frees_it() {
    let mut turn = Turn::default();
    assert!(turn.idle(), "nothing observed is no turn to lose");
    turn.hear(&events(vec![said(Role::User, "fix it"), said(Role::Agent, "looking")]));
    assert!(!turn.idle(), "an agent mid-turn");
    turn.hear(&events(vec![ended()]));
    assert!(turn.idle(), "the turn ended");
}

#[test]
fn a_queued_prompt_holds_the_pane_past_the_turns_end() {
    // The shim sends it the moment the turn ends; a pane moved then loses it.
    let mut turn = Turn::default();
    let queued = AgentEvent::PromptQueue {
        items: vec![QueuedPrompt { id: "q".into(), text: "and then".into(), images: Vec::new() }],
    };
    turn.hear(&events(vec![said(Role::Agent, "working"), queued, ended()]));
    assert!(!turn.idle());
    turn.hear(&events(vec![AgentEvent::PromptQueue { items: Vec::new() }]));
    assert!(turn.idle());
}

#[test]
fn an_open_ask_does_not_hold_the_pane() {
    // Nothing can answer a chat's ask any more, so it would hold it forever.
    let mut turn = Turn::default();
    let ask = AgentEvent::Permission {
        id: "p".into(),
        tool_call: String::new(),
        options: vec![PermissionOption { id: "allow".into(), name: "Allow".into(), kind: "allow_once".into() }],
    };
    turn.hear(&events(vec![said(Role::Agent, "may I"), ask]));
    assert!(turn.idle());
}

#[test]
fn the_shim_names_the_conversation_and_says_when_its_agent_stopped() {
    let mut turn = Turn::default();
    turn.hear(ESTABLISHED);
    assert_eq!(turn.session().as_deref(), Some("s-1"));
    assert!(!turn.stopped());
    turn.hear("{\"kind\":\"failed\",\"failure\":\"adapter-failed\"}");
    assert!(turn.stopped());
}

#[test]
fn a_line_it_cannot_read_changes_nothing() {
    let mut turn = Turn::default();
    turn.hear(&events(vec![said(Role::Agent, "working")]));
    turn.hear("not json");
    turn.hear("{\"kind\":\"events\",\"events\":[{\"seq\":0,\"event\":{\"FromTheFuture\":{}}}]}");
    assert!(!turn.idle(), "still the turn it was");
}

#[test]
fn the_notice_is_printed_before_the_agent_starts() {
    let command = with_notice("env A=1 claude --resume s-1");
    assert!(command.starts_with("printf '%s\\n' 'Far Cooler: chat view retired"), "{command}");
    assert!(command.ends_with("; env A=1 claude --resume s-1"), "{command}");
}

#[test]
fn a_socket_path_is_the_one_the_chat_pane_dials() {
    let terminal = Uuid::parse_str("0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b").unwrap();
    assert_eq!(socket_path(Path::new("/run/fc"), terminal), PathBuf::from("/run/fc/a-0c1d2e3f4a5b.sock"));
}

/// A shim on the other end of the socket, as the build before wrote one.
async fn shim(socket: &Path) -> (tokio::net::unix::OwnedWriteHalf, String) {
    let stream = loop {
        match UnixStream::connect(socket).await {
            Ok(stream) => break stream,
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    };
    let (read, write) = stream.into_split();
    let mut asked = String::new();
    BufReader::new(read).read_line(&mut asked).await.unwrap();
    (write, asked)
}

#[tokio::test]
async fn a_pane_mid_turn_moves_only_when_the_turn_ends() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_path(dir.path(), Uuid::now_v7());
    let watching = tokio::spawn({
        let socket = socket.clone();
        async move { watch(&socket, quick()).await }
    });

    let (mut write, asked) = shim(&socket).await;
    assert_eq!(asked.trim(), "{\"kind\":\"subscribe\",\"from_seq\":0}", "the whole ring, from the start");
    write.write_all(ESTABLISHED.as_bytes()).await.unwrap();
    write.write_all(events(vec![said(Role::User, "fix it"), said(Role::Agent, "on it")]).as_bytes()).await.unwrap();

    // Many settles later, still waiting on the turn.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!watching.is_finished(), "a pane moved mid-turn loses the turn");

    write.write_all(events(vec![ended()]).as_bytes()).await.unwrap();
    let verdict = tokio::time::timeout(Duration::from_secs(30), watching).await.unwrap().unwrap();
    assert_eq!(verdict, Verdict { why: Why::Idle, session: Some("s-1".into()) });
    assert!(!socket.exists(), "the socket goes with the watch");
}

#[tokio::test]
async fn an_idle_pane_moves_once_its_replay_settles() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_path(dir.path(), Uuid::now_v7());
    let watching = tokio::spawn({
        let socket = socket.clone();
        async move { watch(&socket, quick()).await }
    });
    let (mut write, _) = shim(&socket).await;
    write.write_all(ESTABLISHED.as_bytes()).await.unwrap();
    write.write_all(events(vec![said(Role::User, "hi"), said(Role::Agent, "hello"), ended()]).as_bytes()).await.unwrap();
    let verdict = tokio::time::timeout(Duration::from_secs(30), watching).await.unwrap().unwrap();
    assert_eq!(verdict.why, Why::Idle);
}

#[tokio::test]
async fn a_shim_whose_agent_stopped_moves_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_path(dir.path(), Uuid::now_v7());
    let watching = tokio::spawn({
        let socket = socket.clone();
        async move { watch(&socket, Timing { settle: Duration::from_secs(60), ..quick() }).await }
    });
    let (mut write, _) = shim(&socket).await;
    write.write_all(events(vec![said(Role::Agent, "half a turn")]).as_bytes()).await.unwrap();
    write.write_all(b"{\"kind\":\"failed\",\"failure\":\"adapter-failed\"}\n").await.unwrap();
    let verdict = tokio::time::timeout(Duration::from_secs(30), watching).await.unwrap().unwrap();
    assert_eq!(verdict.why, Why::Stopped);
}

#[tokio::test]
async fn no_shim_dialing_moves_the_pane_after_the_grace() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_path(dir.path(), Uuid::now_v7());
    let verdict = watch(&socket, Timing { grace: Duration::from_millis(50), ..quick() }).await;
    assert_eq!(verdict, Verdict { why: Why::NoShim, session: None });
}

#[tokio::test]
async fn a_shim_that_hangs_up_moves_the_pane() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_path(dir.path(), Uuid::now_v7());
    let watching = tokio::spawn({
        let socket = socket.clone();
        async move { watch(&socket, quick()).await }
    });
    let (mut write, _) = shim(&socket).await;
    write.write_all(events(vec![said(Role::Agent, "mid-turn")]).as_bytes()).await.unwrap();
    drop(write);
    let verdict = tokio::time::timeout(Duration::from_secs(30), watching).await.unwrap().unwrap();
    assert_eq!(verdict.why, Why::NoShim);
}

/// A turn can be one tool call that runs for an hour, saying nothing the
/// whole time; moving the pane then would kill it. Paused time, so the hour
/// passes at once.
#[tokio::test(start_paused = true)]
async fn a_long_silent_turn_is_never_moved_until_it_ends() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_path(dir.path(), Uuid::now_v7());
    let watching = tokio::spawn({
        let socket = socket.clone();
        async move { watch(&socket, Timing::default()).await }
    });
    let (mut write, _) = shim(&socket).await;
    write.write_all(events(vec![said(Role::User, "run the tests"), said(Role::Agent, "running cargo test")]).as_bytes()).await.unwrap();

    tokio::time::sleep(Duration::from_secs(3 * 60 * 60)).await;
    assert!(!watching.is_finished(), "three silent hours moved a pane mid-turn");

    write.write_all(events(vec![ended()]).as_bytes()).await.unwrap();
    let verdict = watching.await.unwrap();
    assert_eq!(verdict.why, Why::Idle);
}

/// A move that fails is tried again, backing off, until it lands.
#[tokio::test(start_paused = true)]
async fn a_failed_move_is_tried_again_until_it_lands() {
    let tries = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let started = tokio::time::Instant::now();
    let landed = retrying(quick(), || {
        let tries = tries.clone();
        async move {
            match tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                0..=3 => Err(farcooler_core::DomainError::Conflict { what: "busy" }),
                _ => Ok(()),
            }
        }
    })
    .await;
    assert!(landed);
    assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 5);
    // 10 + 20 + 40 + 40: doubled, then held at the cap.
    assert_eq!(started.elapsed(), Duration::from_millis(110));
}

/// A terminal that is gone has nothing left to move.
#[tokio::test(start_paused = true)]
async fn a_move_for_a_terminal_that_is_gone_stops() {
    let tries = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let landed = retrying(quick(), || {
        let tries = tries.clone();
        async move {
            tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(farcooler_core::DomainError::NotFound)
        }
    })
    .await;
    assert!(!landed);
    assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 1);
}
