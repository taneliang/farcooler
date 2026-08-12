//! `Connection`: framed send/recv plus the two protocol-level rules that
//! apply to every transport (Unix socket or stdio) identically:
//!
//! - rule 2: the first client frame must be `ClientHello`; nothing else is
//!   dispatched until a compatible `ServerHello` goes out.
//! - rule 4: queued unwritten control bytes are capped at
//!   `MAX_QUEUED_CONTROL_BYTES`; staying above that for `TOO_SLOW_DISCONNECT`
//!   disconnects the client with `ERROR_CODE_CLIENT_TOO_SLOW`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use farcooler_core::DomainError;
use farcooler_protocol::v1::{
    ClientHello, Error as WireErrorMsg, ErrorCode, Event, Response, Scope, ServerHello, WireEnvelope,
    response, wire_envelope,
};
use farcooler_protocol::{MAX_QUEUED_CONTROL_BYTES, PROTOCOL_VERSION, ids};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant as TokioInstant;

use crate::Handler;
use crate::codec::{CodecError, FrameReader, FrameWriter, encode_frame};

/// Rule 4: the grace period before a client stuck above the control-channel
/// ceiling gets disconnected.
pub const TOO_SLOW_DISCONNECT: Duration = Duration::from_secs(30);

/// How often the watchdog re-checks the queued-bytes counter. Independent of
/// `TOO_SLOW_DISCONNECT` so it stays cheap in production and responsive in
/// tests that inject a much shorter grace period.
const WATCHDOG_POLL: Duration = Duration::from_millis(20);

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("peer closed the connection")]
    Closed,
    #[error("client exceeded the control-channel ceiling for {0:?}")]
    TooSlow(Duration),
    #[error("first frame was not ClientHello")]
    ExpectedHello,
    #[error("received a frame type that is never dispatched at this point in the protocol")]
    UnexpectedFrame,
    #[error("client protocol version is not compatible with this daemon")]
    VersionIncompatible,
    #[error("handshake rejected: {message}")]
    Rejected { code: i32, message: String },
}

impl ConnectionError {
    /// Maps to the one domain error enum so a caller can hand this to
    /// `core`'s exhaustive wire-code mapping instead of inventing its own.
    pub fn domain(&self) -> Option<DomainError> {
        match self {
            ConnectionError::TooSlow(_) => Some(DomainError::ClientTooSlow),
            ConnectionError::VersionIncompatible => Some(DomainError::VersionIncompatible),
            _ => None,
        }
    }
}

/// Server-side handshake parameters. Auth/scope decisions live above this
/// crate; transport just needs something to put in `ServerHello`.
#[derive(Debug, Clone)]
pub struct HandshakeConfig {
    pub daemon_version: String,
    pub granted_scope: Scope,
}

pub struct Connection<R> {
    reader: FrameReader<R>,
    writer_tx: mpsc::UnboundedSender<Vec<u8>>,
    writer_task: tokio::task::JoinHandle<()>,
    watchdog_task: tokio::task::JoinHandle<()>,
    queued_bytes: Arc<AtomicU64>,
    too_slow: watch::Receiver<bool>,
    too_slow_after: Duration,
}

impl<R> Connection<R> {
    /// Current count of unwritten bytes queued for the peer (the rule-4
    /// metric the watchdog compares against the ceiling).
    pub fn queued_bytes(&self) -> u64 {
        self.queued_bytes.load(Ordering::SeqCst)
    }

    pub async fn send(&mut self, envelope: &WireEnvelope) -> Result<(), ConnectionError> {
        if *self.too_slow.borrow() {
            return Err(ConnectionError::TooSlow(self.too_slow_after));
        }
        let bytes = encode_frame(envelope)?;
        self.queued_bytes.fetch_add(bytes.len() as u64, Ordering::SeqCst);
        self.writer_tx.send(bytes).map_err(|_| ConnectionError::Closed)?;
        Ok(())
    }
}

impl<R> Drop for Connection<R> {
    fn drop(&mut self) {
        self.watchdog_task.abort();

        if *self.too_slow.borrow() {
            // Already declared unrecoverably stuck (rule 4): nothing queued
            // is going to drain in reasonable time, so force the writer
            // closed rather than leak a task blocked on a dead peer forever.
            self.writer_task.abort();
        }
        // Otherwise leave the writer task running detached: `writer_tx` (a
        // plain struct field) drops right after this function returns, which
        // closes the channel once every already-queued frame has been
        // handed to it. The writer keeps draining that backlog and exits on
        // its own. This matters because a `send` is very often immediately
        // followed by dropping the connection (a handshake rejection, a
        // final response before the peer misbehaves) — without this, the
        // last frame could be queued but never actually reach the wire.
    }
}

impl<R: AsyncRead + Unpin> Connection<R> {
    /// Production limits: `MAX_QUEUED_CONTROL_BYTES` and
    /// `TOO_SLOW_DISCONNECT`.
    pub fn new<W>(reader: R, writer: W) -> Self
    where
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::with_limits(reader, writer, MAX_QUEUED_CONTROL_BYTES, TOO_SLOW_DISCONNECT)
    }

    /// Same as `new` with an injectable ceiling and grace period, so tests
    /// can exercise the rule-4 disconnect without pushing megabytes of data
    /// or waiting 30 real seconds.
    pub fn with_limits<W>(reader: R, writer: W, ceiling_bytes: u64, too_slow_after: Duration) -> Self
    where
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let queued_bytes = Arc::new(AtomicU64::new(0));
        let (too_slow_tx, too_slow_rx) = watch::channel(false);

        let writer_task = tokio::spawn(run_writer(writer, writer_rx, queued_bytes.clone()));
        let watchdog_task =
            tokio::spawn(run_watchdog(queued_bytes.clone(), ceiling_bytes, too_slow_after, too_slow_tx));

        Self {
            reader: FrameReader::new(reader),
            writer_tx,
            writer_task,
            watchdog_task,
            queued_bytes,
            too_slow: too_slow_rx,
            too_slow_after,
        }
    }

    /// Reads the next frame, racing it against the rule-4 watchdog so a
    /// connection stuck on a slow peer is interrupted even while `read_frame`
    /// itself has nothing to return yet.
    pub async fn recv(&mut self) -> Result<WireEnvelope, ConnectionError> {
        loop {
            tokio::select! {
                biased;
                changed = self.too_slow.changed() => {
                    if changed.is_ok() && *self.too_slow.borrow() {
                        return Err(ConnectionError::TooSlow(self.too_slow_after));
                    }
                }
                frame = self.reader.read_frame() => {
                    return match frame? {
                        Some(env) => Ok(env),
                        None => Err(ConnectionError::Closed),
                    };
                }
            }
        }
    }

    /// Server side of rule 2. Reads the first frame, requires it to be
    /// `ClientHello`, and replies with either `ServerHello` or an explicit
    /// rejection before anything else is ever dispatched.
    pub async fn handshake(&mut self, cfg: &HandshakeConfig) -> Result<ClientHello, ConnectionError> {
        let first = self.recv().await?;
        let (client_message_id, hello) = match first.body {
            Some(wire_envelope::Body::ClientHello(h)) => (first.message_id, h),
            _ => return Err(ConnectionError::ExpectedHello),
        };

        if !hello.supported_protocol_versions.contains(&PROTOCOL_VERSION) {
            // Best effort: the client gets one explicit reason rather than
            // just an abrupt close. If the send fails, the caller still sees
            // VersionIncompatible below.
            let _ = self.send(&reject_envelope(client_message_id, DomainError::VersionIncompatible)).await;
            return Err(ConnectionError::VersionIncompatible);
        }

        let reply = WireEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: ids::new_id(),
            body: Some(wire_envelope::Body::ServerHello(ServerHello {
                selected_protocol_version: PROTOCOL_VERSION,
                daemon_version: cfg.daemon_version.clone(),
                max_control_envelope_bytes: farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES as u32,
                max_terminal_payload_bytes: farcooler_protocol::MAX_TERMINAL_PAYLOAD_BYTES as u32,
                granted_scope: cfg.granted_scope as i32,
                // Answered in the handshake so every client knows what this
                // machine can do before its first request, at no extra round
                // trip. Built from `capability::ALL`, the same table
                // `daemon.version` and the dispatcher read.
                capabilities: farcooler_protocol::capability::ALL
                    .iter()
                    .map(|c| (*c).to_string())
                    .collect(),
            })),
        };
        self.send(&reply).await?;
        Ok(hello)
    }

    /// Client side of rule 2, offering only `PROTOCOL_VERSION`.
    pub async fn client_handshake(
        &mut self,
        client_name: &str,
        client_version: &str,
    ) -> Result<ServerHello, ConnectionError> {
        self.client_handshake_with_versions(&[PROTOCOL_VERSION], client_name, client_version).await
    }

    /// Same as `client_handshake` with an explicit version list, so tests can
    /// drive version negotiation without hand-building the envelope.
    pub async fn client_handshake_with_versions(
        &mut self,
        versions: &[u32],
        client_name: &str,
        client_version: &str,
    ) -> Result<ServerHello, ConnectionError> {
        let hello = WireEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: ids::new_id(),
            body: Some(wire_envelope::Body::ClientHello(ClientHello {
                supported_protocol_versions: versions.to_vec(),
                client_name: client_name.to_string(),
                client_version: client_version.to_string(),
            })),
        };
        self.send(&hello).await?;

        match self.recv().await?.body {
            Some(wire_envelope::Body::ServerHello(sh)) => Ok(sh),
            Some(wire_envelope::Body::Response(Response {
                outcome: Some(response::Outcome::Error(e)), ..
            })) => {
                if e.code == ErrorCode::VersionIncompatible as i32 {
                    Err(ConnectionError::VersionIncompatible)
                } else {
                    Err(ConnectionError::Rejected { code: e.code, message: e.message })
                }
            }
            _ => Err(ConnectionError::ExpectedHello),
        }
    }
}

/// There is no error variant on `ClientHello`/`ServerHello` in the proto, so
/// a handshake-time rejection is carried as a `Response`/`Error`, echoing the
/// `ClientHello`'s message id so the client can correlate it. This is the one
/// place transport originates an error frame itself rather than relaying one
/// from `Handler`.
fn reject_envelope(client_message_id: Bytes, err: DomainError) -> WireEnvelope {
    let (code, retryable) = err.wire();
    WireEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_id: ids::new_id(),
        body: Some(wire_envelope::Body::Response(Response {
            request_id: client_message_id,
            outcome: Some(response::Outcome::Error(WireErrorMsg {
                code: code as i32,
                retryable,
                message: err.redacted_message(),
            })),
        })),
    }
}

/// Rules 2 + 3 end to end: handshake first, then dispatch only `Request`
/// frames to `handler`, echoing `request_id` on the way out. Any codec or
/// protocol error returned by `recv`/`send` closes the connection before
/// `handler` ever sees it.
pub async fn serve_connection<R, H>(
    conn: &mut Connection<R>,
    cfg: &HandshakeConfig,
    handler: &H,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin,
    H: Handler,
{
    conn.handshake(cfg).await?;

    // Events are pushed, not polled.
    //
    // A client that polls has to choose between latency and cost, and gets
    // both wrong: too slow to notice an agent asking a question, too expensive
    // for a phone on a battery over SSH. Pushing means a change reaches every
    // connected client as it happens and an idle fleet costs nothing.
    let mut events = handler.events();

    loop {
        tokio::select! {
            // Biased so a pending request is always answered before events are
            // drained. Without it a busy fleet could starve request handling,
            // and a user's click would wait behind a queue of notifications.
            biased;

            incoming = conn.recv() => {
                let envelope = incoming?;
                let request = match envelope.body {
                    Some(wire_envelope::Body::Request(req)) => req,
                    _ => return Err(ConnectionError::UnexpectedFrame),
                };

                let request_id = request.request_id.clone();
                let mut response = handler.handle(request).await;
                response.request_id = request_id;

                conn.send(&WireEnvelope {
                    protocol_version: PROTOCOL_VERSION,
                    message_id: ids::new_id(),
                    body: Some(wire_envelope::Body::Response(response)),
                }).await?;
            }

            event = next_event(&mut events) => {
                let Some(event) = event else {
                    // The broadcaster is gone, or this handler emits nothing.
                    // Neither is a reason to drop a working connection, so stop
                    // listening and keep serving requests.
                    events = None;
                    continue;
                };
                conn.send(&WireEnvelope {
                    protocol_version: PROTOCOL_VERSION,
                    message_id: ids::new_id(),
                    body: Some(wire_envelope::Body::Event(event)),
                }).await?;
            }
        }
    }
}

/// Await the next event, or never, when this handler emits none.
///
/// `select!` needs every branch to be a future; a handler without events would
/// otherwise have to be a separate code path. Pending-forever is the honest
/// expression of "this arm will not fire".
async fn next_event(
    events: &mut Option<tokio::sync::broadcast::Receiver<Event>>,
) -> Option<Event> {
    let Some(receiver) = events else {
        std::future::pending::<()>().await;
        unreachable!("pending never resolves");
    };
    loop {
        match receiver.recv().await {
            Ok(event) => return Some(event),
            // A slow client missed some. Dropping the connection would be
            // worse than the gap: the next event still arrives, and clients
            // reconcile against a full read when they need certainty.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "client fell behind the event stream");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        }
    }
}

/// Drains queued frames onto the wire until the sender half closes (the
/// `Connection` was dropped) or a write fails.
async fn run_writer<W>(writer: W, mut rx: mpsc::UnboundedReceiver<Vec<u8>>, queued_bytes: Arc<AtomicU64>)
where
    W: AsyncWrite + Unpin,
{
    let mut writer = FrameWriter::new(writer);
    while let Some(bytes) = rx.recv().await {
        let len = bytes.len() as u64;
        if writer.write_raw(&bytes).await.is_err() {
            break;
        }
        queued_bytes.fetch_sub(len, Ordering::SeqCst);
    }
}

/// Rule 4. Runs independently of the writer so a write blocked on a slow
/// reader (not just a slow producer) is still timed out: the writer task can
/// be stuck inside a single `write_all` for the entire grace period.
async fn run_watchdog(
    queued_bytes: Arc<AtomicU64>,
    ceiling: u64,
    after: Duration,
    too_slow_tx: watch::Sender<bool>,
) {
    let mut over_since: Option<TokioInstant> = None;
    let mut ticker = tokio::time::interval(WATCHDOG_POLL);
    loop {
        ticker.tick().await;
        let bytes = queued_bytes.load(Ordering::SeqCst);
        if bytes > ceiling {
            let since = *over_since.get_or_insert_with(TokioInstant::now);
            if TokioInstant::now().duration_since(since) >= after {
                let _ = too_slow_tx.send(true);
                return;
            }
        } else {
            over_since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_envelope() -> WireEnvelope {
        WireEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: ids::new_id(),
            body: Some(wire_envelope::Body::ClientHello(ClientHello {
                supported_protocol_versions: vec![PROTOCOL_VERSION],
                client_name: "backpressure-test".into(),
                client_version: "0.1.0".into(),
            })),
        }
    }

    /// Rules 1 + 2 together over a duplex pipe: the identical `Connection`
    /// used for a Unix socket completes a handshake over any AsyncRead +
    /// AsyncWrite pair, which is exactly what stdio provides too.
    #[tokio::test]
    async fn handshake_round_trip_over_duplex() {
        let (client_io, server_io) = tokio::io::duplex(4096);
        let (cr, cw) = tokio::io::split(client_io);
        let (sr, sw) = tokio::io::split(server_io);

        let mut server = Connection::new(sr, sw);
        let mut client = Connection::new(cr, cw);

        let cfg = HandshakeConfig { daemon_version: "dtest".into(), granted_scope: Scope::Control };
        let server_task = tokio::spawn(async move {
            let hello = server.handshake(&cfg).await.unwrap();
            assert_eq!(hello.client_name, "itest");
            // `send`'s reply only queues the bytes; the writer task delivers
            // them on its own schedule. Stay alive (as `serve_connection`'s
            // loop naturally would) until the client is done with us, rather
            // than dropping `server` immediately and racing that delivery.
            let _ = server.recv().await;
        });

        let server_hello = client.client_handshake("itest", "0.1.0").await.unwrap();
        assert_eq!(server_hello.daemon_version, "dtest");
        assert_eq!(server_hello.selected_protocol_version, PROTOCOL_VERSION);
        drop(client);
        server_task.await.unwrap();
    }

    /// Rule 4, exercised with an injected ceiling/grace period rather than
    /// the real 4 MiB / 30 s so the test stays fast. The peer end of the
    /// duplex pipe is held open but never read, standing in for a client
    /// that stopped draining its socket.
    #[tokio::test]
    async fn too_slow_client_is_disconnected() {
        let (here, _there) = tokio::io::duplex(16);
        let (read_half, write_half) = tokio::io::split(here);
        let mut conn = Connection::with_limits(read_half, write_half, 32, Duration::from_millis(80));

        for _ in 0..20 {
            let _ = conn.send(&sample_envelope()).await;
        }
        assert!(conn.queued_bytes() > 32, "the scenario should actually cross the ceiling");

        let err = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Err(e) = conn.recv().await {
                    return e;
                }
            }
        })
        .await
        .expect("watchdog must fire within the timeout");

        assert!(matches!(err, ConnectionError::TooSlow(_)));
    }

    /// Below the ceiling, nothing trips: proves the watchdog isn't just a
    /// timer, it actually gates on `queued_bytes`.
    #[tokio::test]
    async fn under_ceiling_never_disconnects() {
        let (here, there) = tokio::io::duplex(4096);
        let (read_half, write_half) = tokio::io::split(here);
        let mut conn =
            Connection::with_limits(read_half, write_half, MAX_QUEUED_CONTROL_BYTES, Duration::from_millis(50));

        conn.send(&sample_envelope()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!*conn.too_slow.borrow());

        drop(there); // let the background tasks unwind cleanly
    }
}
