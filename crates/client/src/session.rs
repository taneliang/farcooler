//! An Far Cooler session, over whichever transport reached the host.
//!
//! Above this line nothing knows whether it is talking to a local Unix socket
//! or to a daemon on the other side of an SSH connection. That is the property
//! worth protecting: a remote host is not a different product with its own
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
        "connected, but `{daemon} --stdio` did not answer. Is Far Cooler installed on that machine?"
    )]
    DaemonMissing { daemon: &'static str },
    #[error("the host runs protocol {daemon}; this client speaks {client}")]
    VersionMismatch { daemon: u32, client: u32 },
    /// The link underneath this session is gone.
    ///
    /// Deliberately distinct from `Protocol`, which is the far side saying
    /// something we could not make sense of — that is a session still worth
    /// talking on, and a client that reconnected over it would be reconnecting
    /// over a bug.
    #[error("the connection to the host was lost: {0}")]
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

/// A connected Far Cooler host.
pub struct Session {
    client: Client<Reader, Writer>,
    /// Held so the SSH connection lives as long as the session does.
    _ssh: Option<ssh::Session>,
}

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
        // ~/.local/bin, where `host install` puts the binary.
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

        Ok(Self { client, _ssh: Some(transport) })
    }

    /// Open a second ssh channel carrying nothing but one terminal's bytes.
    ///
    /// The data plane, kept off the control connection on purpose. That
    /// connection answers one request at a time, so a stream sharing it would
    /// sit behind every fleet refresh and every refresh behind a stream's bytes.
    /// ssh already multiplexes channels; letting it do that is both simpler and
    /// faster than teaching the control protocol to interleave.
    ///
    /// The latency floor becomes the network round trip. Measured at 19ms over a
    /// loopback ssh connection, against a second of polling before it — and the
    /// second was never the network, it was the interval.
    pub async fn open_stream(
        &mut self,
        terminal: Uuid,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, SessionError> {
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

    /// Connect to a daemon on this machine. Used by tests and by any desktop
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
        Ok(Self { client, _ssh: None })
    }

    pub fn daemon_version(&self) -> &str {
        &self.client.server_hello().daemon_version
    }

    /// What the machine on the other end can do, by name.
    ///
    /// Learned in the handshake, so it is available before the first request.
    /// A client that is newer than the machine it reached uses this to DIM the
    /// controls that machine cannot serve, with a reason — rather than hiding
    /// them, which makes the same app look different on two machines for no
    /// stated cause, or letting them fail on use, which teaches people that
    /// buttons sometimes do nothing.
    pub fn capabilities(&self) -> &[String] {
        &self.client.server_hello().capabilities
    }

    /// Whether the machine can do something, by name.
    ///
    /// Empty means a daemon too old to answer the question at all — every one
    /// of those predates capabilities, so it has exactly the feature set that
    /// existed then. Reporting `true` for the two floor capabilities and
    /// `false` for the rest is the honest reading, and it is what lets a new
    /// app talk to an old machine without a special case at every call site.
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
        let terminals = self.terminals().await?;
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
                            "epoch": t.epoch,
                            "paneMode": pane_mode_label(t.pane_mode),
                            // Without this the phone's terminal/chat switch
                            // could never appear: `canSwitchPaneMode` reads it,
                            // and a field the host never sends is a capability
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
        match self.value("terminal.list", None, None).await? {
            result::Value::TerminalList(l) => Ok(l.items),
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

    /// The colour schemes this host defines.
    ///
    /// Only the host's own — the built-ins are compiled into every client, so
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

    // ---- machine settings ----
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

    /// A terminal's visible screen, with escapes intact.
    ///
    /// `known_revision` is the last one received; the host answers `unchanged`
    /// rather than resending a screen already held. Pass 0 to always get one.
    pub async fn screen(
        &mut self,
        terminal: Uuid,
        known_revision: u64,
    ) -> Result<farcooler_protocol::v1::TerminalScreen, SessionError> {
        let payload = request::Payload::TerminalScreenRequest(
            farcooler_protocol::v1::TerminalScreenRequest { known_revision },
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
    /// into the message. That is a path on the machine that picked the file,
    /// and the agent runs on the HOST — so it worked when those were the same
    /// machine and silently referred to nothing when they were not, which is
    /// every phone and every remote host.
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
    /// Relative, never a host path: the same redaction the rest of this
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

    /// Every worktree that changed since it was last looked at.
    pub async fn changes_inbox(&mut self) -> Result<serde_json::Value, SessionError> {
        let payload =
            request::Payload::ChangesInbox(farcooler_protocol::v1::ChangesInboxRequest {});
        match self.value("changes.inbox", None, Some(payload)).await? {
            result::Value::ChangesInbox(inbox) => Ok(json!({
                "items": inbox.items.iter().map(|w| json!({
                    "workspace_id": uuid_of(&w.workspace_id).to_string(),
                    "short": short(&w.workspace_id),
                    "task_name": w.task_name,
                    "branch": w.branch,
                    "changed_since_reviewed": w.changed_since_reviewed,
                    "insertions": w.insertions,
                    "deletions": w.deletions,
                })).collect::<Vec<_>>(),
                "elsewhere": inbox.elsewhere,
            })),
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

    /// Remove a repository root. The paired `add` already exists above.
    pub async fn remove_repository_root(&mut self, root: Uuid) -> Result<(), SessionError> {
        self.value("repository_root.remove", Some(root), None).await?;
        Ok(())
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
    }
}

/// A change set, in the shape `farcooler changes status --json` prints.
///
/// Kept byte-identical to `crates/cli/src/changes.rs::change_set_json` on
/// purpose: the Mac decodes that, the phones decode this, and two spellings of
/// one change set is how the same worktree ends up described two ways.
fn change_set_json(cs: &farcooler_protocol::v1::ChangeSet) -> serde_json::Value {
    json!({
        "branch": cs.branch,
        "base_ref": cs.base_ref,
        "base_source": base_source_name(cs.base_source),
        "base_commit": cs.base_commit,
        "head_commit": cs.head_commit,
        "insertions": cs.insertions,
        "deletions": cs.deletions,
        "commits": cs.commits.iter().map(|c| json!({
            "sha": c.sha,
            "subject": c.subject,
            "author": c.author,
            "timestamp": c.timestamp,
        })).collect::<Vec<_>>(),
        "files": cs.files.iter().map(file_change_json).collect::<Vec<_>>(),
        "working_tree": cs.working_tree.as_ref().map(|w| json!({
            "staged": w.staged.iter().map(|f| &f.path).collect::<Vec<_>>(),
            "unstaged": w.unstaged.iter().map(|f| &f.path).collect::<Vec<_>>(),
            "untracked": w.untracked,
            "conflicted": w.conflicted,
            "changes": w.staged.iter().chain(w.unstaged.iter()).map(|f| json!({
                "path": f.path,
                "status": file_status_name(f.status),
                "old_path": f.old_path,
            })).collect::<Vec<_>>(),
        })),
    })
}

fn file_change_json(f: &farcooler_protocol::v1::FileChange) -> serde_json::Value {
    json!({
        "path": f.path,
        "status": file_status_name(f.status),
        "old_path": f.old_path,
        "insertions": f.insertions,
        "deletions": f.deletions,
        "binary": f.binary,
    })
}

/// One file's patch, flattened out of its hunks.
///
/// The hunk boundaries survive as `gap` markers rather than as nesting: a
/// client draws a diff as one list and needs to know where the unchanged lines
/// were left out, which is exactly what the jump between two hunks' line
/// numbers says. `AgentKit.DiffComputation` already parses this shape.
fn file_diff_json(d: &farcooler_protocol::v1::FileDiff) -> serde_json::Value {
    use farcooler_protocol::v1::{DiffLineKind, DiffUnsupported};
    json!({
        "path": d.path,
        // Why there are no hunks, when the reason is not "nothing changed". An
        // empty list on its own reads as unchanged, which for a binary file is
        // a lie — see `DiffUnsupported` in the protocol.
        "unsupported": d.unsupported.and_then(|u| match DiffUnsupported::try_from(u) {
            Ok(DiffUnsupported::Binary) => Some("binary"),
            Ok(DiffUnsupported::Submodule) => Some("submodule"),
            Ok(DiffUnsupported::CombinedDiff) => Some("combined_diff"),
            Ok(DiffUnsupported::Malformed) => Some("malformed"),
            _ => None,
        }),
        "truncated": d.truncated.is_some(),
        "firstParentOfMerge": d.first_parent_of_merge,
        "hunks": d.hunks.iter().map(|h| json!({
            "index": h.index,
            "header": h.header,
            "oldStart": h.old_start,
            "newStart": h.new_start,
            "lines": h.lines.iter().map(|l| json!({
                "kind": match DiffLineKind::try_from(l.kind) {
                    Ok(DiffLineKind::Added) => "added",
                    Ok(DiffLineKind::Removed) => "removed",
                    _ => "context",
                },
                "oldNumber": l.old_no,
                "newNumber": l.new_no,
                "text": l.text,
                "noNewline": l.no_newline,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn stack_json(l: &farcooler_protocol::v1::StackLinkList) -> serde_json::Value {
    use farcooler_protocol::v1::{CheckState, ParentSource, PrState, ReviewDecision};
    json!({
        // Reported rather than followed: a parent chain that loops is walked as
        // far as it was walked, and a client that silently drew it would draw a
        // stack that does not exist.
        "cycleDetected": l.cycle_detected,
        "links": l.items.iter().map(|k| json!({
            "branch": k.branch,
            "parentBranch": k.parent_branch,
            // Only a GUESS is worth labeling. The others are recorded facts;
            // a guessed parent produces a wrong diff and looks like a right one.
            "parentGuessed": ParentSource::try_from(k.parent_source) == Ok(ParentSource::Guessed),
            "ahead": k.ahead,
            "behind": k.behind,
            "pr": k.pr.as_ref().map(|p| json!({
                "number": p.number,
                "url": p.url,
                "state": match PrState::try_from(p.state) {
                    Ok(PrState::Open) => "open",
                    Ok(PrState::Draft) => "draft",
                    Ok(PrState::Merged) => "merged",
                    Ok(PrState::Closed) => "closed",
                    _ => "unknown",
                },
                "checks": match CheckState::try_from(p.checks) {
                    Ok(CheckState::Passing) => "passing",
                    Ok(CheckState::Failing) => "failing",
                    Ok(CheckState::Pending) => "pending",
                    _ => "unknown",
                },
                "review": match ReviewDecision::try_from(p.review_decision) {
                    Ok(ReviewDecision::Approved) => "approved",
                    Ok(ReviewDecision::ChangesRequested) => "changes_requested",
                    Ok(ReviewDecision::ReviewRequired) => "review_required",
                    _ => "unknown",
                },
                // Whether this was read from GitHub long enough ago to doubt.
                "stale": p.stale,
            })),
        })).collect::<Vec<_>>(),
    })
}

fn file_status_name(status: i32) -> &'static str {
    use farcooler_protocol::v1::FileStatus;
    match FileStatus::try_from(status).unwrap_or(FileStatus::Unspecified) {
        FileStatus::Added => "added",
        FileStatus::Modified => "modified",
        FileStatus::Deleted => "deleted",
        FileStatus::Renamed => "renamed",
        FileStatus::Copied => "copied",
        FileStatus::TypeChanged => "type_changed",
        FileStatus::Untracked => "untracked",
        FileStatus::Conflicted => "conflicted",
        FileStatus::Unspecified => "modified",
    }
}

/// Where the base came from, because only one of these is a guess and only a
/// guess is worth warning about.
fn base_source_name(source: i32) -> &'static str {
    use farcooler_protocol::v1::BaseSource;
    match BaseSource::try_from(source).unwrap_or(BaseSource::Unspecified) {
        BaseSource::Recorded => "recorded",
        BaseSource::Upstream => "upstream",
        BaseSource::Guessed => "guessed",
        BaseSource::PrBase => "pr_base",
        BaseSource::DefaultBranch => "default_branch",
        BaseSource::Unspecified => "unknown",
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
