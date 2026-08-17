//! End-to-end tests against the crate's public API: a real Unix socket in a
//! tempdir, the Hello handshake, rules 2/3 protocol-error closes, and
//! version negotiation. Rule 4 (backpressure) is exercised in
//! `connection.rs`'s own unit tests, where an injectable ceiling keeps the
//! test fast; here the socket-level plumbing is what's under test.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bytes::{BufMut, BytesMut};
use farcooler_protocol::PROTOCOL_VERSION;
use farcooler_protocol::v1::{self, Request, Response, WireEnvelope, wire_envelope};
use farcooler_transport::{Connection, ConnectionError, FrameReader, FrameWriter, Handler, HandshakeConfig, Peer, UnixListenerServer};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[derive(Clone, Default)]
struct RecordingHandler {
    calls: Arc<AtomicUsize>,
}

impl Handler for RecordingHandler {
    /// A local caller: no device named, and everything permitted. The scope
    /// reaches `ServerHello` from here, which is what the round-trip test's
    /// `granted_scope` assertion is reading.
    fn peer(&self) -> Peer {
        Peer { client_id: None, scope: v1::Scope::Control }
    }

    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let request_id = req.request_id.clone();
        async move { Response { request_id, outcome: Some(v1::response::Outcome::Result(v1::Result { value: None })) } }
    }
}

/// A handler something outside the connection can end.
///
/// The shape the daemon's session registry has, reduced to the part transport
/// is responsible for: `closed` resolves, and the connection stops.
#[derive(Clone)]
struct ClosableHandler {
    inner: RecordingHandler,
    /// Zero permits, so awaiting it can only finish by the semaphore closing.
    gate: Arc<tokio::sync::Semaphore>,
}

impl ClosableHandler {
    fn new() -> Self {
        Self { inner: RecordingHandler::default(), gate: Arc::new(tokio::sync::Semaphore::new(0)) }
    }
}

impl Handler for ClosableHandler {
    fn peer(&self) -> Peer {
        Peer { client_id: Some("phone".into()), scope: v1::Scope::Read }
    }

    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        self.inner.handle(req)
    }

    fn closed(&self) -> impl std::future::Future<Output = ()> + Send {
        let gate = self.gate.clone();
        async move {
            let _ = gate.acquire().await;
        }
    }
}

fn handshake_cfg() -> HandshakeConfig {
    HandshakeConfig { daemon_version: "test-daemon".into() }
}

fn request_envelope(method: &str) -> WireEnvelope {
    // Uses `uuid` directly (rather than the protocol crate's own id helper)
    // to exercise the crate's declared dependency on it, not just its
    // transitive presence via farcooler-protocol.
    let request_id = farcooler_protocol::ids::to_bytes(uuid::Uuid::now_v7());
    WireEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_id: farcooler_protocol::ids::new_id(),
        body: Some(wire_envelope::Body::Request(Request {
            request_id,
            method: method.into(),
            target_resource_id: None,
            expected_resource_version: None,
            expected_lease_generation: None,
            idempotency_key: None,
            required_capabilities: Vec::new(),
            payload: Some(v1::request::Payload::Empty(v1::Empty {})),
        })),
    }
}

#[tokio::test]
async fn round_trip_request_response_over_real_unix_socket() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("farcooler.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        server.serve(move |_| Some((handshake_cfg(), server_handler.clone()))).await.unwrap();
    });

    let stream = UnixStream::connect(&sock_path).await.unwrap();
    let (read_half, write_half) = stream.into_split();
    let mut client = Connection::new(read_half, write_half);

    let server_hello = client.client_handshake("itest", "0.1.0").await.unwrap();
    assert_eq!(server_hello.selected_protocol_version, PROTOCOL_VERSION);
    assert_eq!(server_hello.daemon_version, "test-daemon");

    client.send(&request_envelope("Ping")).await.unwrap();
    let reply = client.recv().await.unwrap();
    match reply.body {
        Some(wire_envelope::Body::Response(resp)) => {
            assert!(matches!(resp.outcome, Some(v1::response::Outcome::Result(_))));
        }
        other => panic!("expected a Response envelope, got {other:?}"),
    }
    assert_eq!(handler.calls.load(Ordering::SeqCst), 1, "the handler must have been dispatched exactly once");
}

/// Rule 2: a non-Hello first frame is a protocol error that closes the
/// connection before dispatch, not a message the server tries to interpret.
#[tokio::test]
async fn non_hello_first_frame_closes_connection_without_dispatch() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("farcooler.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server.serve(move |_| Some((handshake_cfg(), server_handler.clone()))).await;
    });

    let stream = UnixStream::connect(&sock_path).await.unwrap();
    let (read_half, write_half) = stream.into_split();
    let mut client = Connection::new(read_half, write_half);

    client.send(&request_envelope("Ping")).await.unwrap();
    let err = client.recv().await.unwrap_err();
    assert!(matches!(err, ConnectionError::Closed | ConnectionError::Codec(_)));
    assert_eq!(handler.calls.load(Ordering::SeqCst), 0, "dispatch must never see a pre-handshake frame");
}

/// Rule 3: an oversized length prefix must close the connection promptly,
/// never hang trying to allocate `u32::MAX` bytes.
#[tokio::test]
async fn oversized_length_prefix_closes_connection_promptly() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("farcooler.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server.serve(move |_| Some((handshake_cfg(), server_handler.clone()))).await;
    });

    let mut stream = UnixStream::connect(&sock_path).await.unwrap();
    let mut prefix = BytesMut::new();
    prefix.put_u32(u32::MAX); // declared length far past MAX_CONTROL_ENVELOPE_BYTES
    stream.write_all(&prefix).await.unwrap();
    stream.write_all(b"junk-not-a-real-frame-body").await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = [0u8; 8];
    let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
        .await
        .expect("server must close promptly rather than hang trying to allocate")
        .unwrap();
    assert_eq!(n, 0, "server closes without ever sending a ServerHello for an oversized prefix");
    assert_eq!(handler.calls.load(Ordering::SeqCst), 0);
}

/// Rule 3: a frame that is announced but never fully delivered must never
/// reach dispatch, even after a valid handshake.
#[tokio::test]
async fn truncated_frame_after_handshake_does_not_dispatch() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("farcooler.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server.serve(move |_| Some((handshake_cfg(), server_handler.clone()))).await;
    });

    let stream = UnixStream::connect(&sock_path).await.unwrap();
    let (read_half, write_half) = stream.into_split();
    let mut raw_reader = FrameReader::new(read_half);
    let mut raw_writer = FrameWriter::new(write_half);

    // Complete the handshake at the raw frame level so only rule 3, not
    // rule 2, is under test here.
    let hello = WireEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_id: farcooler_protocol::ids::new_id(),
        body: Some(wire_envelope::Body::ClientHello(v1::ClientHello {
            supported_protocol_versions: vec![PROTOCOL_VERSION],
            client_name: "itest".into(),
            client_version: "0.1.0".into(),
        })),
    };
    raw_writer.write_frame(&hello).await.unwrap();
    let server_hello = raw_reader.read_frame().await.unwrap().unwrap();
    assert!(matches!(server_hello.body, Some(wire_envelope::Body::ServerHello(_))));

    // Send a Request's length prefix and everything except its final byte,
    // then close the write side: a truncated frame must never dispatch.
    let full = farcooler_protocol::framing::encode(&request_envelope("Ping")).unwrap();
    raw_writer.write_raw(&full[..full.len() - 1]).await.unwrap();
    // A real shutdown(SHUT_WR), not just a drop: `read_half` is still alive
    // in `raw_reader`, and a plain drop of only the write half would not
    // guarantee the peer observes EOF.
    raw_writer.shutdown().await.unwrap();

    let outcome = tokio::time::timeout(Duration::from_secs(5), raw_reader.read_frame())
        .await
        .expect("server must close, not hang, on a truncated frame");
    assert!(outcome.is_err() || outcome.unwrap().is_none(), "the connection must close, never respond");
    assert_eq!(handler.calls.load(Ordering::SeqCst), 0, "a truncated frame must never reach the handler");
}

/// Version negotiation: a `ClientHello` that shares no version with the
/// daemon yields `ConnectionError::VersionIncompatible`, mapping to
/// `ERROR_CODE_VERSION_INCOMPATIBLE` on the wire (see `DomainError::wire`).
#[tokio::test]
async fn incompatible_client_hello_yields_version_incompatible() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("farcooler.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server.serve(move |_| Some((handshake_cfg(), server_handler.clone()))).await;
    });

    let stream = UnixStream::connect(&sock_path).await.unwrap();
    let (read_half, write_half) = stream.into_split();
    let mut client = Connection::new(read_half, write_half);

    let err = client.client_handshake_with_versions(&[9999], "itest", "0.1.0").await.unwrap_err();
    assert!(matches!(err, ConnectionError::VersionIncompatible));
    assert_eq!(
        err.domain().map(|d| d.code()),
        Some(farcooler_protocol::v1::ErrorCode::VersionIncompatible)
    );
    assert_eq!(handler.calls.load(Ordering::SeqCst), 0);
}

// ---- the line a session may open with, before the protocol ----

/// A preamble names the session and is never mistaken for a frame.
///
/// The only sender is a `farcoolerd --stdio` process relaying an ssh session
/// into the daemon already listening here, telling it what sshd's forced command
/// granted. If those bytes reached the codec the handshake would fail on the
/// first frame, and the symptom would be a hang-up rather than a reason.
#[tokio::test]
async fn a_session_preamble_names_the_connection_and_is_not_dispatched() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("farcooler.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    tokio::spawn(async move {
        let _ = server
            .serve(move |preamble| {
                let said = preamble.expect("the preamble must reach the daemon");
                recorded.lock().unwrap().push(format!("{} {:?}", said.scope, said.client));
                Some((handshake_cfg(), server_handler.clone()))
            })
            .await;
    });

    let mut stream = UnixStream::connect(&sock_path).await.unwrap();
    stream.write_all(b"farcooler-session read phone-7\n").await.unwrap();
    let (read_half, write_half) = stream.into_split();
    let mut client = Connection::new(read_half, write_half);

    let hello = client.client_handshake("itest", "0.1.0").await.unwrap();
    assert_eq!(hello.daemon_version, "test-daemon");
    assert_eq!(seen.lock().unwrap().as_slice(), ["read Some(\"phone-7\")"]);
}

/// A connection the daemon will not place is closed, never served.
///
/// The daemon says no to a preamble whose scope word it does not have — a typo
/// in someone's `authorized_keys` must not be rounded up to host admin — and
/// that refusal has to happen before the handshake, or the connection would be
/// answered by whatever handler was going to be built for it anyway.
#[tokio::test]
async fn a_refused_session_never_reaches_the_handshake() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("farcooler.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server
            .serve(move |preamble: Option<farcooler_transport::SessionPreamble>| {
                let said = preamble?;
                (said.scope == "read").then(|| (handshake_cfg(), server_handler.clone()))
            })
            .await;
    });

    let mut stream = UnixStream::connect(&sock_path).await.unwrap();
    stream.write_all(b"farcooler-session reed -\n").await.unwrap();
    let (read_half, write_half) = stream.into_split();
    let mut client = Connection::new(read_half, write_half);

    let err = client.client_handshake("itest", "0.1.0").await.unwrap_err();
    assert!(matches!(err, ConnectionError::Closed | ConnectionError::Codec(_)), "{err:?}");
    assert_eq!(handler.calls.load(Ordering::SeqCst), 0);
}

/// A connection closed from above stops being served, and serves nothing more.
///
/// The seam revocation is built on, tested where it lives: transport still owns
/// the socket, and the only thing it is told is that this connection is over.
/// The request sent afterwards is the assertion that matters — it is answered
/// by nobody, because the handler's dispatch is never reached again.
#[tokio::test]
async fn a_handler_can_end_its_own_connection() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("farcooler.sock");
    let handler = ClosableHandler::new();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server.serve(move |_| Some((handshake_cfg(), server_handler.clone()))).await;
    });

    let stream = UnixStream::connect(&sock_path).await.unwrap();
    let (read_half, write_half) = stream.into_split();
    let mut client = Connection::new(read_half, write_half);
    client.client_handshake("itest", "0.1.0").await.unwrap();
    client.send(&request_envelope("anything")).await.unwrap();
    client.recv().await.expect("answered while the connection was open");

    handler.gate.close();

    // Sent after the close, so the only way it can be answered is the serve
    // loop preferring a readable socket over the fact that this connection is
    // finished.
    let _ = client.send(&request_envelope("anything")).await;
    match client.recv().await {
        Ok(frame) => panic!("a closed connection answered a request: {frame:?}"),
        Err(err) => {
            assert!(matches!(err, ConnectionError::Closed | ConnectionError::Codec(_)), "{err:?}")
        }
    }
    assert_eq!(handler.inner.calls.load(Ordering::SeqCst), 1, "the second request was dispatched");
}
