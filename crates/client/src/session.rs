//! An Overnight session, over whichever transport reached the host.
//!
//! Above this line nothing knows whether it is talking to a local Unix socket
//! or to a daemon on the other side of an SSH connection. That is the property
//! worth protecting: a remote host is not a different product with its own
//! subtly different behaviour.
//!
//! Answers come back as JSON rather than protobuf. Not because JSON is better —
//! it is not, and the wire stays protobuf — but because the alternative is a
//! protobuf runtime and generated types in Swift and again in Kotlin, to
//! describe messages this crate has already decoded. JSON at the boundary means
//! the iOS app decodes the same shapes the Mac app already decodes from the
//! CLI, with the model types it already has.

use overnight_protocol::v1::{
    Repository, RepositoryRoot, Terminal, TerminalState, Workspace, WorkspaceState, request, result,
};
use overnight_transport::{Client, ClientError};
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
    #[error(
        "connected, but `overnightd --stdio` did not answer. Is Overnight installed on that host?"
    )]
    DaemonMissing,
    #[error("the host runs protocol {daemon}; this client speaks {client}")]
    VersionMismatch { daemon: u32, client: u32 },
}

impl From<ClientError> for SessionError {
    fn from(error: ClientError) -> Self {
        match error {
            // Silence from the far side almost always means the command did not
            // exist, not that the protocol went wrong.
            ClientError::Closed | ClientError::NoHello => SessionError::DaemonMissing,
            ClientError::VersionMismatch { daemon, client } => {
                SessionError::VersionMismatch { daemon, client }
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

/// A connected Overnight host.
pub struct Session {
    client: Client<Reader, Writer>,
    /// Held so the SSH connection lives as long as the session does.
    _ssh: Option<ssh::Session>,
}

impl Session {
    /// Connect over SSH and start a daemon session on the far side.
    pub async fn connect_ssh(destination: &ssh::Destination) -> Result<Self, SessionError> {
        let mut transport = ssh::Session::open(destination).await?;
        let streams = transport.exec("overnightd --stdio").await?;

        let client = Client::over(
            Box::new(streams.reader) as Reader,
            Box::new(streams.writer) as Writer,
            "overnight-mobile",
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
        let streams = ssh.exec(&format!("overnightd --stream {terminal}")).await?;
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
            "overnight-mobile",
            env!("CARGO_PKG_VERSION"),
        )
        .await?;
        Ok(Self { client, _ssh: None })
    }

    pub fn daemon_version(&self) -> &str {
        &self.client.server_hello().daemon_version
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
            host.self_health != overnight_protocol::v1::SelfHealth::Degraded as i32;

        let items: Vec<_> = workspaces
            .iter()
            .map(|w| {
                json!({
                    "id": uuid_of(&w.id).to_string(),
                    "short": short(&w.id),
                    "task": w.task_name,
                    "branch": w.branch,
                    "worktree": w.worktree_path,
                    "state": workspace_label(w.state()),
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

    pub async fn host(&mut self) -> Result<overnight_protocol::v1::Host, SessionError> {
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

    // ---- mutations ----

    pub async fn create_workspace(
        &mut self,
        repository: Uuid,
        task: &str,
        branch: &str,
        base: &str,
    ) -> Result<Workspace, SessionError> {
        let payload = request::Payload::WorkspaceCreate(overnight_protocol::v1::WorkspaceCreate {
            task_name: task.into(),
            branch: branch.into(),
            base_revision: base.into(),
            cli_preset: String::new(),
            adopt_existing: false,
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
        let payload = request::Payload::TerminalCreate(overnight_protocol::v1::TerminalCreate {
            title: title.into(),
            command_preset: preset.into(),
            join_active_group,
        });
        match self.value("terminal.create", Some(workspace), Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    pub async fn archive_workspace(&mut self, workspace: Uuid) -> Result<(), SessionError> {
        self.value("workspace.archive", Some(workspace), None).await.map(|_| ())
    }

    pub async fn restore_workspace(&mut self, workspace: Uuid) -> Result<(), SessionError> {
        self.value("workspace.restore", Some(workspace), None).await.map(|_| ())
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

    /// A terminal's visible screen, with escapes intact.
    ///
    /// `known_revision` is the last one received; the host answers `unchanged`
    /// rather than resending a screen already held. Pass 0 to always get one.
    pub async fn screen(
        &mut self,
        terminal: Uuid,
        known_revision: u64,
    ) -> Result<overnight_protocol::v1::TerminalScreen, SessionError> {
        let payload = request::Payload::TerminalScreenRequest(
            overnight_protocol::v1::TerminalScreenRequest { known_revision },
        );
        match self.value("terminal.screen", Some(terminal), Some(payload)).await? {
            result::Value::TerminalScreen(s) => Ok(s),
            other => Err(wrong("terminal_screen", &other)),
        }
    }

    /// Send exact bytes to a terminal.
    pub async fn write(&mut self, terminal: Uuid, bytes: Vec<u8>) -> Result<(), SessionError> {
        let payload = request::Payload::TerminalWrite(overnight_protocol::v1::TerminalWrite {
            payload: bytes.into(),
        });
        self.value("terminal.write", Some(terminal), Some(payload)).await?;
        Ok(())
    }

    pub async fn resize_terminal(
        &mut self,
        terminal: Uuid,
        columns: u32,
        rows: u32,
    ) -> Result<(), SessionError> {
        let payload = request::Payload::TerminalResize(overnight_protocol::v1::TerminalResize {
            columns,
            rows,
            view_activity_id: 0,
        });
        self.value("terminal.resize", Some(terminal), Some(payload)).await.map(|_| ())
    }

    async fn value(
        &mut self,
        method: &str,
        target: Option<Uuid>,
        payload: Option<request::Payload>,
    ) -> Result<result::Value, SessionError> {
        let mut request = overnight_transport::request(method);
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
    use overnight_protocol::v1::AgentActivity;
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
fn activity_since(t: &overnight_protocol::v1::Terminal) -> Option<i64> {
    t.activity_changed_at.as_ref().map(|ts| ts.seconds * 1000 + (ts.nanos as i64) / 1_000_000)
}

/// The pane mode, as a word rather than a number.
///
/// Same reason `activity_label` exists: a client that switched on an integer
/// would hold a second copy of the enum and drift from it silently.
fn pane_mode_label(mode: i32) -> &'static str {
    match overnight_protocol::v1::PaneMode::try_from(mode) {
        Ok(overnight_protocol::v1::PaneMode::Agent) => "agent",
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
        WorkspaceState::Archived => "archived",
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
        assert!(matches!(error, SessionError::DaemonMissing));
        assert!(error.to_string().contains("installed"));
    }

    #[test]
    fn a_terminal_reports_its_pane_mode_as_a_word_a_client_can_switch_on() {
        // Numbers would make every client carry a copy of the enum and drift
        // from it. The label is the daemon's answer, not a code to look up.
        assert_eq!(pane_mode_label(overnight_protocol::v1::PaneMode::Terminal as i32), "terminal");
        assert_eq!(pane_mode_label(overnight_protocol::v1::PaneMode::Agent as i32), "agent");
        // An unknown value is the mode that always works, not a guess.
        assert_eq!(pane_mode_label(99), "terminal");
    }
}
