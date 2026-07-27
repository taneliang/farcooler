//! End-to-end tests against the crate's public API: a real Unix socket in a
//! tempdir, the Hello handshake, rules 2/3 protocol-error closes, and
//! version negotiation. Rule 4 (backpressure) is exercised in
//! `connection.rs`'s own unit tests, where an injectable ceiling keeps the
//! test fast; here the socket-level plumbing is what's under test.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bytes::{BufMut, BytesMut};
use overnight_protocol::PROTOCOL_VERSION;
use overnight_protocol::v1::{self, Request, Response, WireEnvelope, wire_envelope};
use overnight_transport::{Connection, ConnectionError, FrameReader, FrameWriter, Handler, HandshakeConfig, UnixListenerServer};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[derive(Clone, Default)]
struct RecordingHandler {
    calls: Arc<AtomicUsize>,
}

impl Handler for RecordingHandler {
    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let request_id = req.request_id.clone();
        async move { Response { request_id, outcome: Some(v1::response::Outcome::Result(v1::Result { value: None })) } }
    }
}

fn handshake_cfg() -> HandshakeConfig {
    HandshakeConfig { daemon_version: "test-daemon".into(), granted_scope: v1::Scope::Control }
}

fn request_envelope(method: &str) -> WireEnvelope {
    // Uses `uuid` directly (rather than the protocol crate's own id helper)
    // to exercise the crate's declared dependency on it, not just its
    // transitive presence via overnight-protocol.
    let request_id = overnight_protocol::ids::to_bytes(uuid::Uuid::now_v7());
    WireEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_id: overnight_protocol::ids::new_id(),
        body: Some(wire_envelope::Body::Request(Request {
            request_id,
            method: method.into(),
            target_resource_id: None,
            expected_resource_version: None,
            expected_lease_generation: None,
            idempotency_key: None,
            payload: Some(v1::request::Payload::Empty(v1::Empty {})),
        })),
    }
}

#[tokio::test]
async fn round_trip_request_response_over_real_unix_socket() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("overnight.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        server.serve(handshake_cfg(), server_handler).await.unwrap();
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
    let sock_path = dir.path().join("overnight.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server.serve(handshake_cfg(), server_handler).await;
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
    let sock_path = dir.path().join("overnight.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server.serve(handshake_cfg(), server_handler).await;
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
    let sock_path = dir.path().join("overnight.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server.serve(handshake_cfg(), server_handler).await;
    });

    let stream = UnixStream::connect(&sock_path).await.unwrap();
    let (read_half, write_half) = stream.into_split();
    let mut raw_reader = FrameReader::new(read_half);
    let mut raw_writer = FrameWriter::new(write_half);

    // Complete the handshake at the raw frame level so only rule 3, not
    // rule 2, is under test here.
    let hello = WireEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_id: overnight_protocol::ids::new_id(),
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
    let full = overnight_protocol::framing::encode(&request_envelope("Ping")).unwrap();
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
    let sock_path = dir.path().join("overnight.sock");
    let handler = RecordingHandler::default();
    let server = UnixListenerServer::bind(&sock_path).unwrap();
    let server_handler = handler.clone();
    tokio::spawn(async move {
        let _ = server.serve(handshake_cfg(), server_handler).await;
    });

    let stream = UnixStream::connect(&sock_path).await.unwrap();
    let (read_half, write_half) = stream.into_split();
    let mut client = Connection::new(read_half, write_half);

    let err = client.client_handshake_with_versions(&[9999], "itest", "0.1.0").await.unwrap_err();
    assert!(matches!(err, ConnectionError::VersionIncompatible));
    assert_eq!(
        err.domain().map(|d| d.code()),
        Some(overnight_protocol::v1::ErrorCode::VersionIncompatible)
    );
    assert_eq!(handler.calls.load(Ordering::SeqCst), 0);
}
