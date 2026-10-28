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
use std::sync::Arc;

use farcooler_client::session::{Session, SessionError};
use farcooler_client::ssh::SshError;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{ErrorCode, TerminalState};
use serde::Serialize;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::error::WebError;
use crate::liveness::Link;
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
            SessionError::Refused { code, what, .. } => WebError::Refused {
                code: ErrorCode::try_from(code)
                    .map(|c| c.as_str_name().trim_start_matches("ERROR_CODE_").to_ascii_lowercase().replace('_', "-"))
                    .unwrap_or_else(|_| "unspecified".into()),
                what,
            },
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
    pub(crate) session: Rc<Session>,
    /// What this connection has heard, when it is watched (`start_watched`).
    link: Option<Arc<Link>>,
    /// Dropped with the runner, which is what stops the watching task.
    _watching: Option<futures::channel::oneshot::Sender<()>>,
}

impl Runner {
    /// Start the control session on an authenticated connection.
    pub async fn start(conn: Connection) -> Result<Runner, WebError> {
        let session = Session::over_ssh(conn, CLIENT_NAME).await?;
        Ok(Runner { session: Rc::new(session), link: None, _watching: None })
    }

    /// Start the control session and watch the connection (ov-772).
    ///
    /// `link` is the one whose [`Link::tap`] wrapped the stream `conn` runs
    /// over, so it hears every byte. A quiet connection is probed, and one
    /// that stays quiet past `Liveness::TUNNEL`'s allowance is given up on:
    /// `on_lost` is called once, and every pane's `ended` runs with it. The
    /// watching stops when the runner is dropped, and then `on_lost` is not
    /// called.
    pub async fn start_watched(
        conn: Connection,
        link: Arc<Link>,
        on_lost: impl FnOnce() + 'static,
    ) -> Result<Runner, WebError> {
        let session = Rc::new(Session::over_ssh(conn, CLIENT_NAME).await?);
        let (stop, stopped) = futures::channel::oneshot::channel::<()>();
        let (watched, watching) = (session.clone(), link.clone());
        spawn(async move {
            let probe = {
                let session = watched.clone();
                move || {
                    let session = session.clone();
                    spawn(async move {
                        let _ = session.probe().await;
                    });
                }
            };
            let run = std::pin::pin!(watching.run(probe, || watched.link_ended()));
            // Only the link's own verdict calls `on_lost`: a runner dropped
            // by the page (the sender going away) is not a loss.
            if let futures::future::Either::Left(_) = futures::future::select(run, stopped).await {
                on_lost();
            }
        });
        Ok(Runner { session, link: Some(link), _watching: Some(stop) })
    }

    /// Whether the connection answers, right now: the foreground probe
    /// (`Session::probe`, ov-613) and the verdict the phones give it. Silence
    /// within its deadline ends the link, and so does a closed connection.
    /// Called when the tab comes back and when the browser says it is online.
    pub async fn check(&self) -> bool {
        if self.lost() || self.session.link_ended() {
            return false;
        }
        match self.session.probe().await {
            Ok(()) => true,
            Err(e) if farcooler_client::deadlines::ends_the_link(farcooler_client::deadlines::PROBE_METHOD, &e) => {
                if let Some(link) = &self.link {
                    link.lose();
                }
                false
            }
            // A refusal is an answer.
            Err(_) => true,
        }
    }

    /// Whether the watch has given up on this connection.
    pub fn lost(&self) -> bool {
        self.link.as_ref().is_some_and(|l| l.is_lost())
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

    /// Answer an ask Needs You lists, as the person at this browser
    /// (`terminal.agent_answer`, the call a phone's Allow makes). `ask` is the
    /// item's `ask_id` and `option` the action's `id`.
    ///
    /// Sent the moment it is called and answered when the runner has taken it,
    /// which is when the runner drops the item from every device's list.
    pub async fn answer_ask(&self, terminal: Uuid, ask: &str, option: &str) -> Result<(), WebError> {
        self.session.agent_answer(terminal, ask, option, Default::default()).await?;
        Ok(())
    }

    /// Answer a decision Needs You lists with the option's text, as a note on
    /// its task (`task.note`, the call a phone's answer makes).
    pub async fn answer_decision(&self, task: Uuid, body: &str) -> Result<(), WebError> {
        self.session.task_note(farcooler_client::session::task_note_append(task, farcooler_protocol::v1::TaskNoteKind::Answer, body)).await?;
        Ok(())
    }

    /// A workspace's plan, in the phones' shape (`plan_json`).
    pub async fn plan(&self, workspace: Uuid) -> Result<serde_json::Value, WebError> {
        Ok(farcooler_client::plan_json::plan_json(&self.session.plan(workspace).await?))
    }

    /// The runner's workspaces, in the shape the phones and the Mac read
    /// (`workspace_json`): the page picks the board whose plan it shows.
    pub async fn workspaces(&self) -> Result<serde_json::Value, WebError> {
        Ok(self.session.workspaces().await?.iter().map(farcooler_client::workspaces_json::workspace_json).collect())
    }

    /// Write a theme's Next line (`board_theme.update`).
    pub async fn set_theme_next(&self, theme: Uuid, next: &str) -> Result<(), WebError> {
        Ok(self.session.set_theme_next(theme, next).await?)
    }

    /// Keep one open ruling (`ruling.set`, as a person).
    pub async fn keep_ruling(&self, ruling: Uuid) -> Result<(), WebError> {
        Ok(self.session.keep_ruling(ruling).await?)
    }

    /// Keep every open ruling on a board (`ruling.keep_all`); how many it kept.
    pub async fn keep_all_rulings(&self, workspace: Uuid) -> Result<usize, WebError> {
        Ok(self.session.keep_all_rulings(workspace).await?)
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
        let link = self.link.clone();
        let (close, mut closing) = futures::channel::oneshot::channel::<()>();
        spawn(async move {
            let mut buf = vec![0u8; 1 << 15];
            loop {
                // A stream over a dead link never ends by itself, so the
                // link's loss ends it.
                let gone = async {
                    match &link {
                        Some(link) => link.lost().await,
                        None => std::future::pending().await,
                    }
                };
                let read = std::pin::pin!(stream.read(&mut buf));
                let gone = std::pin::pin!(gone);
                match futures::future::select(futures::future::select(read, gone), &mut closing).await {
                    futures::future::Either::Left((futures::future::Either::Left((Ok(n), _)), _)) if n > 0 => {
                        output(&buf[..n])
                    }
                    // The end of the stream, a failed read, the link's loss,
                    // or `close`. The
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
