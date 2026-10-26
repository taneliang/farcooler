//! The runner's protocol over SSH channels: one runner, its terminals, and a
//! pane on one of them.
//!
//! The layout is the native client's (`crates/client/src/session.rs`,
//! `connect_ssh` and `attach_stream`): a control session on one channel for
//! calls — `terminal.list`, `terminal.write`, `terminal.resize` — and, per
//! pane, a second session that carries nothing but `terminal.attach` and the
//! frames that follow it. A pane's bytes never queue behind a call's answer,
//! and a call's answer never waits behind a pane's backlog: SSH gives each
//! channel its own window.
//!
//! Both channels run the same command, `~/.local/bin/<daemon> --stdio`. On a
//! fenced key sshd substitutes the forced command anyway, so what is asked for
//! is what a plain key would run too.
//!
//! Single-threaded by design: the browser has one thread, and the tests run
//! on a `LocalSet`. Shared state is `Rc<RefCell<…>>`, and every task is
//! spawned local.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use bytes::{Bytes, BytesMut};
use farcooler_protocol::v1::{
    ClientHello, ErrorCode, Request, TerminalAttach, TerminalResize, TerminalState, TerminalWrite, WireEnvelope,
    event, request, response, result, terminal_frame, wire_envelope,
};
use farcooler_protocol::{capability, framing};
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use serde::Serialize;
use uuid::Uuid;

use crate::error::WebError;
use crate::ssh::Connection;

/// The name this client gives in its hello, beside its build, so a runner's
/// log says which client and which build connected.
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

/// The command both channels run. The binary carries this build's channel, so
/// a canary page only ever speaks to a canary daemon.
fn daemon_command() -> String {
    format!("~/.local/bin/{} --stdio", farcooler_protocol::CHANNEL.daemon_binary_name())
}

fn envelope(body: wire_envelope::Body) -> WireEnvelope {
    WireEnvelope {
        protocol_version: farcooler_protocol::PROTOCOL_VERSION,
        message_id: Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
        body: Some(body),
    }
}

fn request(method: &str, target: Option<Uuid>, payload: Option<request::Payload>, required: &[&str]) -> Request {
    Request {
        request_id: Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
        method: method.to_string(),
        target_resource_id: target.map(|t| Bytes::copy_from_slice(t.as_bytes())),
        required_capabilities: required.iter().map(|c| c.to_string()).collect(),
        payload: Some(payload.unwrap_or(request::Payload::Empty(farcooler_protocol::v1::Empty {}))),
        ..Default::default()
    }
}

/// A refusal's code, by its proto name.
fn refused(error: &farcooler_protocol::v1::Error) -> WebError {
    let name = ErrorCode::try_from(error.code).map(|c| c.as_str_name()).unwrap_or("ERROR_CODE_UNSPECIFIED");
    WebError::Refused(name.to_string())
}

/// One `--stdio` session on its own channel, up to and including the hello.
struct Wire {
    read: ChannelReadHalf,
    write: ChannelWriteHalf<Msg>,
    buf: BytesMut,
}

impl Wire {
    async fn open(conn: &Connection) -> Result<(Wire, Vec<String>), WebError> {
        let channel = conn.channel_open_session().await.map_err(|e| WebError::Exec(e.to_string()))?;
        channel.exec(true, daemon_command().into_bytes()).await.map_err(|e| WebError::Exec(e.to_string()))?;
        let (read, write) = channel.split();
        let mut wire = Wire { read, write, buf: BytesMut::new() };
        let hello = envelope(wire_envelope::Body::ClientHello(ClientHello {
            supported_protocol_versions: vec![farcooler_protocol::PROTOCOL_VERSION],
            client_name: CLIENT_NAME.into(),
            client_version: farcooler_protocol::BUILD.into(),
        }));
        wire.send(&hello).await?;
        match wire.next().await?.and_then(|e| e.body) {
            Some(wire_envelope::Body::ServerHello(hello)) => Ok((wire, hello.capabilities)),
            Some(_) => Err(WebError::Protocol("the first answer was not a hello".into())),
            None => Err(WebError::Closed),
        }
    }

    async fn send(&mut self, env: &WireEnvelope) -> Result<(), WebError> {
        let frame = framing::encode(env).map_err(|e| WebError::Protocol(e.to_string()))?;
        self.write.data(&frame[..]).await.map_err(|_| WebError::Closed)
    }

    /// The next envelope, or `None` when the channel ends.
    async fn next(&mut self) -> Result<Option<WireEnvelope>, WebError> {
        next_envelope(&mut self.read, &mut self.buf).await
    }
}

async fn next_envelope(read: &mut ChannelReadHalf, buf: &mut BytesMut) -> Result<Option<WireEnvelope>, WebError> {
    loop {
        if let Some(env) = framing::decode(buf).map_err(|e| WebError::Protocol(e.to_string()))? {
            return Ok(Some(env));
        }
        match read.wait().await {
            Some(ChannelMsg::Data { data }) => buf.extend_from_slice(&data),
            Some(ChannelMsg::Eof | ChannelMsg::Close) | None => return Ok(None),
            Some(_) => {}
        }
    }
}

type Answer = Result<Option<result::Value>, WebError>;

/// The callers waiting on an answer, by request id: `Some` while the session is
/// open; taken, and every caller answered `Closed`, when it ends.
type Waiting = Rc<RefCell<Option<HashMap<Bytes, oneshot::Sender<Answer>>>>>;

/// The control session: calls go out in the order they were made, and each
/// answer finds its caller by request id.
struct Control {
    outbox: mpsc::UnboundedSender<Vec<u8>>,
    waiting: Waiting,
}

impl Control {
    fn start(wire: Wire) -> Control {
        let Wire { mut read, write, mut buf } = wire;
        let (outbox, mut frames) = mpsc::unbounded::<Vec<u8>>();
        let waiting: Waiting = Rc::new(RefCell::new(Some(HashMap::new())));

        spawn(async move {
            while let Some(frame) = frames.next().await {
                if write.data(&frame[..]).await.is_err() {
                    break;
                }
            }
        });

        let answers = waiting.clone();
        spawn(async move {
            while let Ok(Some(env)) = next_envelope(&mut read, &mut buf).await {
                let Some(wire_envelope::Body::Response(r)) = env.body else { continue };
                let caller = answers.borrow_mut().as_mut().and_then(|w| w.remove(&r.request_id));
                let Some(caller) = caller else { continue };
                let _ = caller.send(match r.outcome {
                    Some(response::Outcome::Result(value)) => Ok(value.value),
                    Some(response::Outcome::Error(e)) => Err(refused(&e)),
                    None => Err(WebError::Protocol("an answer with no outcome".into())),
                });
            }
            for (_, caller) in answers.borrow_mut().take().into_iter().flatten() {
                let _ = caller.send(Err(WebError::Closed));
            }
        });

        Control { outbox, waiting }
    }

    /// Send a call now, and answer a future for its result.
    ///
    /// Not `async` on purpose: the call is queued before this returns, so two
    /// calls made one after the other reach the runner in that order however
    /// their answers are awaited. Keystrokes depend on it.
    fn call(&self, req: Request) -> impl Future<Output = Answer> + 'static {
        let (tx, rx) = oneshot::channel();
        let queued = (|| {
            let frame = framing::encode(&envelope(wire_envelope::Body::Request(req.clone())))
                .map_err(|e| WebError::Protocol(e.to_string()))?;
            let mut waiting = self.waiting.borrow_mut();
            let waiting = waiting.as_mut().ok_or(WebError::Closed)?;
            waiting.insert(req.request_id.clone(), tx);
            self.outbox.unbounded_send(frame.to_vec()).map_err(|_| WebError::Closed)
        })();
        async move {
            queued?;
            rx.await.unwrap_or(Err(WebError::Closed))
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

/// A connected runner: one SSH connection, its control session, and what its
/// hello said it can do.
pub struct Runner {
    conn: Rc<Connection>,
    control: Rc<Control>,
    capabilities: Vec<String>,
}

impl Runner {
    /// Start the control session on an authenticated connection.
    pub async fn start(conn: Connection) -> Result<Runner, WebError> {
        let (wire, capabilities) = Wire::open(&conn).await?;
        Ok(Runner { conn: Rc::new(conn), control: Rc::new(Control::start(wire)), capabilities })
    }

    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    /// Every terminal this runner has, as `terminal.list` gives them.
    pub async fn terminals(&self) -> Result<Vec<TerminalSummary>, WebError> {
        let answer = self.control.call(request("terminal.list", None, None, &[])).await?;
        let Some(result::Value::TerminalList(list)) = answer else {
            return Err(WebError::Protocol("terminal.list answered something else".into()));
        };
        Ok(list
            .items
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
        if !self.capabilities.iter().any(|c| c == capability::TERMINAL_STREAM) {
            return Err(WebError::Capability(capability::TERMINAL_STREAM.into()));
        }
        let (mut wire, _) = Wire::open(&self.conn).await?;
        let attach = request(
            "terminal.attach",
            Some(terminal),
            Some(request::Payload::TerminalAttach(TerminalAttach { last_acked_sequence: None, resume_token: None })),
            // Named on the envelope too, for the reason `Request.
            // required_capabilities` exists: the hello was read at connect
            // time, and the runner may have changed since.
            &[capability::TERMINAL_STREAM],
        );
        let id = attach.request_id.clone();
        wire.send(&envelope(wire_envelope::Body::Request(attach))).await?;

        let deliver = move |env: WireEnvelope, output: &mut dyn FnMut(&[u8])| {
            if let Some(wire_envelope::Body::Event(ev)) = env.body
                && let Some(event::Payload::TerminalFrame(frame)) = ev.payload
                && frame.terminal_id.as_ref() == terminal.as_bytes()
                && let Some(terminal_frame::Kind::Output(o)) = frame.kind
            {
                output(&o.payload);
            }
        };

        loop {
            let Some(env) = wire.next().await? else { return Err(WebError::Closed) };
            match &env.body {
                Some(wire_envelope::Body::Response(r)) if r.request_id == id => match &r.outcome {
                    Some(response::Outcome::Result(v))
                        if matches!(v.value, Some(result::Value::TerminalAttach(_))) =>
                    {
                        break;
                    }
                    Some(response::Outcome::Error(e)) => return Err(refused(e)),
                    _ => return Err(WebError::Protocol("terminal.attach answered something else".into())),
                },
                _ => deliver(env, &mut output),
            }
        }

        let Wire { mut read, write, mut buf } = wire;
        spawn(async move {
            while let Ok(Some(env)) = next_envelope(&mut read, &mut buf).await {
                deliver(env, &mut output);
            }
            ended();
        });

        Ok(Pane { terminal, control: self.control.clone(), stream: Some(write) })
    }
}

/// One attached terminal. Input and resizes go through the runner's control
/// session; the pane's own channel only ever carries output.
pub struct Pane {
    terminal: Uuid,
    control: Rc<Control>,
    /// Held so the attach channel stays open: dropping it closes the channel's
    /// write side, which the daemon reads as nobody watching any more.
    stream: Option<ChannelWriteHalf<Msg>>,
}

impl Pane {
    /// Type into the terminal. Queued before this returns; see `Control::call`.
    pub fn write(&self, bytes: &[u8]) -> impl Future<Output = Result<(), WebError>> + 'static {
        let payload = request::Payload::TerminalWrite(TerminalWrite { payload: Bytes::copy_from_slice(bytes) });
        let answer = self.control.call(request("terminal.write", Some(self.terminal), Some(payload), &[]));
        async move { answer.await.map(|_| ()) }
    }

    /// Tell the runner this pane's size.
    pub fn resize(&self, columns: u32, rows: u32) -> impl Future<Output = Result<(), WebError>> + 'static {
        let payload = request::Payload::TerminalResize(TerminalResize { columns, rows, view_activity_id: 0 });
        let answer = self.control.call(request("terminal.resize", Some(self.terminal), Some(payload), &[]));
        async move { answer.await.map(|_| ()) }
    }

    /// Stop watching. The terminal keeps running on the runner.
    pub fn close(&mut self) -> impl Future<Output = ()> + 'static {
        let stream = self.stream.take();
        async move {
            if let Some(write) = stream {
                let _ = write.close().await;
            }
        }
    }
}
