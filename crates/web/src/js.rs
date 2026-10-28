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

use crate::enroll;
use crate::error::WebError;
use crate::handshake;
use crate::liveness::{Link, PageClock};
use crate::passkey::{self, Assertion, Authenticate, PasskeySigner};
use crate::ssh::SignError;
use crate::wire::{self, Pane, Runner, spawn};

fn failure(word: &str, detail: &str) -> JsValue {
    let error = js_sys::Error::new(word);
    let _ = Reflect::set(&error, &"detail".into(), &detail.into());
    error.into()
}

fn web_failure(e: WebError) -> JsValue {
    failure(e.word(), &e.to_string())
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

/// The offer this browser shows, as the JSON its QR code carries.
#[wasm_bindgen(js_name = makeOffer)]
pub fn make_offer(name: &str, account: &str, key_a: &str, node_key: &str) -> String {
    farcooler_ceremony::encode_offer(&enroll::offer(name, account, key_a, node_key))
}

/// Take the phone's reply to the offer this browser showed at `shown_at_ms`.
/// Answers `enroll::Taken` as JSON — who granted it when it isn't this
/// account, and the runners, each with its pinned host key — or refuses with
/// the ceremony's word (`stale` past the window).
///
/// "Now" is read here, not passed in, so the page cannot leave the window out.
#[wasm_bindgen(js_name = acceptReply)]
pub fn accept_reply(reply: &str, offer: &str, already_taken: bool, shown_at_ms: f64) -> Result<String, JsValue> {
    let offer = farcooler_ceremony::decode_offer(offer).map_err(|e| failure(e.code(), &e.to_string()))?;
    let manifest = enroll::accept(reply, &offer, already_taken, shown_at_ms, js_sys::Date::now())
        .map_err(|e| failure(e.code(), &e.to_string()))?;
    serde_json::to_string(&enroll::taken(&manifest)).map_err(|e| failure("malformed", &e.to_string()))
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

/// SSH to a runner over a dialed tunnel connection, refusing any host key but
/// `host_key`, signing in with the passkey `key_a` through `assert` (one
/// touch), and start its control session.
///
/// The connection is watched from the moment it is up (ov-772): `on_lost()` is
/// called once if the tunnel goes quiet for `Liveness::TUNNEL`'s allowance
/// (nine seconds), or the connection closes. Dropping the runner stops the
/// watching without calling it.
#[wasm_bindgen]
pub async fn connect(
    conn: JsValue,
    host_key: String,
    user: String,
    key_a: String,
    assert: Function,
    on_lost: Function,
) -> Result<WebRunner, JsValue> {
    let clock: std::sync::Arc<dyn crate::liveness::Clock> = std::sync::Arc::new(PageClock);
    let link = Link::new(clock.clone(), Liveness::TUNNEL);
    let stream = link.tap(bridge(conn)?);
    let signer = PasskeySigner(JsAuthenticator(assert));
    let runner = handshake::sign_in(clock, handshake::HANDSHAKE, stream, &host_key, &user, &key_a, signer, link, move || {
        let _ = on_lost.call0(&JsValue::NULL);
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

    /// Whether the connection has been given up on.
    #[wasm_bindgen(getter)]
    pub fn lost(&self) -> bool {
        self.0.lost()
    }

    /// `terminal.list`, as JSON (`wire::TerminalSummary`).
    pub async fn terminals(&self) -> Result<String, JsValue> {
        let list = self.0.terminals().await.map_err(web_failure)?;
        serde_json::to_string(&list).map_err(|e| failure("protocol", &e.to_string()))
    }

    /// `needs_you.list`, as JSON in the phones' shape (`needs_you_json`).
    #[wasm_bindgen(js_name = needsYou)]
    pub async fn needs_you(&self) -> Result<String, JsValue> {
        let value = self.0.needs_you().await.map_err(web_failure)?;
        Ok(value.to_string())
    }

    /// A workspace's plan, as JSON in the phones' shape (`plan_json`).
    pub async fn plan(&self, workspace: String) -> Result<String, JsValue> {
        let id = Uuid::parse_str(&workspace).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.plan(id).await.map_err(web_failure)?.to_string())
    }

    /// One orchestrator page, as JSON in the phones' shape (`page_json`).
    pub async fn page(&self, workspace: String, slot: String) -> Result<String, JsValue> {
        let id = Uuid::parse_str(&workspace).map_err(|e| failure("protocol", &e.to_string()))?;
        Ok(self.0.page(id, &slot).await.map_err(web_failure)?.to_string())
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
