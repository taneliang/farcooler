//! An Far Cooler session, over whichever transport reached the runner.
//!
//! Above this line nothing knows whether it is talking to a local Unix socket
//! or to a daemon on the other side of an SSH connection. That is the property
//! worth protecting: a remote runner is not a different product with its own
//! subtly different behavior.
//!
//! Answers come back as JSON rather than protobuf. Not because JSON is better —
//! it is not, and the wire stays protobuf — but because the alternative is a
//! protobuf runtime and generated types in Swift and again in Kotlin, to
//! describe messages this crate has already decoded. JSON at the boundary means
//! the iOS app decodes the same shapes the Mac app already decodes from the
//! CLI, with the model types it already has.

use farcooler_protocol::v1::{
    AgentEventBatch, PaneMode, Repository, RepositoryRoot, Terminal, TerminalState, Workspace,
    WorkspaceState, request, result,
};
use farcooler_transport::{Client, ClientError, CodecError};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use crate::changes_json::{stack_json, change_set_json, file_change_json, file_diff_json, inbox_json};
use crate::ssh;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Ssh(#[from] ssh::SshError),
    #[error("{0}")]
    Protocol(String),
    #[error("the daemon returned {got} where {expected} was expected")]
    WrongResult { expected: &'static str, got: &'static str },
    /// Names the binary this client actually asked for.
    ///
    /// A preview client asks for `farcoolerd-preview`, so a message naming
    /// `farcoolerd` would send someone to check the wrong thing — and finding
    /// it installed would make the error look like a lie.
    #[error(
        "connected, but `{daemon} --stdio` did not answer. Is Far Cooler installed on that runner?"
    )]
    DaemonMissing { daemon: &'static str },
    #[error("the runner runs protocol {daemon}; this client speaks {client}")]
    VersionMismatch { daemon: u32, client: u32 },
    /// The link underneath this session is gone.
    ///
    /// Deliberately distinct from `Protocol`, which is the far side saying
    /// something we could not make sense of — that is a session still worth
    /// talking on, and a client that reconnected over it would be reconnecting
    /// over a bug.
    #[error("the connection to the runner was lost: {0}")]
    Disconnected(String),
}

impl SessionError {
    /// Whether this means the link is gone, rather than that the request was
    /// refused.
    ///
    /// Only meaningful for a call on an ESTABLISHED session. At connect time
    /// these same variants answer a different question — what could not be
    /// reached in the first place — and `DaemonMissing` in particular means
    /// something specific and actionable there ("go install it"), which is why
    /// `connect_ssh` keeps that reading and this does not. Once a session
    /// exists, a closed pipe is a closed pipe.
    pub fn is_disconnect(&self) -> bool {
        match self {
            SessionError::Ssh(_)
            | SessionError::Disconnected(_)
            | SessionError::DaemonMissing { .. } => true,
            SessionError::Protocol(_)
            | SessionError::WrongResult { .. }
            | SessionError::VersionMismatch { .. } => false,
        }
    }
}

impl From<ClientError> for SessionError {
    fn from(error: ClientError) -> Self {
        match error {
            // Silence from the far side almost always means the command did not
            // exist, not that the protocol went wrong.
            ClientError::Closed | ClientError::NoHello => {
                SessionError::DaemonMissing { daemon: daemon_binary() }
            }
            ClientError::VersionMismatch { daemon, client } => {
                SessionError::VersionMismatch { daemon, client }
            }
            // The socket, not the conversation on it. `Framing` is deliberately
            // not here: bytes that do not decode are a bug on one side or the
            // other, and calling that a dropped link would send a client into a
            // reconnect loop that reproduces it on every attempt.
            ClientError::Connect(e) => SessionError::Disconnected(e.to_string()),
            ClientError::Codec(CodecError::Io(e)) => SessionError::Disconnected(e.to_string()),
            ClientError::Codec(e @ CodecError::Truncated) => {
                SessionError::Disconnected(e.to_string())
            }
            other => SessionError::Protocol(other.to_string()),
        }
    }
}

type Reader = Box<dyn AsyncRead + Unpin + Send>;
type Writer = Box<dyn AsyncWrite + Unpin + Send>;

/// A terminal stream: the bytes, plus the write half that keeps it open.
///
/// The writer is never written to and exists only to not be dropped. See
/// `Session::open_stream` for what dropping it does.
struct StreamReader {
    reader: ssh::ChannelReader,
    _writer: std::pin::Pin<Box<dyn AsyncWrite + Send>>,
}

impl AsyncRead for StreamReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

/// A connected Far Cooler runner.
pub struct Session {
    client: Client<Reader, Writer>,
    /// Held so the SSH connection lives as long as the session does.
    _ssh: Option<ssh::Session>,
    /// Where fleet news goes, once somebody has asked for it.
    ///
    /// `None` until `subscribe` is called, which is the honest default: this
    /// type is also the CLI's and the tests', and neither wants a callback
    /// firing on a runtime thread. Set by `subscribe` BEFORE it opens its
    /// channel, so a runner where the dedicated channel could not be opened
    /// still forwards whatever fleet news arrives on a terminal attachment —
    /// see `attach_stream`, which is subscribed to the same broadcast for free.
    events: Option<EventSink>,
    /// The socket this session was opened on, when it was opened on one.
    ///
    /// Only `subscribe` reads it, and only so the push path can be proven
    /// against a real daemon without an ssh server in the test — the same
    /// argument `tests/against_a_real_daemon.rs` already makes for everything
    /// else it covers: ssh is a byte pipe, and what can go wrong here is above
    /// it. A desktop client on a local runner gets the same channel for free.
    socket: Option<std::path::PathBuf>,
}

/// What a client is told when the runner says something changed.
///
/// **These carry no delta on purpose.** Every variant means "re-read it", and
/// that is the whole contract: the daemon's own `fleet_changed` says so in the
/// proto ("a client re-reads the fleet rather than trying to apply it as a
/// delta"), reconciliation both creates and deletes rows in one pass, and three
/// editors — the app, the CLI and an agent driving the CLI — move the same
/// state. A client that applied deltas would need to be right about every one
/// of those; a client that re-reads needs only to be told that it should.
///
/// The consequence is the one that makes the whole push path cheap: losing one
/// of these is harmless as long as the client learns it missed SOMETHING, so
/// the boundary is free to drop and coalesce rather than buffer. See
/// `EventQueue` in `ffi.rs`, which is where that policy is written down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FleetEvent {
    /// Something in the fleet moved: a terminal's state, a workspace, a
    /// layout, an operation, or the set of workspaces itself.
    ///
    /// One variant for all five because they have one answer — re-read
    /// `fleet` — and a client that told them apart would make the same call
    /// five times. It is also what makes coalescing work at all: a busy runner
    /// samples every second (`SAMPLE_INTERVAL` in `crates/daemon/src/watch.rs`)
    /// and a working agent can move a row on every sample, so the events that
    /// arrive in a burst have to collapse to one notice or the "push" is just
    /// a faster poll.
    Fleet,
    /// One worktree's diff moved. The set itself is not carried — see
    /// `announce_change_set` in `crates/daemon/src/watch.rs:2511` for why the
    /// daemon does not fan thousands of file records out to every device.
    ChangeSet { workspace: Uuid },
    /// A repository's pull request state was re-read, because somebody's
    /// `stack.get` found a cache nobody had filled.
    ///
    /// The event the phone's pull request row was invented around: the daemon
    /// fills that cache in the BACKGROUND and answers the read from cache at
    /// local speed, so without this the only delivery mechanism was asking
    /// again on the next poll. See `crates/daemon/src/watch.rs:2538`.
    Stack { repository: Uuid },
}

impl FleetEvent {
    /// Read one wire event as news, or as nothing.
    ///
    /// `None` for the arms that are not fleet news: `TerminalFrame` is one
    /// connection's terminal bytes and has a reader of its own, and
    /// `AgentEvents` is a transcript batch which `agent.events` pages by
    /// cursor — turning either into "re-read the fleet" would spend a round
    /// trip re-reading state that did not change.
    fn of(payload: farcooler_protocol::v1::event::Payload) -> Option<Self> {
        use farcooler_protocol::v1::event::Payload;
        match payload {
            Payload::TerminalChanged(_)
            | Payload::WorkspaceChanged(_)
            | Payload::OperationChanged(_)
            | Payload::LayoutChanged(_)
            | Payload::FleetChanged(_) => Some(FleetEvent::Fleet),
            Payload::ChangeSetChanged(c) => {
                Some(FleetEvent::ChangeSet { workspace: uuid_of(&c.workspace_id) })
            }
            Payload::StackChanged(s) => {
                Some(FleetEvent::Stack { repository: uuid_of(&s.repository_id) })
            }
            Payload::TerminalFrame(_) | Payload::AgentEvents(_) => None,
            // Reserved arms no daemon emits. Named one by one rather than
            // swept up by `_`, so that the day one of them starts being sent
            // this match stops compiling and somebody decides what a client
            // should re-read — which for these is the repository list, not the
            // fleet, and so is not a decision to make by default.
            Payload::HostChanged(_)
            | Payload::RepositoryRootChanged(_)
            | Payload::RepositoryChanged(_) => None,
        }
    }
}

/// Where fleet news is delivered.
///
/// A plain callback rather than a channel, and that is the design rather than
/// a shortcut. A bounded channel would make the daemon's reader wait on a slow
/// consumer — the one thing a push path must never do — and an unbounded one
/// would grow without limit behind a consumer that stopped. A callback pushes
/// the decision to the layer that can actually make it: the FFI takes a lock,
/// coalesces into a fixed-size queue, and returns. Nothing here ever blocks.
///
/// It is called from a runtime thread, so it must not block and must not panic.
pub type EventSink = std::sync::Arc<dyn Fn(FleetEvent) + Send + Sync>;

/// The daemon this client is built to talk to.
///
/// One place rather than at each call site: the two `exec` invocations below
/// and the error that names the binary must agree, and a client that asked for
/// one daemon and then reported another would be actively misleading.
fn daemon_binary() -> &'static str {
    farcooler_protocol::CHANNEL.daemon_binary_name()
}

impl Session {
    /// Connect over SSH and start a daemon session on the far side.
    pub async fn connect_ssh(destination: &ssh::Destination) -> Result<Self, SessionError> {
        let mut transport = ssh::Session::open(destination).await?;
        // Named by tilde, not bare: a non-login ssh exec's PATH often lacks
        // ~/.local/bin, where `runner install` puts the binary.
        //
        // The name carries this client's own channel, which is what keeps a
        // preview app off a stable daemon: they are different binaries at
        // different paths, so the two never meet rather than meeting and
        // having to negotiate.
        let streams = transport.exec(&format!("~/.local/bin/{} --stdio", daemon_binary())).await?;

        let client = Client::over(
            Box::new(streams.reader) as Reader,
            Box::new(streams.writer) as Writer,
            "farcooler-mobile",
            env!("CARGO_PKG_VERSION"),
        )
        .await?;

        Ok(Self { client, _ssh: Some(transport), events: None, socket: None })
    }

    /// Open a second ssh channel carrying nothing but one terminal's bytes.
    ///
    /// The data plane, kept off the control connection on purpose. That
    /// connection answers one request at a time — `Client::call` reads frames
    /// until it sees its own response — so a stream sharing it would sit behind
    /// every fleet refresh and every refresh behind a scrollback replay. ssh
    /// already multiplexes channels; letting it do that is both simpler and
    /// faster than teaching the client to interleave.
    ///
    /// The latency floor becomes the network round trip. Measured at 19ms over a
    /// loopback ssh connection, against a second of polling before it — and the
    /// second was never the network, it was the interval.
    ///
    /// **Which command runs on that channel is the whole of this method.** For
    /// most of this product's life it was `farcoolerd --stream <id>`, and on
    /// every enrolled device that was silence: `crates/fence` writes
    /// `restrict,command="~/.local/bin/farcoolerd --stdio …"`, a forced command
    /// is OpenSSH discarding what the client asked to run, and what actually ran
    /// was a `--stdio` protocol relay — which says nothing until it is spoken
    /// to. The channel opened, this returned a reader, and no byte ever arrived.
    /// It worked on a Mac and in every loopback test because both hold a plain
    /// key, which forces nothing.
    ///
    /// So a runner that speaks `terminal_stream` is spoken to. `--stdio` is the
    /// command BOTH key types run — sshd substitutes it on a forced-command
    /// line, and `main.rs` matches it before `--stream` on a plain one — so
    /// which line this device holds stops being something the stream depends on.
    pub async fn open_stream(
        &mut self,
        terminal: Uuid,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, SessionError> {
        if self.capabilities().iter().any(|c| c == farcooler_protocol::capability::TERMINAL_STREAM)
        {
            return self.attach_stream(terminal).await;
        }

        // A runner too old to know the method. `--stream` still works there for
        // a client whose key forces nothing — a Mac's plain shell line, a local
        // test — and for one whose key does, this is the silence that was there
        // before, which the caller already survives by falling back to polling.
        // Not an error: a runner that cannot stream is not a runner that cannot
        // show a terminal, and refusing here would take the poll fallback's
        // trigger away from it.
        let ssh = self._ssh.as_mut().ok_or_else(|| {
            SessionError::Protocol("streaming needs an ssh session".into())
        })?;
        // Named by tilde, not bare: see connect_ssh above.
        let streams =
            ssh.exec(&format!("~/.local/bin/{} --stream {terminal}", daemon_binary())).await?;
        // The write half is kept, though nothing is ever written to it.
        //
        // Dropping it closes that side of the channel, and ssh reports a closed
        // stdin to the far end — which the daemon reads, correctly, as "nobody
        // is watching any more" and exits. So a stream that dropped its writer
        // ended a second or two after it opened, every time, and did it with a
        // clean end-of-file that looked exactly like a pane finishing.
        //
        // Held open, the same closure means what it should: this stream ends
        // when the session holding it goes away.
        Ok(Box::new(StreamReader { reader: streams.reader, _writer: streams.writer }))
    }

    /// The stream as a wire method: a second `--stdio` session, one
    /// `terminal.attach`, and the `Event` frames that follow it.
    ///
    /// A whole second protocol session rather than a bare byte pipe, which is
    /// more than the old exec cost — a handshake and one round trip before the
    /// first byte. That is the price of the second channel running the one
    /// command sshd cannot substitute away, and it is paid once per pane
    /// opened, against a replay that has to be captured anyway.
    ///
    /// The result is read and discarded on purpose. Its `epoch`,
    /// `next_sequence` and `resume_token` describe a resume no runner serves
    /// yet, and reading them into a reader that has nowhere to put them would
    /// be inventing a client-side model of something the daemon does not do.
    /// Awaiting it is not optional, though: it is what turns "this terminal is
    /// not running" into an error the caller can report, rather than a stream
    /// that opens and immediately ends looking exactly like a pane that
    /// finished.
    async fn attach_stream(
        &mut self,
        terminal: Uuid,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, SessionError> {
        let ssh = self._ssh.as_mut().ok_or_else(|| {
            SessionError::Protocol("streaming needs an ssh session".into())
        })?;
        // The same command the control connection runs, and deliberately so.
        let streams = ssh.exec(&format!("~/.local/bin/{} --stdio", daemon_binary())).await?;
        let mut client = Client::over(
            Box::new(streams.reader) as Reader,
            Box::new(streams.writer) as Writer,
            "farcooler-mobile",
            env!("CARGO_PKG_VERSION"),
        )
        .await?;

        let mut attach = farcooler_transport::request("terminal.attach");
        attach.target_resource_id = Some(bytes::Bytes::copy_from_slice(terminal.as_bytes()));
        // Named even though `capabilities()` was already checked, for the reason
        // `Request.required_capabilities` exists: the check above reads a hello
        // this session took at connect time, and the runner may have been
        // upgraded — or downgraded — since.
        attach.required_capabilities =
            vec![farcooler_protocol::capability::TERMINAL_STREAM.to_string()];
        attach.payload = Some(request::Payload::TerminalAttach(
            farcooler_protocol::v1::TerminalAttach {
                // Nothing to resume from. See `TerminalAttach` in the proto.
                last_acked_sequence: None,
                resume_token: None,
            },
        ));
        let result = client.call(attach).await?;
        let Some(result::Value::TerminalAttach(_)) = result.value else {
            return Err(SessionError::WrongResult {
                expected: "TerminalAttachResult",
                got: "something else",
            });
        };

        // The seam back to a reader, so the FFI and both apps see the shape they
        // always saw: bytes arriving, and an end when they stop. Only one
        // direction of the duplex is used — nothing is ever written back to the
        // daemon on this channel, because input goes through the control
        // connection's `terminal.write`.
        //
        // Dropping the far half is what gives the reader its end of stream, and
        // it happens exactly when the task below returns: on a closed ssh
        // channel, on a daemon that stopped, or on the reader itself being
        // dropped, which fails the write. The last of those is what returns the
        // ssh session slot — `ChannelGuard` in `ssh.rs` — so a client that stops
        // reading stops holding one of sshd's ten.
        // Fleet news arriving on THIS channel, forwarded rather than dropped.
        //
        // Free, and worth having for that alone: a terminal attachment is
        // subscribed to the daemon's broadcast like every other connection
        // (`Rpc::events` in `crates/daemon/src/rpc.rs:230` subscribes each one
        // with no opt-in), so these frames were already crossing the wire and
        // being thrown away. It is also the backstop for the case
        // `subscribe` cannot serve: a runner where the dedicated channel could
        // not be opened still pushes while a pane is open.
        //
        // Duplicates with the dedicated channel are expected and are exactly
        // what the boundary's coalescing is for — two "re-read the fleet"
        // notices a millisecond apart are one notice.
        let news = self.events.clone();
        let (mine, theirs) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut sink = theirs;
            loop {
                let Ok(event) = client.next_event().await else { break };
                let Some(payload) = event.payload else { continue };
                // A `match` rather than the `let ... else` this replaces,
                // because the else arm now needs the payload it did not take.
                let frame = match payload {
                    farcooler_protocol::v1::event::Payload::TerminalFrame(frame) => frame,
                    other => {
                        if let (Some(news), Some(what)) = (&news, FleetEvent::of(other)) {
                            news(what);
                        }
                        continue;
                    }
                };
                // A frame for a pane this session did not attach to could only
                // come from an attachment that was replaced, and feeding its
                // tail to this emulator would paint another terminal's output.
                if frame.terminal_id.as_ref() != terminal.as_bytes() {
                    continue;
                }
                let Some(farcooler_protocol::v1::terminal_frame::Kind::Output(output)) = frame.kind
                else {
                    continue;
                };
                if sink.write_all(&output.payload).await.is_err() {
                    break;
                }
            }
        });

        Ok(Box::new(mine))
    }

    /// Receive fleet news on a channel of its own, for as long as this session
    /// lives.
    ///
    /// **Why a third channel and not the control connection.** `Client::call`
    /// owns the reader for the length of a request — it reads frames until it
    /// sees its own response — so nothing reads the control socket between
    /// calls, and events sit in the kernel buffer until the next request
    /// happens to drain them. That is the shape that produced polling: the only
    /// way to learn something changed was to ask. Teaching the control client
    /// to demultiplex responses from a background reader is a transport rewrite
    /// with a correlation table and a lock in it; ssh already multiplexes
    /// channels, and letting it do that is the same trade `open_stream` made
    /// one layer down and for the same reason.
    ///
    /// The cost is one sshd channel and one handshake, paid once per
    /// connection, against a round trip and a radio wake every three seconds
    /// for as long as the app is open.
    ///
    /// No request is sent on it. `Rpc::events` in
    /// `crates/daemon/src/rpc.rs:230` subscribes every connection to the
    /// broadcast with no opt-in — "a client that connected wants to know when
    /// something changes, and making it ask would just be a round trip before
    /// the first event" — so the handshake IS the subscription.
    ///
    /// `--stdio`, like `attach_stream` and for the same reason: it is the one
    /// command both key types run, because sshd substitutes it on an enrolled
    /// device's forced-command line and `main.rs` matches it on a plain key.
    ///
    /// The returned handle finishes when the subscription does — the ssh
    /// channel closed, the daemon stopped, or this session was dropped. A
    /// caller that shows "live" anywhere should await it and stop saying so.
    pub async fn subscribe(
        &mut self,
        sink: EventSink,
    ) -> Result<tokio::task::JoinHandle<()>, SessionError> {
        // Kept even if the channel below cannot be opened. See the field: a
        // terminal attachment carries the same broadcast, so a session that
        // failed to get its own channel is not a session with no push at all.
        self.events = Some(std::sync::Arc::clone(&sink));

        let mut client = match self._ssh.as_mut() {
            Some(ssh) => {
                // Named by tilde, not bare: see `connect_ssh` above.
                let streams =
                    ssh.exec(&format!("~/.local/bin/{} --stdio", daemon_binary())).await?;
                Client::over(
                    Box::new(streams.reader) as Reader,
                    Box::new(streams.writer) as Writer,
                    "farcooler-mobile",
                    env!("CARGO_PKG_VERSION"),
                )
                .await?
            }
            // A second connection to the same socket. Not an ssh channel, but
            // the same thing one layer down, and the daemon cannot tell the
            // difference: it subscribes whatever connected.
            None => {
                let socket = self.socket.as_ref().ok_or_else(|| {
                    SessionError::Protocol("this session has nothing to open a channel on".into())
                })?;
                let stream = tokio::net::UnixStream::connect(socket).await.map_err(|e| {
                    SessionError::Disconnected(format!("cannot reach the daemon: {e}"))
                })?;
                let (read, write) = stream.into_split();
                Client::over(
                    Box::new(read) as Reader,
                    Box::new(write) as Writer,
                    "farcooler-mobile",
                    env!("CARGO_PKG_VERSION"),
                )
                .await?
            }
        };

        Ok(tokio::spawn(async move {
            loop {
                // Any error ends the subscription rather than retrying here.
                // Reconnecting is the session's business and it already has a
                // schedule for it; a retry loop at this depth would be a second
                // one, disagreeing with the first about when to give up.
                let Ok(event) = client.next_event().await else { break };
                let Some(payload) = event.payload else { continue };
                if let Some(what) = FleetEvent::of(payload) {
                    sink(what);
                }
            }
        }))
    }

    /// Connect to a daemon on this runner. Used by tests and by any desktop
    /// client that wants the same API as the mobile one.
    pub async fn connect_local(socket: &std::path::Path) -> Result<Self, SessionError> {
        let stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(|e| SessionError::Protocol(format!("cannot reach the daemon: {e}")))?;
        let (read, write) = stream.into_split();
        let client = Client::over(
            Box::new(read) as Reader,
            Box::new(write) as Writer,
            "farcooler-mobile",
            env!("CARGO_PKG_VERSION"),
        )
        .await?;
        Ok(Self { client, _ssh: None, events: None, socket: Some(socket.to_path_buf()) })
    }

    pub fn daemon_version(&self) -> &str {
        &self.client.server_hello().daemon_version
    }

    /// What the runner on the other end can do, by name.
    ///
    /// Learned in the handshake, so it is available before the first request.
    /// A client that is newer than the runner it reached uses this to DIM the
    /// controls that runner cannot serve, with a reason — rather than hiding
    /// them, which makes the same app look different on two runners for no
    /// stated cause, or letting them fail on use, which teaches people that
    /// buttons sometimes do nothing.
    pub fn capabilities(&self) -> &[String] {
        &self.client.server_hello().capabilities
    }

    /// What THIS session is allowed to ask for, as the word `authorized_keys`
    /// spells it.
    ///
    /// The pair to `capabilities`, and neither substitutes for the other: a
    /// capability is what the RUNNER can serve, a scope is what this connection
    /// may ask it for, and a control needs both. Learned in the same handshake,
    /// so it is available before the first request at no round trip —
    /// `crates/transport`'s `handshake` has always computed it from the session's
    /// real grant and sent it in `ServerHello`. Nothing above the transport read
    /// it, which is why both phones ask and report the refusal.
    ///
    /// Deliberately NOT read out of `client.list`, which looks like it answers
    /// this and does not, three ways over: it reports what is written in the
    /// fence rather than what sshd applied to THIS session; a key somebody added
    /// by hand has no client id to match on and reads back `unspecified` while
    /// the real grant is host_admin; and it is stale by construction, because
    /// sshd reads `authorized_keys` once, at authentication.
    ///
    /// `unspecified` in two cases, and neither is "no grant". The wire's own
    /// default is zero, so a daemon that did not set the field reads as this —
    /// though there is no such daemon in this tree's history: `granted_scope` has
    /// been `ServerHello` field 5 since the first vertical slice, and every
    /// daemon since has set it from the session's real grant. The case that will
    /// actually happen is the other one: a NEWER runner naming a scope this build
    /// has no word for. Either way a caller must read it as "no answer" and keep
    /// offering what it offers today — see `can` above for the same shape.
    pub fn granted_scope(&self) -> &'static str {
        scope_label(self.client.server_hello().granted_scope)
    }

    /// Whether the runner can do something, by name.
    ///
    /// Empty means a daemon too old to answer the question at all — every one
    /// of those predates capabilities, so it has exactly the feature set that
    /// existed then. Reporting `true` for the two floor capabilities and
    /// `false` for the rest is the honest reading, and it is what lets a new
    /// app talk to an old runner without a special case at every call site.
    pub fn can(&self, capability: &str) -> bool {
        let advertised = self.capabilities();
        if advertised.is_empty() {
            return capability == farcooler_protocol::capability::WORKSPACES
                || capability == farcooler_protocol::capability::TERMINALS;
        }
        advertised.iter().any(|c| c == capability)
    }

    // ---- reads ----

    /// Everything a fleet view needs, in one round trip per resource.
    ///
    /// Shaped identically to the CLI's `workspace list --json`, so a client
    /// decodes one set of types no matter which one it is talking to.
    pub async fn fleet(&mut self) -> Result<serde_json::Value, SessionError> {
        let workspaces = self.workspaces().await?;
        // The whole list, not `self.terminals()`, because the FLEET's trace
        // hangs off the wrapper rather than off any one terminal — and
        // `terminals()` throws the wrapper away. The fleet's row is summed by
        // the daemon on purpose: each terminal's row snaps to the shortest
        // window that holds its own activity, so adding bucket 4 of a
        // five-minute row to bucket 4 of a two-hour row would add two different
        // spans of time. A phone holding only the rendered rows cannot do it.
        let list = self.terminal_list().await?;
        let terminals = &list.items;
        let host = self.host().await?;
        let healthy =
            host.self_health != farcooler_protocol::v1::SelfHealth::Degraded as i32;

        let items: Vec<_> = workspaces
            .iter()
            .map(|w| {
                json!({
                    "id": uuid_of(&w.id).to_string(),
                    "short": short(&w.id),
                    // Which repository this worktree belongs to.
                    //
                    // Carried on the wire message all along and dropped here,
                    // which is what made a phone unable to ask anything
                    // repository-scoped about a workspace it was looking at —
                    // `stack.get` and `pr.refresh` both take a repository id,
                    // and the fleet was the only place a client learned about
                    // workspaces at all.
                    "repository": uuid_of(&w.repository_id).to_string(),
                    "task": w.task_name,
                    "branch": w.branch,
                    "worktree": w.worktree_path,
                    "state": workspace_label(w.state()),
                    // Offering to remove the repository's own checkout would
                    // offer to delete the directory the repository itself
                    // lives in — the client keeps the button off the menu
                    // entirely rather than relying only on the daemon's own
                    // refusal, same reasoning macOS's sidebar already uses.
                    "isMainCheckout": w.is_main_checkout,
                    "terminals": terminals.iter()
                        .filter(|t| t.workspace_id == w.id)
                        .map(|t| json!({
                            "id": uuid_of(&t.id).to_string(),
                            "short": short(&t.id),
                            "title": t.title,
                            "preset": if t.current_command.is_empty() { t.command_preset.clone() } else { t.current_command.clone() },
                            "state": terminal_label(t.state()),
                            "activity": activity_label(t.activity),
                                    "activitySince": activity_since(t),
                            // How it ENDED, which is the difference between a
                            // shell you closed and a build that broke.
                            "exitCode": t.exit_status.as_ref().and_then(|e| e.code),
                            "exitSignal": t.exit_status.as_ref().and_then(|e| e.signal),
                            "turnStartedAt": turn_started_at(t),
                            "blockedQuestion": t.blocked_question.clone(),
                            // The last three things the agent did, already
                            // redacted and cut to a row's width by the daemon.
                            // The phone is the client this matters most to:
                            // it is the one with no screen to scroll back
                            // through and the smallest row to say it in.
                            "feed": t.feed.clone(),
                            // The last of those messages WHOLE and from its
                            // start, which is what the phone's `Notifier` puts
                            // in a banner. It cannot be recovered from the
                            // lines above — they are wrapped rows, so the last
                            // of them is the end of the window rather than the
                            // beginning of a sentence, which is how a lock
                            // screen came to read `batches to avoid N+1
                            // shits.` See `farcooler_core::feed::Feed::said`.
                            "said": t.said.clone(),
                            // The agents it spawned and has not finished with,
                            // named, on the same terms. A phone shows these
                            // under the row exactly as the Mac does, and their
                            // COUNT is already inside `line` for the surfaces
                            // with room for only one string.
                            "subagents": t.subagents.clone(),
                            // The compact ladder, decided on the host. This is
                            // the projection a Live Activity will be built
                            // from — a lock screen, an Island, a watch face —
                            // and each of those has room for a different rung,
                            // so all four travel together rather than the
                            // phone re-deriving the narrow ones from the wide
                            // one and disagreeing with the Mac about the same
                            // pane.
                            "glyph": t.glyph.clone(),
                            "headline": t.headline.clone(),
                            "line": t.line.clone(),
                            "rank": t.rank,
                            // How far the agent is through its OWN task list,
                            // as the two numbers `line` may have composed into
                            // `3/7`. Carried separately because the phone must
                            // not read them back out of that string: `line` is
                            // a rung, so a blocked agent's is the question and
                            // holds no numbers at all, and scraping it would be
                            // a second derivation of a fact the host derives
                            // once. Absent — not zero — when the host has
                            // nothing to say, which is a pane with no list and
                            // every codex and cursor pane; see the fields'
                            // comments in `proto/farcooler.proto`.
                            "planDone": t.plan_done,
                            "planTotal": t.plan_total,
                            // Thirteen buckets of what this pane has been
                            // doing, base64 because JSON has no bytes. Passed
                            // straight across without being unpacked: the
                            // widget holds a whole snapshot per timeline entry
                            // across thirteen entries in a memory-capped
                            // extension, and decoding 66 bytes into three
                            // arrays per agent is the cost the bytes encoding
                            // exists to avoid. `farcooler_core::trace`
                            // documents the layout.
                            //
                            // ABSENT, not an empty string, when the pane has
                            // done nothing the trace can see — a flat zero row
                            // and "no history" are different claims and only
                            // one of them is true of a pane nobody has used.
                            "activityTrace": (!t.activity_trace.is_empty())
                                .then(|| farcooler_core::base64::encode(&t.activity_trace)),
                            // How the last turn ended, which `activity` cannot
                            // say: a turn that died reads as `done` there. The
                            // rungs above already carry it, and this is what
                            // lets a phone draw its own indicator without
                            // having to parse one of them back apart.
                            "turnFailed": t.turn_failed,
                            "epoch": t.epoch,
                            "paneMode": pane_mode_label(t.pane_mode),
                            // Without this the phone's terminal/chat switch
                            // could never appear: `canSwitchPaneMode` reads it,
                            // and a field the runner never sends is a capability
                            // the client always denies.
                            "chatCapable": t.chat_capable,
                            "agentSessionId": t.agent_session_id.clone(),
                            "agentMode": t.agent_mode.clone(),
                            "availableAgentModes": t.available_agent_modes.clone(),
                        }))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();

        Ok(json!({
            "runtime_healthy": healthy,
            "live_panes": host.live_terminal_count,
            // Every pane's trace added together at one width, summed by the
            // daemon. Same encoding and the same absent-not-empty rule as a
            // terminal's own. See the comment on `activityTrace` above.
            "fleetTrace": (!list.fleet_trace.is_empty())
                .then(|| farcooler_core::base64::encode(&list.fleet_trace)),
            "workspaces": items,
        }))
    }

    pub async fn host(&mut self) -> Result<farcooler_protocol::v1::Host, SessionError> {
        match self.value("host.health", None, None).await? {
            result::Value::Host(h) => Ok(h),
            other => Err(wrong("host", &other)),
        }
    }

    pub async fn workspaces(&mut self) -> Result<Vec<Workspace>, SessionError> {
        match self.value("workspace.list", None, None).await? {
            result::Value::WorkspaceList(l) => Ok(l.items),
            other => Err(wrong("workspaces", &other)),
        }
    }

    pub async fn terminals(&mut self) -> Result<Vec<Terminal>, SessionError> {
        Ok(self.terminal_list().await?.items)
    }

    /// The terminal list with its wrapper intact.
    ///
    /// `terminals()` is the right shape for every caller that wants panes, and
    /// throws away the one field that belongs to the LIST rather than to any
    /// pane in it: `fleet_trace`. `fleet` needs both, so it asks for both here
    /// rather than making a second round trip for one field.
    async fn terminal_list(
        &mut self,
    ) -> Result<farcooler_protocol::v1::TerminalList, SessionError> {
        match self.value("terminal.list", None, None).await? {
            result::Value::TerminalList(l) => Ok(l),
            other => Err(wrong("terminals", &other)),
        }
    }

    pub async fn repositories(&mut self) -> Result<Vec<Repository>, SessionError> {
        match self.value("repository.list", None, None).await? {
            result::Value::RepositoryList(l) => Ok(l.items),
            other => Err(wrong("repositories", &other)),
        }
    }

    pub async fn roots(&mut self) -> Result<Vec<RepositoryRoot>, SessionError> {
        match self.value("repository_root.list", None, None).await? {
            result::Value::RepositoryRootList(l) => Ok(l.items),
            other => Err(wrong("roots", &other)),
        }
    }

    /// The colour schemes this runner defines.
    ///
    /// Only the runner's own — the built-ins are compiled into every client, so
    /// sending eleven fixed palettes down an ssh link on every connection
    /// would be a round trip spent to be told what the client already knows.
    pub async fn themes(&mut self) -> Result<Vec<farcooler_protocol::v1::Theme>, SessionError> {
        match self.value("theme.list", None, None).await? {
            result::Value::ThemeList(l) => Ok(l.items),
            other => Err(wrong("themes", &other)),
        }
    }

    // ---- mutations ----

    /// Create a worktree and branch, optionally with a terminal already in it.
    ///
    /// `terminal_preset` empty means no terminal — which is what a caller about
    /// to create its own agent terminal wants, and what keeps this compatible
    /// with every caller that predates the parameter.
    /// `adopt` takes `branch` over instead of creating it — the case where work
    /// ARRIVED on a branch rather than starting on one: pushed from another
    /// machine, handed over, or produced by a cloud agent. Creating is still
    /// the default, because it is still the common case.
    pub async fn create_workspace(
        &mut self,
        repository: Uuid,
        task: &str,
        branch: &str,
        base: &str,
        terminal_preset: &str,
        adopt: bool,
    ) -> Result<Workspace, SessionError> {
        let payload = request::Payload::WorkspaceCreate(farcooler_protocol::v1::WorkspaceCreate {
            task_name: task.into(),
            branch: branch.into(),
            base_revision: base.into(),
            terminal_preset: terminal_preset.into(),
            adopt_existing: adopt,
        });
        match self.value("workspace.create", Some(repository), Some(payload)).await? {
            result::Value::Workspace(w) => Ok(w),
            other => Err(wrong("workspace", &other)),
        }
    }

    pub async fn create_terminal(
        &mut self,
        workspace: Uuid,
        title: &str,
        preset: &str,
        join_active_group: bool,
    ) -> Result<Terminal, SessionError> {
        let payload = request::Payload::TerminalCreate(farcooler_protocol::v1::TerminalCreate {
            title: title.into(),
            command_preset: preset.into(),
            join_active_group,
        });
        match self.value("terminal.create", Some(workspace), Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    pub async fn hide_workspace(&mut self, workspace: Uuid) -> Result<(), SessionError> {
        Ok(crate::actions::hide_workspace(&mut self.client, workspace).await?)
    }

    pub async fn unhide_workspace(&mut self, workspace: Uuid) -> Result<(), SessionError> {
        Ok(crate::actions::unhide_workspace(&mut self.client, workspace).await?)
    }

    /// Remove a worktree, or find out it needs the task name typed first —
    /// see `actions::remove_worktree` for what `confirm` means.
    pub async fn remove_worktree(
        &mut self,
        workspace: Uuid,
        confirm: &str,
    ) -> Result<crate::actions::RemoveWorktreeOutcome, SessionError> {
        Ok(crate::actions::remove_worktree(&mut self.client, workspace, confirm).await?)
    }

    pub async fn add_repository_root(
        &mut self,
        absolute_path: &str,
    ) -> Result<RepositoryRoot, SessionError> {
        Ok(crate::actions::add_repository_root(&mut self.client, absolute_path).await?)
    }

    pub async fn register_repository(
        &mut self,
        relative_path: &str,
    ) -> Result<Repository, SessionError> {
        Ok(crate::actions::register_repository(&mut self.client, relative_path).await?)
    }

    // ---- runner settings ----
    //
    // Every write answers with what the file now says, read back by the daemon
    // rather than echoed from the request, so a value the writer normalized is
    // what the caller ends up holding.

    pub async fn set_branch_prefix(
        &mut self,
        prefix: &str,
    ) -> Result<farcooler_protocol::v1::Host, SessionError> {
        let payload =
            request::Payload::HostSettings(farcooler_protocol::v1::HostSettings {
                branch_prefix: prefix.to_string(),
            });
        match self.value("settings.set_branch_prefix", None, Some(payload)).await? {
            result::Value::Host(h) => Ok(h),
            other => Err(wrong("host", &other)),
        }
    }

    pub async fn upsert_theme(
        &mut self,
        theme: farcooler_protocol::v1::Theme,
    ) -> Result<Vec<farcooler_protocol::v1::Theme>, SessionError> {
        let payload = request::Payload::Theme(theme);
        match self.value("theme.upsert", None, Some(payload)).await? {
            result::Value::ThemeList(l) => Ok(l.items),
            other => Err(wrong("themes", &other)),
        }
    }

    pub async fn delete_theme(
        &mut self,
        name: &str,
    ) -> Result<Vec<farcooler_protocol::v1::Theme>, SessionError> {
        // The name travels in `TypedConfirmation`, which carries exactly one
        // string. Not a confirmation of intent: deleting a theme touches no
        // files and is undone by saving it again.
        let payload = request::Payload::TypedConfirmation(
            farcooler_protocol::v1::TypedConfirmation { typed_confirmation: name.to_string() },
        );
        match self.value("theme.delete", None, Some(payload)).await? {
            result::Value::ThemeList(l) => Ok(l.items),
            other => Err(wrong("themes", &other)),
        }
    }

    pub async fn adapters(
        &mut self,
    ) -> Result<Vec<farcooler_protocol::v1::Adapter>, SessionError> {
        match self.value("adapter.list", None, None).await? {
            result::Value::AdapterList(l) => Ok(l.items),
            other => Err(wrong("adapters", &other)),
        }
    }

    pub async fn upsert_adapter(
        &mut self,
        adapter: farcooler_protocol::v1::Adapter,
    ) -> Result<Vec<farcooler_protocol::v1::Adapter>, SessionError> {
        let payload = request::Payload::Adapter(adapter);
        match self.value("adapter.upsert", None, Some(payload)).await? {
            result::Value::AdapterList(l) => Ok(l.items),
            other => Err(wrong("adapters", &other)),
        }
    }

    pub async fn delete_adapter(
        &mut self,
        preset: &str,
    ) -> Result<Vec<farcooler_protocol::v1::Adapter>, SessionError> {
        let payload = request::Payload::TypedConfirmation(
            farcooler_protocol::v1::TypedConfirmation { typed_confirmation: preset.to_string() },
        );
        match self.value("adapter.delete", None, Some(payload)).await? {
            result::Value::AdapterList(l) => Ok(l.items),
            other => Err(wrong("adapters", &other)),
        }
    }

    /// Prove an adapter works, without saving it first.
    ///
    /// Takes the adapter rather than a name on purpose: the point is to answer
    /// "will this work" about a form the user has not committed yet.
    pub async fn test_adapter(
        &mut self,
        adapter: farcooler_protocol::v1::Adapter,
    ) -> Result<farcooler_protocol::v1::AdapterTestResult, SessionError> {
        let payload = request::Payload::Adapter(adapter);
        match self.value("adapter.test", None, Some(payload)).await? {
            result::Value::AdapterTestResult(r) => Ok(r),
            other => Err(wrong("adapter test result", &other)),
        }
    }

    pub async fn stop_terminal(&mut self, terminal: Uuid) -> Result<(), SessionError> {
        self.value("terminal.stop", Some(terminal), None).await.map(|_| ())
    }

    pub async fn restart_terminal(&mut self, terminal: Uuid) -> Result<(), SessionError> {
        self.value("terminal.restart", Some(terminal), None).await.map(|_| ())
    }

    pub async fn dismiss_lost(&mut self, terminal: Uuid) -> Result<(), SessionError> {
        self.value("terminal.dismiss_lost", Some(terminal), None).await.map(|_| ())
    }

    /// A user has looked at this terminal, which is what ends `Done`.
    ///
    /// `Done` is idle-and-UNSEEN, so a client that can show a terminal has to be
    /// able to say it showed one. Without this the phone could open a finished
    /// agent, read it, and leave it still announcing itself — on the phone, on
    /// the Mac, and in every push notification after.
    pub async fn mark_seen(&mut self, terminal: Uuid) -> Result<(), SessionError> {
        self.value("terminal.seen", Some(terminal), None).await.map(|_| ())
    }

    /// The terminals this client currently has in front of a person, and the
    /// whole set of them.
    ///
    /// What stops a push arriving about a pane you are sitting there reading.
    /// `mark_seen` above answers the same question one beat too late — a client
    /// reports it on the poll AFTER the agent finished, and the notification
    /// crossed the relay while that poll was in flight — so the runner has to be
    /// told a pane is being watched BEFORE the turn ends, not after. See
    /// `attention` in `crates/daemon/src/watch.rs`.
    ///
    /// Sent repeatedly, on whatever clock the caller already polls on, and the
    /// runner only believes it for a few seconds: a client that crashes, is
    /// suspended, or loses its link stops suppressing on its own rather than
    /// leaving a terminal permanently silent. An empty slice is a real call and
    /// the one that matters most — "I am looking at nothing" — which is what an
    /// app sends as it goes to the background, releasing every terminal it named
    /// in one round trip instead of waiting the claim out.
    ///
    /// Several ids because one window can show a whole tiled layout, and a pane
    /// tiled beside the focused one is as much on screen as the focused one.
    pub async fn report_watching(&mut self, terminals: &[Uuid]) -> Result<(), SessionError> {
        let payload = request::Payload::TerminalsWatched(
            farcooler_protocol::v1::TerminalsWatched {
                terminal_ids: terminals
                    .iter()
                    .map(|id| bytes::Bytes::copy_from_slice(id.as_bytes()))
                    .collect(),
            },
        );
        self.value("terminal.watching", None, Some(payload)).await.map(|_| ())
    }

    /// A terminal's visible screen, with escapes intact.
    ///
    /// `known_revision` is the last one received; the runner answers `unchanged`
    /// rather than resending a screen already held. Pass 0 to always get one.
    ///
    /// `history_lines` is how much of the pane's scrollback to send above that
    /// screen, and 0 — an ordinary poll — is none. It costs a second capture on
    /// the runner and a second one on the wire, so a client asks once, when it
    /// opens a pane, and polls with 0 from then on. The answer arrives in
    /// `history` even when the screen comes back `unchanged`: a client asking
    /// for scrollback is asking because it has none.
    pub async fn screen(
        &mut self,
        terminal: Uuid,
        known_revision: u64,
        history_lines: u32,
    ) -> Result<farcooler_protocol::v1::TerminalScreen, SessionError> {
        let payload = request::Payload::TerminalScreenRequest(
            farcooler_protocol::v1::TerminalScreenRequest { known_revision, history_lines },
        );
        match self.value("terminal.screen", Some(terminal), Some(payload)).await? {
            result::Value::TerminalScreen(s) => Ok(s),
            other => Err(wrong("terminal_screen", &other)),
        }
    }

    /// Send exact bytes to a terminal.
    pub async fn write(&mut self, terminal: Uuid, bytes: Vec<u8>) -> Result<(), SessionError> {
        let payload = request::Payload::TerminalWrite(farcooler_protocol::v1::TerminalWrite {
            payload: bytes.into(),
        });
        self.value("terminal.write", Some(terminal), Some(payload)).await?;
        Ok(())
    }

    /// Send a file into a terminal, and return the path it landed at.
    ///
    /// The whole transfer lives here rather than in each app for the reason
    /// this crate exists: the chunking, the retry-free failure behavior and the
    /// size ceiling must not differ between a Mac, a phone and the CLI, and
    /// three implementations of a loop is three chances for them to.
    ///
    /// `progress` is called after each chunk with (sent, total). It is how a
    /// client draws a ring; nothing here depends on what it does.
    pub async fn paste_file(
        &mut self,
        terminal: Uuid,
        name: &str,
        mime: &str,
        file: &[u8],
        progress: impl FnMut(u64, u64),
    ) -> Result<String, SessionError> {
        // The size check is here rather than in `actions` so a phone can say
        // something a person can act on. `actions` refuses too, because the CLI
        // reaches it without passing through this.
        let total = file.len() as u64;
        if total == 0 {
            return Err(SessionError::Protocol("that file is empty".into()));
        }
        if total > farcooler_protocol::MAX_PASTE_FILE_BYTES {
            return Err(SessionError::Protocol(format!(
                "that file is {} MB, and the limit is {} MB",
                total / (1024 * 1024),
                farcooler_protocol::MAX_PASTE_FILE_BYTES / (1024 * 1024)
            )));
        }
        Ok(crate::actions::paste_file(&mut self.client, terminal, name, mime, file, progress).await?)
    }

    pub async fn resize_terminal(
        &mut self,
        terminal: Uuid,
        columns: u32,
        rows: u32,
    ) -> Result<(), SessionError> {
        let payload = request::Payload::TerminalResize(farcooler_protocol::v1::TerminalResize {
            columns,
            rows,
            view_activity_id: 0,
        });
        self.value("terminal.resize", Some(terminal), Some(payload)).await.map(|_| ())
    }

    // ---- agent channel ----
    //
    // Every payload here names its own `terminal_id` rather than the
    // envelope's `target_resource_id`, matching the daemon side: `agent_subscribe`
    // in particular legitimately targets a terminal that holds no session yet,
    // which is not the versioned-resource shape `target_resource_id` is for.

    /// Switch a pane between a terminal and its agent, or back.
    ///
    /// `force` is the client's word for "yes, discard the turn in flight" —
    /// the daemon refuses a bare switch to TERMINAL mid-turn, because
    /// `claude --resume` cannot reattach to work that was never finished.
    pub async fn set_pane_mode(
        &mut self,
        terminal: Uuid,
        mode: PaneMode,
        force: bool,
    ) -> Result<Terminal, SessionError> {
        let payload = request::Payload::SetPaneMode(farcooler_protocol::v1::SetPaneMode {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            pane_mode: mode as i32,
            force,
        });
        match self.value("terminal.set_pane_mode", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// New agent events for one terminal, from a cursor.
    ///
    /// Never an error for a terminal with no agent session: a client attaches
    /// to a PANE, not a session, so a chat view has to be able to open before
    /// the first turn — the daemon answers an empty batch rather than
    /// refusing, and this call must not turn that into a special case here.
    pub async fn agent_subscribe(
        &mut self,
        terminal: Uuid,
        from_seq: u64,
        epoch: u64,
    ) -> Result<AgentEventBatch, SessionError> {
        let payload = request::Payload::AgentSubscribe(farcooler_protocol::v1::AgentSubscribe {
            epoch,
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            from_seq,
        });
        match self.value("terminal.agent_subscribe", None, Some(payload)).await? {
            result::Value::AgentEventBatch(b) => Ok(b),
            other => Err(wrong("agent_event_batch", &other)),
        }
    }

    /// Send one plain-text prompt block.
    ///
    /// Images ride WITH the prompt, as content blocks.
    ///
    /// The composer used to attach a picture by writing `@/Users/you/x.png`
    /// into the message. That is a path on the device that picked the file,
    /// and the agent runs on the RUNNER — so it worked when those were the same
    /// machine and silently referred to nothing when they were not, which is
    /// every phone and every remote runner.
    pub async fn agent_prompt(
        &mut self,
        terminal: Uuid,
        text: &str,
        images: &[(String, Vec<u8>)],
    ) -> Result<Terminal, SessionError> {
        use farcooler_protocol::v1::agent_prompt_block::Content;

        let mut blocks: Vec<farcooler_protocol::v1::AgentPromptBlock> = images
            .iter()
            .map(|(mime, data)| farcooler_protocol::v1::AgentPromptBlock {
                content: Some(Content::Image(farcooler_protocol::v1::ImageBlock {
                    mime_type: mime.clone(),
                    data: bytes::Bytes::copy_from_slice(data),
                })),
            })
            .collect();
        blocks.push(farcooler_protocol::v1::AgentPromptBlock {
            content: Some(Content::Text(text.to_string())),
        });

        let payload = request::Payload::AgentPrompt(farcooler_protocol::v1::AgentPrompt {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            blocks,
        });
        match self.value("terminal.agent_prompt", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// Answer a pending permission request.
    pub async fn agent_answer(
        &mut self,
        terminal: Uuid,
        request_id: &str,
        option_id: &str,
    ) -> Result<Terminal, SessionError> {
        let payload = request::Payload::AgentAnswer(farcooler_protocol::v1::AgentAnswer {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            request_id: request_id.to_string(),
            option_id: option_id.to_string(),
        });
        match self.value("terminal.agent_answer", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    pub async fn agent_set_mode(
        &mut self,
        terminal: Uuid,
        agent_mode: &str,
    ) -> Result<Terminal, SessionError> {
        let payload = request::Payload::AgentSetMode(farcooler_protocol::v1::AgentSetMode {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            agent_mode: agent_mode.to_string(),
        });
        match self.value("terminal.agent_set_mode", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// Rewrite a prompt that is still waiting for the current turn to end.
    pub async fn agent_edit_queued(
        &mut self,
        terminal: Uuid,
        queued_id: &str,
        text: &str,
    ) -> Result<Terminal, SessionError> {
        let payload = request::Payload::AgentEditQueued(farcooler_protocol::v1::AgentEditQueued {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            queued_id: queued_id.to_string(),
            text: text.to_string(),
        });
        match self.value("terminal.agent_edit_queued", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// Withdraw a prompt that is still waiting for the current turn to end.
    pub async fn agent_cancel_queued(
        &mut self,
        terminal: Uuid,
        queued_id: &str,
    ) -> Result<Terminal, SessionError> {
        let payload =
            request::Payload::AgentCancelQueued(farcooler_protocol::v1::AgentCancelQueued {
                terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
                queued_id: queued_id.to_string(),
            });
        match self.value("terminal.agent_cancel_queued", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// Send a queued prompt into the turn already running.
    pub async fn agent_steer_queued(
        &mut self,
        terminal: Uuid,
        queued_id: &str,
    ) -> Result<Terminal, SessionError> {
        let payload =
            request::Payload::AgentSteerQueued(farcooler_protocol::v1::AgentSteerQueued {
                terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
                queued_id: queued_id.to_string(),
            });
        match self.value("terminal.agent_steer_queued", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    pub async fn agent_cancel(&mut self, terminal: Uuid) -> Result<Terminal, SessionError> {
        let payload = request::Payload::AgentCancel(farcooler_protocol::v1::AgentCancel {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
        });
        match self.value("terminal.agent_cancel", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// Worktree-relative paths matching `query`, for an `@`-mention.
    ///
    /// Relative, never a runner path: the same redaction the rest of this
    /// protocol applies to everything below `host_admin`.
    pub async fn search_worktree_files(
        &mut self,
        workspace: Uuid,
        query: &str,
        limit: u32,
    ) -> Result<Vec<String>, SessionError> {
        let payload =
            request::Payload::WorktreeFileSearch(farcooler_protocol::v1::WorktreeFileSearch {
                workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
                query: query.to_string(),
                limit,
            });
        match self.value("worktree.file_search", None, Some(payload)).await? {
            result::Value::WorktreeFileList(l) => Ok(l.paths),
            other => Err(wrong("worktree_file_list", &other)),
        }
    }

    // ---- changes, stacks and branches ----
    //
    // JSON out, like `fleet` and for the same reason stated at the top of this
    // file: the phone clients reach the daemon only through this crate's FFI,
    // and the shapes below are deliberately the ones `farcooler changes … --json`
    // already prints, so a model decoded on the Mac decodes unchanged on a
    // phone. Anything else would be two descriptions of one change set, free to
    // disagree.

    /// What a worktree changed, against its base.
    pub async fn change_set(
        &mut self,
        workspace: Uuid,
        fresh: bool,
    ) -> Result<serde_json::Value, SessionError> {
        let payload =
            request::Payload::ChangeSetRequest(farcooler_protocol::v1::ChangeSetRequest {
                workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
                selector: None,
                fresh,
            });
        match self.value("changes.change_set", None, Some(payload)).await? {
            result::Value::ChangeSet(cs) => Ok(change_set_json(&cs)),
            other => Err(wrong("change_set", &other)),
        }
    }

    /// One file's patch, in the comparison `scope` names.
    ///
    /// `context` of zero means git's own three, which is what a first read
    /// wants; a caller opening one of the gaps between hunks asks for enough to
    /// swallow it. See the Mac's `ChangesStore.open(gap:of:in:)` for why that is
    /// per-gap rather than per-file.
    pub async fn file_diff(
        &mut self,
        workspace: Uuid,
        path: &str,
        scope: &str,
        context: u32,
    ) -> Result<serde_json::Value, SessionError> {
        use farcooler_protocol::v1::diff_selector::Kind;
        let kind = match scope {
            "local" => Kind::Local(farcooler_protocol::v1::Empty {}),
            "staged" => Kind::Staged(farcooler_protocol::v1::Empty {}),
            "unstaged" => Kind::Unstaged(farcooler_protocol::v1::Empty {}),
            // Anything else is a commit sha, and the empty string is the branch
            // range — the default every caller that names no scope gets.
            "" | "branch" => Kind::Range(farcooler_protocol::v1::Empty {}),
            sha => Kind::Commit(sha.to_string()),
        };
        let payload = request::Payload::FileDiffRequest(farcooler_protocol::v1::FileDiffRequest {
            workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
            selector: Some(farcooler_protocol::v1::DiffSelector { kind: Some(kind) }),
            path: path.to_string(),
            from_hunk: 0,
            context,
        });
        match self.value("changes.file_diff", None, Some(payload)).await? {
            result::Value::FileDiff(d) => Ok(file_diff_json(&d)),
            other => Err(wrong("file_diff", &other)),
        }
    }

    /// The files one commit touched.
    pub async fn commit_files(
        &mut self,
        workspace: Uuid,
        sha: &str,
    ) -> Result<serde_json::Value, SessionError> {
        let payload =
            request::Payload::CommitFilesRequest(farcooler_protocol::v1::CommitFilesRequest {
                workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
                sha: sha.to_string(),
            });
        match self.value("changes.commit_files", None, Some(payload)).await? {
            result::Value::FileChangeList(l) => Ok(json!({
                "files": l.items.iter().map(file_change_json).collect::<Vec<_>>()
            })),
            other => Err(wrong("file_change_list", &other)),
        }
    }

    /// Every worktree with something to say, and whether anybody has looked.
    ///
    /// Not "every worktree that changed since it was last looked at", which is
    /// what this said and is narrower than what `review_ops::inbox` returns: a
    /// worktree with a nonzero diff is listed even after you mark it read, and
    /// `changed_since_reviewed` is how a row says which it is. A front door
    /// built on the old sentence would have dropped every branch you had
    /// already seen out of a list whose whole job is "is this worth opening".
    ///
    /// Shared with `farcooler changes inbox --json` since the two shapes became
    /// one — see `changes_json::inbox_json` for what they used to be and why
    /// this half won.
    pub async fn changes_inbox(&mut self) -> Result<serde_json::Value, SessionError> {
        let payload =
            request::Payload::ChangesInbox(farcooler_protocol::v1::ChangesInboxRequest {});
        match self.value("changes.inbox", None, Some(payload)).await? {
            result::Value::ChangesInbox(inbox) => Ok(inbox_json(&inbox)),
            other => Err(wrong("changes_inbox", &other)),
        }
    }

    /// Mark a worktree as read, which is what clears its inbox badge.
    pub async fn changes_mark_read(&mut self, workspace: Uuid) -> Result<(), SessionError> {
        let payload =
            request::Payload::ChangesMarkRead(farcooler_protocol::v1::ChangesMarkRead {
                workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
                branch: String::new(),
            });
        self.value("changes.mark_read", None, Some(payload)).await?;
        Ok(())
    }

    /// Pin what this worktree is compared against.
    ///
    /// The affordance that exists because a GUESSED base produces a wrong diff
    /// that looks exactly like a right one — see `BaseSource` in the protocol.
    pub async fn changes_set_base(
        &mut self,
        workspace: Uuid,
        base_ref: &str,
    ) -> Result<serde_json::Value, SessionError> {
        let payload = request::Payload::ChangesSetBase(farcooler_protocol::v1::ChangesSetBase {
            workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
            base_ref: base_ref.to_string(),
        });
        match self.value("changes.set_base", None, Some(payload)).await? {
            result::Value::ChangeSet(cs) => Ok(change_set_json(&cs)),
            other => Err(wrong("change_set", &other)),
        }
    }

    /// The branches in a repository, for resuming onto one that already exists.
    pub async fn branches(&mut self, repository: Uuid) -> Result<serde_json::Value, SessionError> {
        match self.value("branch.list", Some(repository), None).await? {
            result::Value::BranchList(l) => Ok(json!({
                "branches": l.items.iter().map(|b| json!({
                    "name": b.name,
                    "local": b.local,
                    "remote": b.remote,
                    // git refuses a second checkout of the same branch, so a
                    // client has to be able to show this before someone picks it.
                    "checkedOut": b.checked_out,
                    "subject": b.subject,
                    "updatedAt": b.updated_at.as_ref().map(|t| t.seconds * 1000),
                })).collect::<Vec<_>>()
            })),
            other => Err(wrong("branch_list", &other)),
        }
    }

    /// A branch's parent chain and the PR state along it.
    pub async fn stack(
        &mut self,
        repository: Uuid,
        branch: &str,
    ) -> Result<serde_json::Value, SessionError> {
        let payload = request::Payload::StackGet(farcooler_protocol::v1::StackGet {
            repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
            branch: branch.to_string(),
        });
        match self.value("stack.get", Some(repository), Some(payload)).await? {
            result::Value::StackLinkList(l) => Ok(stack_json(&l)),
            other => Err(wrong("stack_link_list", &other)),
        }
    }

    /// Ask GitHub again, rather than answering from what was last read.
    pub async fn pr_refresh(&mut self, repository: Uuid) -> Result<serde_json::Value, SessionError> {
        let payload = request::Payload::PrRefresh(farcooler_protocol::v1::PrRefresh {
            repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
        });
        match self.value("pr.refresh", Some(repository), Some(payload)).await? {
            result::Value::StackLinkList(l) => Ok(stack_json(&l)),
            other => Err(wrong("stack_link_list", &other)),
        }
    }

    /// What the daemon is, and what it can do.
    ///
    /// Named apart from the `daemon_version` accessor above, which answers from
    /// the hello this session already exchanged and costs nothing. This one is a
    /// round trip and is worth it only for `capabilities`, which the hello does
    /// not carry.
    pub async fn daemon_capabilities(&mut self) -> Result<serde_json::Value, SessionError> {
        match self.value("daemon.version", None, None).await? {
            result::Value::DaemonVersion(v) => Ok(json!({
                "daemonVersion": v.daemon_version,
                "protocolVersions": v.protocol_versions,
                "capabilities": v.capabilities,
            })),
            other => Err(wrong("daemon_version", &other)),
        }
    }

    // ---- device enrollment ----
    //
    // Which devices may log in to this runner. The daemon owns every rule about
    // what may be written into `authorized_keys` and this owns none of them; the
    // one judgement here is the scope WORD, because the wire carries an enum and
    // neither Swift nor Kotlin has it.
    //
    // Answers are JSON rather than the wire messages, the same way `stack` and
    // `changes_inbox` are: these come straight back out through the FFI, and a
    // shape assembled in two places is two shapes the day one of them changes.
    //
    // `changes_inbox` was the proof of that rather than an example of it: it
    // WAS assembled in two places, here and in `farcooler changes inbox
    // --json`, and by the time anybody compared them they were two shapes — a
    // bare array with a short id against an object with a UUID. It builds
    // through `changes_json::inbox_json` now, which is where the rule this
    // comment states actually lives.

    /// Every line in this runner's fence, ours and otherwise.
    ///
    /// Foreign lines are included and marked, because a person looking at who
    /// may log in to their runner needs to see the key somebody added by hand as
    /// much as the ones Far Cooler wrote.
    pub async fn enrolled_clients(&mut self) -> Result<serde_json::Value, SessionError> {
        match self.value("client.list", None, None).await? {
            result::Value::ClientList(l) => Ok(enrolled_json(&l.items)),
            other => Err(wrong("client_list", &other)),
        }
    }

    /// Add a device's key to this runner's fence.
    ///
    /// `scope` is a word — `read`, `control` or `host_admin` — and an unknown one
    /// is refused here rather than sent. Refused rather than defaulted for the
    /// reason the daemon refuses an unspecified scope: a key with no scope at all
    /// already means host_admin to sshd, so rounding a misspelling up would turn
    /// a typo into the whole runner.
    ///
    /// `shell_access` chooses the SHAPE of the line: false is the restricted Key
    /// A line with its forced command, true is the plain Key B line that Zed, git
    /// and Terminal need a shell behind. A Mac calls this twice, once each way,
    /// with the same `client_id` — which is what lets one `revoke_client` remove
    /// both. The pairing rule (`shell_access` demands `host_admin`) is NOT
    /// re-checked here: the daemon owns every rule about what may be written into
    /// `authorized_keys`, and a copy of that rule in three client languages is
    /// three places for it to drift from the file's authority.
    pub async fn enroll_client(
        &mut self,
        public_key: &str,
        label: &str,
        client_id: &str,
        scope: &str,
        shell_access: bool,
    ) -> Result<serde_json::Value, SessionError> {
        let Some(scope) = scope_from_word(scope) else {
            return Err(SessionError::Protocol(
                "a device is enrolled at read, control or host_admin".into(),
            ));
        };
        let payload =
            request::Payload::ClientEnroll(farcooler_protocol::v1::ClientEnroll {
                public_key: public_key.to_string(),
                label: label.to_string(),
                client_id: client_id.to_string(),
                scope: scope as i32,
                // False is the restricted line and the proto's default, so a
                // caller that omits the argument at the FFI asks for exactly what
                // this call always asked for. True is the plain line, and the
                // daemon refuses it unless the scope beside it is host_admin — a
                // request asking for a shell while saying `read` does not agree
                // with itself, and the daemon is the one that says so.
                shell_access,
            });
        match self.value("client.enroll", None, Some(payload)).await? {
            result::Value::ClientEnroll(r) => Ok(json!({
                "client": r.client.as_ref().map(enrolled_client_json),
                // Its own field rather than an error, because it is the ordinary
                // outcome of enrolling a Mac on itself and of a ceremony offered
                // a runner the device can already reach. The `client` beside it
                // is then the grant it HAS, not the one that was asked for.
                "alreadyEnrolled": r.already_enrolled,
            })),
            other => Err(wrong("client_enroll", &other)),
        }
    }

    /// Remove a device's line and close the sessions it was holding.
    ///
    /// Answers with what is left, which the daemon reads back out of the file
    /// after writing it: what `authorized_keys` now says is the only claim worth
    /// making about who may log in.
    pub async fn revoke_client(
        &mut self,
        client_id: &str,
    ) -> Result<serde_json::Value, SessionError> {
        let payload =
            request::Payload::ClientRevoke(farcooler_protocol::v1::ClientRevoke {
                client_id: client_id.to_string(),
            });
        match self.value("client.revoke", None, Some(payload)).await? {
            result::Value::ClientList(l) => Ok(enrolled_json(&l.items)),
            other => Err(wrong("client_list", &other)),
        }
    }

    /// Remove a repository root. The paired `add` already exists above.
    ///
    /// `confirm` is the root directory's own name, typed by the person doing it.
    /// Unlike `remove_worktree`, there is no phase that asks first and finds
    /// out: the daemon demands the name every time, so a caller with nothing
    /// typed has nothing to send and `NameDidNotMatch` is the only thing an
    /// empty string can come back as.
    ///
    /// It used to take no `confirm` at all and send `None`, so every request
    /// built here was refused before the runner looked at the root — see
    /// `actions::remove_repository_root` for what that cost.
    pub async fn remove_repository_root(
        &mut self,
        root: Uuid,
        confirm: &str,
    ) -> Result<crate::actions::RemoveRootOutcome, SessionError> {
        Ok(crate::actions::remove_repository_root(&mut self.client, root, confirm).await?)
    }

    async fn value(
        &mut self,
        method: &str,
        target: Option<Uuid>,
        payload: Option<request::Payload>,
    ) -> Result<result::Value, SessionError> {
        let mut request = farcooler_transport::request(method);
        if let Some(id) = target {
            request.target_resource_id = Some(bytes::Bytes::copy_from_slice(id.as_bytes()));
        }
        if let Some(p) = payload {
            request.payload = Some(p);
        }
        let outcome = self.client.call(request).await?;
        outcome.value.ok_or_else(|| SessionError::Protocol(format!("{method} returned nothing")))
    }
}

fn wrong(expected: &'static str, got: &result::Value) -> SessionError {
    SessionError::WrongResult { expected, got: variant_name(got) }
}

fn variant_name(value: &result::Value) -> &'static str {
    match value {
        result::Value::Host(_) => "host",
        result::Value::RepositoryRoot(_) => "repository_root",
        result::Value::RepositoryRootList(_) => "repository_root_list",
        result::Value::Repository(_) => "repository",
        result::Value::RepositoryList(_) => "repository_list",
        result::Value::Workspace(_) => "workspace",
        result::Value::WorkspaceList(_) => "workspace_list",
        result::Value::Terminal(_) => "terminal",
        result::Value::TerminalList(_) => "terminal_list",
        result::Value::Operation(_) => "operation",
        result::Value::DaemonVersion(_) => "daemon_version",
        result::Value::TerminalAttach(_) => "terminal_attach",
        result::Value::BranchList(_) => "branch_list",
        result::Value::PaneGroupList(_) => "pane_group_list",
        result::Value::WorktreeList(_) => "worktree_list",
        result::Value::TerminalScreen(_) => "terminal_screen",
        result::Value::AgentEventBatch(_) => "agent_event_batch",
        result::Value::WorktreeFileList(_) => "worktree_file_list",
        result::Value::ThemeList(_) => "theme_list",
        result::Value::AdapterList(_) => "adapter_list",
        result::Value::AdapterTestResult(_) => "adapter_test_result",
        result::Value::Empty(_) => "empty",
        result::Value::TerminalFilePut(_) => "terminal_file_put",
        result::Value::ChangeSet(_) => "change_set",
        result::Value::FileChangeList(_) => "file_change_list",
        result::Value::FileDiff(_) => "file_diff",
        result::Value::StackLinkList(_) => "stack_link_list",
        result::Value::ChangesInbox(_) => "changes_inbox",
        result::Value::ClientList(_) => "client_list",
        result::Value::ClientEnroll(_) => "client_enroll",
    }
}


pub fn uuid_of(bytes: &[u8]) -> Uuid {
    Uuid::from_slice(bytes).unwrap_or(Uuid::nil())
}

/// UUIDv7 puts a timestamp in its LEADING bytes, so anything created in the
/// same millisecond shares a prefix. Short ids use the trailing random bytes.
pub fn short(bytes: &[u8]) -> String {
    let s = uuid_of(bytes).simple().to_string();
    s[s.len() - 8..].to_string()
}

/// The agent's activity, as the daemon derived it.
///
/// Distinct from `state`, which is about the process. A Claude Code sitting at
/// a permission prompt and one halfway through a file edit are both `running`;
/// the difference between them is the reason to look at a fleet at all.
fn activity_label(a: i32) -> &'static str {
    use farcooler_protocol::v1::AgentActivity;
    match AgentActivity::try_from(a).unwrap_or(AgentActivity::Unspecified) {
        AgentActivity::None => "none",
        AgentActivity::Idle => "idle",
        AgentActivity::Working => "working",
        AgentActivity::Blocked => "blocked",
        AgentActivity::Done => "done",
        AgentActivity::Unknown => "unknown",
        AgentActivity::Unspecified => "none",
    }
}

/// When the activity last changed, as Unix milliseconds.
///
/// A client shows "working for 4m" from this rather than timing it locally,
/// which would restart at every reconnect and lie after a laptop sleeps.
fn activity_since(t: &farcooler_protocol::v1::Terminal) -> Option<i64> {
    t.activity_changed_at.as_ref().map(|ts| ts.seconds * 1000 + (ts.nanos as i64) / 1_000_000)
}

/// When the current turn started, as Unix milliseconds.
///
/// Distinct from `activity_since`: this is held across a permission prompt and
/// cleared when the turn ends, so it answers "how long has this been running"
/// rather than "how long has it been in this particular state".
fn turn_started_at(t: &farcooler_protocol::v1::Terminal) -> Option<i64> {
    t.turn_started_at.as_ref().map(|ts| ts.seconds * 1000 + (ts.nanos as i64) / 1_000_000)
}

/// The pane mode, as a word rather than a number.
///
/// Same reason `activity_label` exists: a client that switched on an integer
/// would hold a second copy of the enum and drift from it silently.
fn pane_mode_label(mode: i32) -> &'static str {
    match farcooler_protocol::v1::PaneMode::try_from(mode) {
        Ok(farcooler_protocol::v1::PaneMode::Agent) => "agent",
        Ok(farcooler_protocol::v1::PaneMode::Changes) => "changes",
        // Unspecified from an older daemon is terminal: the mode that needs no
        // adapter and always works.
        _ => "terminal",
    }
}

fn workspace_label(s: WorkspaceState) -> &'static str {
    match s {
        WorkspaceState::Unspecified => "?",
        WorkspaceState::Creating => "creating",
        WorkspaceState::Ready => "ready",
        WorkspaceState::Active => "active",
        WorkspaceState::Error => "ERROR",
        WorkspaceState::Hidden => "hidden",
        WorkspaceState::WorktreeMissing => "worktree_missing",
    }
}

/// Enrolled devices, in the shape three apps decode.
fn enrolled_json(items: &[farcooler_protocol::v1::EnrolledClient]) -> serde_json::Value {
    json!({ "clients": items.iter().map(enrolled_client_json).collect::<Vec<_>>() })
}

/// One device that may log in to a runner.
///
/// Every field of the wire message is carried, and `scope` is a word rather than
/// the enum's number: neither Swift nor Kotlin has the generated enum, so a
/// number here would be two mapping tables in two languages and the second one
/// would be wrong. Same reasoning as `origin` on an adapter.
fn enrolled_client_json(
    client: &farcooler_protocol::v1::EnrolledClient,
) -> serde_json::Value {
    json!({
        "clientId": client.client_id,
        "fingerprint": client.fingerprint,
        "label": client.label,
        "scope": scope_label(client.scope),
        // Which local account's `authorized_keys` this line was read from.
        // Nothing in the line names it — the file's location does — so this is
        // the daemon's answer and cannot be derived here.
        "account": client.account,
        // 0 for unknown, which is the ordinary answer: `authorized_keys` records
        // no time, so only the reply to an enrollment that just happened has one.
        "enrolledAt": client.enrolled_at,
        // Far Cooler did not write this line. It is reported so a person can see
        // it is there, and it is never touched.
        "foreign": client.foreign,
        // This is the device's PLAIN line — Key B, the one Zed, git and Terminal
        // use — and it is the ONLY thing that tells the two rows of a Mac apart.
        // Both lines carry the same client id and both are ours; without this a
        // Key B row and a Key A row are the same JSON object, Settings › Devices
        // draws one of them twice, and the removal copy's promise ("removing this
        // takes that Mac's ssh, git and Zed access away, not only its Far Cooler
        // access") is attached to whichever row happened to sort first.
        //
        // Never true for a foreign line: a plain line of OURS is one this program
        // rendered and manages, and a stranger's key is left as it was found.
        "shellAccess": client.shell_access,
    })
}

/// A scope as the word `authorized_keys` spells it.
///
/// `unspecified` is reported as itself rather than rounded to anything, and that
/// is the honest answer twice over: it is what a foreign line grants, and what a
/// line of ours whose scope word this build does not have grants — which is
/// nothing, because the daemon serving it refuses the word too. The daemon's own
/// writer rounds an absent scope UP to host_admin when it renders a line, and
/// that asymmetry is deliberate: writing without a restriction is what an
/// unscoped line already means to sshd, whereas READING one as host_admin would
/// tell a person a device has access it does not have.
fn scope_label(scope: i32) -> &'static str {
    use farcooler_protocol::v1::Scope;
    match Scope::try_from(scope) {
        Ok(Scope::Read) => "read",
        Ok(Scope::Control) => "control",
        Ok(Scope::HostAdmin) => "host_admin",
        _ => "unspecified",
    }
}

/// The same words going the other way, for an enrollment request.
///
/// `None` is a word this build does not have, and the caller refuses rather than
/// resolving it — the same rule `fence::scope_from_word` applies on the daemon
/// side, and for the same reason: rounding a typo up is privilege escalation by
/// misspelling. `unspecified` is deliberately not a word a caller may pass; it
/// is a state a line can be in, not a grant anybody can ask for.
fn scope_from_word(word: &str) -> Option<farcooler_protocol::v1::Scope> {
    use farcooler_protocol::v1::Scope;
    match word {
        "read" => Some(Scope::Read),
        "control" => Some(Scope::Control),
        "host_admin" => Some(Scope::HostAdmin),
        _ => None,
    }
}

fn terminal_label(s: TerminalState) -> &'static str {
    match s {
        TerminalState::Unspecified => "?",
        TerminalState::Starting => "starting",
        TerminalState::Running => "running",
        TerminalState::Exited => "exited",
        TerminalState::Error => "error",
        TerminalState::Lost => "LOST",
        // The same word the CLI and both apps use. A phone showing a state the
        // Mac calls something else is two products.
        TerminalState::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ids_use_the_random_tail_not_the_timestamp_head() {
        // Two ids minted in the same millisecond share a leading prefix, so a
        // head-based short id would collide constantly.
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        assert_ne!(short(a.as_bytes()), short(b.as_bytes()));
        assert_eq!(short(a.as_bytes()).len(), 8);
    }

    #[test]
    fn a_wrong_result_variant_names_both_sides() {
        let error = wrong("terminal", &result::Value::Workspace(Workspace::default()));
        let message = error.to_string();
        assert!(message.contains("terminal") && message.contains("workspace"));
    }

    /// A scope crosses as a word, and the two directions agree.
    ///
    /// They have to: the word this sends is the word the daemon writes into the
    /// forced command, and the word it reads back out is the one this labels. A
    /// pair that disagreed would enroll a device at one scope and then report
    /// another.
    #[test]
    fn a_scope_is_the_same_word_in_both_directions() {
        for word in ["read", "control", "host_admin"] {
            let scope = scope_from_word(word).expect("a scope this build has");
            assert_eq!(scope_label(scope as i32), word);
        }
    }

    /// A word nobody has is refused, never rounded up.
    ///
    /// `unspecified` included: it is a state a line can be found in, not a grant
    /// a caller may ask for, and the daemon refuses it too — an unscoped line
    /// already means host_admin to sshd, so accepting the word would turn a
    /// forgotten field into the whole runner.
    #[test]
    fn a_scope_word_this_build_does_not_have_is_refused() {
        for word in ["admin", "", "READ", "unspecified", "host-admin"] {
            assert!(scope_from_word(word).is_none(), "{word} was accepted");
        }
    }

    /// A foreign line reports what it is: no device, no grant.
    #[test]
    fn a_line_far_cooler_did_not_write_grants_nothing_and_names_nobody() {
        let hand_written = farcooler_protocol::v1::EnrolledClient {
            client_id: String::new(),
            fingerprint: "SHA256:whatever".into(),
            label: "me@laptop".into(),
            scope: farcooler_protocol::v1::Scope::Unspecified as i32,
            account: "you".into(),
            enrolled_at: 0,
            foreign: true,
            // Never true for a line Far Cooler did not write: a plain line of
            // OURS is one this program rendered and manages, and a stranger's
            // key is left exactly as it was found.
            shell_access: false,
        };
        let json = enrolled_client_json(&hand_written);
        assert_eq!(json["foreign"], true);
        assert_eq!(json["clientId"], "");
        assert_eq!(json["scope"], "unspecified", "reading an unscoped line as admin would lie");
        assert_eq!(json["enrolledAt"], 0);
        assert_eq!(json["shellAccess"], false);
    }

    /// The two lines of one Mac are distinguishable in the JSON three apps decode.
    ///
    /// The point of the field. Both lines carry the same client id and both are
    /// ours, so `shellAccess` is the only thing separating "Far Cooler access"
    /// from "shell access" on screen — and the copy under the second row promises
    /// that removing it takes ssh, git and Zed away. A row that could not tell
    /// which it was would make that promise about the wrong key.
    #[test]
    fn a_mac_s_two_lines_are_told_apart_by_the_field_the_rows_need() {
        let plain = farcooler_protocol::v1::EnrolledClient {
            client_id: "mac-9".into(),
            fingerprint: "SHA256:b".into(),
            label: "farcooler-shell-macbook-air-1a2b3c4d".into(),
            // A plain line carries no forced command, so it carries no scope
            // word either — there is nowhere in the line to put one.
            scope: farcooler_protocol::v1::Scope::Unspecified as i32,
            account: "you".into(),
            enrolled_at: 0,
            foreign: false,
            shell_access: true,
        };
        let restricted = farcooler_protocol::v1::EnrolledClient {
            fingerprint: "SHA256:a".into(),
            label: "farcooler-macbook-air-1a2b3c4d".into(),
            scope: farcooler_protocol::v1::Scope::Control as i32,
            shell_access: false,
            ..plain.clone()
        };
        let plain = enrolled_client_json(&plain);
        let restricted = enrolled_client_json(&restricted);
        assert_eq!(plain["clientId"], restricted["clientId"], "one device, two lines");
        assert_eq!(plain["shellAccess"], true);
        assert_eq!(restricted["shellAccess"], false);
        assert_eq!(plain["foreign"], false, "a plain line of ours is managed, not a stranger's");
    }

    #[test]
    fn silence_from_the_far_side_reads_as_a_missing_daemon() {
        // The single most common remote failure, and it must not surface as a
        // protocol error nobody can act on.
        let error: SessionError = ClientError::Closed.into();
        assert!(matches!(error, SessionError::DaemonMissing { .. }));
        assert!(error.to_string().contains("installed"));
        // It names the binary this build actually asked for. A preview client that
        // reported `farcoolerd` would send someone to check the wrong thing,
        // and finding that installed would make the message look like a lie.
        assert!(error.to_string().contains(daemon_binary()));
    }

    #[test]
    fn a_dead_socket_is_told_apart_from_a_confused_one() {
        // The distinction the whole reconnect story rests on. An io error
        // reading a frame means the link is gone and reconnecting is the fix;
        // bytes that do not decode mean one side has a bug, and reconnecting
        // over it would just reproduce it on every attempt.
        let broken: SessionError =
            ClientError::Codec(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()).into();
        assert!(matches!(broken, SessionError::Disconnected(_)));
        assert!(broken.is_disconnect());

        let truncated: SessionError = ClientError::Codec(CodecError::Truncated).into();
        assert!(truncated.is_disconnect());

        let garbage: SessionError = ClientError::Codec(CodecError::Framing(
            farcooler_protocol::framing::FramingError::Malformed,
        ))
        .into();
        assert!(matches!(garbage, SessionError::Protocol(_)));
        assert!(!garbage.is_disconnect());
    }

    #[test]
    fn a_refused_request_leaves_the_session_alone() {
        // Everything the daemon answers — badly, or with the wrong shape — is
        // a session still worth talking on. Treating these as drops would
        // reconnect on every typo'd method a client ever sends.
        assert!(!SessionError::Protocol("nope".into()).is_disconnect());
        assert!(!SessionError::WrongResult { expected: "host", got: "workspace" }.is_disconnect());
        assert!(!SessionError::VersionMismatch { daemon: 2, client: 1 }.is_disconnect());
    }

    #[test]
    fn a_missing_daemon_means_two_things_depending_on_when_it_is_asked() {
        // At connect time it is a diagnosis: go install Far Cooler over there.
        // Mid-session it can only mean the pipe closed under us, so it counts
        // as a drop — and the message stays the useful one either way.
        let missing = SessionError::DaemonMissing { daemon: daemon_binary() };
        assert!(missing.is_disconnect());
        assert!(missing.to_string().contains("installed"));
    }

    /// The three fields the daemon computes and this used to throw away.
    ///
    /// `head_oid` is the only thing that can prove a squash-merged branch holds
    /// nothing unmerged, `merged_at` is when it landed, and `fetched_at` is what
    /// `stale` is derived from — all three already crossed the wire and stopped
    /// at this function. Pinned here because this is the shape both apps decode:
    /// a key that stops being written is a feature that stops existing on every
    /// client at once.
    #[test]
    fn a_pr_carries_every_field_the_daemon_computed_for_it() {
        let list = farcooler_protocol::v1::StackLinkList {
            repository_id: bytes::Bytes::new(),
            items: vec![farcooler_protocol::v1::StackLink {
                branch: "feat/x".into(),
                parent_branch: "main".into(),
                parent_source: farcooler_protocol::v1::ParentSource::Recorded as i32,
                head_commit: "aaa".into(),
                ahead: 1,
                behind: 0,
                pr: Some(farcooler_protocol::v1::PrStatus {
                    number: 335,
                    url: "https://github.com/o/r/pull/335".into(),
                    state: farcooler_protocol::v1::PrState::Open as i32,
                    checks: farcooler_protocol::v1::CheckState::Passing as i32,
                    review_decision: farcooler_protocol::v1::ReviewDecision::Approved as i32,
                    head_oid: "deadbeef".into(),
                    merged_at: Some(1_785_925_777_000),
                    fetched_at: 1_785_925_800_000,
                    stale: true,
                }),
            }],
            cycle_detected: false,
            pr_known: true,
            repo_url: "https://github.com/o/r".into(),
        };

        let json = stack_json(&list);
        let pr = &json["links"][0]["pr"];
        assert_eq!(pr["headOid"], "deadbeef");
        assert_eq!(pr["mergedAt"], 1_785_925_777_000i64);
        assert_eq!(pr["fetchedAt"], 1_785_925_800_000i64);
        assert_eq!(pr["stale"], true);
    }

    /// "GitHub says there is no PR" and "we could not ask GitHub" are the same
    /// absent `pr` on every link, and a client that could not tell them apart
    /// would offer to create a pull request that already exists.
    #[test]
    fn a_read_says_whether_github_answered_at_all() {
        let mut list = farcooler_protocol::v1::StackLinkList {
            repository_id: bytes::Bytes::new(),
            items: Vec::new(),
            cycle_detected: false,
            pr_known: false,
            repo_url: String::new(),
        };
        let json = stack_json(&list);
        assert_eq!(json["prKnown"], false);
        // Empty is not a URL. A client must get nothing rather than a link to
        // nowhere, so the key is null and not "".
        assert!(json["repoUrl"].is_null(), "an unknown repository URL is absent, not empty");

        list.pr_known = true;
        list.repo_url = "https://github.example/o/r".into();
        let json = stack_json(&list);
        assert_eq!(json["prKnown"], true);
        assert_eq!(json["repoUrl"], "https://github.example/o/r");
    }

    #[test]
    fn a_terminal_reports_its_pane_mode_as_a_word_a_client_can_switch_on() {
        // Numbers would make every client carry a copy of the enum and drift
        // from it. The label is the daemon's answer, not a code to look up.
        assert_eq!(pane_mode_label(farcooler_protocol::v1::PaneMode::Terminal as i32), "terminal");
        assert_eq!(pane_mode_label(farcooler_protocol::v1::PaneMode::Agent as i32), "agent");
        // An unknown value is the mode that always works, not a guess.
        assert_eq!(pane_mode_label(99), "terminal");
    }
}
