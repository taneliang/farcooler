//! One connection, several requests at once (ov-118).
//!
//! `serve_connection` used to await each request before reading the next, so a
//! slow call froze everything behind it — a phone's keystrokes waited out a
//! diff. These pin the two halves of what replaced it: a slow request holds up
//! nothing that names a different target, and requests that name the SAME
//! target still run one at a time, in the order they arrived.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use farcooler_protocol::PROTOCOL_VERSION;
use farcooler_protocol::v1::{self, Request, Response, WireEnvelope, wire_envelope};
use farcooler_transport::{Connection, Handler, HandshakeConfig, Peer, serve_connection};

/// `slow` never finishes until the test says so; `terminal.write` finishes after
/// however many milliseconds its payload names, and records that it ran.
#[derive(Clone)]
struct Handlers {
    /// What each `terminal.write` typed, in the order the writes FINISHED.
    typed: Arc<Mutex<Vec<String>>>,
    /// Zero permits until the test releases `slow`.
    slow: Arc<tokio::sync::Semaphore>,
    /// Closed to end the connection from above, as revoking a device does.
    revoked: Arc<tokio::sync::Semaphore>,
    /// What `terminal.create` got done: a row, then a window.
    created: Arc<Mutex<Vec<&'static str>>>,
}

impl Default for Handlers {
    fn default() -> Self {
        Self {
            typed: Arc::default(),
            slow: Arc::new(tokio::sync::Semaphore::new(0)),
            revoked: Arc::new(tokio::sync::Semaphore::new(0)),
            created: Arc::default(),
        }
    }
}

impl Handler for Handlers {
    fn peer(&self) -> Peer {
        Peer { client_id: None, scope: v1::Scope::Control }
    }

    fn closed(&self) -> impl std::future::Future<Output = ()> + Send {
        let revoked = self.revoked.clone();
        async move {
            let _ = revoked.acquire().await;
        }
    }

    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        let this = self.clone();
        async move {
            match req.method.as_str() {
                "slow" => {
                    let _ = this.slow.acquire().await;
                }
                "terminal.write" => {
                    let Some(v1::request::Payload::TerminalWrite(write)) = req.payload else {
                        panic!("a write with no payload");
                    };
                    let text = String::from_utf8(write.payload.to_vec()).unwrap();
                    let (delay, _) = text.split_once(':').unwrap();
                    tokio::time::sleep(Duration::from_millis(delay.parse().unwrap())).await;
                    this.typed.lock().unwrap().push(text);
                }
                // The shape of `Service::open_terminal`: a store row, an await,
                // then the tmux window. Cut between the two, a row with no
                // window is left behind.
                "terminal.create" => {
                    this.created.lock().unwrap().push("row");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    this.created.lock().unwrap().push("window");
                }
                other => panic!("unexpected method {other}"),
            }
            Response {
                request_id: req.request_id,
                outcome: Some(v1::response::Outcome::Result(v1::Result { value: None })),
            }
        }
    }
}

fn envelope(method: &str, target: Option<&'static [u8]>, payload: v1::request::Payload) -> (Bytes, WireEnvelope) {
    let request_id = farcooler_protocol::ids::new_id();
    let envelope = WireEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_id: farcooler_protocol::ids::new_id(),
        body: Some(wire_envelope::Body::Request(Request {
            request_id: request_id.clone(),
            method: method.into(),
            target_resource_id: target.map(Bytes::from_static),
            expected_resource_version: None,
            expected_lease_generation: None,
            idempotency_key: None,
            required_capabilities: Vec::new(),
            payload: Some(payload),
        })),
    };
    (request_id, envelope)
}

fn write(target: &'static [u8], text: &str) -> (Bytes, WireEnvelope) {
    envelope(
        "terminal.write",
        Some(target),
        v1::request::Payload::TerminalWrite(v1::TerminalWrite { payload: Bytes::from(text.to_string()) }),
    )
}

fn slow(target: Option<&'static [u8]>) -> (Bytes, WireEnvelope) {
    envelope("slow", target, v1::request::Payload::Empty(v1::Empty {}))
}

/// A handshaken client on a pipe to a server running `handlers`.
async fn connect(
    handlers: Handlers,
) -> Connection<tokio::io::ReadHalf<tokio::io::DuplexStream>> {
    connect_serving(handlers).await.0
}

/// `connect`, and the task serving it, which finishes when the server is done
/// with the connection.
async fn connect_serving(
    handlers: Handlers,
) -> (Connection<tokio::io::ReadHalf<tokio::io::DuplexStream>>, tokio::task::JoinHandle<()>) {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let serving = tokio::spawn(async move {
        let (r, w) = tokio::io::split(server_io);
        let mut conn = Connection::new(r, w);
        let cfg = HandshakeConfig { daemon_version: "t".into() };
        let _ = serve_connection(&mut conn, &cfg, &handlers).await;
    });
    let (r, w) = tokio::io::split(client_io);
    let mut client = Connection::new(r, w);
    client.client_handshake("itest", "0").await.unwrap();
    (client, serving)
}

/// The id of the next response to arrive.
async fn answered(client: &mut Connection<tokio::io::ReadHalf<tokio::io::DuplexStream>>) -> Bytes {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(2), client.recv())
            .await
            .expect("no answer within two seconds")
            .unwrap();
        if let Some(wire_envelope::Body::Response(response)) = frame.body {
            return response.request_id;
        }
    }
}

/// The finding in one test: a request that never finishes, then a keystroke.
/// The keystroke is answered. Before, it waited behind the slow call forever —
/// whether that call named another terminal or no target at all.
#[tokio::test]
async fn a_slow_request_does_not_hold_up_a_write_to_a_terminal() {
    let handlers = Handlers::default();
    let mut client = connect(handlers.clone()).await;

    client.send(&slow(None).1).await.unwrap();
    client.send(&slow(Some(b"another-pane")).1).await.unwrap();
    let (keystroke, frame) = write(b"pane", "0:ls");
    client.send(&frame).await.unwrap();

    assert_eq!(answered(&mut client).await, keystroke, "the write is the first thing answered");
    assert_eq!(*handlers.typed.lock().unwrap(), ["0:ls"]);
}

/// Writes to one terminal land in the order they were sent, even when the
/// first one takes longer than the second. Run concurrently without a lane per
/// target, the second finishes first: `l` then `s` types `sl`.
#[tokio::test]
async fn writes_to_one_terminal_keep_their_order() {
    let handlers = Handlers::default();
    let mut client = connect(handlers.clone()).await;

    let (first, frame) = write(b"pane", "150:l");
    client.send(&frame).await.unwrap();
    let (second, frame) = write(b"pane", "0:s");
    client.send(&frame).await.unwrap();

    assert_eq!(answered(&mut client).await, first);
    assert_eq!(answered(&mut client).await, second);
    assert_eq!(*handlers.typed.lock().unwrap(), ["150:l", "0:s"]);
}

/// And the lane is per target, not per connection: a slow write to one pane
/// does not hold up a write to another.
#[tokio::test]
async fn a_slow_write_to_one_terminal_does_not_hold_up_another() {
    let handlers = Handlers::default();
    let mut client = connect(handlers.clone()).await;

    let (_, frame) = write(b"pane-a", "1000:slow");
    client.send(&frame).await.unwrap();
    let (quick, frame) = write(b"pane-b", "0:quick");
    client.send(&frame).await.unwrap();

    assert_eq!(answered(&mut client).await, quick);
}

fn create() -> (Bytes, WireEnvelope) {
    envelope("terminal.create", Some(b"worktree"), v1::request::Payload::Empty(v1::Empty {}))
}

/// **A request that started finishes, even when its client vanishes** (ov-118
/// review). Requests in flight used to be dropped with the loop the moment the
/// connection ended, so a phone that lost signal mid-`terminal.create` left a
/// store row with no window. The connection is dropped right after the request
/// is sent; the create still gets both halves done.
#[tokio::test]
async fn a_request_whose_client_vanished_is_finished_not_abandoned() {
    let handlers = Handlers::default();
    let (mut client, serving) = connect_serving(handlers.clone()).await;
    client.send(&create().1).await.unwrap();
    // Until the handler has started, there is nothing in flight to lose.
    tokio::time::timeout(Duration::from_secs(2), async {
        while handlers.created.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the create never started");
    drop(client);

    tokio::time::timeout(Duration::from_secs(2), serving).await.expect("the server never let go").unwrap();
    assert_eq!(*handlers.created.lock().unwrap(), ["row", "window"], "a row with no window");
}

/// A client that sends its request and closes its write half still gets the
/// answer: the end of its requests is not the end of its interest in them.
#[tokio::test]
async fn a_client_that_half_closes_is_still_answered() {
    use farcooler_transport::{FrameReader, FrameWriter};

    let handlers = Handlers::default();
    let (server_io, client_io) = tokio::net::UnixStream::pair().unwrap();
    let serving_handlers = handlers.clone();
    tokio::spawn(async move {
        let (r, w) = server_io.into_split();
        let mut conn = Connection::new(r, w);
        let cfg = HandshakeConfig { daemon_version: "t".into() };
        let _ = serve_connection(&mut conn, &cfg, &serving_handlers).await;
    });

    let (r, w) = client_io.into_split();
    let mut reader = FrameReader::new(r);
    let mut writer = FrameWriter::new(w);
    writer
        .write_frame(&WireEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: farcooler_protocol::ids::new_id(),
            body: Some(wire_envelope::Body::ClientHello(v1::ClientHello {
                supported_protocol_versions: vec![PROTOCOL_VERSION],
                client_name: "half".into(),
                client_version: "0".into(),
            })),
        })
        .await
        .unwrap();
    reader.read_frame().await.unwrap().expect("a server hello");

    let (id, frame) = create();
    writer.write_frame(&frame).await.unwrap();
    writer.shutdown().await.unwrap();

    let answer = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match reader.read_frame().await.unwrap() {
                Some(WireEnvelope { body: Some(wire_envelope::Body::Response(r)), .. }) => return Some(r),
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await
    .expect("no answer within two seconds");
    assert_eq!(answer.expect("closed without an answer").request_id, id);
}

/// **Revoking a device serves it nothing new** (ov-118 review). A request is
/// running when the connection is closed from above. Nothing sent after the
/// close is answered, nothing queued behind the running request is started,
/// and the running one is finished — unanswered — before the server lets go.
#[tokio::test]
async fn a_revoked_connection_finishes_what_ran_and_starts_nothing_else() {
    let handlers = Handlers::default();
    let (mut client, serving) = connect_serving(handlers.clone()).await;

    let (slow_id, frame) = slow(Some(b"pane"));
    client.send(&frame).await.unwrap();
    // Queued behind the slow one: same target.
    client.send(&write(b"pane", "0:queued").1).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    handlers.revoked.close();
    tokio::time::sleep(Duration::from_millis(50)).await;
    client.send(&write(b"other", "0:after").1).await.unwrap();

    let early = tokio::time::timeout(Duration::from_millis(300), client.recv()).await;
    assert!(early.is_err(), "a revoked connection answered something: {early:?}");
    assert!(!serving.is_finished(), "the running request was abandoned, not finished");

    handlers.slow.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), serving).await.expect("the server never let go").unwrap();
    assert!(handlers.typed.lock().unwrap().is_empty(), "served after the close: {:?}", handlers.typed.lock().unwrap());
    // Whatever arrives now is not an answer to the slow request.
    while let Ok(Ok(frame)) = tokio::time::timeout(Duration::from_millis(50), client.recv()).await {
        if let Some(wire_envelope::Body::Response(r)) = frame.body {
            assert_ne!(r.request_id, slow_id, "a revoked device was answered");
        }
    }
}

/// At `MAX_REQUESTS_IN_FLIGHT` the connection stops reading: a 33rd request
/// waits in the socket until one of the 32 finishes, then is served.
#[tokio::test]
async fn the_thirty_third_request_waits_for_one_of_the_thirty_two() {
    const TARGETS: [&[u8]; 32] = [
        b"t00", b"t01", b"t02", b"t03", b"t04", b"t05", b"t06", b"t07", b"t08", b"t09", b"t10",
        b"t11", b"t12", b"t13", b"t14", b"t15", b"t16", b"t17", b"t18", b"t19", b"t20", b"t21",
        b"t22", b"t23", b"t24", b"t25", b"t26", b"t27", b"t28", b"t29", b"t30", b"t31",
    ];
    assert_eq!(TARGETS.len(), farcooler_transport::connection::MAX_REQUESTS_IN_FLIGHT);

    let handlers = Handlers::default();
    let mut client = connect(handlers.clone()).await;
    for target in TARGETS {
        client.send(&slow(Some(target)).1).await.unwrap();
    }
    let (keystroke, frame) = write(b"pane", "0:ls");
    client.send(&frame).await.unwrap();

    let early = tokio::time::timeout(Duration::from_millis(300), client.recv()).await;
    assert!(early.is_err(), "the 33rd request was read while 32 were running");
    assert!(handlers.typed.lock().unwrap().is_empty());

    handlers.slow.add_permits(1);
    let mut answered_ids = Vec::new();
    while !answered_ids.contains(&keystroke) {
        answered_ids.push(answered(&mut client).await);
    }
}
