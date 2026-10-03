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
}

impl Default for Handlers {
    fn default() -> Self {
        Self { typed: Arc::default(), slow: Arc::new(tokio::sync::Semaphore::new(0)) }
    }
}

impl Handler for Handlers {
    fn peer(&self) -> Peer {
        Peer { client_id: None, scope: v1::Scope::Control }
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
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let (r, w) = tokio::io::split(server_io);
        let mut conn = Connection::new(r, w);
        let cfg = HandshakeConfig { daemon_version: "t".into() };
        let _ = serve_connection(&mut conn, &cfg, &handlers).await;
    });
    let (r, w) = tokio::io::split(client_io);
    let mut client = Connection::new(r, w);
    client.client_handshake("itest", "0").await.unwrap();
    client
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
