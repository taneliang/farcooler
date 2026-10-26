//! The browser's client against a runner, in one process: an in-process russh
//! server holding a host key and an `authorized_keys` of one, behind which a
//! fake daemon speaks enough of `--stdio` to list, attach, write and resize.
//!
//! What it proves, each with a test that goes red without the code it names:
//!
//! - A reply whose host key was altered is refused with `host_key_changed`
//!   before any authentication (ov-764's acceptance line 3). Remove the pin in
//!   `ssh::Pinned::check_server_key` and the first test fails.
//! - A runner entry with no pin is never dialed at all.
//! - The pinned runner connects with the browser's signer, lists its
//!   terminals, and a pane's input goes on the control channel while its
//!   output arrives on a channel of its own (`crates/client/src/session.rs`'s
//!   layout).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::{Bytes, BytesMut};
use farcooler_ceremony::{Reach, RunnerEntry};
use farcooler_protocol::v1::{
    Empty, Event, Request, Response, ServerHello, Terminal, TerminalAttachResult, TerminalFrame, TerminalList,
    TerminalOutput, TerminalState, WireEnvelope, event, request, response, result, terminal_frame, wire_envelope,
};
use farcooler_protocol::{capability, framing};
use farcooler_web::enroll;
use farcooler_web::error::WebError;
use farcooler_web::ssh::{self, Ed25519Signer, RawSign};
use farcooler_web::wire::Runner;
use russh::keys::signature::Signer as _;
use russh::keys::{HashAlg, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId, ChannelMsg};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Test-only keys, made with `ssh-keygen -t ed25519` for this file and used
/// nowhere else.
const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACDrHqI6bZA6rDiHw08pI2x1g5u1XpJ1O93BotpMao5D8AAAAJj82X/w/Nl/
8AAAAAtzc2gtZWQyNTUxOQAAACDrHqI6bZA6rDiHw08pI2x1g5u1XpJ1O93BotpMao5D8A
AAAEDV/+21K7r3FLx6zWWAjhNL6YrHtCNQCuee1YujxseqTuseojptkDqsOIfDTykjbHWD
m7VeknU73cGi2kxqjkPwAAAAEHRlc3QtcnVubmVyLWhvc3QBAgMEBQ==
-----END OPENSSH PRIVATE KEY-----
";
const BROWSER_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACBSYRFFwXDo87KLOWhcsOoXw6xEVh3ovWpCLnWCRhtoEQAAAJCsBa8ArAWv
AAAAAAtzc2gtZWQyNTUxOQAAACBSYRFFwXDo87KLOWhcsOoXw6xEVh3ovWpCLnWCRhtoEQ
AAAECbNZ//3NB9oUIEn6e2KeWBMT6bWNWtNldI0V7N4r9CDFJhEUXBcOjzsos5aFyw6hfD
rERWHei9akIudYJGG2gRAAAADHRlc3QtYnJvd3NlcgE=
-----END OPENSSH PRIVATE KEY-----
";
/// A key the runner does not know: an interceptor's host key, or a stranger's
/// client key.
const OTHER_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACB/LCyoI6KHr/YF8Mtz04YyG4ewww/J3r/myI7EggLnIwAAAIjax4+B2seP
gQAAAAtzc2gtZWQyNTUxOQAAACB/LCyoI6KHr/YF8Mtz04YyG4ewww/J3r/myI7EggLnIw
AAAEBLeLqOORq2dTB1vinZsxe+KfIbU+DxcQVei6HTcvIEt38sLKgjooev9gXwy3PThjIb
h7DDD8nev+bIjsSCAucjAAAABW90aGVy
-----END OPENSSH PRIVATE KEY-----
";
const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const TERMINAL: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0001);

fn key(text: &str) -> PrivateKey {
    PrivateKey::from_openssh(text).expect("test key")
}

fn line(text: &str) -> String {
    key(text).public_key().to_openssh().unwrap()
}

fn fingerprint(text: &str) -> String {
    key(text).public_key().fingerprint(HashAlg::Sha256).to_string()
}

/// The browser's signer, with the key in memory where a browser has WebCrypto.
struct InMemory(PrivateKey);

impl RawSign for InMemory {
    fn sign(&mut self, data: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send>> {
        let signed: Result<russh::keys::ssh_key::Signature, _> = self.0.try_sign(&data);
        Box::pin(async move { signed.map(|s| s.as_bytes().to_vec()).map_err(|e| e.to_string()) })
    }
}

/// What the fake runner saw.
#[derive(Default)]
struct Seen {
    /// Authentication attempts of any kind. A refused host key must leave this
    /// at zero: the browser's key never signs for a runner it doesn't trust.
    auths: AtomicUsize,
    /// `(channel, method)` for every request, in arrival order.
    requests: Mutex<Vec<(ChannelId, String)>>,
    /// What `terminal.write` carried, in arrival order.
    typed: Mutex<Vec<u8>>,
    resized: Mutex<Option<(u32, u32)>>,
    /// The attach channel's write half, so a write can echo onto it.
    attached: Mutex<Option<russh::ChannelWriteHalf<Msg>>>,
}

#[derive(Clone)]
struct FakeRunner {
    authorized: PublicKey,
    seen: Arc<Seen>,
    pending: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
}

impl server::Handler for FakeRunner {
    type Error = russh::Error;

    async fn auth_publickey_offered(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        self.seen.auths.fetch_add(1, Ordering::SeqCst);
        Ok(if key.key_data() == self.authorized.key_data() { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_publickey(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        self.seen.auths.fetch_add(1, Ordering::SeqCst);
        Ok(if key.key_data() == self.authorized.key_data() { Auth::Accept } else { Auth::reject() })
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

fn envelope(body: wire_envelope::Body) -> Vec<u8> {
    let env = WireEnvelope {
        protocol_version: farcooler_protocol::PROTOCOL_VERSION,
        message_id: Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
        body: Some(body),
    };
    framing::encode(&env).unwrap().to_vec()
}

fn answer(req: &Request, value: result::Value) -> Vec<u8> {
    envelope(wire_envelope::Body::Response(Response {
        request_id: req.request_id.clone(),
        outcome: Some(response::Outcome::Result(farcooler_protocol::v1::Result { value: Some(value) })),
    }))
}

fn output(bytes: &[u8]) -> Vec<u8> {
    envelope(wire_envelope::Body::Event(Event {
        payload: Some(event::Payload::TerminalFrame(TerminalFrame {
            terminal_id: Bytes::copy_from_slice(TERMINAL.as_bytes()),
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
            Some(wire_envelope::Body::ClientHello(_)) => envelope(wire_envelope::Body::ServerHello(ServerHello {
                selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
                capabilities: vec![capability::TERMINAL_STREAM.to_string()],
                ..Default::default()
            })),
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
                        if let Some(attached) = seen.attached.lock().await.as_ref() {
                            attached.data(&output(&w.payload)[..]).await.unwrap();
                        }
                        answer(&req, result::Value::Empty(Empty {}))
                    }
                    ("terminal.resize", Some(request::Payload::TerminalResize(r))) => {
                        *seen.resized.lock().await = Some((r.columns, r.rows));
                        answer(&req, result::Value::Empty(Empty {}))
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

/// A runner holding `host_key`, admitting only the browser's key, on one end
/// of an in-memory pipe. The other end is what a Tailcat connection would be.
fn runner(host_key: &str) -> (tokio::io::DuplexStream, Arc<Seen>) {
    let (browser_side, runner_side) = tokio::io::duplex(1 << 16);
    let seen = Arc::new(Seen::default());
    let handler = FakeRunner {
        authorized: key(BROWSER_KEY).public_key().clone(),
        seen: seen.clone(),
        pending: Default::default(),
    };
    let config = Arc::new(server::Config {
        keys: vec![key(host_key)],
        auth_rejection_time: std::time::Duration::ZERO,
        auth_rejection_time_initial: Some(std::time::Duration::ZERO),
        ..Default::default()
    });
    tokio::spawn(async move {
        if let Ok(session) = server::run_stream(config, runner_side, handler).await {
            let _ = session.await;
        }
    });
    (browser_side, seen)
}

/// The reply a phone would show for this browser's offer, granting one
/// tunneled runner whose host key prints as `host_key`, taken through the
/// browser's own acceptance.
fn granted_runner(host_key: &str) -> enroll::Granted {
    let offer = enroll::offer("Chrome on Linux", "acct-owner", &line(BROWSER_KEY), NODE_KEY);
    let entry = RunnerEntry {
        id: "01990000-0000-7000-8000-00000000a001".into(),
        label: "studio".into(),
        alias: "studio".into(),
        user: "owner".into(),
        host_key: host_key.to_string(),
        reach: Reach::Tailcat { token: "tc-token".into(), client_key: String::new() },
        alternative: None,
        pending: false,
    };
    let reply = farcooler_ceremony::encode_manifest(&farcooler_ceremony::manifest(&offer, vec![entry]));
    let manifest = enroll::accept(&reply, &offer, false).expect("the browser accepts its own reply");
    enroll::granted(&manifest).remove(0)
}

async fn dial(stream: tokio::io::DuplexStream, granted: &enroll::Granted, client_key: &str) -> Result<Runner, WebError> {
    let mut signer = Ed25519Signer(InMemory(key(client_key)));
    let conn = ssh::connect(stream, &granted.host_key, &granted.user, &line(client_key), &mut signer).await?;
    Runner::start(conn).await
}

/// ov-764 acceptance 3: a manifest whose host key was altered is refused with
/// the host-key-changed word, and never connected.
///
/// The reply is the real shape, accepted by the real `accept_manifest`; only
/// its `host_key` names a different key than the one the runner holds — what
/// an interceptor presenting its own key looks like from here.
#[tokio::test]
async fn a_reply_whose_host_key_was_altered_is_refused_and_never_connects() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let altered = granted_runner(&fingerprint(OTHER_KEY));
            assert_ne!(altered.host_key, fingerprint(HOST_KEY));
            let (stream, seen) = runner(HOST_KEY);

            let refused = dial(stream, &altered, BROWSER_KEY).await.err().expect("an altered pin must not connect");

            assert_eq!(refused.word(), "host_key_changed");
            assert_eq!(
                refused,
                WebError::HostKeyChanged { expected: fingerprint(OTHER_KEY), actual: fingerprint(HOST_KEY) }
            );
            assert_eq!(seen.auths.load(Ordering::SeqCst), 0, "the browser's key was offered to an unpinned runner");
            assert!(seen.requests.lock().await.is_empty());
        })
        .await;
}

/// A runner with nothing pinned is not trusted on first use: it is never
/// dialed, so there is no handshake to accept anything in.
#[tokio::test]
async fn a_runner_with_no_pinned_host_key_is_never_dialed() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let unpinned = granted_runner("");
            let (stream, seen) = runner(HOST_KEY);
            let refused = dial(stream, &unpinned, BROWSER_KEY).await.err().expect("no pin, no connection");
            assert_eq!(refused.word(), "host_key_unknown");
            assert_eq!(seen.auths.load(Ordering::SeqCst), 0);
        })
        .await;
}

/// The pinned runner, but a key it never enrolled: refused by the runner, and
/// named as such rather than as a network failure.
#[tokio::test]
async fn a_key_the_runner_never_enrolled_is_rejected_by_name() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (stream, _) = runner(HOST_KEY);
            let refused =
                dial(stream, &granted_runner(&fingerprint(HOST_KEY)), OTHER_KEY).await.err().expect("not enrolled");
            assert_eq!(refused.word(), "key_rejected");
        })
        .await;
}

/// The whole slice-1 path past the tunnel: the pinned runner connects with the
/// browser's signer, lists one terminal, and a pane types on the control
/// channel and reads on its own.
#[tokio::test]
async fn the_pinned_runner_lists_attaches_and_takes_keystrokes_on_the_control_channel() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (stream, seen) = runner(HOST_KEY);
            let runner = dial(stream, &granted_runner(&fingerprint(HOST_KEY)), BROWSER_KEY).await.expect("connects");

            let terminals = runner.terminals().await.expect("terminal.list");
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].id, TERMINAL.to_string());
            assert_eq!(terminals[0].state, "running");

            let (shown_tx, mut shown) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            let pane = runner
                .attach(TERMINAL, move |bytes| { let _ = shown_tx.send(bytes.to_vec()); }, || {})
                .await
                .expect("terminal.attach");
            assert_eq!(shown.recv().await.unwrap(), b"replay$ ");

            // Awaited last to first: the order the calls were MADE is the order
            // the runner gets them, whenever the page gets round to awaiting.
            let typing = [pane.write(b"e"), pane.write(b"c"), pane.write(b"h"), pane.write(b"o")];
            for typed in typing.into_iter().rev() {
                typed.await.expect("terminal.write");
            }
            pane.resize(132, 40).await.expect("terminal.resize");

            let mut echoed = Vec::new();
            while echoed.len() < 4 {
                echoed.extend(shown.recv().await.unwrap());
            }
            assert_eq!(echoed, b"echo");
            assert_eq!(&*seen.typed.lock().await, b"echo");
            assert_eq!(*seen.resized.lock().await, Some((132, 40)));

            let requests = seen.requests.lock().await.clone();
            let control = requests.iter().find(|(_, m)| m == "terminal.list").unwrap().0;
            let stream = requests.iter().find(|(_, m)| m == "terminal.attach").unwrap().0;
            assert_ne!(control, stream, "a pane's bytes have a channel of their own");
            for (channel, method) in &requests {
                if method == "terminal.write" || method == "terminal.resize" {
                    assert_eq!(*channel, control, "{method} goes on the control channel");
                }
            }
        })
        .await;
}

/// A browser dials runners through the tunnel only, and a runner the phone
/// could not write its key to is listed but never dialed.
#[test]
fn only_a_written_tunneled_runner_is_dialable_from_a_browser() {
    let offer = enroll::offer("Chrome on Linux", "acct-owner", &line(BROWSER_KEY), NODE_KEY);
    let entry = |id: &str, reach: Reach, pending: bool| RunnerEntry {
        id: id.into(),
        label: id.into(),
        alias: id.into(),
        user: "owner".into(),
        host_key: fingerprint(HOST_KEY),
        reach,
        alternative: None,
        pending,
    };
    let tunnel = || Reach::Tailcat { token: "tc-token".into(), client_key: String::new() };
    let reply = farcooler_ceremony::encode_manifest(&farcooler_ceremony::manifest(
        &offer,
        vec![
            entry("tunneled", tunnel(), false),
            entry("pending", tunnel(), true),
            entry("address", Reach::Direct { host: "box.local".into(), port: 22 }, false),
        ],
    ));
    let granted = enroll::granted(&enroll::accept(&reply, &offer, false).unwrap());
    let why: Vec<(&str, &str, &str)> =
        granted.iter().map(|g| (g.id.as_str(), g.token.as_str(), g.why.as_str())).collect();
    assert_eq!(
        why,
        vec![("tunneled", "tc-token", ""), ("pending", "", "pending"), ("address", "", "address_only")]
    );
}

/// The signature a russh server verifies is `string("ssh-ed25519")
/// string(sig)` after the signed data; anything but 64 raw bytes is refused
/// rather than sent.
#[test]
fn a_raw_signature_is_framed_as_ssh_ed25519_or_refused() {
    let framed = ssh::ed25519_signature_blob(b"data".to_vec(), &[7u8; 64]).unwrap();
    let mut want = b"data".to_vec();
    want.extend_from_slice(&83u32.to_be_bytes());
    want.extend_from_slice(&11u32.to_be_bytes());
    want.extend_from_slice(b"ssh-ed25519");
    want.extend_from_slice(&64u32.to_be_bytes());
    want.extend_from_slice(&[7u8; 64]);
    assert_eq!(framed, want);
    assert!(ssh::ed25519_signature_blob(b"data".to_vec(), &[7u8; 63]).is_err());
}

/// WebCrypto's 32 raw public-key bytes become the same line ssh-keygen prints.
#[test]
fn a_raw_public_key_becomes_its_openssh_line() {
    let public = key(BROWSER_KEY).public_key().clone();
    let raw = public.key_data().ed25519().unwrap().0;
    assert_eq!(ssh::ed25519_public_line(&raw, "test-browser").unwrap(), line(BROWSER_KEY));
}
