//! A fake runner for crates/web's tests, whose frames the browser check's
//! `examples/fake_daemon.rs` shares: a russh server holding a host key, behind
//! which a fake daemon speaks enough of `farcoolerd --stdio` for one pane —
//! hello, `terminal.list`, `terminal.attach` (a replay, then an echo of every
//! write), `terminal.write` and `terminal.resize`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use farcooler_protocol::v1::{
    Empty, Error as WireError, ErrorCode, Event, NeedsYouList, Plan, Workspace, WorkspaceList, Request, Response, ServerHello, Terminal, TerminalAttachResult, TerminalFrame, TerminalList,
    TerminalOutput, TerminalState, WireEnvelope, event, request, response, result, terminal_frame, wire_envelope,
};
use farcooler_protocol::{capability, framing};
use russh::keys::{HashAlg, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId, ChannelMsg};
use farcooler_web::liveness::Clock;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

/// Test-only keys, made with `ssh-keygen -t ed25519` for these tests and used
/// nowhere else.
pub const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACDrHqI6bZA6rDiHw08pI2x1g5u1XpJ1O93BotpMao5D8AAAAJj82X/w/Nl/
8AAAAAtzc2gtZWQyNTUxOQAAACDrHqI6bZA6rDiHw08pI2x1g5u1XpJ1O93BotpMao5D8A
AAAEDV/+21K7r3FLx6zWWAjhNL6YrHtCNQCuee1YujxseqTuseojptkDqsOIfDTykjbHWD
m7VeknU73cGi2kxqjkPwAAAAEHRlc3QtcnVubmVyLWhvc3QBAgMEBQ==
-----END OPENSSH PRIVATE KEY-----
";
pub const BROWSER_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACBSYRFFwXDo87KLOWhcsOoXw6xEVh3ovWpCLnWCRhtoEQAAAJCsBa8ArAWv
AAAAAAtzc2gtZWQyNTUxOQAAACBSYRFFwXDo87KLOWhcsOoXw6xEVh3ovWpCLnWCRhtoEQ
AAAECbNZ//3NB9oUIEn6e2KeWBMT6bWNWtNldI0V7N4r9CDFJhEUXBcOjzsos5aFyw6hfD
rERWHei9akIudYJGG2gRAAAADHRlc3QtYnJvd3NlcgE=
-----END OPENSSH PRIVATE KEY-----
";
/// A key the runner does not know: an interceptor's host key, or a stranger's
/// client key.
pub const OTHER_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACB/LCyoI6KHr/YF8Mtz04YyG4ewww/J3r/myI7EggLnIwAAAIjax4+B2seP
gQAAAAtzc2gtZWQyNTUxOQAAACB/LCyoI6KHr/YF8Mtz04YyG4ewww/J3r/myI7EggLnIw
AAAEBLeLqOORq2dTB1vinZsxe+KfIbU+DxcQVei6HTcvIEt38sLKgjooev9gXwy3PThjIb
h7DDD8nev+bIjsSCAucjAAAABW90aGVy
-----END OPENSSH PRIVATE KEY-----
";
pub const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
pub const WORKSPACE: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_00a1);
pub const TERMINAL: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0001);

/// An Ed25519 key in memory as the browser's signer. The in-process russh
/// server can't verify a passkey's `webauthn-sk` signature, so the tests that
/// are about the pin and the protocol sign with this; the passkey's own
/// signing is tested against a real sshd (`a_passkey_signs_in_to_a_real_sshd.rs`).
pub struct Ed25519InMemory(pub PrivateKey);

impl russh::Signer for Ed25519InMemory {
    type Error = farcooler_web::ssh::SignError;

    fn auth_sign(
        &mut self,
        _key: &russh::keys::agent::AgentIdentity,
        _hash_alg: Option<HashAlg>,
        mut to_sign: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        use russh::keys::signature::Signer as _;
        let signed: Result<russh::keys::ssh_key::Signature, _> = self.0.try_sign(&to_sign);
        async move {
            let raw = signed.map_err(|e| farcooler_web::ssh::SignError::Sign(e.to_string()))?;
            // string("ssh-ed25519") string(sig), as one SSH string (RFC 8709 §6).
            let (alg, sig) = (b"ssh-ed25519", raw.as_bytes());
            to_sign.extend_from_slice(&((8 + alg.len() + sig.len()) as u32).to_be_bytes());
            to_sign.extend_from_slice(&(alg.len() as u32).to_be_bytes());
            to_sign.extend_from_slice(alg);
            to_sign.extend_from_slice(&(sig.len() as u32).to_be_bytes());
            to_sign.extend_from_slice(sig);
            Ok(to_sign)
        }
    }
}

pub fn key(text: &str) -> PrivateKey {
    PrivateKey::from_openssh(text).expect("test key")
}

pub fn line(text: &str) -> String {
    key(text).public_key().to_openssh().unwrap()
}

pub fn fingerprint(text: &str) -> String {
    key(text).public_key().fingerprint(HashAlg::Sha256).to_string()
}

/// What the fake runner saw.
#[derive(Default)]
pub struct Seen {
    /// Authentication attempts of any kind. A refused host key must leave this
    /// at zero: the browser's key never signs for a runner it doesn't trust.
    pub auths: AtomicUsize,
    /// `(channel, method)` for every request, in arrival order.
    pub requests: Mutex<Vec<(ChannelId, String)>>,
    /// What `terminal.write` carried, in arrival order.
    pub typed: Mutex<Vec<u8>>,
    pub resized: Mutex<Option<(u32, u32)>>,
    /// `daemon.version` calls answered: the liveness probe (ov-772).
    pub probes: AtomicUsize,
    /// The `client_name` of every hello, one per channel.
    pub hellos: Mutex<Vec<String>>,
    /// Set, the runner takes each hello and never answers it: a daemon that
    /// started and then went quiet (ov-834).
    pub mute_hello: AtomicBool,
    /// What Needs You lists, when a test sets it: the runner answers
    /// `needs_you.list` with it, and an answer takes its item off.
    pub needs_you: Mutex<Option<NeedsYouList>>,
    /// Every `terminal.agent_answer` as `(terminal, request id, option id,
    /// actor)`, in arrival order.
    pub answers: Mutex<Vec<(Vec<u8>, String, String, String)>>,
    /// Every `task.note` as `(task, kind, body)`.
    pub notes: Mutex<Vec<(Vec<u8>, i32, String)>>,
    /// Set, the runner refuses every answer with this code and `what`: a
    /// device enrolled to look, or an ask someone else got to first.
    pub refuse_answers: Mutex<Option<(ErrorCode, &'static str)>>,
    /// What the plan holds, when a test sets it: the runner answers
    /// `workspace.list` with one board and `plan.get` with this, and an edit
    /// to a theme's Next line changes it.
    pub plan: Mutex<Option<Plan>>,
    /// Every `board_theme.update` as `(theme, next, actor)`.
    pub nexts: Mutex<Vec<(Vec<u8>, Option<String>, String)>>,
    /// Every `ruling.set` as `(ruling, state, actor)`, and each `ruling.keep_all`'s board.
    pub rulings_set: Mutex<Vec<(Vec<u8>, i32, String)>>,
    pub kept_all: Mutex<Vec<Vec<u8>>>,
    /// Set, the runner refuses every plan edit with this code and `what`: a
    /// member's device asking for an owner's call.
    pub refuse_plan: Mutex<Option<(ErrorCode, &'static str)>>,
    /// The attach channel's write half, so a write can echo onto it.
    attached: Mutex<Option<russh::ChannelWriteHalf<Msg>>>,
    /// Told of each write as it arrives, for a caller watching from outside.
    pub typing: std::sync::Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
}

#[derive(Clone)]
struct FakeRunner {
    /// The one client key admitted, or any key when `None`.
    authorized: Option<PublicKey>,
    seen: Arc<Seen>,
    pending: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
}

impl FakeRunner {
    fn admits(&self, key: &PublicKey) -> bool {
        self.authorized.as_ref().is_none_or(|a| a.key_data() == key.key_data())
    }
}

impl server::Handler for FakeRunner {
    type Error = russh::Error;

    async fn auth_publickey_offered(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        self.seen.auths.fetch_add(1, Ordering::SeqCst);
        Ok(if self.admits(key) { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_publickey(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        self.seen.auths.fetch_add(1, Ordering::SeqCst);
        Ok(if self.admits(key) { Auth::Accept } else { Auth::reject() })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.pending.lock().await.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(&mut self, id: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        assert!(String::from_utf8_lossy(data).ends_with("--stdio"), "both channels run the daemon's --stdio");
        session.channel_success(id)?;
        if let Some(channel) = self.pending.lock().await.remove(&id) {
            tokio::spawn(fake_daemon(channel, self.seen.clone()));
        }
        Ok(())
    }
}

pub fn envelope(body: wire_envelope::Body) -> Vec<u8> {
    let env = WireEnvelope {
        protocol_version: farcooler_protocol::PROTOCOL_VERSION,
        message_id: Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
        body: Some(body),
    };
    framing::encode(&env).unwrap().to_vec()
}

pub fn answer(req: &Request, value: result::Value) -> Vec<u8> {
    envelope(wire_envelope::Body::Response(Response {
        request_id: req.request_id.clone(),
        outcome: Some(response::Outcome::Result(farcooler_protocol::v1::Result { value: Some(value) })),
    }))
}

/// The runner refusing a request, as the daemon's frame does.
pub fn refusal(req: &Request, code: ErrorCode, what: &str) -> Vec<u8> {
    envelope(wire_envelope::Body::Response(Response {
        request_id: req.request_id.clone(),
        outcome: Some(response::Outcome::Error(WireError {
            code: code as i32,
            retryable: false,
            message: String::new(),
            what: what.to_string(),
        })),
    }))
}

pub fn output(bytes: &[u8]) -> Vec<u8> {
    output_for(TERMINAL, bytes)
}

/// Output as terminal `id` prints it.
pub fn output_for(id: Uuid, bytes: &[u8]) -> Vec<u8> {
    envelope(wire_envelope::Body::Event(Event {
        payload: Some(event::Payload::TerminalFrame(TerminalFrame {
            terminal_id: Bytes::copy_from_slice(id.as_bytes()),
            kind: Some(terminal_frame::Kind::Output(TerminalOutput {
                payload: Bytes::copy_from_slice(bytes),
                ..Default::default()
            })),
            ..Default::default()
        })),
        ..Default::default()
    }))
}

/// Enough of `farcoolerd --stdio` for one pane.
async fn fake_daemon(channel: Channel<Msg>, seen: Arc<Seen>) {
    let id = channel.id();
    let (mut read, write) = channel.split();
    let mut write = Some(write);
    let mut buf = BytesMut::new();
    loop {
        let env = loop {
            if let Some(env) = framing::decode(&mut buf).unwrap() {
                break Some(env);
            }
            match read.wait().await {
                Some(ChannelMsg::Data { data }) => buf.extend_from_slice(&data),
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => break None,
                Some(_) => {}
            }
        };
        let Some(env) = env else { return };
        let out = match env.body {
            Some(wire_envelope::Body::ClientHello(hello)) => {
                seen.hellos.lock().await.push(hello.client_name);
                if seen.mute_hello.load(Ordering::SeqCst) {
                    continue;
                }
                envelope(wire_envelope::Body::ServerHello(ServerHello {
                selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
                capabilities: vec![
                    capability::TERMINAL_STREAM.to_string(),
                    capability::NEEDS_YOU.to_string(),
                    capability::TASKS.to_string(),
                    capability::WORKSTREAMS.to_string(),
                    capability::BOARD_PLAN.to_string(),
                    capability::BOARD_RULING_ACTIONS.to_string(),
                ],
                ..Default::default()
            }))
            }
            Some(wire_envelope::Body::Request(req)) => {
                seen.requests.lock().await.push((id, req.method.clone()));
                match (req.method.as_str(), req.payload.clone()) {
                    ("terminal.list", _) => answer(
                        &req,
                        result::Value::TerminalList(TerminalList {
                            items: vec![Terminal {
                                id: Bytes::copy_from_slice(TERMINAL.as_bytes()),
                                title: "shell".into(),
                                state: TerminalState::Running as i32,
                                columns: 80,
                                rows: 24,
                                ..Default::default()
                            }],
                            ..Default::default()
                        }),
                    ),
                    ("terminal.attach", _) => {
                        let w = write.take().expect("one attach per channel");
                        w.data(&answer(&req, result::Value::TerminalAttach(TerminalAttachResult::default()))[..])
                            .await
                            .unwrap();
                        w.data(&output(b"replay$ ")[..]).await.unwrap();
                        *seen.attached.lock().await = Some(w);
                        continue;
                    }
                    ("terminal.write", Some(request::Payload::TerminalWrite(w))) => {
                        seen.typed.lock().await.extend_from_slice(&w.payload);
                        if let Some(tx) = seen.typing.lock().unwrap().as_ref() {
                            let _ = tx.send(w.payload.to_vec());
                        }
                        if let Some(attached) = seen.attached.lock().await.as_ref() {
                            attached.data(&output(&w.payload)[..]).await.unwrap();
                        }
                        answer(&req, result::Value::Empty(Empty {}))
                    }
                    ("daemon.version", _) => {
                        seen.probes.fetch_add(1, Ordering::SeqCst);
                        answer(
                            &req,
                            result::Value::DaemonVersion(farcooler_protocol::v1::DaemonVersion {
                                daemon_version: "test".into(),
                                ..Default::default()
                            }),
                        )
                    }
                    ("terminal.resize", Some(request::Payload::TerminalResize(r))) => {
                        *seen.resized.lock().await = Some((r.columns, r.rows));
                        answer(&req, result::Value::Empty(Empty {}))
                    }
                    ("needs_you.list", _) => {
                        let list = seen.needs_you.lock().await.clone().unwrap_or_default();
                        answer(&req, result::Value::NeedsYouList(list))
                    }
                    ("terminal.agent_answer", Some(request::Payload::AgentAnswer(a))) => {
                        seen.answers.lock().await.push((a.terminal_id.to_vec(), a.request_id.clone(), a.option_id, a.actor));
                        if let Some((code, what)) = *seen.refuse_answers.lock().await {
                            refusal(&req, code, what)
                        } else {
                            if let Some(list) = seen.needs_you.lock().await.as_mut() {
                                list.items.retain(|i| i.ask_id.as_deref() != Some(a.request_id.as_str()));
                            }
                            answer(&req, result::Value::Terminal(Terminal { id: a.terminal_id, ..Default::default() }))
                        }
                    }
                    ("task.note", Some(request::Payload::TaskNoteAppend(n))) => {
                        seen.notes.lock().await.push((n.task_id.to_vec(), n.kind, n.body.clone()));
                        if let Some((code, what)) = *seen.refuse_answers.lock().await {
                            refusal(&req, code, what)
                        } else {
                            if let Some(list) = seen.needs_you.lock().await.as_mut() {
                                list.items.retain(|i| i.task.as_ref().is_none_or(|t| t.id != n.task_id));
                            }
                            answer(&req, result::Value::TaskNote(Default::default()))
                        }
                    }
                    ("workspace.list", _) => answer(
                        &req,
                        result::Value::WorkspaceList(WorkspaceList {
                            items: vec![Workspace { id: Bytes::copy_from_slice(WORKSPACE.as_bytes()), name: "Main".into(), is_main: true, ..Default::default() }],
                        }),
                    ),
                    ("plan.get", _) => answer(&req, result::Value::Plan(seen.plan.lock().await.clone().unwrap_or_default())),
                    ("board_theme.update", Some(request::Payload::BoardThemeUpdate(u))) => {
                        seen.nexts.lock().await.push((u.theme_id.to_vec(), u.next.clone(), u.actor.clone()));
                        if let Some((code, what)) = *seen.refuse_plan.lock().await {
                            refusal(&req, code, what)
                        } else {
                            if let (Some(plan), Some(next)) = (seen.plan.lock().await.as_mut(), u.next) {
                                for view in &mut plan.themes {
                                    if let Some(theme) = view.theme.as_mut().filter(|t| t.id == u.theme_id) {
                                        theme.next = next.clone();
                                    }
                                }
                            }
                            answer(&req, result::Value::BoardThemeView(Default::default()))
                        }
                    }
                    ("ruling.set", Some(request::Payload::RulingSet(r))) => {
                        seen.rulings_set.lock().await.push((r.ruling_id.to_vec(), r.state, r.actor));
                        match *seen.refuse_plan.lock().await {
                            Some((code, what)) => refusal(&req, code, what),
                            None => answer(&req, result::Value::BoardRuling(Default::default())),
                        }
                    }
                    ("ruling.keep_all", Some(request::Payload::RulingKeepAll(k))) => {
                        seen.kept_all.lock().await.push(k.workspace_id.to_vec());
                        match *seen.refuse_plan.lock().await {
                            Some((code, what)) => refusal(&req, code, what),
                            None => answer(&req, result::Value::RulingsKept(Default::default())),
                        }
                    }
                    (other, _) => panic!("the fake runner doesn't know {other}"),
                }
            }
            _ => continue,
        };
        if let Some(w) = write.as_ref() {
            w.data(&out[..]).await.unwrap();
        }
    }
}

/// Serve one SSH connection on `stream` as a runner holding `host_key`,
/// admitting `authorized` (any key when `None`).
pub fn serve<S>(stream: S, host_key: &str, authorized: Option<PublicKey>, seen: Arc<Seen>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let handler = FakeRunner { authorized, seen, pending: Default::default() };
    let config = Arc::new(server::Config {
        keys: vec![key(host_key)],
        auth_rejection_time: std::time::Duration::ZERO,
        auth_rejection_time_initial: Some(std::time::Duration::ZERO),
        ..Default::default()
    });
    tokio::spawn(async move {
        if let Ok(session) = server::run_stream(config, stream, handler).await {
            let _ = session.await;
        }
    });
}

// ---- time, by hand ----

#[derive(Default)]
pub struct ManualClock {
    pub now: AtomicU64,
    sleepers: StdMutex<Vec<(u64, tokio::sync::oneshot::Sender<()>)>>,
}

impl ManualClock {
    pub fn advance(&self, ms: u64) {
        let now = self.now.fetch_add(ms, Ordering::SeqCst) + ms;
        let mut sleepers = self.sleepers.lock().unwrap();
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut *sleepers).into_iter().partition(|(t, _)| *t <= now);
        *sleepers = later;
        for (_, wake) in due {
            let _ = wake.send(());
        }
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
    fn sleep(&self, ms: u64) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let (wake, woken) = tokio::sync::oneshot::channel();
        self.sleepers.lock().unwrap().push((self.now_ms() + ms, wake));
        Box::pin(async move {
            let _ = woken.await;
        })
    }
}

// ---- a tunnel that can die ----

#[derive(Default)]
pub struct Tunnel {
    pub dead: AtomicBool,
    /// Writes swallowed since it died: what the page sent into the dark.
    pub lost_writes: AtomicUsize,
}

pub struct Valve {
    pub inner: DuplexStream,
    pub tunnel: Arc<Tunnel>,
}

impl AsyncRead for Valve {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        if self.tunnel.dead.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Valve {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        if self.tunnel.dead.load(Ordering::SeqCst) {
            self.tunnel.lost_writes.fetch_add(1, Ordering::SeqCst);
            return Poll::Ready(Ok(buf.len()));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

