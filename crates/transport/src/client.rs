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
//!   with `TimedOut` naming the method. An urgent call's starts when it is
//!   written. `call` has none, as before: the CLI
//!   and the tests wait as long as the daemon takes.
//! - **Urgent calls.** The writer sends urgent frames before ordinary ones that
//!   are still queued, so a keystroke never waits behind a large upload in the
//!   client's own queue. Order among urgent frames, and among ordinary ones, is
//!   the order they were sent in.
//! - **Cancellation.** A caller that gives up — its future dropped, or its
//!   deadline passed — is forgotten at once. If its request had not reached the
//!   wire yet it never does; if it had, the answer arrives for nobody and is
//!   dropped. Urgent requests are the exception: input is never taken out of
//!   the queue, so no key goes missing ahead of one that was sent.
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
    /// An urgent (input) call whose connection ended before the writer
    /// began its frame, so no byte of it ever reached the wire. `cause` is
    /// what the call would have failed with otherwise.
    ///
    /// Never built once the writer has started the frame, even if it
    /// stopped partway: a partial frame may still have been read.
    #[error(transparent)]
    NotWritten(Box<ClientError>),
}

/// How one call is made. The default is how `call` always behaved: no
/// deadline, and queued in turn.
#[derive(Debug, Clone, Copy, Default)]
pub struct CallOptions {
    /// Fail with `TimedOut` if no answer has arrived by then. For an urgent
    /// call, counted from when the request was written, not queued.
    pub deadline: Option<Duration>,
    /// Input: put the request on the wire ahead of ordinary requests still
    /// queued, and never take it out of the queue.
    ///
    /// An ordinary request whose caller gives up is never written. That is
    /// wrong for keys: on a writer stalled past the deadline, the oldest keys
    /// would be dropped while newer ones, Enter among them, still went out. So
    /// an urgent request, once queued, is written in its turn whatever its
    /// caller does, and its deadline starts only once it is on the wire. If
    /// the connection ends first, every queued one fails together.
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
    /// Urgent: written even if its caller has gone. See `CallOptions::urgent`.
    keep: bool,
    /// Told when the frame is on the wire, for a deadline that starts there.
    written: Option<oneshot::Sender<()>>,
    /// Set by the writer, under the table lock that checks the connection
    /// is still up, just before it writes any of this frame. See
    /// `ClientError::NotWritten`.
    begun: Arc<AtomicBool>,
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
/// forgotten at once, and a request not yet written is never written — unless
/// it is urgent (input), which keeps its place in the queue.
pub struct Answer {
    shared: Arc<Shared>,
    request_id: Bytes,
    method: String,
    deadline: Option<Duration>,
    answer: oneshot::Receiver<Result<Response, Gone>>,
    /// For an urgent call: fires once the request is on the wire, which is
    /// where its deadline starts.
    written: Option<oneshot::Receiver<()>>,
    /// Whether the writer began this request's frame: see `Outgoing::begun`.
    begun: Arc<AtomicBool>,
    urgent: bool,
}

impl Answer {
    /// Wait for the answer, until the deadline the call was sent with.
    pub async fn answer(mut self) -> Result<farcooler_protocol::v1::Result, ClientError> {
        if let Some(written) = self.written.take() {
            // No deadline while it waits its turn. An answer can beat the
            // signal; a dropped signal means the writer stopped, and the
            // connection's end is what answers then.
            tokio::select! {
                biased;
                answered = &mut self.answer => return self.settled(answered),
                _ = written => {}
            }
        }
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
        self.settled(answered)
    }

    /// `settle`, except that input the writer never began is reported as
    /// `NotWritten`. Read after the failure arrived: the writer sets `begun`
    /// under the lock `end` takes, so a frame it began before the end is
    /// always seen as begun here.
    fn settled(
        &self,
        answered: Result<Result<Response, Gone>, oneshot::error::RecvError>,
    ) -> Result<farcooler_protocol::v1::Result, ClientError> {
        match settle(answered) {
            Err(e) if self.urgent && failed_with_connection(&e) && !self.begun.load(Ordering::SeqCst) => {
                Err(ClientError::NotWritten(Box::new(e)))
            }
            other => other,
        }
    }
}

/// The errors a connection's end produces, as opposed to an answer's.
fn failed_with_connection(e: &ClientError) -> bool {
    matches!(e, ClientError::Closed | ClientError::Codec(_))
}

fn settle(
    answered: Result<Result<Response, Gone>, oneshot::error::RecvError>,
) -> Result<farcooler_protocol::v1::Result, ClientError> {
    match answered {
        Ok(Ok(response)) => unwrap_response(response),
        Ok(Err(gone)) => Err(gone.error()),
        // The sender went without a word. `end` answers every waiter, so
        // only a client torn down mid-call gets here.
        Err(_) => Err(ClientError::Closed),
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
        // Every waiter fails now, rather than an `Answer` that outlived its
        // client waiting forever; the writer stops at that, and ends once both
        // queues' senders are gone. The reader would wait on the socket.
        self.reader.abort();
        self.shared.end(Gone::Closed);
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

    /// Whether the connection has ended, for whatever reason.
    ///
    /// True once the reader has seen the close, not when the peer went away: the
    /// two differ by however long the reader takes to notice. A test that needs
    /// "the link is known dead" waits on this rather than on a sleep.
    pub fn has_ended(&self) -> bool {
        self.shared.table().gone.is_some()
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
                let error = gone.error();
                // Nothing was queued, so nothing can have been written.
                return Err(match how.urgent {
                    true => ClientError::NotWritten(Box::new(error)),
                    false => error,
                });
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
        let begun = Arc::new(AtomicBool::new(false));
        let (written_tx, written) = match how.urgent {
            true => {
                let (tx, rx) = oneshot::channel();
                (Some(tx), Some(rx))
            }
            false => (None, None),
        };
        let waiting = Answer {
            shared: Arc::clone(&self.shared),
            request_id: request_id.clone(),
            method,
            deadline: how.deadline,
            answer,
            written,
            begun: Arc::clone(&begun),
            urgent: how.urgent,
        };

        // Encoded here rather than in the writer, so a request too large to
        // send fails this call alone and leaves the connection as it was.
        let frame = encode_frame(&WireEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: ids::new_id(),
            body: Some(wire_envelope::Body::Request(request)),
        })?;
        let queue = if how.urgent { &self.urgent } else { &self.ordinary };
        let outgoing = Outgoing { request_id, frame, keep: how.urgent, written: written_tx, begun };
        if queue.send(outgoing).is_err() {
            // The writer has stopped, and `end` has said why.
            let error = self.shared.table().gone.clone().unwrap_or(Gone::Closed).error();
            return Err(match how.urgent {
                true => ClientError::NotWritten(Box::new(error)),
                false => error,
            });
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
                // Unbounded only while a call waits, and that is bounded in
                // practice: it holds what arrives during one call's life,
                // every phone call has a deadline, the connections that read
                // events (a terminal stream, the fleet channel) make one call
                // before streaming, and the phones' control connection
                // ignores its events. A client that waits on a call with no
                // deadline while a runner streams at it is the case left, and
                // before ov-147 it buffered the same events the same way.
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
        let (ended, waited_for) = {
            let table = shared.table();
            let ended = table.gone.is_some();
            // Under the lock `end` takes: see `ClientError::NotWritten`.
            if !ended {
                next.begun.store(true, Ordering::SeqCst);
            }
            (ended, table.waiting.contains_key(&next.request_id))
        };
        // Every queued call was failed when the connection ended; writing one
        // now would deliver what its caller was told did not go.
        if ended {
            break;
        }
        // Its caller has gone, so nothing is waiting for this answer: a
        // request cancelled before it was sent is never sent — unless it is
        // input, which keeps its place. See `CallOptions::urgent`.
        if !waited_for && !next.keep {
            continue;
        }
        if let Err(e) = writer.write_raw(&next.frame).await {
            shared.end(Gone::of(e));
            break;
        }
        if let Some(written) = next.written {
            let _ = written.send(());
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
