//! What the page calls: the wasm-bindgen surface, and nothing else.
//!
//! Every failure reaches the page as an `Error` whose `message` is one stable
//! word ([`WebError::word`], or the ceremony's own `CeremonyError::code`) and
//! whose `detail` is this crate's description, for the console only. The page
//! owns the sentence a person reads.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use js_sys::{Function, Promise, Reflect, Uint8Array};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use uuid::Uuid;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use farcooler_ceremony::Liveness;
use farcooler_client::terminal_input::{self, BufferedKeys, RawKey, WriterInput};

use crate::enroll;
use crate::error::WebError;
use crate::handshake;
use crate::liveness::{Link, PageClock};
use crate::passkey::{self, Assertion, Authenticate, PasskeySigner};
use crate::register;
use crate::ssh::SignError;
use crate::wire::{self, Pane, Runner, spawn};

fn failure(word: &str, detail: &str) -> JsValue {
    let error = js_sys::Error::new(word);
    let _ = Reflect::set(&error, &"detail".into(), &detail.into());
    error.into()
}

fn web_failure(e: WebError) -> JsValue {
    let error = failure(e.word(), &e.to_string());
    // A refusal also carries what the runner said it was, for the page to word.
    if let Some((code, what)) = e.refusal() {
        let _ = Reflect::set(&error, &"code".into(), &code.into());
        let _ = Reflect::set(&error, &"what".into(), &what.into());
    }
    error
}

fn writer_input(input: &str) -> Result<WriterInput, JsValue> {
    match input {
        "claiming" => Ok(WriterInput::Claiming),
        "writable" => Ok(WriterInput::Writable),
        "viewing" => Ok(WriterInput::Viewing),
        _ => Err(failure("invalid_input", "unknown writer input state")),
    }
}

#[wasm_bindgen(js_name = composerInput)]
pub fn composer_input(input: &str, sending: bool, connected: bool, has_draft: bool) -> Result<String, JsValue> {
    let view = terminal_input::composer(writer_input(input)?, sending, connected, has_draft);
    serde_json::to_string(&view).map_err(|e| failure("json", &e.to_string()))
}

#[wasm_bindgen(js_name = terminalInputChange)]
pub fn terminal_input_change(previous: &str, next: &str, buffered: bool) -> Result<String, JsValue> {
    let action = match terminal_input::writer_changed(writer_input(previous)?, writer_input(next)?, buffered) {
        BufferedKeys::Keep => "keep",
        BufferedKeys::Wait => "wait",
        BufferedKeys::Flush => "flush",
        BufferedKeys::Clear => "clear",
        BufferedKeys::Refused => "writer_keys_refused",
    };
    Ok(action.into())
}

#[wasm_bindgen(js_name = terminalKeyAction)]
pub fn terminal_key_action(input: &str, attaching: bool, buffered_bytes: usize, key_bytes: usize) -> Result<String, JsValue> {
    let action = match terminal_input::raw_key(writer_input(input)?, attaching, buffered_bytes, key_bytes) {
        RawKey::Ignore => "ignore",
        RawKey::Buffer => "buffer",
        RawKey::BufferFull => "still_attaching",
        RawKey::Write => "write",
    };
    Ok(action.into())
}

/// A browser future is not `Send`, and russh's signer must return one that is.
/// The page has one thread, so nothing can ever move it to another; this says
/// so to the compiler and nothing more.
struct AssertSend<F>(F);
// SAFETY: wasm32-unknown-unknown without threads has exactly one thread.
unsafe impl<F> Send for AssertSend<F> {}
impl<F: Future> Future for AssertSend<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        // SAFETY: a projection to the only field, which is never moved out.
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx)
    }
}

/// `assert(challenge: Uint8Array) → Promise<{authenticatorData,
/// clientDataJSON, signature}>`: `navigator.credentials.get` with this
/// browser's passkey, each field an `ArrayBuffer` as the browser gave it. It
/// rejects with an Error whose message is `declined` when the person dismissed
/// the prompt or the browser wouldn't show one.
struct JsAuthenticator(Function);

impl Authenticate for JsAuthenticator {
    fn assert(&mut self, challenge: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<Assertion, SignError>> + Send>> {
        let assert = self.0.clone();
        Box::pin(AssertSend(async move {
            let promise = assert
                .call1(&JsValue::NULL, &Uint8Array::from(&challenge[..]))
                .map_err(|_| SignError::Sign("the passkey call threw".into()))?;
            let answer = JsFuture::from(Promise::from(promise)).await.map_err(|e| {
                match e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message())) {
                    Some(word) if word == "declined" => SignError::Declined,
                    _ => SignError::Sign("the passkey refused".into()),
                }
            })?;
            let field = |name: &str| -> Result<Vec<u8>, SignError> {
                let value = Reflect::get(&answer, &name.into()).map_err(|_| SignError::Sign(format!("no {name}")))?;
                if !value.is_instance_of::<js_sys::ArrayBuffer>() && !value.is_instance_of::<Uint8Array>() {
                    return Err(SignError::Sign(format!("{name} is not bytes")));
                }
                Ok(Uint8Array::new(&value).to_vec())
            };
            Ok(Assertion {
                authenticator_data: field("authenticatorData")?,
                client_data_json: field("clientDataJSON")?,
                signature: field("signature")?,
            })
        }))
    }
}

/// The SSH stream over a dialed tunnel connection the Go module handed the
/// page: `{read() → Promise<Uint8Array | null>, write(bytes), close()}`.
///
/// Two local tasks copy between it and one end of an in-memory pipe; russh
/// reads and writes the other end. Either side ending ends both.
fn bridge(conn: JsValue) -> Result<DuplexStream, JsValue> {
    let method = |name: &str| -> Result<Function, JsValue> {
        Reflect::get(&conn, &name.into())?.dyn_into::<Function>().map_err(|_| failure("io", "not a connection"))
    };
    let (read, write, close) = (method("read")?, method("write")?, method("close")?);
    let (ssh_side, page_side) = tokio::io::duplex(1 << 16);
    let (mut from_ssh, mut to_ssh) = tokio::io::split(page_side);

    let (reader, writer) = (conn.clone(), conn.clone());
    let closer = close.clone();
    spawn(async move {
        loop {
            let Ok(promise) = read.call0(&reader) else { break };
            let Ok(bytes) = JsFuture::from(Promise::from(promise)).await else { break };
            if bytes.is_null() || bytes.is_undefined() {
                break;
            }
            if to_ssh.write_all(&Uint8Array::new(&bytes).to_vec()).await.is_err() {
                break;
            }
        }
        let _ = to_ssh.shutdown().await;
        let _ = closer.call0(&reader);
    });
    spawn(async move {
        let mut buf = vec![0u8; 1 << 15];
        loop {
            match from_ssh.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if write.call1(&writer, &Uint8Array::from(&buf[..n])).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = close.call0(&writer);
    });
    Ok(ssh_side)
}

/// A new ceremony's reply key, as its secret half: the page keeps it for the
/// life of the code on screen (in `sessionStorage`, never IndexedDB) and gives
/// it back to [`make_offer`] and [`open_reply`].
#[wasm_bindgen(js_name = newReplySecret)]
pub fn new_reply_secret() -> String {
    enroll::new_reply_secret()
}

/// The offer this browser shows, as the JSON its QR code carries. It names the
/// public half of `reply_secret`, so a phone can post a sealed reply.
#[wasm_bindgen(js_name = makeOffer)]
pub fn make_offer(name: &str, account: &str, key_a: &str, node_key: &str, reply_secret: &str) -> Result<String, JsValue> {
    let offer = enroll::offer(name, account, key_a, node_key, reply_secret).map_err(|e| failure(e.code(), &e.to_string()))?;
    Ok(farcooler_ceremony::encode_offer(&offer))
}

/// Open the offer's own account's reply in the relay's answer to a read
/// (`{replies:[{sealed, posted_by}]}`), or the earliest teammate's when there is
/// none, with this ceremony's key, and judge it
/// as every device does. Answers a JSON list of `{code, granted_by, runners}`,
/// best first (the account's own reply alone, else every teammate's that passes), or
/// refuses with the ceremony's word (`unopened` for a reply that doesn't open
/// with this key, `stale` past the window).
///
/// The code is this browser's own, from its own open. Nothing is kept: the
/// page holds the answer until the person says the code matches their phone's.
/// "Now" is read here, not passed in, so the page cannot leave the window out.
#[wasm_bindgen(js_name = openReply)]
pub fn open_reply(
    relay_answer: &str,
    offer: &str,
    reply_secret: &str,
    already_taken: bool,
    shown_at_ms: f64,
) -> Result<String, JsValue> {
    let offer = farcooler_ceremony::decode_offer(offer).map_err(|e| failure(e.code(), &e.to_string()))?;
    let opened = enroll::open_all(relay_answer, &offer, reply_secret, already_taken, shown_at_ms, js_sys::Date::now())
        .map_err(|e| failure(e.code(), &e.to_string()))?;
    serde_json::to_string(&opened).map_err(|e| failure("malformed", &e.to_string()))
}

/// Open a reply transferred directly from the runner's CLI.
#[wasm_bindgen(js_name = openCliReply)]
pub fn open_cli_reply(sealed: &str, offer: &str, reply_secret: &str, shown_at_ms: f64) -> Result<String, JsValue> {
    let offer = farcooler_ceremony::decode_offer(offer).map_err(|e| failure(e.code(), &e.to_string()))?;
    let (manifest, code) = enroll::open(sealed.trim(), &offer, reply_secret, false, shown_at_ms, js_sys::Date::now())
        .map_err(|e| failure(e.code(), &e.to_string()))?;
    serde_json::to_string(&enroll::Opened { code, taken: enroll::taken(&manifest) })
        .map_err(|e| failure("malformed", &e.to_string()))
}

#[wasm_bindgen(js_name = mergeRunnerGrants)]
pub fn merge_runner_grants(known: &str, added: &str) -> Result<String, JsValue> {
    let parse = |grants: &str| serde_json::from_str(grants).map_err(|e| failure("protocol", &e.to_string()));
    serde_json::to_string(&enroll::merge_grants(parse(known)?, parse(added)?))
        .map_err(|e| failure("protocol", &e.to_string()))
}

/// The bytes this browser's passkey signs a challenge of, to register `key_line`
/// for `account` with the directory against the relay's `challenge`.
#[wasm_bindgen(js_name = registrationData)]
pub fn registration_data(account: &str, challenge: &str, key_line: &str) -> Result<Uint8Array, JsValue> {
    let data = register::signed_data(account, challenge, key_line).map_err(|e| failure(e.word(), &e.to_string()))?;
    Ok(Uint8Array::from(&data[..]))
}

/// The armored proof `devices/register` takes, from the passkey's answer to
/// [`registration_data`]. Refused unless the answer is a touch, for this
/// build's host, over exactly that data.
#[wasm_bindgen(js_name = registrationProof)]
pub fn registration_proof(
    account: &str,
    challenge: &str,
    key_line: &str,
    authenticator_data: &[u8],
    client_data_json: &[u8],
    signature: &[u8],
) -> Result<String, JsValue> {
    let assertion = Assertion {
        authenticator_data: authenticator_data.to_vec(),
        client_data_json: client_data_json.to_vec(),
        signature: signature.to_vec(),
    };
    register::proof(account, challenge, key_line, &assertion).map_err(|e| failure(e.word(), &e.to_string()))
}

/// The ceremony's window in milliseconds, so the page can retire a code on
/// time without a second spelling of the number.
#[wasm_bindgen(js_name = freshnessMs)]
pub fn freshness_ms() -> f64 {
    farcooler_ceremony::FRESHNESS.as_millis() as f64
}

/// Whether a code shown at `shown_at_ms` is past the ceremony's window.
#[wasm_bindgen(js_name = offerIsStale)]
pub fn offer_is_stale(shown_at_ms: f64) -> bool {
    enroll::offer_is_stale(shown_at_ms, js_sys::Date::now())
}

/// The RP ID this build makes and signs with its passkey for: its channel's
/// app host, never the page's location.
#[wasm_bindgen(js_name = rpId)]
pub fn rp_id() -> String {
    passkey::rp_id().to_string()
}

/// A new passkey's public key (`getPublicKey()`'s SPKI) as the OpenSSH line an
/// offer carries.
#[wasm_bindgen(js_name = passkeyKeyLine)]
pub fn passkey_key_line(spki: &[u8], comment: &str) -> Result<String, JsValue> {
    passkey::public_line(spki, comment).map_err(web_failure)
}

/// The `SHA256:` fingerprint the phone shows beside this browser's name.
#[wasm_bindgen]
pub fn fingerprint(public_key: &str) -> Option<String> {
    farcooler_ceremony::fingerprint(public_key)
}

/// A connected runner.
#[wasm_bindgen]
pub struct WebRunner(std::rc::Rc<Runner>);

/// One attached pane.
#[wasm_bindgen]
pub struct WebPane(std::rc::Rc<std::cell::RefCell<Pane>>);

/// Keep the tab’s writer identity when its tunnel is replaced.
#[wasm_bindgen(js_name = newWriterKey)]
pub fn new_writer_key() -> String {
    Uuid::new_v4().to_string()
}

/// SSH to a runner over a dialed tunnel connection, refusing any host key but
/// `host_key`, signing in with the passkey `key_a` through `assert` (one
/// touch), and start its control session.
///
/// The connection is watched from the moment it is up (ov-772):
/// `on_lost(cause, idle_ms)` is called once if the tunnel goes quiet for
/// `Liveness::TUNNEL`'s allowance (nine seconds), the connection closes, or a
/// `check` finds it dead. `cause` is `farcooler_protocol::link_ending`'s word
/// and `idle_ms` how long nothing had arrived. Dropping the runner stops the
/// watching without calling it.
#[wasm_bindgen]
pub async fn connect(
    conn: JsValue,
    host_key: String,
    user: String,
    key_a: String,
    assert: Function,
    on_lost: Function,
    writer_key: Option<String>,
) -> Result<WebRunner, JsValue> {
    let clock: std::sync::Arc<dyn crate::liveness::Clock> = std::sync::Arc::new(PageClock);
    let link = Link::new(clock.clone(), Liveness::TUNNEL);
    let stream = link.tap(bridge(conn)?);
    let signer = PasskeySigner(JsAuthenticator(assert));
    let writer_instance = writer_key.and_then(|key| Uuid::parse_str(&key).ok());
    let watched = link.clone();
    let runner = handshake::sign_in(clock, handshake::HANDSHAKE, stream, &host_key, &user, &key_a, signer, link, writer_instance, move || {
        let (cause, idle_ms) = watched.ended().map_or(("", 0), |e| (e.cause, e.idle_ms));
        let _ = on_lost.call2(&JsValue::NULL, &cause.into(), &(idle_ms as f64).into());
    })
    .await
    .map_err(web_failure)?;
    Ok(WebRunner(std::rc::Rc::new(runner)))
}

#[wasm_bindgen]
impl WebRunner {
    /// Whether the runner answers right now. The page asks when the tab comes
    /// back and when the browser comes online; false means the connection is
    /// over and `on_lost` is called.
    ///
    /// A promise made by hand rather than an `async fn` on `&self`: that would
    /// hold a borrow of this object until the answer, and the page frees the
    /// runner from inside `on_lost`, which a failed check itself causes.
    pub fn check(&self) -> Promise {
        let runner = self.0.clone();
        wasm_bindgen_futures::future_to_promise(async move { Ok(JsValue::from_bool(runner.check().await)) })
    }

    /// Say that the person is signing out: the runner is told, and the answer
    /// is how many milliseconds this link had been quiet, for the page's log.
    pub fn signing_out(&self) -> f64 {
        self.0.signing_out() as f64
    }

    /// Whether the connection has been given up on.
    #[wasm_bindgen(getter)]
    pub fn lost(&self) -> bool {
        self.0.lost()
    }

    pub fn terminal_writer_supported(&self) -> bool {
        self.0.terminal_writer_supported()
    }

    pub fn on_terminal_writer_change(&self, listener: Option<Function>) {
        self.0.on_terminal_writer_change(listener.map(|listener| {
            std::rc::Rc::new(move |terminal: Option<Uuid>| {
                let id = terminal.map_or(JsValue::NULL, |id| JsValue::from_str(&id.to_string()));
                let _ = listener.call1(&JsValue::NULL, &id);
            }) as std::rc::Rc<dyn Fn(Option<Uuid>)>
        }));
    }

    pub fn terminal_writer(&self, terminal: String, action: String) -> Promise {
        let runner = self.0.clone();
        wasm_bindgen_futures::future_to_promise(async move {
            use farcooler_protocol::v1::terminal_writer_request::Action;
            let id = Uuid::parse_str(&terminal).map_err(|_| failure("protocol", "invalid terminal id"))?;
            let action = match action.as_str() {
                "status" => Action::Status,
                "watch" => Action::Watch,
                "claim" => Action::Claim,
                "take-over" => Action::TakeOver,
                "release" => Action::Release,
                _ => return Err(failure("protocol", "invalid writer action")),
            };
            Ok(JsValue::from_str(&runner.terminal_writer(id, action).await.map_err(web_failure)?.to_string()))
        })
    }

    pub async fn work_inventory(&self) -> Result<String, JsValue> {
        let inventory = self.0.session.work_inventory().await.map_err(|e| web_failure(e.into()))?;
        serde_json::to_string(&inventory).map_err(|e| failure("protocol", &e.to_string()))
    }

    pub async fn work_intent(&self, intent: &str) -> Result<Option<String>, JsValue> {
        let intent = serde_json::from_str(intent).map_err(|e| failure("protocol", &e.to_string()))?;
        let id = self.0.session.work_intent(intent).await.map_err(|e| web_failure(e.into()))?;
        Ok(id.map(|id| id.to_string()))
    }

    pub async fn board(&self, workspace: &str, repository: &str) -> Result<String, JsValue> {
        let (workspace, repository) = (board_id(workspace)?, board_id(repository)?);
        Ok(self.0.session.web_board(workspace, repository).await.map_err(|e| web_failure(e.into()))?.to_string())
    }

    pub async fn board_ticket(&self, workspace: &str, task: &str) -> Result<String, JsValue> {
        let (workspace, task) = (board_id(workspace)?, board_id(task)?);
        Ok(self.0.session.web_ticket(workspace, task).await.map_err(|e| web_failure(e.into()))?.to_string())
    }

    pub async fn board_intent(&self, workspace: &str, intent: &str) -> Result<String, JsValue> {
        let workspace = board_id(workspace)?;
        let intent = serde_json::from_str(intent).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.session.web_board_intent(workspace, intent).await.map_err(|e| web_failure(e.into()))?.to_string())
    }

    pub async fn web_review(&self, workspace: &str, ticket: &str, request: &str) -> Result<String, JsValue> {
        let (workspace, ticket) = (board_id(workspace)?, board_id(ticket)?);
        let request = serde_json::from_str(request).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.session.web_review(workspace, ticket, request).await.map_err(|e| web_failure(e.into()))?.to_string())
    }

    /// `terminal.list`, as JSON (`work_view::Terminal`).
    pub async fn terminals(&self) -> Result<String, JsValue> {
        let list = self.0.terminals().await.map_err(web_failure)?;
        serde_json::to_string(&list).map_err(|e| failure("protocol", &e.to_string()))
    }

    /// Authenticated workflow routes using the shared Session JSON contract.
    pub async fn workflow_call(&self, method: String, args: String) -> Result<String, JsValue> {
        let args = serde_json::from_str(&args).map_err(|e| failure("invalid", &e.to_string()))?;
        Ok(self.0.workflow_call(&method, args).await.map_err(web_failure)?.to_string())
    }

    /// `needs_you.list`, as JSON in the phones' shape (`needs_you_json`).
    #[wasm_bindgen(js_name = needsYou)]
    pub async fn needs_you(&self) -> Result<String, JsValue> {
        let value = self.0.needs_you().await.map_err(web_failure)?;
        Ok(value.to_string())
    }

    /// Answer an ask Needs You lists: its `ask_id` and the action's `id`, for
    /// the terminal it is about. Rejects with `refused` and the runner's `code`
    /// and `what` when it does not take the answer.
    pub async fn answer(&self, terminal: String, ask: String, option: String) -> Result<(), JsValue> {
        let id = Uuid::parse_str(&terminal).map_err(|e| failure("protocol", &e.to_string()))?;
        self.0.answer_ask(id, &ask, &option).await.map_err(web_failure)
    }

    /// Answer a decision Needs You lists, with the option's text, on its task.
    #[wasm_bindgen(js_name = answerDecision)]
    pub async fn answer_decision(&self, task: String, body: String) -> Result<(), JsValue> {
        let id = Uuid::parse_str(&task).map_err(|e| failure("protocol", &e.to_string()))?;
        self.0.answer_decision(id, &body).await.map_err(web_failure)
    }

    pub async fn runtime_health(&self) -> Result<String, JsValue> {
        let host = self.0.session.host().await.map_err(|e| web_failure(e.into()))?;
        Ok(farcooler_client::workday::runtime_health(&host))
    }

    pub async fn review_files(&self, worktree: String) -> Result<String, JsValue> {
        let id = Uuid::parse_str(&worktree).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.review_files(id).await.map_err(web_failure)?.to_string())
    }

    pub async fn review_patch(&self, worktree: String, path: String) -> Result<String, JsValue> {
        let id = Uuid::parse_str(&worktree).map_err(|e| failure("protocol", &e.to_string()))?;
        self.0.review_patch(id, &path).await.map_err(web_failure)
    }

    /// A workspace's plan, as JSON in the phones' shape (`plan_json`).
    pub async fn plan(&self, workspace: String) -> Result<String, JsValue> {
        let id = Uuid::parse_str(&workspace).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.plan(id).await.map_err(web_failure)?.to_string())
    }

    #[wasm_bindgen(js_name = boardOrder)]
    pub async fn board_order(&self, workspace: String) -> Result<Option<String>, JsValue> {
        let id = Uuid::parse_str(&workspace).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.board_order(id).await.map_err(web_failure)?.map(|view| view.to_string()))
    }

    #[wasm_bindgen(js_name = boardOrderLabel)]
    pub fn board_order_label(&self) -> Option<String> {
        self.0.board_order_label().map(str::to_owned)
    }

    #[wasm_bindgen(js_name = reorderTask)]
    pub async fn reorder_task(&self, task: String, anchor: String, after: bool) -> Result<(), JsValue> {
        let id = Uuid::parse_str(&task).map_err(|e| failure("protocol", &e.to_string()))?;
        let anchor = Uuid::parse_str(&anchor).map_err(|e| failure("protocol", &e.to_string()))?;
        self.0.reorder_task(id, anchor, after).await.map_err(web_failure)
    }

    /// `workspace.list`, as JSON in the phones' shape (`workspace_json`).
    pub async fn workspaces(&self) -> Result<String, JsValue> {
        Ok(self.0.workspaces().await.map_err(web_failure)?.to_string())
    }

    /// Write a theme's Next line (`board_theme.update`). Rejects with
    /// `refused` and the runner's `code` and `what` when its gate says no.
    #[wasm_bindgen(js_name = setThemeNext)]
    pub async fn set_theme_next(&self, theme: String, next: String) -> Result<(), JsValue> {
        let id = Uuid::parse_str(&theme).map_err(|e| failure("protocol", &e.to_string()))?;
        self.0.set_theme_next(id, &next).await.map_err(web_failure)
    }

    /// Keep one open ruling (`ruling.set`).
    #[wasm_bindgen(js_name = keepRuling)]
    pub async fn keep_ruling(&self, ruling: String) -> Result<(), JsValue> {
        let id = Uuid::parse_str(&ruling).map_err(|e| failure("protocol", &e.to_string()))?;
        self.0.keep_ruling(id).await.map_err(web_failure)
    }

    /// Keep every open ruling on a board (`ruling.keep_all`); how many it kept.
    #[wasm_bindgen(js_name = keepAllRulings)]
    pub async fn keep_all_rulings(&self, workspace: String) -> Result<u32, JsValue> {
        let id = Uuid::parse_str(&workspace).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.keep_all_rulings(id).await.map_err(web_failure)? as u32)
    }

    /// An agent's newest rows, or up to `limit` before row `before`, as JSON
    /// in the apps' shape (`rows_json::rows_page_json`). Epochs and revisions are
    /// numbers here: an epoch is the daemon's start in milliseconds times a
    /// thousand, inside a double's exact integers until the year 2255.
    #[wasm_bindgen(js_name = agentRows)]
    pub async fn agent_rows(&self, terminal: String, before: Option<f64>, limit: u32, agent: Option<String>) -> Result<String, JsValue> {
        let id = Uuid::parse_str(&terminal).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.agent_rows(id, before.map(|b| b as u64), limit, agent.as_deref()).await.map_err(web_failure)?.to_string())
    }

    /// What changed in an agent's rows after `after_rev` of projection
    /// `epoch`, held up to `wait_ms` while nothing has, as JSON in the apps'
    /// shape (`rows_json::changes_json`).
    #[wasm_bindgen(js_name = agentRowsFollow)]
    pub async fn agent_rows_follow(&self, terminal: String, epoch: f64, after_rev: f64, wait_ms: u32, agent: Option<String>) -> Result<String, JsValue> {
        let id = Uuid::parse_str(&terminal).map_err(|e| failure("protocol", &e.to_string()))?;
        let changes = self.0.agent_rows_follow(id, epoch as u64, after_rev as u64, wait_ms, agent.as_deref()).await.map_err(web_failure)?;
        Ok(changes.to_string())
    }

    /// File `index` of the send on tool row `row`, as a `Uint8Array`
    /// (`agent.file`). With `agent`, a subagent's own row.
    #[wasm_bindgen(js_name = agentFile)]
    pub async fn agent_file(&self, terminal: String, row: String, index: u32, agent: Option<String>) -> Result<Uint8Array, JsValue> {
        let id = Uuid::parse_str(&terminal).map_err(|e| failure("protocol", &e.to_string()))?;
        let bytes = self.0.agent_file(id, &row, index, agent.as_deref()).await.map_err(web_failure)?;
        Ok(Uint8Array::from(&bytes[..]))
    }

    /// Image `index` of the prompt on turn row `row`, as a `Uint8Array`.
    #[wasm_bindgen(js_name = agentImage)]
    pub async fn agent_image(&self, terminal: String, row: String, index: u32) -> Result<Uint8Array, JsValue> {
        let id = Uuid::parse_str(&terminal).map_err(|e| failure("protocol", &e.to_string()))?;
        let bytes = self.0.agent_image(id, &row, index).await.map_err(web_failure)?;
        Ok(Uint8Array::from(&bytes[..]))
    }

    /// Send `text` to an agent as the person's message. True when the agent
    /// was busy and the message waits in its queue.
    pub async fn compose(&self, terminal: String, text: String) -> Result<bool, JsValue> {
        let id = Uuid::parse_str(&terminal).map_err(|e| failure("protocol", &e.to_string()))?;
        self.0.compose(id, &text).await.map_err(web_failure)
    }

    /// Send `text` and `images` to an agent as the person's message. `images`
    /// is an array of `{mime, data}`, `data` a `Uint8Array`. True when the
    /// agent was busy and it waits in its queue. A refusal for size rejects
    /// with `refused` and `what` of `image_too_large` or `images_too_large`.
    #[wasm_bindgen(js_name = composeImages)]
    pub async fn compose_images(&self, terminal: String, text: String, images: js_sys::Array) -> Result<bool, JsValue> {
        let id = Uuid::parse_str(&terminal).map_err(|e| failure("protocol", &e.to_string()))?;
        let mut parsed = Vec::with_capacity(images.length() as usize);
        for image in images.iter() {
            let mime = Reflect::get(&image, &"mime".into()).ok().and_then(|m| m.as_string());
            let data = Reflect::get(&image, &"data".into()).ok().filter(|d| d.is_instance_of::<Uint8Array>());
            let (Some(mime), Some(data)) = (mime, data) else { return Err(failure("protocol", "an image is {mime, data}")) };
            parsed.push((mime, Uint8Array::from(data).to_vec()));
        }
        self.0.compose_images(id, &text, &parsed).await.map_err(web_failure)
    }

    /// What the composer may send, as JSON: `{images, each, total}`.
    #[wasm_bindgen(js_name = composeLimits)]
    pub fn compose_limits(&self) -> String {
        serde_json::to_string(&self.0.compose_limits()).unwrap_or_default()
    }

    /// One task with its record, as JSON in `task show --json`'s shape.
    pub async fn task(&self, task: String) -> Result<String, JsValue> {
        let id = Uuid::parse_str(&task).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.task(id).await.map_err(web_failure)?.to_string())
    }

    /// Add a note to a task as the person's.
    #[wasm_bindgen(js_name = addNote)]
    pub async fn add_note(&self, task: String, body: String) -> Result<(), JsValue> {
        let id = Uuid::parse_str(&task).map_err(|e| failure("protocol", &e.to_string()))?;
        self.0.comment_on_task(id, &body).await.map_err(web_failure)
    }

    /// Stop the turn an agent is working on.
    pub async fn interrupt(&self, terminal: String) -> Result<(), JsValue> {
        let id = Uuid::parse_str(&terminal).map_err(|e| failure("protocol", &e.to_string()))?;
        self.0.interrupt(id).await.map_err(web_failure)
    }

    /// Attach a pane: `output(Uint8Array)` for every byte, `ended()` once.
    pub async fn attach(&self, terminal: String, output: Function, ended: Function) -> Result<WebPane, JsValue> {
        let id = Uuid::parse_str(&terminal).map_err(|e| failure("protocol", &e.to_string()))?;
        let pane = self
            .0
            .attach(
                id,
                move |bytes| {
                    let _ = output.call1(&JsValue::NULL, &Uint8Array::from(bytes));
                },
                move || {
                    let _ = ended.call0(&JsValue::NULL);
                },
            )
            .await
            .map_err(web_failure)?;
        Ok(WebPane(std::rc::Rc::new(std::cell::RefCell::new(pane))))
    }
}

#[wasm_bindgen]
impl WebPane {
    /// Type into the terminal. Bytes reach the runner in call order whether or
    /// not the page awaits each answer.
    pub fn write(&self, bytes: &[u8]) -> Promise {
        let answer = self.0.borrow().write(bytes);
        wasm_bindgen_futures::future_to_promise(async move {
            answer.await.map(|_| JsValue::UNDEFINED).map_err(web_failure)
        })
    }

    pub fn resize(&self, columns: u32, rows: u32) -> Promise {
        let answer = self.0.borrow().resize(columns, rows);
        wasm_bindgen_futures::future_to_promise(async move {
            answer.await.map(|_| JsValue::UNDEFINED).map_err(web_failure)
        })
    }

    pub fn close(&self) {
        spawn(self.0.borrow_mut().close());
    }
}

/// The client's name and build, for the page's About line and the runner's log.
#[wasm_bindgen]
pub fn build() -> String {
    format!("{} {}", wire::CLIENT_NAME, farcooler_protocol::BUILD)
}

/// Pure projection of the runner inventory and the browser's destination.
#[wasm_bindgen(js_name = workView)]
pub fn work_view(inventory: &str, selection: &str) -> Result<String, JsValue> {
    let inventory = serde_json::from_str(inventory).map_err(|e| failure("protocol", &e.to_string()))?;
    let selection = serde_json::from_str(selection).map_err(|e| failure("protocol", &e.to_string()))?;
    Ok(farcooler_client::work_view::view(&inventory, &selection).to_string())
}

#[wasm_bindgen(js_name = workFailure)]
pub fn work_failure(word: &str, code: &str, what: &str) -> String {
    farcooler_client::work_view::failure(word, code, what).into()
}


fn board_id(id: &str) -> Result<Uuid, JsValue> {
    Uuid::parse_str(id).map_err(|e| failure("protocol", &e.to_string()))
}

#[wasm_bindgen(js_name = ticketForm)]
pub fn ticket_form() -> String {
    farcooler_client::web_board::form(None).to_string()
}

#[wasm_bindgen(js_name = boardFailure)]
pub fn board_failure(word: &str, code: &str, what: &str) -> String {
    farcooler_client::web_board::failure(word, code, what).into()
}


#[wasm_bindgen(js_name = reviewFailure)]
pub fn review_failure(word: &str, code: &str, what: &str) -> String {
    farcooler_client::web_review::failure(word, code, what).into()
}

#[wasm_bindgen(js_name = reviewApprovalResult)]
pub fn review_approval_result(raw: &str) -> String {
    farcooler_client::web_review::approval_result(raw).into()
}

#[wasm_bindgen(js_name = runnerHealth)]
pub fn runner_health(runner: &str, connection: &str) -> Result<String, JsValue> {
    let connection = serde_json::from_str(connection).map_err(|e| failure("protocol", &e.to_string()))?;
    serde_json::to_string(&farcooler_client::workday::health(runner, connection))
        .map_err(|e| failure("protocol", &e.to_string()))
}

#[wasm_bindgen(js_name = needsAttention)]
pub fn needs_attention(count: usize) -> Option<String> {
    farcooler_client::workday::attention(count)
}
