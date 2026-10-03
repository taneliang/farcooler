//! `Connection`: framed send/recv plus the two protocol-level rules that
//! apply to every transport (Unix socket or stdio) identically:
//!
//! - rule 2: the first client frame must be `ClientHello`; nothing else is
//!   dispatched until a compatible `ServerHello` goes out.
//! - rule 4: queued unwritten control bytes are capped at
//!   `MAX_QUEUED_CONTROL_BYTES`; staying above that for `TOO_SLOW_DISCONNECT`
//!   disconnects the client with `ERROR_CODE_CLIENT_TOO_SLOW`.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use farcooler_core::DomainError;
use farcooler_protocol::v1::{
    ClientHello, Error as WireErrorMsg, ErrorCode, Event, Request, Response, Scope, ServerHello, WireEnvelope,
    response, wire_envelope,
};
use farcooler_protocol::{MAX_QUEUED_CONTROL_BYTES, PROTOCOL_VERSION, ids};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::Instant as TokioInstant;

use crate::Handler;
use crate::codec::{CodecError, FrameReader, FrameWriter, encode_frame};
use crate::push::PushReceiver;

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
///
/// The scope is deliberately NOT here. It used to be, and the dispatcher above
/// this crate had a second copy of it: what a session was TOLD it held and what
/// it was actually permitted were two values, and they were once two different
/// values — the dispatcher hardcoded host admin, so a read session was
/// advertised `read` and allowed everything. It now comes from `Handler::peer`,
/// the same place enforcement reads it, so there is no second value left to
/// disagree.
#[derive(Debug, Clone)]
pub struct HandshakeConfig {
    pub daemon_version: String,
}

pub struct Connection<R> {
    reader: FrameReader<R>,
    writer_tx: mpsc::UnboundedSender<Vec<u8>>,
    writer_task: tokio::task::JoinHandle<()>,
    watchdog_task: tokio::task::JoinHandle<()>,
    queued_bytes: Arc<AtomicU64>,
    /// Fired by the writer each time it puts a frame on the wire, so a loop
    /// that stopped taking pushes at `PUSH_HIGH_WATER` learns when to start
    /// again without polling.
    written: Arc<Notify>,
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

        let written = Arc::new(Notify::new());
        let writer_task =
            tokio::spawn(run_writer(writer, writer_rx, queued_bytes.clone(), written.clone()));
        let watchdog_task =
            tokio::spawn(run_watchdog(queued_bytes.clone(), ceiling_bytes, too_slow_after, too_slow_tx));

        Self {
            reader: FrameReader::new(reader),
            writer_tx,
            writer_task,
            watchdog_task,
            queued_bytes,
            written,
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
    ///
    /// `granted` is passed rather than held in `cfg` so it can only come from
    /// the same place the dispatcher reads it — the connection's `Peer`.
    pub async fn handshake(
        &mut self,
        cfg: &HandshakeConfig,
        granted: Scope,
    ) -> Result<ClientHello, ConnectionError> {
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
                granted_scope: granted as i32,
                // Answered in the handshake so every client knows what this
                // runner can do before its first request, at no extra round
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
                // Filled from the same source as `rpc::error_response`, so the
                // two places that build this frame cannot answer differently.
                // Always `""` today: the only error that reaches here is
                // `VersionIncompatible`, which carries no detail.
                what: err.what().to_string(),
            })),
        })),
    }
}

/// How far ahead of the wire this connection's writer may get on pushed
/// output before `serve_connection` stops taking more from `Handler::pushes`.
///
/// The second half of what bounds a stalled client. A loop that drains the push
/// queue into the writer as fast as it can just moves the backlog somewhere the
/// sender cannot see it — which is what this used to do, into a channel only
/// rule 4 was watching, and only after 30 seconds over 4 MiB. Stopping here
/// leaves it in the push queue, whose sender can drop it and resync.
///
/// A quarter of `MAX_UNACKED_TERMINAL_BYTES`: enough to keep a link that is
/// keeping up full, and well under the rule-4 ceiling, so terminal output alone
/// never disconnects a slow client — it gets a fresh picture instead.
pub const PUSH_HIGH_WATER: u64 = farcooler_protocol::MAX_UNACKED_TERMINAL_BYTES / 4;

/// How many requests one connection may have in flight at once.
///
/// Requests no longer wait for the one before them (see `serve_connection`),
/// so something has to stop a client from opening a thousand. At the limit the
/// loop stops reading new requests, which leaves the rest in the socket: the
/// client is slowed, not refused.
pub const MAX_REQUESTS_IN_FLIGHT: usize = 32;

/// Which requests on one connection must run in the order they arrived: those
/// naming the same target. See `serve_connection`.
type Lane = Option<Bytes>;

/// Rules 2 + 3 end to end: handshake first, then dispatch only `Request`
/// frames to `handler`, echoing `request_id` on the way out. Any codec or
/// protocol error returned by `recv`/`send` closes the connection before
/// `handler` ever sees it.
///
/// **Requests run concurrently, in lanes.** This used to await each request
/// inside the loop, so one slow call — a diff, a git query, a tmux timeout —
/// froze everything else on the connection: other requests, pushed terminal
/// output and fleet news, and even the close arm, so a revoked device stayed
/// connected until its call finished. Now a request is started and the loop
/// goes on serving.
///
/// What still needs order is order WITHIN one object. Keystrokes to a pane are
/// the case that cannot be got wrong — `terminal.write` "l" then "s" must not
/// type "sl" — and so are a paste's chunks, a resize between writes, and an
/// agent's prompt, queue edits and cancel. Every one of those names the
/// terminal as `target_resource_id`, so requests are queued by target: one at a
/// time per target, in arrival order, and targets run alongside each other.
/// Requests that name no target share one lane of their own, which keeps them
/// in the order they always ran in.
///
/// Responses go out as requests finish, so they may arrive in a different order
/// from the requests; `request_id` is what pairs them, as it always was.
///
/// **What order is guaranteed, for a client that pipelines.** None does today:
/// `Client::call` holds the reader for the length of one request, the mobile
/// `Session` is behind a mutex, and the CLI is one call per process. A client
/// that starts to must rely on exactly this and no more:
///
/// - Two requests with byte-equal `target_resource_id` run one after the
///   other, in the order they were sent. Everything that acts on one terminal
///   names it — write, paste, resize, attach, the agent calls — so those keep
///   their order.
/// - Two requests with no target run in the order they were sent.
/// - Nothing else is ordered. A create targets its PARENT (the repository or
///   worktree), not what it creates; stopping a terminal and then removing its
///   worktree name two different targets; two attaches to two terminals race
///   for the connection's one attachment, and the one that finishes last wins.
///   A client that needs one of those in order waits for the first answer.
///
/// And a request that was started is finished, whatever happens to the
/// connection. See `Ending`.
pub async fn serve_connection<R, H>(
    conn: &mut Connection<R>,
    cfg: &HandshakeConfig,
    handler: &H,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin,
    H: Handler,
{
    let peer = handler.peer();
    conn.handshake(cfg, peer.scope).await?;

    // Created once, before the first request, so nothing can be dispatched
    // between "this connection was closed" and this loop noticing. Polled from
    // inside the `select!` below and never after it fires, because the first
    // time it does the loop returns.
    let closed = handler.closed();
    let mut closed = std::pin::pin!(closed);

    // Events are pushed, not polled.
    //
    // A client that polls has to choose between latency and cost, and gets
    // both wrong: too slow to notice an agent asking a question, too expensive
    // for a phone on a battery over SSH. Pushing means a change reaches every
    // connected client as it happens and an idle fleet costs nothing.
    let mut events = handler.events();

    // And what is addressed to this connection alone: an attached terminal's
    // bytes. Kept off the broadcast above because that one drops frames from a
    // slow reader by design, and a dropped run of terminal output is an escape
    // sequence cut in half rather than a screen one refresh out of date. See
    // `Handler::pushes`.
    let mut pushes = handler.pushes();
    let written = conn.written.clone();

    // The requests running now, at most one per lane.
    let mut running = Vec::new();
    // Every lane with a request running, and what is waiting behind it.
    let mut lanes: HashMap<Lane, VecDeque<Request>> = HashMap::new();
    let mut in_flight = 0usize;

    let (outcome, ending) = loop {
        // Read before the `select!`, because `conn.recv()` below holds `conn`.
        let push_room = conn.queued_bytes() < PUSH_HIGH_WATER;

        tokio::select! {
            // Biased so a pending request is always answered before events are
            // drained. Without it a busy fleet could starve request handling,
            // and a user's click would wait behind a queue of notifications.
            //
            // The close arm goes FIRST, ahead of even that. It is the one thing
            // that must win a tie: a revoked device whose request is already
            // sitting in the socket buffer would otherwise be served it, and
            // the whole point of closing the connection is that it is not.
            // What is already running is finished, unanswered: see `Ending`.
            biased;

            _ = &mut closed => {
                tracing::debug!(client = ?peer.client_id, "this connection was closed from above");
                break (Ok(()), Ending::Revoked);
            }

            (lane, request_id, mut response) = next_finished(&mut running) => {
                response.request_id = request_id;
                in_flight -= 1;
                let sent = conn.send(&WireEnvelope {
                    protocol_version: PROTOCOL_VERSION,
                    message_id: ids::new_id(),
                    body: Some(wire_envelope::Body::Response(response)),
                }).await;

                // The next request in this lane, now that the one ahead of it
                // has finished.
                let next = lanes.get_mut(&lane).and_then(VecDeque::pop_front);
                match next {
                    Some(request) => running.push(Box::pin(dispatch(handler, lane, request))),
                    None => {
                        lanes.remove(&lane);
                    }
                }
                if let Err(err) = sent {
                    break (Err(err), Ending::Unanswerable);
                }
            }

            incoming = conn.recv(), if in_flight < MAX_REQUESTS_IN_FLIGHT => {
                let envelope = match incoming {
                    Ok(envelope) => envelope,
                    Err(err) => break (Err(err), Ending::NoMoreRequests),
                };
                let request = match envelope.body {
                    Some(wire_envelope::Body::Request(req)) => req,
                    _ => break (Err(ConnectionError::UnexpectedFrame), Ending::NoMoreRequests),
                };
                in_flight += 1;
                let lane: Lane = request.target_resource_id.clone();
                match lanes.get_mut(&lane) {
                    Some(waiting) => waiting.push_back(request),
                    None => {
                        lanes.insert(lane.clone(), VecDeque::new());
                        running.push(Box::pin(dispatch(handler, lane, request)));
                    }
                }
            }

            event = next_event(&mut events) => {
                let Some(event) = event else {
                    // The broadcaster is gone, or this handler emits nothing.
                    // Neither is a reason to drop a working connection, so stop
                    // listening and keep serving requests.
                    events = None;
                    continue;
                };
                if let Err(err) = conn.send(&WireEnvelope {
                    protocol_version: PROTOCOL_VERSION,
                    message_id: ids::new_id(),
                    body: Some(wire_envelope::Body::Event(event)),
                }).await {
                    break (Err(err), Ending::Unanswerable);
                }
            }

            // Last in the biased order, behind the broadcast, so a pane writing
            // as fast as it can cannot starve the fleet news this connection is
            // also carrying. The other way round would be the wrong risk:
            // broadcast events are rare, so putting them first costs a busy
            // stream nothing measurable, while a `yes` in a pane would otherwise
            // keep a workspace change waiting indefinitely.
            //
            // And only while the writer has room. See `PUSH_HIGH_WATER`.
            pushed = next_push(&mut pushes), if push_room => {
                let Some(event) = pushed else {
                    // Whatever was pushing has stopped — this handler never had
                    // anything to push. Not a reason to drop a working
                    // connection.
                    pushes = None;
                    continue;
                };
                if let Err(err) = conn.send(&WireEnvelope {
                    protocol_version: PROTOCOL_VERSION,
                    message_id: ids::new_id(),
                    body: Some(wire_envelope::Body::Event(event)),
                }).await {
                    break (Err(err), Ending::Unanswerable);
                }
            }

            // The writer put something on the wire, so there may be room for
            // pushes again. Only listened for while there is not, and it does
            // nothing itself: going round the loop re-reads `push_room`.
            _ = written.notified(), if !push_room && pushes.is_some() => {}
        }
    };

    // Nothing new is read from here on. What was already received is finished
    // rather than cancelled — see `Ending` for why, and for which of it.
    drop(pushes);
    drop(events);
    let drain = async {
        let mut answering = ending == Ending::NoMoreRequests;
        if ending == Ending::Revoked {
            lanes.clear();
        }
        while !running.is_empty() {
            let (lane, request_id, mut response) = next_finished(&mut running).await;
            if answering {
                response.request_id = request_id;
                answering = conn
                    .send(&WireEnvelope {
                        protocol_version: PROTOCOL_VERSION,
                        message_id: ids::new_id(),
                        body: Some(wire_envelope::Body::Response(response)),
                    })
                    .await
                    .is_ok();
            }
            if let Some(request) = lanes.get_mut(&lane).and_then(VecDeque::pop_front) {
                running.push(Box::pin(dispatch(handler, lane, request)));
            }
        }
    };
    let finished = tokio::time::timeout(DRAIN_DEADLINE, drain).await;
    if finished.is_err() {
        tracing::warn!(
            client = ?peer.client_id,
            "requests still running when the connection ended were abandoned at the deadline"
        );
    }
    outcome
}

/// How a connection's loop ended, which decides what becomes of the requests
/// it had already received.
///
/// They used to be safe by construction: the loop awaited each one inline, so a
/// request that had started always finished. Running them alongside each other
/// put them in a list the loop owns, and returning from the loop dropped them
/// part-way through — a `terminal.create` whose store row was written and whose
/// tmux window never was, because the phone that asked went out of signal in
/// between. So every ending finishes what was started, and the endings differ
/// only in what else they owe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The client stopped sending — an EOF, a half-close, a broken frame. Its
    /// requests were received and are all run, queued ones included, and
    /// answered while the write side still takes answers. A client that sends
    /// its request and then closes its write half (`runner_pipe`) still gets
    /// its answer.
    NoMoreRequests,
    /// The connection cannot be written to. Everything received is still run,
    /// for its effects, and nothing is answered.
    Unanswerable,
    /// The connection was closed from above — a device revoked. Nothing new is
    /// served, and that includes requests queued behind a running one: they
    /// were received but never started, so nothing is half done by dropping
    /// them. What was running is finished, unanswered.
    Revoked,
}

/// How long a connection that has ended waits for the requests it was running.
///
/// The backstop, not the mechanism: requests finish in milliseconds to seconds.
/// It exists for one that is waiting on something that will not come, which
/// would otherwise keep a dead connection's task alive forever.
pub const DRAIN_DEADLINE: Duration = Duration::from_secs(30);

/// One request, answered, and labeled with what the loop needs to file the
/// answer: the lane to release, and the id to echo.
async fn dispatch<H: Handler>(handler: &H, lane: Lane, request: Request) -> (Lane, Bytes, Response) {
    let request_id = request.request_id.clone();
    let response = handler.handle(request).await;
    (lane, request_id, response)
}

/// Whichever running request finishes first, or never, when none is running.
///
/// A hand-rolled `FuturesUnordered`: at most `MAX_REQUESTS_IN_FLIGHT` of them,
/// so polling each in turn costs nothing worth a dependency. A finished one is
/// removed before it is returned, so nothing polls it again.
fn next_finished<F>(
    running: &mut Vec<std::pin::Pin<Box<F>>>,
) -> impl std::future::Future<Output = (Lane, Bytes, Response)> + '_
where
    F: std::future::Future<Output = (Lane, Bytes, Response)>,
{
    std::future::poll_fn(move |cx| {
        for index in 0..running.len() {
            if let std::task::Poll::Ready(out) = running[index].as_mut().poll(cx) {
                // `swap_remove` reorders the rest, which is harmless: a lane has
                // at most one request in here, so order between them means
                // nothing.
                drop(running.swap_remove(index));
                return std::task::Poll::Ready(out);
            }
        }
        std::task::Poll::Pending
    })
}

/// Await the next event addressed to this connection, or never, when there is
/// nothing pushing to it. The `pending` arm is there for the reason
/// `next_event` gives above: `select!` needs every branch to be a future.
async fn next_push(pushes: &mut Option<PushReceiver>) -> Option<Event> {
    let Some(receiver) = pushes else {
        std::future::pending::<()>().await;
        unreachable!("pending never resolves");
    };
    // No lag arm to handle: the queue never drops on its own. A sender that
    // outruns this connection is refused at the queue's limit, and decides what
    // to do about it. `None` means every sender is gone.
    receiver.recv().await
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
    match receiver.recv().await {
        Ok(event) => Some(event),
        // A slow client missed some. Dropping the connection would be
        // worse than the gap: the next event still arrives. But the
        // client is told, in place of what it lost — every event is a
        // "re-read" notice, harmless to lose only if the client learns it
        // missed SOMETHING, and a phone that reads its boards on their
        // own news had no other way to.
        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
            tracing::warn!(skipped, "client fell behind the event stream");
            Some(Event {
                event_id: ids::new_id(),
                sequence: 0,
                payload: Some(farcooler_protocol::v1::event::Payload::EventsMissed(
                    farcooler_protocol::v1::Empty {},
                )),
            })
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
    }
}

/// Drains queued frames onto the wire until the sender half closes (the
/// `Connection` was dropped) or a write fails.
async fn run_writer<W>(
    writer: W,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
    queued_bytes: Arc<AtomicU64>,
    written: Arc<Notify>,
) where
    W: AsyncWrite + Unpin,
{
    let mut writer = FrameWriter::new(writer);
    while let Some(bytes) = rx.recv().await {
        let len = bytes.len() as u64;
        if writer.write_raw(&bytes).await.is_err() {
            break;
        }
        queued_bytes.fetch_sub(len, Ordering::SeqCst);
        written.notify_one();
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

    /// **A connection that fell behind the event stream is told so** (ov-20
    /// R-M9). The broadcast drops what a slow receiver missed; the daemon used
    /// to log it and carry on, and the client never learned anything was
    /// gone. Now the gap arrives as `events_missed`, and the events after it
    /// as themselves.
    #[tokio::test]
    async fn a_connection_that_fell_behind_is_told_it_missed_some() {
        use farcooler_protocol::v1::event::Payload;
        let (sender, receiver) = tokio::sync::broadcast::channel::<Event>(2);
        let fleet = || Event {
            event_id: ids::new_id(),
            sequence: 0,
            payload: Some(Payload::FleetChanged(farcooler_protocol::v1::Empty {})),
        };
        for _ in 0..5 {
            sender.send(fleet()).unwrap();
        }
        let mut events = Some(receiver);

        let first = next_event(&mut events).await.expect("the gap, said");
        assert!(
            matches!(first.payload, Some(Payload::EventsMissed(_))),
            "a dropped event went unmentioned: {:?}",
            first.payload
        );
        let next = next_event(&mut events).await.expect("what came after the gap");
        assert!(matches!(next.payload, Some(Payload::FleetChanged(_))));
    }

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

        let cfg = HandshakeConfig { daemon_version: "dtest".into() };
        let server_task = tokio::spawn(async move {
            let hello = server.handshake(&cfg, Scope::Control).await.unwrap();
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

    /// **A client that stops reading costs a bounded number of bytes, however
    /// fast its pushes come** (ov-118). A pane printing as fast as it can into a
    /// connection whose peer never reads: the loop used to move every push
    /// straight into the writer's channel, where it sat uncounted by anything
    /// but rule 4 — and rule 4 waits 30 seconds over 4 MiB before acting. Now
    /// the loop stops taking pushes at `PUSH_HIGH_WATER`, the backlog stays in
    /// the push queue, and the push queue refuses past its own limit.
    ///
    /// The producer here clears the queue on a refusal, the policy
    /// `terminal.attach` follows, so what is measured is the most this
    /// connection ever holds: the queue plus the writer.
    #[tokio::test]
    async fn a_stalled_reader_holds_a_bounded_backlog_of_pushes() {
        use crate::push::{Pushed, push_queue};
        use farcooler_protocol::v1::{TerminalFrame, TerminalOutput, event::Payload, terminal_frame};

        struct Pushing(std::sync::Mutex<Option<PushReceiver>>);
        impl crate::Handler for Pushing {
            fn peer(&self) -> crate::Peer {
                crate::Peer { client_id: None, scope: Scope::Control }
            }
            async fn handle(&self, req: Request) -> Response {
                Response { request_id: req.request_id, outcome: None }
            }
            fn pushes(&self) -> Option<PushReceiver> {
                self.0.lock().unwrap().take()
            }
        }

        const QUEUE_LIMIT: usize = 64 * 1024;
        const CHUNK: usize = 16 * 1024;
        let (push, pushes) = push_queue(QUEUE_LIMIT);

        // A tiny pipe whose far end is never read past the handshake.
        let (server_io, client_io) = tokio::io::duplex(1024);
        let (sr, sw) = tokio::io::split(server_io);
        let mut server = Connection::new(sr, sw);
        let backlog = server.queued_bytes.clone();
        let handler = Pushing(std::sync::Mutex::new(Some(pushes)));
        let serving = tokio::spawn(async move {
            let cfg = HandshakeConfig { daemon_version: "t".into() };
            let _ = serve_connection(&mut server, &cfg, &handler).await;
        });

        let (cr, cw) = tokio::io::split(client_io);
        let mut client = Connection::new(cr, cw);
        client.client_handshake("stalled", "0").await.unwrap();

        // 32 MiB of output: eight times rule 4's ceiling.
        let mut most = 0u64;
        let mut refused = 0;
        for i in 0..(32 * 1024 * 1024 / CHUNK) {
            let frame = Event {
                event_id: ids::new_id(),
                sequence: 0,
                payload: Some(Payload::TerminalFrame(TerminalFrame {
                    terminal_id: Bytes::from_static(b"pane"),
                    epoch: 0,
                    kind: Some(terminal_frame::Kind::Output(TerminalOutput {
                        start_sequence: (i * CHUNK) as u64,
                        payload: Bytes::from(vec![b'y'; CHUNK]),
                    })),
                })),
            };
            if push.push(frame) == Pushed::Full {
                refused += 1;
                push.clear();
            }
            // The loop gets a turn after every push, as it would against a
            // pane on another thread: the most favorable case for draining.
            tokio::task::yield_now().await;
            most = most.max(backlog.load(Ordering::SeqCst) + push.queued_bytes() as u64);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        most = most.max(backlog.load(Ordering::SeqCst) + push.queued_bytes() as u64);

        let frame_overhead = 1024;
        let bound = PUSH_HIGH_WATER + (CHUNK + frame_overhead) as u64 + QUEUE_LIMIT as u64;
        assert!(refused > 0, "the scenario should actually fill the queue");
        assert!(most <= bound, "a client that never reads held {most} bytes, more than {bound}");

        serving.abort();
        drop(client);
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
