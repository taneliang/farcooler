//! A runner's last word to a device it revoked reaches the phone's queue
//! (ov-820), and survives the queue overflowing.
//!
//! Against a stand-in runner on a Unix socket, served by the real
//! `serve_connection`, which says the farewell when the stand-in's gate
//! closes, as the daemon's session registry closes a revoked device's.

use std::sync::Arc;
use std::time::{Duration, Instant};

use farcooler_protocol::v1::{self as pb, Request, Response, Scope, response, result};
use farcooler_transport::{Connection, HandshakeConfig, Handler, Peer, serve_connection};

use super::*;

/// A runner every one of whose connections ends, with a farewell, when
/// `gate` closes.
#[derive(Clone)]
struct Revoking {
    gate: Arc<tokio::sync::Semaphore>,
}

impl Handler for Revoking {
    fn peer(&self) -> Peer {
        Peer { client_id: Some("phone".into()), scope: Scope::Control }
    }

    async fn handle(&self, req: Request) -> Response {
        Response {
            request_id: req.request_id,
            outcome: Some(response::Outcome::Result(pb::Result { value: Some(result::Value::Empty(pb::Empty {})) })),
        }
    }

    fn closed(&self) -> impl std::future::Future<Output = ()> + Send {
        let gate = Arc::clone(&self.gate);
        async move {
            let _ = gate.acquire().await;
        }
    }

    fn farewell(&self) -> Option<pb::Event> {
        Some(pb::Event { payload: Some(pb::event::Payload::DeviceReleased(pb::Empty {})), ..Default::default() })
    }
}

fn next_line(handle: *mut c_void) -> Option<String> {
    unsafe { farcooler_client_next_event(handle).as_ref() }
        .map(|line| unsafe { CStr::from_ptr(line) }.to_str().unwrap().to_string())
}

/// **The phone hears `released` from a runner closing its connections.** The
/// line its apps store, and turn into the no-access screen.
#[test]
fn a_revoked_phone_hears_it_was_released() {
    let handle = farcooler_client_new();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("runner.sock");
    let runner = Revoking { gate: Arc::new(tokio::sync::Semaphore::new(0)) };
    let h = unsafe { as_handle(handle) }.unwrap();
    let serving = runner.clone();
    h.runtime.block_on(async {
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let runner = serving.clone();
                tokio::spawn(async move {
                    let (read, write) = stream.into_split();
                    let mut conn = Connection::new(read, write);
                    let cfg = HandshakeConfig { daemon_version: "stand-in".into() };
                    let _ = serve_connection(&mut conn, &cfg, &runner).await;
                });
            }
        });
    });

    let config = std::ffi::CString::new(json!({ "socket": socket.display().to_string() }).to_string()).unwrap();
    let ticket = unsafe { farcooler_client_connect(handle, config.as_ptr()) };
    let answer = super::concurrency_tests::answer_for(handle, ticket, Instant::now() + Duration::from_secs(60))
        .expect("the connect answered");
    assert!(answer.contains("\"ok\":true"), "got {answer}");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !unsafe { farcooler_client_events_live(handle) } {
        assert!(Instant::now() < deadline, "the event channel never opened");
        std::thread::sleep(Duration::from_millis(5));
    }
    while next_line(handle).is_some() {}

    runner.gate.close();

    let deadline = Instant::now() + Duration::from_secs(60);
    let heard = loop {
        if let Some(line) = next_line(handle) {
            if line == r#"{"event":"released"}"# {
                break true;
            }
        }
        if Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(heard, "the phone never heard it was released");
    unsafe { farcooler_client_free(handle) };
    drop(dir);
}

/// **An overflowing queue keeps `released`.** Every other notice is a
/// re-read that the overflow's `resync` stands for; this one isn't, and a
/// phone that lost it would show a revoked runner as one that's asleep.
#[test]
fn an_overflow_keeps_released() {
    use crate::session::FleetEvent;
    let stack = || event_line(&FleetEvent::Stack { repository: uuid::Uuid::now_v7() });
    let released = event_line(&FleetEvent::Released);

    let mut held = EventQueue::default();
    held.push(released.clone());
    for _ in 0..EVENT_QUEUE_LIMIT + 10 {
        held.push(stack());
    }
    assert!(held.pending.contains(&released), "an overflow dropped a waiting released: {:?}", held.pending);

    let mut full = EventQueue::default();
    for _ in 0..EVENT_QUEUE_LIMIT {
        full.push(stack());
    }
    full.push(released.clone());
    assert_eq!(full.pending, [r#"{"event":"resync"}"#.to_string(), released], "the released that overflowed it was lost");
}

/// A reader that stopped costs a bounded amount of memory and loses no
/// news it could act on.
///
/// Overflow COLLAPSES rather than truncates, and that is the whole safety
/// argument: dropping the oldest would silently lose a worktree nobody
/// then re-reads, and a client left believing it is current is the one
/// outcome this queue must not produce. `resync` covers every notice that
/// was thrown away because every notice only ever meant "re-read".
#[test]
fn a_reader_that_stopped_overflows_into_one_resync() {
    use crate::session::FleetEvent;
    let mut queue = EventQueue::default();
    for _ in 0..EVENT_QUEUE_LIMIT + 10 {
        queue.push(event_line(&FleetEvent::Stack { repository: uuid::Uuid::now_v7() }));
    }
    assert!(queue.pending.len() <= EVENT_QUEUE_LIMIT);
    assert!(
        queue.pending.iter().any(|line| line == r#"{"event":"resync"}"#),
        "an overflowed queue has to say so: {:?}",
        queue.pending
    );
}
