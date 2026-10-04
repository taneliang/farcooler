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

/// What the stand-in runner is holding back, and what it has seen.
#[derive(Default)]
struct Stalls {
    /// Releases `changes.file_diff`.
    release: tokio::sync::Notify,
    /// Set once a diff has reached the runner.
    diffing: std::sync::atomic::AtomicBool,
    /// Releases `terminal.paste_file`.
    paste: tokio::sync::Notify,
    /// Set once a paste has reached the runner.
    pasting: std::sync::atomic::AtomicBool,
    /// What was typed, in the order it ran — for one pane, the order it
    /// arrived. A finished paste shows as `<paste>`.
    typed: std::sync::Mutex<Vec<u8>>,
}

/// A runner that answers everything at once, except a diff and a paste,
/// which wait until they are released.
struct Stalling(Arc<Stalls>);

impl Handler for Stalling {
    fn peer(&self) -> Peer {
        Peer { client_id: None, scope: Scope::HostAdmin }
    }

    async fn handle(&self, req: Request) -> Response {
        let stalls = &self.0;
        if let Some(pb::request::Payload::TerminalWrite(w)) = &req.payload {
            locked(&stalls.typed).extend_from_slice(&w.payload);
        }
        let value = match req.method.as_str() {
            "changes.file_diff" => {
                stalls.diffing.store(true, std::sync::atomic::Ordering::SeqCst);
                stalls.release.notified().await;
                result::Value::FileDiff(pb::FileDiff::default())
            }
            // The first chunk waits to be released. The last one "types" the
            // path, as the daemon does.
            "terminal.paste_file" => {
                let Some(pb::request::Payload::TerminalFilePut(put)) = &req.payload else {
                    panic!("a paste with no chunk");
                };
                if put.offset == 0 {
                    stalls.pasting.store(true, std::sync::atomic::Ordering::SeqCst);
                    stalls.paste.notified().await;
                }
                let stored = put.offset + put.chunk.len() as u64;
                let done = stored == put.total_size;
                if done {
                    locked(&stalls.typed).extend_from_slice(b"<paste>");
                }
                result::Value::TerminalFilePut(pb::TerminalFilePutResult {
                    stored,
                    path: done.then(|| "/tmp/pasted.png".to_string()),
                })
            }
            _ => result::Value::Empty(pb::Empty {}),
        };
        Response {
            request_id: req.request_id,
            outcome: Some(response::Outcome::Result(pb::Result { value: Some(value) })),
        }
    }
}

/// A handle connected to a `Stalling` runner. Aborting `runner` drops the
/// connection, as a runner that went away does.
struct Rig {
    handle: *mut c_void,
    stalls: Arc<Stalls>,
    runner: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let handle = farcooler_client_new();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("runner.sock");
    let stalls = Arc::new(Stalls::default());
    let h = unsafe { as_handle(handle) }.unwrap();
    let serving = Arc::clone(&stalls);
    let (runner, session) = h.runtime.block_on(async {
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        let runner = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else { return };
            let (read, write) = stream.into_split();
            let mut conn = Connection::new(read, write);
            let cfg = HandshakeConfig { daemon_version: "stalling".into() };
            let _ = serve_connection(&mut conn, &cfg, &Stalling(serving)).await;
        });
        (runner, Session::connect_local(&socket).await.expect("connect"))
    });
    h.put_session(session);
    Rig { handle, stalls, runner, _dir: dir }
}

/// Wait until `flag` is set. A bound for a hang, not a measurement.
fn until(flag: &std::sync::atomic::AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !flag.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the runner never saw it");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn call(handle: *mut c_void, method: &str, args: Value) -> u64 {
    let method = std::ffi::CString::new(method).unwrap();
    let args = std::ffi::CString::new(args.to_string()).unwrap();
    unsafe { farcooler_client_call(handle, method.as_ptr(), args.as_ptr()) }
}

fn a_diff(handle: *mut c_void) -> u64 {
    call(handle, "changes.file_diff", json!({ "worktree": uuid::Uuid::now_v7().to_string(), "path": "big.rs" }))
}

/// Whether `line` answers `ticket` — the answer, not a paste's progress.
fn is_for(line: &str, ticket: u64) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return false };
    v["ticket"].as_u64() == Some(ticket) && v.get("progress").is_none()
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
    // Not typed until the runner is working on the diff, so the key really is
    // behind it.
    until(&rig.stalls.diffing);
    let typed = call(rig.handle, "terminal.write", json!({ "terminal": terminal.to_string(), "hex": "6c73" }));

    // No clock in the verdict: the diff is released only after the key is
    // answered, so a key queued behind it is never answered. The bound turns
    // that into a failure, and is generous because reaching it IS the failure.
    let answer = answer_for(rig.handle, typed, Instant::now() + Duration::from_secs(60));
    let answer = answer.expect("the keystroke waited behind the diff");
    assert!(answer.contains("\"ok\":true"), "got {answer}");

    // The diff is still outstanding, and still answered once released.
    rig.stalls.release.notify_one();
    let diff = answer_for(rig.handle, slow, Instant::now() + Duration::from_secs(60)).expect("the diff answered");
    assert!(diff.contains("\"ok\":true"), "got {diff}");

    unsafe { farcooler_client_free(rig.handle) };
}

#[test]
fn a_key_typed_during_a_paste_arrives_after_it() {
    // The path a paste types is the end of the paste, and a key typed while
    // the image uploads was typed after it. The runner keeps one pane's
    // requests in order, but a paste is several: a key sent while the first
    // chunk is with the runner would land between two chunks, ahead of the
    // path.
    let rig = rig();
    let terminal = uuid::Uuid::now_v7();
    let image = vec![7u8; farcooler_protocol::PASTE_CHUNK_BYTES * 2 + 1];
    let pasted = unsafe {
        farcooler_client_paste_file(
            rig.handle,
            std::ffi::CString::new(terminal.to_string()).unwrap().as_ptr(),
            c"shot.png".as_ptr(),
            c"image/png".as_ptr(),
            image.as_ptr(),
            image.len(),
        )
    };
    until(&rig.stalls.pasting);
    let typed = call(rig.handle, "terminal.write", json!({ "terminal": terminal.to_string(), "hex": "78" }));
    rig.stalls.paste.notify_one();

    let mut waiting = vec![pasted, typed];
    let deadline = Instant::now() + Duration::from_secs(60);
    while !waiting.is_empty() {
        assert!(Instant::now() < deadline, "unanswered: {waiting:?}");
        match unsafe { farcooler_client_poll(rig.handle).as_ref() } {
            Some(line) => {
                let line = unsafe { CStr::from_ptr(line) }.to_str().unwrap();
                waiting.retain(|t| !is_for(line, *t));
            }
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    assert_eq!(String::from_utf8(locked(&rig.stalls.typed).clone()).unwrap(), "<paste>x");
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
    let deadline = Instant::now() + Duration::from_secs(60);
    while locked(&rig.stalls.typed).len() < sent.len() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_ne!(last, 0);
    assert_eq!(String::from_utf8(locked(&rig.stalls.typed).clone()).unwrap(), String::from_utf8(sent).unwrap());
    unsafe { farcooler_client_free(rig.handle) };
}

#[test]
fn a_runner_that_goes_away_fails_every_waiting_call_promptly() {
    let rig = rig();
    let first = a_diff(rig.handle);
    let second = a_diff(rig.handle);
    until(&rig.stalls.diffing);

    rig.runner.abort();
    // Well inside the diffs' own two-minute deadline, which is what they
    // would otherwise wait out.
    let deadline = Instant::now() + Duration::from_secs(60);
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

#[test]
fn only_a_call_with_no_session_says_it_was_never_sent() {
    let queue = Arc::new(std::sync::Mutex::new(VecDeque::new()));
    push_call(&queue, 1, Err(Lost::Already), true);
    push_call(&queue, 2, Err(Lost::Call(SessionError::Disconnected("gone".into()))), true);
    let late = SessionError::TimedOut { method: "terminal.write".into(), after: Duration::from_secs(15) };
    push_call(&queue, 3, Err(Lost::Call(late)), false);
    let lines: Vec<Value> = locked(&queue).iter().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines[0]["not_sent"], true, "{}", lines[0]);
    assert!(lines[1].get("not_sent").is_none(), "a dropped link may have carried it: {}", lines[1]);
    assert!(lines[2].get("not_sent").is_none(), "a deadline cannot unsend it: {}", lines[2]);
}
