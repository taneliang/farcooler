//! The runner's protocol, through the client core every phone runs
//! (`farcooler_client::session::Session`, ov-768).
//!
//! This file used to be a protocol client of its own, laid out as
//! `crates/client/src/session.rs` lays one out: a control session on one
//! channel, and a second session per pane. Now it is that `Session`, built
//! for the browser (`farcooler-client`'s `web` feature) over the connection
//! `ssh::connect` authenticated with the passkey. What a phone reads, the page
//! reads in the same shapes: `needs_you_json`, `plan_json`, `page_json`.
//!
//! The layout is the session's: calls on the control channel, a pane's bytes
//! on a channel of their own, both running `~/.local/bin/<daemon> --stdio`.
//! A pane's bytes never queue behind a call's answer, and a call's answer
//! never waits behind a pane's backlog: SSH gives each channel its own window.
//!
//! What is left here is the page's: its shapes for a terminal list, a pane
//! that hands its bytes to callbacks, and the words a failure crosses as.

use std::future::Future;
use std::rc::Rc;

use farcooler_client::session::{Session, SessionError};
use farcooler_client::ssh::SshError;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{ErrorCode, TerminalState};
use serde::Serialize;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::error::WebError;
use crate::ssh::Connection;

/// The name this client gives in its hello, so a runner's log says which
/// client connected.
pub const CLIENT_NAME: &str = "farcooler-web";

/// Run a task on this thread.
pub(crate) fn spawn(task: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(task);
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::task::spawn_local(task);
    }
}

/// A session's failure, as the page's word.
///
/// Each is the nearest word the page already had, so its sentences are
/// unchanged: a link that ended is `closed` however it ended, and a refusal
/// keeps the runner's own code's name.
impl From<SessionError> for WebError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Ssh(SshError::Exec(why)) => WebError::Exec(why),
            SessionError::Ssh(other) => WebError::Handshake(other.to_string()),
            // `--stdio` never said hello: the daemon is not there to start.
            missing @ SessionError::DaemonMissing { .. } => WebError::Exec(missing.to_string()),
            SessionError::Refused { code, .. } => WebError::Refused(
                ErrorCode::try_from(code).map(|c| c.as_str_name()).unwrap_or("ERROR_CODE_UNSPECIFIED").to_string(),
            ),
            SessionError::Disconnected(_) | SessionError::NotSent(_) => WebError::Closed,
            other => WebError::Protocol(other.to_string()),
        }
    }
}

/// One terminal, as the page lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TerminalSummary {
    pub id: String,
    pub title: String,
    /// `running`, `exited` and so on: the daemon's word, lowercased. Clients
    /// render the daemon's state and never derive their own.
    pub state: String,
    pub columns: u32,
    pub rows: u32,
}

/// A connected runner: the client core's session over one SSH connection.
pub struct Runner {
    session: Rc<Session>,
}

impl Runner {
    /// Start the control session on an authenticated connection.
    pub async fn start(conn: Connection) -> Result<Runner, WebError> {
        let session = Session::over_ssh(conn, CLIENT_NAME).await?;
        Ok(Runner { session: Rc::new(session) })
    }

    pub fn capabilities(&self) -> &[String] {
        self.session.capabilities()
    }

    /// Every terminal this runner has, as `terminal.list` gives them.
    pub async fn terminals(&self) -> Result<Vec<TerminalSummary>, WebError> {
        Ok(self
            .session
            .terminals()
            .await?
            .into_iter()
            .map(|t| TerminalSummary {
                id: Uuid::from_slice(&t.id).map(|u| u.to_string()).unwrap_or_default(),
                title: t.title,
                state: TerminalState::try_from(t.state)
                    .map(|s| s.as_str_name().trim_start_matches("TERMINAL_STATE_").to_ascii_lowercase())
                    .unwrap_or_else(|_| "unknown".into()),
                columns: t.columns,
                rows: t.rows,
            })
            .collect())
    }

    /// What needs this person, in the phones' shape (`needs_you_json`).
    pub async fn needs_you(&self) -> Result<serde_json::Value, WebError> {
        Ok(self.session.needs_you().await?)
    }

    /// A workspace's plan, in the phones' shape (`plan_json`).
    pub async fn plan(&self, workspace: Uuid) -> Result<serde_json::Value, WebError> {
        Ok(farcooler_client::plan_json::plan_json(&self.session.plan(workspace).await?))
    }

    /// One of a workspace's orchestrator pages, in the phones' shape
    /// (`page_json`).
    pub async fn page(&self, workspace: Uuid, slot: &str) -> Result<serde_json::Value, WebError> {
        Ok(farcooler_client::page_json::page_json(&self.session.page(workspace, slot).await?))
    }

    /// Attach a pane: a second channel carrying the terminal's bytes.
    ///
    /// `output` gets every byte the terminal prints, starting with the replay
    /// the runner sends on attach; `ended` is called once when the stream
    /// stops, for any reason. Answers once the runner has accepted the attach,
    /// so "this terminal is not running" is an error here rather than a pane
    /// that opens and immediately ends looking like one that finished.
    pub async fn attach(
        &self,
        terminal: Uuid,
        mut output: impl FnMut(&[u8]) + 'static,
        ended: impl FnOnce() + 'static,
    ) -> Result<Pane, WebError> {
        // Asked first: on a runner without it the session falls back to an
        // exec that a browser's forced-command key never sees answered.
        if !self.session.can(capability::TERMINAL_STREAM) {
            return Err(WebError::Capability(capability::TERMINAL_STREAM.into()));
        }
        let mut stream = self.session.open_stream(terminal).await?;
        let (close, mut closing) = futures::channel::oneshot::channel::<()>();
        spawn(async move {
            let mut buf = vec![0u8; 1 << 15];
            loop {
                let read = std::pin::pin!(stream.read(&mut buf));
                match futures::future::select(read, &mut closing).await {
                    futures::future::Either::Left((Ok(n), _)) if n > 0 => output(&buf[..n]),
                    // The end of the stream, a failed read, or `close`. The
                    // stream is dropped with this task, which is what tells
                    // the runner nobody is watching.
                    _ => break,
                }
            }
            ended();
        });
        Ok(Pane { terminal, session: self.session.clone(), close: Some(close) })
    }
}

/// One attached terminal. Input and resizes go through the runner's control
/// session; the pane's own channel only ever carries output.
pub struct Pane {
    terminal: Uuid,
    session: Rc<Session>,
    /// Sent, or dropped with the pane, to stop reading: see `close`.
    close: Option<futures::channel::oneshot::Sender<()>>,
}

impl Pane {
    /// Type into the terminal.
    ///
    /// Not `async` on purpose: the write is queued before this returns
    /// (`Session::start_write`), so two writes made one after the other reach
    /// the runner in that order however their answers are awaited. Keystrokes
    /// depend on it.
    pub fn write(&self, bytes: &[u8]) -> impl Future<Output = Result<(), WebError>> + 'static {
        let queued = self.session.start_write(self.terminal, bytes.to_vec());
        async move { Ok(queued?.value().await.map(|_| ())?) }
    }

    /// Tell the runner this pane's size. Queued as `write` is.
    pub fn resize(&self, columns: u32, rows: u32) -> impl Future<Output = Result<(), WebError>> + 'static {
        let queued = self.session.start_resize(self.terminal, columns, rows);
        async move { Ok(queued?.value().await.map(|_| ())?) }
    }

    /// Stop watching. The terminal keeps running on the runner.
    pub fn close(&mut self) -> impl Future<Output = ()> + 'static {
        if let Some(close) = self.close.take() {
            let _ = close.send(());
        }
        async {}
    }
}
