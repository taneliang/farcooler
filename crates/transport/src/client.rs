//! The client half of the wire: connect, handshake, call.
//!
//! Written once and shared by the CLI, the tests, and eventually the SSH
//! transport, because a second implementation is a second set of bugs about
//! version negotiation and request correlation.
//!
//! Correlation is by `request_id` rather than by arrival order. The daemon is
//! free to answer out of order and to interleave events between responses, so a
//! client that assumed the next frame was its answer would eventually read an
//! event as a reply — rarely, and under load, which is the worst way to find
//! out.
//!
//! **Calls run alongside each other (ov-147).** `call` used to own the reader
//! for the length of one request — it wrote, then read frames until its own
//! answer came — so it took `&mut self`, and anything sharing a connection
//! queued behind whatever was in flight. On a phone that was a keystroke behind
//! a diff: the daemon has run requests concurrently since `serve_connection`
//! grew lanes, but the client never sent the second one until the first was
//! answered. Now a reader task owns the read half and hands each `Response` to
//! the caller waiting on its `request_id`, and a writer task owns the write
//! half, so `call` takes `&self` and any number can be outstanding.
//!
//! What that brings with it, each decided here once:
//!
//! - **Deadlines.** `call_with` takes one, and a call that outlives it fails
//!   with `TimedOut` naming the method. `call` has none, as before: the CLI
//!   and the tests wait as long as the daemon takes.
//! - **Urgent calls.** The writer sends urgent frames before ordinary ones that
//!   are still queued, so a keystroke never waits behind a large upload in the
//!   client's own queue. Order among urgent frames, and among ordinary ones, is
//!   the order they were sent in.
//! - **Cancellation.** A caller that gives up — its future dropped, or its
//!   deadline passed — is forgotten at once. If its request had not reached the
//!   wire yet it never does; if it had, the answer arrives for nobody and is
//!   dropped.
//! - **A reply for nobody** is dropped, logged, and counted
//!   (`stray_replies`), never handed to the next caller.
//! - **Disconnect.** When either half fails, every waiting call fails at once
//!   with the reason, and every later call fails before it is sent. Nothing
//!   waits for a deadline to learn the link is gone.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bytes::Bytes;
use farcooler_protocol::framing::FramingError;
use farcooler_protocol::v1::{
    ClientHello, Event, Request, Response, ServerHello, WireEnvelope, response, wire_envelope,
};
use farcooler_protocol::{PROTOCOL_VERSION, ids};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};

use crate::codec::{CodecError, FrameReader, FrameWriter, encode_frame};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("could not reach the daemon: {0}")]
    Connect(#[source] std::io::Error),
    #[error("the daemon closed the connection")]
    Closed,
    #[error("the daemon did not answer with a ServerHello")]
    NoHello,
    #[error("the daemon speaks protocol {daemon}, this client speaks {client}")]
    VersionMismatch { daemon: u32, client: u32 },
    /// The runner refused, and said what it refused.
    ///
    /// `what` names WHICH argument when `code` alone does not — a cycle, a
    /// blocker naming no task and a bad actor all arrive as
    /// `INVALID_ARGUMENT` otherwise, and a caller telling them apart by
    /// guessing from the call it just made is a confident, wrong sentence
    /// waiting to happen. Empty for every code that is its own whole answer.
    ///
    /// A word to switch on, never text to show: `Display` here is still the
    /// message alone, so nothing that wants prose has to change, and
    /// `farcooler_core::error::word_for` remains what an app renders from.
    #[error("{message}")]
    Daemon { code: i32, retryable: bool, message: String, what: String },
    #[error("the daemon returned an empty result")]
    EmptyResult,
    #[error("the daemon returned {got} where {expected} was expected")]
    WrongResult { expected: &'static str, got: &'static str },
    /// No answer by the call's deadline. The connection is left as it is:
    /// one slow answer is not a dead link, and the reader is still there to
    /// notice if it is one.
    #[error("{method} got no answer within {after:?}")]
    TimedOut { method: String, after: Duration },
}

/// How one call is made. The default is how `call` always behaved: no
/// deadline, and queued in turn.
#[derive(Debug, Clone, Copy, Default)]
pub struct CallOptions {
    /// Fail with `TimedOut` if no answer has arrived by then.
    pub deadline: Option<Duration>,
    /// Put the request on the wire ahead of ordinary requests still queued.
    pub urgent: bool,
}

/// How many events wait for `next_event` before the reader stops reading —
/// while no call is waiting.
///
/// The same pushback there always was. Before ov-147 nothing read the socket
/// between calls, so a client that stopped reading events — a terminal stream
/// whose viewer went away — left them in the socket, where the daemon's own
/// limits see them; and during a call, the events read past on the way to its
/// answer were kept, however many. So: past this many, the reader waits for
/// `next_event` to take one, unless a call is waiting for an answer, which is
/// never held up behind events nobody has read. A bound, not a tuned size.
const EVENT_BACKLOG: usize = 1024;

pub struct Client<R, W> {
    shared: Arc<Shared>,
    urgent: mpsc::UnboundedSender<Outgoing>,
    ordinary: mpsc::UnboundedSender<Outgoing>,
    server: ServerHello,
    reader: tokio::task::JoinHandle<()>,
    /// The halves now belong to the tasks; the types stay in the signature so
    /// every caller that names a `Client<R, W>` still does.
    _halves: std::marker::PhantomData<fn() -> (R, W)>,
}

/// What the reader, the writer and the callers share.
struct Shared {
    table: Mutex<Table>,
    /// Replies that arrived for nobody. See `stray_replies`.
    strays: AtomicU64,
    /// Set by `ignore_events`.
    ignore_events: AtomicBool,
    events: Mutex<Events>,
    /// An event was queued, or the connection ended.
    arrived: tokio::sync::Notify,
    /// The backlog shrank, or a call started waiting: the reader re-checks
    /// whether it may read again. See `EVENT_BACKLOG`.
    room: tokio::sync::Notify,
}

/// Events waiting for `next_event`, oldest first.
#[derive(Default)]
struct Events {
    queue: std::collections::VecDeque<Event>,
    /// The reader has stopped; what is queued is all there will be.
    ended: bool,
}

#[derive(Default)]
struct Table {
    waiting: HashMap<Bytes, oneshot::Sender<Result<Response, Gone>>>,
    /// Why the connection ended, once it has. Checked under the same lock a
    /// call registers under, so no call can register after the last one was
    /// failed and then wait forever.
    gone: Option<Gone>,
}

/// One request's frame, already encoded, and the id it waits under.
struct Outgoing {
    request_id: Bytes,
    frame: Vec<u8>,
}

/// Why the connection ended, kept in a form every waiting call can be handed
/// its own copy of. `ClientError` holds an `io::Error`, which cannot be cloned.
#[derive(Debug)]
enum Gone {
    Closed,
    Io(std::io::ErrorKind, String),
    Truncated,
    Framing(FramingError),
}

impl Gone {
    fn of(error: CodecError) -> Self {
        match error {
            CodecError::Io(e) => Gone::Io(e.kind(), e.to_string()),
            CodecError::Truncated => Gone::Truncated,
            CodecError::Framing(f) => Gone::Framing(f),
        }
    }

    /// The same error a caller got when it read the frame itself, so nothing
    /// that matched on `Closed` or `Codec` has to learn a new variant.
    fn error(&self) -> ClientError {
        match self {
            Gone::Closed => ClientError::Closed,
            Gone::Io(kind, message) => {
                ClientError::Codec(CodecError::Io(std::io::Error::new(*kind, message.clone())))
            }
            Gone::Truncated => ClientError::Codec(CodecError::Truncated),
            Gone::Framing(f) => ClientError::Codec(CodecError::Framing(copy(f))),
        }
    }
}

impl Clone for Gone {
    fn clone(&self) -> Self {
        match self {
            Gone::Closed => Gone::Closed,
            Gone::Io(kind, message) => Gone::Io(*kind, message.clone()),
            Gone::Truncated => Gone::Truncated,
            Gone::Framing(f) => Gone::Framing(copy(f)),
        }
    }
}

/// `FramingError` is not `Clone`, and is not this crate's to change for one
/// caller.
fn copy(f: &FramingError) -> FramingError {
    match f {
        FramingError::Oversized(a, b) => FramingError::Oversized(*a, *b),
        FramingError::ZeroLength => FramingError::ZeroLength,
        FramingError::Malformed => FramingError::Malformed,
    }
}

impl Shared {
    fn table(&self) -> MutexGuard<'_, Table> {
        lock(&self.table)
    }

    /// Fail every waiting call with `why`, and every later one before it is
    /// sent. The first reason wins: a writer that fails because the reader
    /// already saw the socket close is not news.
    fn end(&self, why: Gone) {
        let (why, waiting) = {
            let mut table = self.table();
            let why = table.gone.get_or_insert(why).clone();
            (why, std::mem::take(&mut table.waiting))
        };
        for (_, caller) in waiting {
            let _ = caller.send(Err(why.clone()));
        }
    }
}

/// A request on its way, and the answer it is waiting for.
///
/// Returned by `Client::send` once the request is queued for the wire, which
/// is what lets a caller fix the ORDER of two requests synchronously and wait
/// for their answers however it likes. Dropping it is giving up: the call is
/// forgotten at once, and a request not yet written is never written.
pub struct Answer {
    shared: Arc<Shared>,
    request_id: Bytes,
    method: String,
    deadline: Option<Duration>,
    answer: oneshot::Receiver<Result<Response, Gone>>,
}

impl Answer {
    /// Wait for the answer, until the deadline the call was sent with.
    pub async fn answer(mut self) -> Result<farcooler_protocol::v1::Result, ClientError> {
        let answered = match self.deadline {
            None => (&mut self.answer).await,
            Some(after) => match tokio::time::timeout(after, &mut self.answer).await {
                Ok(answered) => answered,
                Err(_) => {
                    let method = std::mem::take(&mut self.method);
                    return Err(ClientError::TimedOut { method, after });
                }
            },
        };
        match answered {
            Ok(Ok(response)) => unwrap_response(response),
            Ok(Err(gone)) => Err(gone.error()),
            // The sender went without a word, which only the reader being
            // aborted does: the client is being dropped.
            Err(_) => Err(ClientError::Closed),
        }
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        // Already gone if it was answered; there if the caller gave up.
        self.shared.table().waiting.remove(&self.request_id);
    }
}

impl Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf> {
    /// Connect to a daemon's Unix socket and complete the handshake.
    pub async fn connect(
        socket: impl AsRef<Path>,
        client_name: &str,
        client_version: &str,
    ) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket.as_ref()).await.map_err(ClientError::Connect)?;
        let (read, write) = stream.into_split();
        Self::over(read, write, client_name, client_version).await
    }
}

impl<R, W> Client<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    /// Handshake over any pair of streams, so the same client works over stdio
    /// through sshd as it does over a local socket.
    ///
    /// Then hands the halves to a reader task and a writer task, which is why
    /// this needs a tokio runtime and `'static` halves.
    pub async fn over(
        read: R,
        write: W,
        client_name: &str,
        client_version: &str,
    ) -> Result<Self, ClientError> {
        let mut reader = FrameReader::new(read);
        let mut writer = FrameWriter::new(write);

        writer
            .write_frame(&WireEnvelope {
                protocol_version: PROTOCOL_VERSION,
                message_id: ids::new_id(),
                body: Some(wire_envelope::Body::ClientHello(ClientHello {
                    supported_protocol_versions: vec![PROTOCOL_VERSION],
                    client_name: client_name.to_string(),
                    client_version: client_version.to_string(),
                })),
            })
            .await?;

        let envelope = reader.read_frame().await?.ok_or(ClientError::Closed)?;
        let server = match envelope.body {
            Some(wire_envelope::Body::ServerHello(server)) => server,
            // Refused at the handshake, with a reason. It used to land on
            // `NoHello` with the reason dropped, which every caller reads as
            // "nothing answered" and words as "is Far Cooler installed there?"
            // — to a person whose runner had answered, and said exactly what
            // was wrong. `Connection::refuse` and the version check in
            // `Connection::handshake` are what send this.
            Some(wire_envelope::Body::Response(Response {
                outcome: Some(response::Outcome::Error(e)), ..
            })) => {
                return Err(ClientError::Daemon {
                    code: e.code,
                    retryable: e.retryable,
                    message: e.message,
                    what: e.what,
                });
            }
            _ => return Err(ClientError::NoHello),
        };
        if server.selected_protocol_version != PROTOCOL_VERSION {
            return Err(ClientError::VersionMismatch {
                daemon: server.selected_protocol_version,
                client: PROTOCOL_VERSION,
            });
        }

        let shared = Arc::new(Shared {
            table: Mutex::new(Table::default()),
            strays: AtomicU64::new(0),
            ignore_events: AtomicBool::new(false),
            events: Mutex::new(Events::default()),
            arrived: tokio::sync::Notify::new(),
            room: tokio::sync::Notify::new(),
        });
        let (urgent, urgent_rx) = mpsc::unbounded_channel();
        let (ordinary, ordinary_rx) = mpsc::unbounded_channel();
        let reader = tokio::spawn(read_replies(reader, Arc::clone(&shared)));
        tokio::spawn(write_requests(writer, urgent_rx, ordinary_rx, Arc::clone(&shared)));

        Ok(Self {
            shared,
            urgent,
            ordinary,
            server,
            reader,
            _halves: std::marker::PhantomData,
        })
    }
}

impl<R, W> Drop for Client<R, W> {
    fn drop(&mut self) {
        // The writer ends by itself once both queues' senders are gone, after
        // sending what was queued; the reader would wait on the socket.
        self.reader.abort();
    }
}

impl<R, W> Client<R, W> {
    pub fn server_hello(&self) -> &ServerHello {
        &self.server
    }

    /// Drop events instead of keeping them for `next_event`.
    ///
    /// For a connection that only makes calls — a phone's control connection,
    /// whose events arrive on a channel of their own. Without this they pile
    /// up here while calls wait, and in the socket between calls once
    /// `EVENT_BACKLOG` is reached, for as long as the connection lives.
    pub fn ignore_events(&self) {
        self.shared.ignore_events.store(true, Ordering::Relaxed);
    }

    /// How many replies have arrived for a call nobody was waiting on.
    ///
    /// Each is also logged. Most are a call that gave up before its answer
    /// came; any other is a daemon answering an id it was never sent.
    pub fn stray_replies(&self) -> u64 {
        self.shared.strays.load(Ordering::Relaxed)
    }

    /// Wait for the next event.
    ///
    /// Events that arrived while a call was in flight come out first, in the
    /// order they arrived, so nothing is lost by having made a request at the
    /// wrong moment.
    pub async fn next_event(&mut self) -> Result<Event, ClientError> {
        let shared = &self.shared;
        loop {
            {
                let mut events = lock(&shared.events);
                if let Some(event) = events.queue.pop_front() {
                    shared.room.notify_one();
                    return Ok(event);
                }
                if events.ended {
                    break;
                }
            }
            // A permit is stored if the reader queued one between the check
            // and here, so this cannot miss it.
            shared.arrived.notified().await;
        }
        Err(shared.table().gone.clone().unwrap_or(Gone::Closed).error())
    }

    /// Send a request and wait for the response that matches it, with no
    /// deadline.
    ///
    /// `&mut self` though nothing here needs it: it kept the many one-caller
    /// sites — the CLI, every daemon test — compiling without a warning when
    /// calls became concurrent. A caller that shares a client uses `call_with`
    /// or `send`, which take `&self`.
    pub async fn call(
        &mut self,
        request: Request,
    ) -> Result<farcooler_protocol::v1::Result, ClientError> {
        self.call_with(request, CallOptions::default()).await
    }

    /// `call`, with a deadline, or ahead of ordinary requests, or both.
    pub async fn call_with(
        &self,
        request: Request,
        how: CallOptions,
    ) -> Result<farcooler_protocol::v1::Result, ClientError> {
        self.send(request, how)?.answer().await
    }

    /// Queue `request` for the wire now, and return what will wait for its
    /// answer.
    ///
    /// Synchronous on purpose: two calls to this are queued in the order they
    /// were made, whichever task then waits on which answer first.
    pub fn send(&self, mut request: Request, how: CallOptions) -> Result<Answer, ClientError> {
        let method = request.method.clone();
        let (answer_tx, answer) = oneshot::channel();
        let request_id = {
            let mut table = self.shared.table();
            if let Some(gone) = &table.gone {
                return Err(gone.error());
            }
            // An id is what an answer is filed under, so two calls cannot
            // share one. A caller that built its own request and left it
            // empty, or reused one, gets a fresh one rather than another
            // caller's answer.
            if request.request_id.is_empty() || table.waiting.contains_key(&request.request_id) {
                request.request_id = ids::new_id();
            }
            table.waiting.insert(request.request_id.clone(), answer_tx);
            request.request_id.clone()
        };
        // A reader holding off for a full event backlog must read this
        // call's answer regardless. See `EVENT_BACKLOG`.
        self.shared.room.notify_one();
        let waiting = Answer {
            shared: Arc::clone(&self.shared),
            request_id: request_id.clone(),
            method,
            deadline: how.deadline,
            answer,
        };

        // Encoded here rather than in the writer, so a request too large to
        // send fails this call alone and leaves the connection as it was.
        let frame = encode_frame(&WireEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: ids::new_id(),
            body: Some(wire_envelope::Body::Request(request)),
        })?;
        let queue = if how.urgent { &self.urgent } else { &self.ordinary };
        if queue.send(Outgoing { request_id, frame }).is_err() {
            // The writer has stopped, and `end` has said why.
            return Err(self.shared.table().gone.clone().unwrap_or(Gone::Closed).error());
        }
        Ok(waiting)
    }
}

/// The read half, for the life of the connection: answers to their callers,
/// events to `next_event`.
async fn read_replies<R: AsyncRead + Unpin>(mut reader: FrameReader<R>, shared: Arc<Shared>) {
    let why = loop {
        let envelope = match reader.read_frame().await {
            Ok(Some(envelope)) => envelope,
            Ok(None) => break Gone::Closed,
            Err(e) => break Gone::of(e),
        };
        match envelope.body {
            Some(wire_envelope::Body::Response(r)) => {
                let caller = shared.table().waiting.remove(&r.request_id);
                match caller {
                    // A caller that gave up between the lookup and this send
                    // has dropped its receiver; nothing is owed to it.
                    Some(caller) => drop(caller.send(Ok(r))),
                    None => {
                        shared.strays.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            request_id = ?r.request_id,
                            "dropped a reply nobody was waiting for"
                        );
                    }
                }
            }
            Some(wire_envelope::Body::Event(e)) => {
                if shared.ignore_events.load(Ordering::Relaxed) {
                    continue;
                }
                lock(&shared.events).queue.push_back(e);
                shared.arrived.notify_one();
                // Holds off reading while the backlog is full and no call is
                // waiting: see `EVENT_BACKLOG`.
                loop {
                    let full = lock(&shared.events).queue.len() >= EVENT_BACKLOG;
                    if !full || !shared.table().waiting.is_empty() {
                        break;
                    }
                    shared.room.notified().await;
                }
            }
            // Anything else has no business after the handshake, and never
            // ended a connection before; it does not now.
            _ => continue,
        }
    };
    shared.end(why);
    lock(&shared.events).ended = true;
    shared.arrived.notify_one();
}

/// A lock whose holder panicking leaves data that is still correct — a map
/// of senders, a queue of events — so poisoning is not a reason to fail.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The write half: urgent frames first, then ordinary ones, each queue in the
/// order it was filled.
async fn write_requests<W: AsyncWrite + Unpin>(
    mut writer: FrameWriter<W>,
    mut urgent: mpsc::UnboundedReceiver<Outgoing>,
    mut ordinary: mpsc::UnboundedReceiver<Outgoing>,
    shared: Arc<Shared>,
) {
    loop {
        let next = tokio::select! {
            biased;
            Some(next) = urgent.recv() => next,
            Some(next) = ordinary.recv() => next,
            else => break,
        };
        // Its caller has gone, so nothing is waiting for this answer: a
        // request cancelled before it was sent is never sent.
        if !shared.table().waiting.contains_key(&next.request_id) {
            continue;
        }
        if let Err(e) = writer.write_raw(&next.frame).await {
            shared.end(Gone::of(e));
            break;
        }
    }
}

fn unwrap_response(r: Response) -> Result<farcooler_protocol::v1::Result, ClientError> {
    match r.outcome {
        Some(response::Outcome::Result(value)) => Ok(value),
        Some(response::Outcome::Error(e)) => {
            Err(ClientError::Daemon {
                code: e.code,
                retryable: e.retryable,
                message: e.message,
                what: e.what,
            })
        }
        None => Err(ClientError::EmptyResult),
    }
}

/// Build a request. `method` decides everything else about it.
pub fn request(method: &str) -> Request {
    Request {
        request_id: ids::to_bytes(uuid::Uuid::now_v7()),
        method: method.to_string(),
        target_resource_id: None,
        expected_resource_version: None,
        expected_lease_generation: None,
        idempotency_key: None,
        // Empty by default, which is the honest answer for every call that
        // asks only for what has always existed. A caller filling in a field a
        // newer daemon added names the capability it belongs to; see
        // `Request.required_capabilities` in the proto for why that is on the
        // envelope rather than checked at each call site.
        required_capabilities: Vec::new(),
        payload: Some(farcooler_protocol::v1::request::Payload::Empty(
            farcooler_protocol::v1::Empty {},
        )),
    }
}
