//! The client's half of running calls at once (ov-147).
//!
//! `serve_connection` has answered requests concurrently since ov-118, but
//! `Client::call` wrote one request and then read until its own answer came,
//! so nothing sharing a connection could send a second request until the first
//! was answered. These pin what replaced it: a reader task that files each
//! answer under its request id, per-call deadlines, urgent requests first,
//! cancellation, and a disconnect that fails every waiting call at once.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use farcooler_protocol::PROTOCOL_VERSION;
use farcooler_protocol::v1::{self, Request, Response, WireEnvelope, response, result, wire_envelope};
use farcooler_transport::{
    CallOptions, Client, ClientError, Connection, FrameReader, FrameWriter, Handler, HandshakeConfig,
    Peer, request, serve_connection,
};

type Duplex = tokio::io::DuplexStream;
type TestClient = Client<tokio::io::ReadHalf<Duplex>, tokio::io::WriteHalf<Duplex>>;
type Reads = FrameReader<tokio::io::ReadHalf<Duplex>>;
type Writes = FrameWriter<tokio::io::WriteHalf<Duplex>>;
type Script = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// Answers every call at once with its own method name, except `slow`, which
/// waits until the test releases it.
#[derive(Clone)]
struct Runner {
    slow: Arc<tokio::sync::Semaphore>,
}

impl Handler for Runner {
    fn peer(&self) -> Peer {
        Peer { client_id: None, scope: v1::Scope::Control }
    }

    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        let slow = self.slow.clone();
        async move {
            if req.method == "slow" {
                let _ = slow.acquire().await;
            } else if let Some(ms) = req.method.strip_prefix("sleep:").and_then(|r| r.split(':').next()) {
                tokio::time::sleep(Duration::from_millis(ms.parse().unwrap())).await;
            }
            answer(req.request_id, &req.method)
        }
    }
}

/// An answer that says which request it answers, so a misfiled one shows.
fn answer(request_id: Bytes, method: &str) -> Response {
    Response {
        request_id,
        outcome: Some(response::Outcome::Result(v1::Result {
            value: Some(result::Value::Host(v1::Host { platform: method.to_string(), ..Default::default() })),
        })),
    }
}

/// A request naming `target`. The daemon runs requests naming the same
/// target — or none — one at a time, so a test of concurrency gives each
/// call a target of its own, as a keystroke (its pane) and a diff (none) have.
fn aimed(method: &str, target: &str) -> Request {
    let mut req = request(method);
    req.target_resource_id = Some(Bytes::copy_from_slice(target.as_bytes()));
    req
}

fn answered_as(outcome: Result<v1::Result, ClientError>) -> String {
    match outcome.expect("answered").value {
        Some(result::Value::Host(host)) => host.platform,
        other => panic!("not a Host: {other:?}"),
    }
}

/// A client connected to `Runner` through `serve_connection`, the daemon's own
/// loop, and the semaphore that releases `slow`.
async fn connected() -> (TestClient, Arc<tokio::sync::Semaphore>) {
    let slow = Arc::new(tokio::sync::Semaphore::new(0));
    let runner = Runner { slow: slow.clone() };
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let (r, w) = tokio::io::split(server_io);
        let mut conn = Connection::new(r, w);
        let cfg = HandshakeConfig { daemon_version: "t".into() };
        let _ = serve_connection(&mut conn, &cfg, &runner).await;
    });
    let (r, w) = tokio::io::split(client_io);
    (Client::over(r, w, "test", "0").await.expect("handshake"), slow)
}

/// A hand-rolled runner for what `serve_connection` would never do: answer an
/// id it was not sent, or drop the connection with calls outstanding. Says
/// hello, then hands every request to `script` along with the writer.
async fn scripted<F, Fut>(script: F) -> TestClient
where
    F: FnOnce(Reads, Writes) -> Fut
        + Send
        + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let (r, w) = tokio::io::split(server_io);
        let mut reader = FrameReader::new(r);
        let mut writer = FrameWriter::new(w);
        let Ok(Some(_hello)) = reader.read_frame().await else { return };
        let hello = envelope(wire_envelope::Body::ServerHello(v1::ServerHello {
            selected_protocol_version: PROTOCOL_VERSION,
            daemon_version: "scripted".into(),
            ..Default::default()
        }));
        if writer.write_frame(&hello).await.is_ok() {
            script(reader, writer).await;
        }
    });
    let (r, w) = tokio::io::split(client_io);
    Client::over(r, w, "test", "0").await.expect("handshake")
}

fn envelope(body: wire_envelope::Body) -> WireEnvelope {
    WireEnvelope { protocol_version: PROTOCOL_VERSION, message_id: farcooler_protocol::ids::new_id(), body: Some(body) }
}

async fn next_request(reader: &mut Reads) -> Option<Request> {
    match reader.read_frame().await.ok()??.body {
        Some(wire_envelope::Body::Request(req)) => Some(req),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_call_does_not_delay_a_fast_one() {
    let (client, slow) = connected().await;

    let slow_call = client.call_with(request("slow"), CallOptions::default());
    let fast_call = async {
        // Behind the slow one on the wire, so it really is queued after it.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let started = Instant::now();
        let outcome = client.call_with(aimed("fast", "a pane"), CallOptions::default()).await;
        let took = started.elapsed();
        // Released only now: the slow call is still outstanding until here.
        slow.add_permits(1);
        (answered_as(outcome), took)
    };
    // Bounded, so a client that queues the fast call behind the slow one
    // fails here rather than hanging: the slow one is released only after
    // the fast one is answered.
    let (slow_answer, (fast_answer, took)) =
        tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(slow_call, fast_call) })
            .await
            .expect("the fast call waited behind the slow one");

    assert_eq!(fast_answer, "fast");
    assert!(took < Duration::from_millis(500), "the fast call waited {took:?} behind the slow one");
    assert_eq!(answered_as(slow_answer), "slow", "and the slow one still got its own answer");
}

#[tokio::test]
async fn a_reply_for_an_unknown_id_is_dropped_and_counted() {
    let client = scripted(|mut reader, mut writer| async move {
        let Some(req) = next_request(&mut reader).await else { return };
        // An answer to something nobody asked, first, then the real one.
        let stray = answer(farcooler_protocol::ids::new_id(), "nobody's");
        let _ = writer.write_frame(&envelope(wire_envelope::Body::Response(stray))).await;
        let real = answer(req.request_id, &req.method);
        let _ = writer.write_frame(&envelope(wire_envelope::Body::Response(real))).await;
        // Kept open, so a dropped connection is not what ends the call.
        std::future::pending::<()>().await;
    })
    .await;

    let outcome = client.call_with(request("mine"), CallOptions::default()).await;
    assert_eq!(answered_as(outcome), "mine", "the stray was not handed to the waiting caller");
    assert_eq!(client.stray_replies(), 1);
}

#[tokio::test]
async fn a_deadline_fires_and_leaves_the_connection_working() {
    let (client, slow) = connected().await;

    let started = Instant::now();
    let how = CallOptions { deadline: Some(Duration::from_millis(100)), urgent: false };
    let outcome = tokio::time::timeout(Duration::from_secs(5), client.call_with(request("slow"), how))
        .await
        .expect("the deadline never fired");
    let took = started.elapsed();
    match outcome {
        Err(ClientError::TimedOut { method, after }) => {
            assert_eq!(method, "slow");
            assert_eq!(after, Duration::from_millis(100));
        }
        other => panic!("expected a timeout, got {other:?}"),
    }
    assert!(took < Duration::from_secs(2), "the deadline fired after {took:?}");
    let shown = ClientError::TimedOut { method: "slow".into(), after: Duration::from_millis(100) };
    assert_eq!(shown.to_string(), "slow got no answer within 100ms");

    // The answer that comes after the caller gave up is for nobody.
    slow.add_permits(1);
    let outcome = client.call_with(aimed("after", "a pane"), CallOptions::default()).await;
    assert_eq!(answered_as(outcome), "after", "one slow answer is not a dead link");
    assert_eq!(client.stray_replies(), 1, "the late answer was dropped, not misfiled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnect_fails_every_waiting_call_at_once() {
    let client = scripted(|mut reader, writer| async move {
        // Take three requests, answer none, and hang up.
        for _ in 0..3 {
            if next_request(&mut reader).await.is_none() {
                return;
            }
        }
        drop(writer);
        drop(reader);
    })
    .await;

    let started = Instant::now();
    // No deadline: only the disconnect can end these.
    let call = |name: &'static str| client.call_with(request(name), CallOptions::default());
    let (a, b, c) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(call("a"), call("b"), call("c"))
    })
    .await
    .expect("a pending call hung after the connection dropped");
    for outcome in [a, b, c] {
        assert!(matches!(outcome, Err(ClientError::Closed)), "got {outcome:?}");
    }
    assert!(started.elapsed() < Duration::from_secs(2));

    // And a call made afterwards fails before it is sent.
    let after = client.call_with(request("late"), CallOptions::default()).await;
    assert!(matches!(after, Err(ClientError::Closed)), "got {after:?}");
}

/// What a scripted runner saw, in the order it arrived.
fn recording() -> (Arc<Mutex<Vec<String>>>, impl FnOnce(Reads, Writes) -> Script) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = seen.clone();
    let script = move |mut reader: Reads, mut writer: Writes| -> Script {
        Box::pin(async move {
            while let Some(req) = next_request(&mut reader).await {
                kept.lock().unwrap().push(req.method.clone());
                let reply = answer(req.request_id, &req.method);
                if writer.write_frame(&envelope(wire_envelope::Body::Response(reply))).await.is_err() {
                    return;
                }
            }
        })
    };
    (seen, script)
}

#[tokio::test]
async fn a_call_given_up_before_it_was_sent_is_never_sent() {
    let (seen, script) = recording();
    let client = scripted(script).await;

    // On this single-threaded runtime the writer has not run yet, so the
    // request is still queued when its caller gives up.
    drop(client.send(request("abandoned"), CallOptions::default()).expect("queued"));
    let outcome = client.call_with(request("kept"), CallOptions::default()).await;
    assert_eq!(answered_as(outcome), "kept");
    assert_eq!(*seen.lock().unwrap(), vec!["kept".to_string()]);
}

#[tokio::test]
async fn urgent_requests_go_ahead_of_ordinary_ones_still_queued() {
    let (seen, script) = recording();
    let client = scripted(script).await;

    let urgent = CallOptions { deadline: None, urgent: true };
    let plain = CallOptions::default();
    // All queued before the writer runs; the order sent is the order the
    // writer chose.
    let a = client.send(request("diff"), plain).unwrap();
    let b = client.send(request("key l"), urgent).unwrap();
    let c = client.send(request("log"), plain).unwrap();
    let d = client.send(request("key s"), urgent).unwrap();
    for answer in [a, b, c, d] {
        answer.answer().await.expect("answered");
    }
    assert_eq!(*seen.lock().unwrap(), vec!["key l", "key s", "diff", "log"]);
}

/// Many calls at once, of every speed, each answered by its own answer.
///
/// The one to run repeatedly: a misfiled answer, a lost one or a hang shows up
/// here as a wrong name, a missing one or the timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_calls_at_once_each_get_their_own_answer() {
    let (client, slow) = connected().await;
    let client = Arc::new(client);

    let mut running = tokio::task::JoinSet::new();
    for i in 0..400u64 {
        let client = client.clone();
        running.spawn(async move {
            let name = format!("sleep:{}:{i}", (i * 7919) % 23);
            let how = CallOptions { deadline: Some(Duration::from_secs(20)), urgent: i % 5 == 0 };
            let outcome = client.call_with(aimed(&name, &name), how).await;
            (name, outcome)
        });
    }
    let slow_ones: Vec<_> = (0..8)
        .map(|i| {
            let client = client.clone();
            let slow = aimed("slow", &format!("slow {i}"));
            tokio::spawn(async move { client.call_with(slow, CallOptions::default()).await })
        })
        .collect();

    let finished = tokio::time::timeout(Duration::from_secs(30), async {
        let mut n = 0;
        while let Some(joined) = running.join_next().await {
            let (name, outcome) = joined.unwrap();
            assert_eq!(answered_as(outcome), name);
            n += 1;
        }
        n
    })
    .await
    .expect("calls hung");
    assert_eq!(finished, 400);

    slow.add_permits(8);
    for call in slow_ones {
        assert_eq!(answered_as(call.await.unwrap()), "slow");
    }
    assert_eq!(client.stray_replies(), 0);
}

fn an_event(i: u64) -> WireEnvelope {
    envelope(wire_envelope::Body::Event(v1::Event { event_id: Bytes::from(i.to_be_bytes().to_vec()), ..Default::default() }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_never_hold_up_an_answer() {
    // More events than the backlog holds, all ahead of the answer, read by
    // nobody until the call returns.
    let mut client = scripted(|mut reader, mut writer| async move {
        let Some(req) = next_request(&mut reader).await else { return };
        for i in 0..3000 {
            if writer.write_frame(&an_event(i)).await.is_err() {
                return;
            }
        }
        let reply = answer(req.request_id, &req.method);
        let _ = writer.write_frame(&envelope(wire_envelope::Body::Response(reply))).await;
        std::future::pending::<()>().await;
    })
    .await;

    let outcome = tokio::time::timeout(Duration::from_secs(5), client.call_with(request("mine"), CallOptions::default()))
        .await
        .expect("the answer was held up behind unread events");
    assert_eq!(answered_as(outcome), "mine");
    // And none of them was lost, or reordered.
    for i in 0..3000u64 {
        let event = client.next_event().await.expect("an event");
        assert_eq!(event.event_id, Bytes::from(i.to_be_bytes().to_vec()));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unread_events_push_back_on_the_runner_between_calls() {
    // The pushback a terminal stream depends on: a viewer that stops reading
    // leaves the backlog with the runner, where its limits see it, rather
    // than growing here without bound.
    let written = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counted = written.clone();
    let mut client = scripted(move |_reader, mut writer| async move {
        for i in 0..20_000 {
            if writer.write_frame(&an_event(i)).await.is_err() {
                return;
            }
            counted.store(i + 1, std::sync::atomic::Ordering::SeqCst);
        }
    })
    .await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    let stalled_at = written.load(std::sync::atomic::Ordering::SeqCst);
    assert!(stalled_at < 20_000, "nothing pushed back: the runner wrote all {stalled_at}");

    for i in 0..20_000u64 {
        let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
            .await
            .expect("the reader never resumed")
            .expect("an event");
        assert_eq!(event.event_id, Bytes::from(i.to_be_bytes().to_vec()));
    }
}
