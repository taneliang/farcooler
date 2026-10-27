//! How a `Session` reaches its runner, and the channels it opens there: the
//! control connection, a pane's stream and the fleet's news.
//!
//! Its own file since the browser joined (ov-768): the connect is the one
//! part that differs between a phone and a page. A phone connects and
//! authenticates here (`connect_ssh`, `native`); a page authenticates with its
//! passkey in crates/web and hands over the connection (`over_ssh`). From the
//! connection on, both run the code below, and every task it starts goes
//! through `farcooler_transport::rt`, never `tokio::spawn`.

use farcooler_protocol::v1::request;
use farcooler_protocol::v1::result;
use farcooler_transport::Client;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use super::{EventSink, FleetEvent, Session, SessionError, daemon_binary};
use crate::ssh;

/// The name a phone's or a Mac's sessions give in their hello.
#[cfg(feature = "native")]
pub(super) const NATIVE_CLIENT: &str = "farcooler-mobile";

pub(super) type Reader = Box<dyn AsyncRead + Unpin + Send>;
pub(super) type Writer = Box<dyn AsyncWrite + Unpin + Send>;

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

/// The reader `attach_stream` returns: the bytes, and the task feeding them.
///
/// Dropping it aborts the task, which drops the protocol client and with it the
/// ssh channel. Without that the task only noticed a gone reader when its next
/// frame failed to write, so a pane closed while idle held one of sshd's ten
/// session slots until the terminal next printed (ov-825).
struct AttachReader {
    inner: tokio::io::DuplexStream,
    task: farcooler_transport::rt::Task<()>,
}

impl AsyncRead for AttachReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl Drop for AttachReader {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Session {
    /// Connect over SSH and start a daemon session on the far side.
    #[cfg(feature = "native")]
    pub async fn connect_ssh(destination: &ssh::Destination) -> Result<Self, SessionError> {
        // The rendezvous, applied before the dial that needs it. tailcat's
        // default DERP map is documented as best-effort and revocable at any
        // time, and DERP is where every tunneled connection meets rather than
        // a fallback for the ones that could not go direct — so this is the
        // line that makes recovering from a revocation a setting instead of
        // three app releases.
        //
        // Empty is left alone rather than sent. Empty already means "the
        // library default" to `set_derp_map_url`, so sending it would be a
        // no-op for an app that configured nothing and a CLEARING for any
        // process that was configured another way — a `farcoolerd` that read
        // `FARCOOLER_DERP_MAP` and then opened a client session of its own
        // would move itself off its own rendezvous on the way past.
        if !destination.derp_map.is_empty() {
            farcooler_tailcat::set_derp_map_url(&destination.derp_map);
        }
        let transport = ssh::Session::open(destination).await?;
        Self::over_link(Box::new(transport), NATIVE_CLIENT).await
    }

    /// Start a daemon session over an SSH connection somebody else made and
    /// authenticated: the browser's, which signs in with its passkey over a
    /// stream the page dialed (crates/web, ov-768). `client_name` is what the
    /// hello says this client is, for the runner's log.
    ///
    /// The connection is held for as long as the session is, and every
    /// channel this session opens is opened on it.
    pub async fn over_ssh<H: russh::client::Handler + 'static>(
        connection: russh::client::Handle<H>,
        client_name: &'static str,
    ) -> Result<Self, SessionError> {
        Self::over_link(Box::new(connection), client_name).await
    }

    async fn over_link(link: Box<dyn ssh::Exec>, client_name: &'static str) -> Result<Self, SessionError> {
        // Named by tilde, not bare: a non-login ssh exec's PATH often lacks
        // ~/.local/bin, where `runner install` puts the binary.
        //
        // The name carries this client's own channel, which is what keeps a
        // preview app off a stable daemon: they are different binaries at
        // different paths, so the two never meet rather than meeting and
        // having to negotiate.
        let streams = link.exec(&format!("~/.local/bin/{} --stdio", daemon_binary())).await?;

        let client = Client::over(
            Box::new(streams.reader) as Reader,
            Box::new(streams.writer) as Writer,
            client_name,
            env!("CARGO_PKG_VERSION"),
        )
        .await?;

        // This connection's own events are dropped: fleet news comes on the
        // channel `subscribe` opens, and nothing reads them here.
        client.ignore_events();
        Ok(Self {
            client,
            link: Some(link),
            client_name,
            events: std::sync::Mutex::new(None),
            #[cfg(feature = "native")]
            socket: None,
        })
    }

    /// Open a second ssh channel carrying nothing but one terminal's bytes.
    ///
    /// The data plane, kept off the control connection on purpose. That
    /// connection carries calls, whose answers it holds no more than it must,
    /// and a pane's bytes would be pushback on all of them: a viewer that
    /// stops reading would stall every call's answer behind its backlog. ssh
    /// already multiplexes channels, each with its own window.
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
        &self,
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
        // Named by tilde, not bare: see connect_ssh above.
        let streams =
            self.exec(&format!("~/.local/bin/{} --stream {terminal}", daemon_binary())).await?;
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
        &self,
        terminal: Uuid,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, SessionError> {
        // The same command the control connection runs, and deliberately so.
        let streams = self.exec(&format!("~/.local/bin/{} --stdio", daemon_binary())).await?;
        let mut client = Client::over(
            Box::new(streams.reader) as Reader,
            Box::new(streams.writer) as Writer,
            self.client_name,
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
        // dropped, which fails the write, but only when the next frame arrives.
        // `AttachReader` closes that gap: dropping it aborts the task at once. The
        // last of those is what returns the ssh session slot — `ChannelGuard` in `ssh.rs` — so a client that stops
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
        let news = self.events.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let (mine, theirs) = tokio::io::duplex(64 * 1024);
        let task = farcooler_transport::rt::spawn(async move {
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

        Ok(Box::new(AttachReader { inner: mine, task }))
    }

    /// Receive fleet news on a channel of its own, for as long as this session
    /// lives.
    ///
    /// **Why a third channel and not the control connection.** Chosen when
    /// `Client::call` owned the reader for the length of a request, so nothing
    /// read the control socket between calls. It has a reader of its own now
    /// (ov-147), but the control connection still ignores its events: a
    /// channel of their own keeps fleet news and its pushback apart from calls.
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
        &self,
        sink: EventSink,
    ) -> Result<farcooler_transport::rt::Task<()>, SessionError> {
        // Kept even if the channel below cannot be opened. See the field: a
        // terminal attachment carries the same broadcast, so a session that
        // failed to get its own channel is not a session with no push at all.
        *self.events.lock().unwrap_or_else(|p| p.into_inner()) = Some(std::sync::Arc::clone(&sink));

        let mut client = match &self.link {
            Some(_) => {
                // Named by tilde, not bare: see `connect_ssh` above.
                let streams =
                    self.exec(&format!("~/.local/bin/{} --stdio", daemon_binary())).await?;
                Client::over(
                    Box::new(streams.reader) as Reader,
                    Box::new(streams.writer) as Writer,
                    self.client_name,
                    env!("CARGO_PKG_VERSION"),
                )
                .await?
            }
            // A second connection to the same socket. Not an ssh channel, but
            // the same thing one layer down, and the daemon cannot tell the
            // difference: it subscribes whatever connected.
            #[cfg(feature = "native")]
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
                    self.client_name,
                    env!("CARGO_PKG_VERSION"),
                )
                .await?
            }
            // Every session a browser holds is over ssh.
            #[cfg(not(feature = "native"))]
            None => {
                return Err(SessionError::Protocol("this session has nothing to open a channel on".into()));
            }
        };

        Ok(farcooler_transport::rt::spawn(async move {
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
    #[cfg(feature = "native")]
    pub async fn connect_local(socket: &std::path::Path) -> Result<Self, SessionError> {
        let stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(|e| SessionError::Protocol(format!("cannot reach the daemon: {e}")))?;
        let (read, write) = stream.into_split();
        let client = Client::over(
            Box::new(read) as Reader,
            Box::new(write) as Writer,
            NATIVE_CLIENT,
            env!("CARGO_PKG_VERSION"),
        )
        .await?;
        client.ignore_events();
        Ok(Self {
            client,
            link: None,
            client_name: NATIVE_CLIENT,
            events: std::sync::Mutex::new(None),
            socket: Some(socket.to_path_buf()),
        })
    }

    /// Run `command` on a channel of this session's ssh connection.
    async fn exec(&self, command: &str) -> Result<ssh::Streams, SessionError> {
        let link = self.link.as_ref().ok_or_else(|| {
            SessionError::Protocol("streaming needs an ssh session".into())
        })?;
        Ok(link.exec(command).await?)
    }
}
