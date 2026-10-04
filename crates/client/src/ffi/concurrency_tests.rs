//! The FFI's calls run alongside each other: a keystroke is not queued behind
//! a slow diff (ov-147).
//!
//! Against a stand-in runner served by the real `serve_connection`, so the
//! runner side is the daemon's own concurrency and only the client is under
//! test. The stand-in stalls one method until it is released.

use std::sync::Arc;
use std::time::{Duration, Instant};

use farcooler_protocol::v1::{self as pb, Request, Response, Scope, response, result};
use farcooler_transport::{Connection, HandshakeConfig, Handler, Peer, serve_connection};

use super::*;

/// A runner that answers everything at once, except `changes.file_diff`,
/// which waits until `release` is notified. Keeps what was typed, in the
/// order it ran — which for one pane is the order it arrived.
struct Stalling {
    release: Arc<tokio::sync::Notify>,
    typed: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl Handler for Stalling {
    fn peer(&self) -> Peer {
        Peer { client_id: None, scope: Scope::HostAdmin }
    }

    async fn handle(&self, req: Request) -> Response {
        if let Some(pb::request::Payload::TerminalWrite(w)) = &req.payload {
            locked(&self.typed).extend_from_slice(&w.payload);
        }
        let value = if req.method == "changes.file_diff" {
            self.release.notified().await;
            result::Value::FileDiff(pb::FileDiff::default())
        } else {
            result::Value::Empty(pb::Empty {})
        };
        Response {
            request_id: req.request_id,
            outcome: Some(response::Outcome::Result(pb::Result { value: Some(value) })),
        }
    }
}

/// Serve one connection on `socket` with `Stalling`. Aborting the handle
/// drops the connection, as a runner that went away does.
async fn a_runner_that_stalls_diffs(
    socket: &std::path::Path,
    release: Arc<tokio::sync::Notify>,
    typed: Arc<std::sync::Mutex<Vec<u8>>>,
) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::UnixListener::bind(socket).expect("bind");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else { return };
        let (read, write) = stream.into_split();
        let mut conn = Connection::new(read, write);
        let cfg = HandshakeConfig { daemon_version: "stalling".into() };
        let _ = serve_connection(&mut conn, &cfg, &Stalling { release, typed }).await;
    })
}

/// A handle connected to a `Stalling` runner.
struct Rig {
    handle: *mut c_void,
    release: Arc<tokio::sync::Notify>,
    typed: Arc<std::sync::Mutex<Vec<u8>>>,
    runner: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let handle = farcooler_client_new();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("runner.sock");
    let release = Arc::new(tokio::sync::Notify::new());
    let typed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let h = unsafe { as_handle(handle) }.unwrap();
    let (runner, session) = h.runtime.block_on(async {
        let runner = a_runner_that_stalls_diffs(&socket, Arc::clone(&release), Arc::clone(&typed)).await;
        (runner, Session::connect_local(&socket).await.expect("connect"))
    });
    h.put_session(session);
    Rig { handle, release, typed, runner, _dir: dir }
}

fn call(handle: *mut c_void, method: &str, args: Value) -> u64 {
    let method = std::ffi::CString::new(method).unwrap();
    let args = std::ffi::CString::new(args.to_string()).unwrap();
    unsafe { farcooler_client_call(handle, method.as_ptr(), args.as_ptr()) }
}

fn a_diff(handle: *mut c_void) -> u64 {
    call(handle, "changes.file_diff", json!({ "worktree": uuid::Uuid::now_v7().to_string(), "path": "big.rs" }))
}

/// Whether `line` answers `ticket`.
fn is_for(line: &str, ticket: u64) -> bool {
    serde_json::from_str::<Value>(line).ok().and_then(|v| v["ticket"].as_u64()) == Some(ticket)
}

/// The first line on `handle`'s queue for `ticket`, or `None` by `deadline`.
fn answer_for(handle: *mut c_void, ticket: u64, deadline: Instant) -> Option<String> {
    while Instant::now() < deadline {
        if let Some(line) = unsafe { farcooler_client_poll(handle).as_ref() } {
            let line = unsafe { CStr::from_ptr(line) }.to_str().unwrap().to_string();
            if is_for(&line, ticket) {
                return Some(line);
            }
            continue;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    None
}

#[test]
fn a_keystroke_is_answered_while_a_slow_diff_is_outstanding() {
    let rig = rig();
    let terminal = uuid::Uuid::now_v7();

    let slow = a_diff(rig.handle);
    // Let the diff reach the runner first, so the keystroke really is behind it.
    std::thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    let typed = call(rig.handle, "terminal.write", json!({ "terminal": terminal.to_string(), "hex": "6c73" }));

    let answer = answer_for(rig.handle, typed, started + Duration::from_secs(3));
    let took = started.elapsed();
    let answer = answer.expect("the keystroke waited behind the diff");
    assert!(answer.contains("\"ok\":true"), "got {answer}");
    assert!(took < Duration::from_secs(1), "the keystroke took {took:?}");

    // The diff is still outstanding, and still answered once released.
    rig.release.notify_one();
    let diff = answer_for(rig.handle, slow, Instant::now() + Duration::from_secs(5)).expect("the diff answered");
    assert!(diff.contains("\"ok\":true"), "got {diff}");

    unsafe { farcooler_client_free(rig.handle) };
}

#[test]
fn keys_arrive_in_the_order_they_were_sent() {
    // Calls no longer wait for each other, so nothing but the queueing in
    // `calls::queue` keeps two keys from racing to the wire.
    let rig = rig();
    let terminal = uuid::Uuid::now_v7().to_string();
    let sent: Vec<u8> = (0..400u32).map(|i| b'a' + (i % 26) as u8).collect();
    let mut last = 0;
    for byte in &sent {
        last = call(rig.handle, "terminal.write", json!({ "terminal": terminal, "hex": format!("{byte:02x}") }));
    }
    // Answers come back in any order; the last key's is not the last answer.
    let deadline = Instant::now() + Duration::from_secs(10);
    while locked(&rig.typed).len() < sent.len() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_ne!(last, 0);
    assert_eq!(String::from_utf8(locked(&rig.typed).clone()).unwrap(), String::from_utf8(sent).unwrap());
    unsafe { farcooler_client_free(rig.handle) };
}

#[test]
fn a_runner_that_goes_away_fails_every_waiting_call_promptly() {
    let rig = rig();
    let first = a_diff(rig.handle);
    let second = a_diff(rig.handle);
    std::thread::sleep(Duration::from_millis(50));

    rig.runner.abort();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut answers = vec![];
    while answers.len() < 2 && Instant::now() < deadline {
        if let Some(line) = unsafe { farcooler_client_poll(rig.handle).as_ref() } {
            answers.push(unsafe { CStr::from_ptr(line) }.to_str().unwrap().to_string());
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    assert_eq!(answers.len(), 2, "a waiting call outlived its runner: {answers:?}");
    for line in &answers {
        assert!(line.contains("\"disconnected\":true"), "got {line}");
    }
    assert!(answers.iter().any(|l| is_for(l, first)));
    assert!(answers.iter().any(|l| is_for(l, second)));
    assert!(!unsafe { farcooler_client_connected(rig.handle) }, "the dead session is still in the slot");
    unsafe { farcooler_client_free(rig.handle) };
}

#[test]
fn a_missed_deadline_crosses_as_its_own_flag_and_not_a_drop() {
    let queue = Arc::new(std::sync::Mutex::new(VecDeque::new()));
    let late = SessionError::TimedOut { method: "changes.file_diff".into(), after: Duration::from_secs(120) };
    push_call(&queue, 7, Err(Lost::Call(late)), false);
    push_call(&queue, 8, Err(Lost::Call(SessionError::Protocol("x".into()))), false);
    let lines: Vec<Value> = locked(&queue).iter().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines[0]["timed_out"], true, "{}", lines[0]);
    assert_eq!(lines[0]["disconnected"], false);
    assert!(lines[0].get("code").is_none(), "no runner refused anything");
    assert!(lines[1].get("timed_out").is_none(), "absent unless true: {}", lines[1]);
}
