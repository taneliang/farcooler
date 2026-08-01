//! The C ABI, for Swift and later Kotlin.
//!
//! Everything here is asynchronous underneath and synchronous at the boundary,
//! because a UI toolkit's idea of async is not a Rust runtime's and bridging
//! the two through callbacks means one of them is always wrong about which
//! thread it is on.
//!
//! The model instead is a **request queue with polling**:
//!
//! 1. `overnight_call` submits work and returns immediately with a ticket.
//! 2. The runtime, on its own thread, does the work.
//! 3. `overnight_poll` returns finished results, oldest first.
//!
//! A UI already has a frame loop or a timer, so polling costs it nothing and
//! removes every question about callback threading, re-entrancy, and what
//! happens when a view disappears mid-flight. The same shape as the VT core's
//! revision counter, for the same reason.
//!
//! Every answer is JSON. The wire stays protobuf; this is the boundary, and a
//! protobuf runtime in Swift and again in Kotlin — to describe messages this
//! crate has already decoded — would be work with nothing to show for it.

use std::collections::VecDeque;
use std::ffi::{CStr, c_char, c_void};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::session::{Session, uuid_of};
use crate::ssh::{Destination, HostKeyPolicy};

/// A client handle: a runtime, a session, and a queue of finished work.
pub struct ClientHandle {
    runtime: tokio::runtime::Runtime,
    session: Arc<tokio::sync::Mutex<Option<Session>>>,
    /// Finished results, oldest first.
    finished: Arc<Mutex<VecDeque<String>>>,
    next_ticket: Arc<Mutex<u64>>,
    /// Whatever the last call returned that the caller has not taken yet, held
    /// so the pointer stays valid until the following call.
    scratch: Option<std::ffi::CString>,
    /// Running terminal streams, by terminal id, so a second attach replaces the
    /// first rather than pumping the same bytes twice.
    streams: Arc<Mutex<std::collections::HashMap<String, tokio::task::JoinHandle<()>>>>,
}

/// Create a client. Free with `overnight_client_free`.
#[unsafe(no_mangle)]
pub extern "C" fn overnight_client_new() -> *mut c_void {
    // A dedicated multi-threaded runtime: SSH keepalives have to keep running
    // while a call is in flight, which a single-threaded runtime driven only
    // during calls could not do.
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    else {
        return std::ptr::null_mut();
    };

    let handle = Box::new(ClientHandle {
        runtime,
        session: Arc::new(tokio::sync::Mutex::new(None)),
        finished: Arc::new(Mutex::new(VecDeque::new())),
        next_ticket: Arc::new(Mutex::new(1)),
        scratch: None,
        streams: Arc::new(Mutex::new(std::collections::HashMap::new())),
    });
    Box::into_raw(handle) as *mut c_void
}

/// Destroy a client, ending any SSH session it holds. Safe with null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn overnight_client_free(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(handle as *mut ClientHandle) });
}

/// Connect to a host.
///
/// `config` is JSON:
///
/// ```text
/// {"host":"box","port":22,"user":"me","private_key":"-----BEGIN...",
///  "passphrase":null,"host_fingerprint":"SHA256:..."}
/// ```
///
/// Omitting `host_fingerprint` means first contact: the call fails with the
/// fingerprint in its message so the client can show it and ask. Passing
/// `"accept-any"` connects without pinning, which is the user's decision to
/// make and not a default.
///
/// Returns a ticket. Poll for the result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn overnight_client_connect(
    handle: *mut c_void,
    config: *const c_char,
) -> u64 {
    let Some(h) = (unsafe { as_handle(handle) }) else { return 0 };
    let Some(config) = (unsafe { read_str(config) }) else { return 0 };

    let ticket = h.take_ticket();
    let session = Arc::clone(&h.session);
    let finished = Arc::clone(&h.finished);

    h.runtime.spawn(async move {
        let outcome = match parse_destination(&config) {
            Ok(destination) => match Session::connect_ssh(&destination).await {
                Ok(open) => {
                    let version = open.daemon_version().to_string();
                    *session.lock().await = Some(open);
                    Ok(json!({ "daemon_version": version }))
                }
                Err(e) => Err(e.to_string()),
            },
            Err(message) => Err(message),
        };
        push(&finished, ticket, outcome);
    });

    ticket
}

/// Invoke a method. `args` is JSON; the shape depends on the method.
///
/// Returns a ticket, or 0 if the arguments could not be read at all.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn overnight_client_call(
    handle: *mut c_void,
    method: *const c_char,
    args: *const c_char,
) -> u64 {
    let Some(h) = (unsafe { as_handle(handle) }) else { return 0 };
    let Some(method) = (unsafe { read_str(method) }) else { return 0 };
    let args = unsafe { read_str(args) }.unwrap_or_else(|| "{}".into());

    let ticket = h.take_ticket();
    let session = Arc::clone(&h.session);
    let finished = Arc::clone(&h.finished);

    h.runtime.spawn(async move {
        let parsed: Value = serde_json::from_str(&args).unwrap_or(json!({}));
        let mut guard = session.lock().await;
        let outcome = match guard.as_mut() {
            None => Err("not connected".to_string()),
            Some(session) => dispatch(session, &method, &parsed).await,
        };
        push(&finished, ticket, outcome);
    });

    ticket
}

/// Generate a new ed25519 key pair for this device.
///
/// Generated here rather than in Swift or Kotlin for the same reason nothing
/// else in this project implements cryptography: there is one place that does
/// it, and it is a library that people who do this for a living maintain.
/// Writing an OpenSSH private-key encoder twice, in two languages, to save a
/// function call would be a poor trade.
///
/// Writes JSON into `out`:
///
/// ```text
/// {"private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\n...",
///  "public_key": "ssh-ed25519 AAAA... overnight"}
/// ```
///
/// Returns the number of bytes needed. If that exceeds `capacity`, nothing is
/// written; call again with a larger buffer. 2048 bytes is ample.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn overnight_client_generate_key(
    comment: *const c_char,
    out: *mut u8,
    capacity: usize,
) -> usize {
    use russh::keys::ssh_key::{Algorithm, LineEnding, PrivateKey};

    let comment = unsafe { read_str(comment) }.unwrap_or_else(|| "overnight".into());

    let Ok(mut key) = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519) else {
        return 0;
    };
    key.set_comment(comment.as_str());

    let Ok(private) = key.to_openssh(LineEnding::LF) else { return 0 };
    let Ok(public) = key.public_key().to_openssh() else { return 0 };

    let payload = json!({
        "private_key": private.to_string(),
        "public_key": public,
    })
    .to_string();

    let bytes = payload.as_bytes();
    if out.is_null() || bytes.len() > capacity {
        return bytes.len();
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    bytes.len()
}

/// Start streaming a terminal's output. Chunks arrive through `poll`.
///
/// Each chunk is `{"stream": "<terminal>", "chunk": "<base64>"}` — no ticket,
/// because a stream is not an answer to anything. A client feeds the bytes to
/// its own emulator, which is the same one the daemon would have used.
///
/// Returns false if the client is not connected over ssh.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn overnight_client_stream_start(
    handle: *mut c_void,
    terminal: *const c_char,
) -> bool {
    let Some(h) = (unsafe { as_handle(handle) }) else { return false };
    let Some(terminal) = (unsafe { read_str(terminal) }) else { return false };
    let Ok(id) = terminal.parse::<uuid::Uuid>() else { return false };

    let session = h.session.clone();
    let finished = h.finished.clone();
    let streams = h.streams.clone();
    let key = terminal.clone();

    let task = h.runtime.spawn(async move {
        use tokio::io::AsyncReadExt;

        let reader = {
            let mut guard = session.lock().await;
            let Some(session) = guard.as_mut() else { return };
            match session.open_stream(id).await {
                Ok(reader) => reader,
                Err(e) => {
                    push_line(
                        &finished,
                        json!({ "stream": key, "error": e.to_string() }).to_string(),
                    );
                    return;
                }
            }
            // The lock is released here, deliberately: the stream reads for as
            // long as the pane lives, and holding the session for that long
            // would block every other call on this client forever.
        };

        let mut reader = reader;
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => push_line(
                    &finished,
                    json!({ "stream": key, "chunk": base64(&buf[..n]) }).to_string(),
                ),
                Err(_) => break,
            }
        }
        push_line(&finished, json!({ "stream": key, "ended": true }).to_string());
    });

    let mut running = streams.lock().expect("streams");
    if let Some(previous) = running.insert(terminal, task) {
        previous.abort();
    }
    true
}

/// Stop streaming a terminal. Safe to call when nothing is running.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn overnight_client_stream_stop(
    handle: *mut c_void,
    terminal: *const c_char,
) {
    let Some(h) = (unsafe { as_handle(handle) }) else { return };
    let Some(terminal) = (unsafe { read_str(terminal) }) else { return };
    if let Some(task) = h.streams.lock().expect("streams").remove(&terminal) {
        task.abort();
    }
}

/// The public key belonging to a private key.
///
/// Derived on demand rather than stored beside it. A device has exactly one
/// identity, and keeping the public half in a second place meant two sources for
/// one fact: clear one and they disagree silently, which is what happened — an
/// app whose keychain survived a reinstall but whose preferences did not went on
/// offering a key it no longer knew the name of, and reported a stale one to the
/// human trying to authorise it.
///
/// Returns the number of bytes needed, as `overnight_client_generate_key` does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn overnight_client_public_key(
    private_key: *const c_char,
    out: *mut u8,
    capacity: usize,
) -> usize {
    use russh::keys::ssh_key::PrivateKey;

    let Some(text) = (unsafe { read_str(private_key) }) else { return 0 };
    let Ok(key) = text.parse::<PrivateKey>() else { return 0 };
    let Ok(public) = key.public_key().to_openssh() else { return 0 };

    let bytes = public.as_bytes();
    if out.is_null() || bytes.len() > capacity {
        return bytes.len();
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    bytes.len()
}

/// Take the oldest finished result, or NULL if none is ready.
///
/// The returned pointer is owned by the handle and stays valid until the next
/// call on it. Each result is returned exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn overnight_client_poll(handle: *mut c_void) -> *const c_char {
    let Some(h) = (unsafe { as_handle(handle) }) else { return std::ptr::null() };
    let next = h.finished.lock().expect("queue").pop_front();
    match next {
        Some(json) => {
            h.scratch = std::ffi::CString::new(json).ok();
            h.scratch.as_ref().map_or(std::ptr::null(), |s| s.as_ptr())
        }
        None => std::ptr::null(),
    }
}

/// True once a session is established.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn overnight_client_connected(handle: *mut c_void) -> bool {
    let Some(h) = (unsafe { as_handle(handle) }) else { return false };
    // try_lock rather than blocking: this is called from a UI thread, and a
    // call in flight holds the session for as long as the host takes to answer.
    h.session.try_lock().map(|guard| guard.is_some()).unwrap_or(true)
}

async fn dispatch(session: &mut Session, method: &str, args: &Value) -> Result<Value, String> {
    let id = |key: &str| -> Result<uuid::Uuid, String> {
        args.get(key)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<uuid::Uuid>().ok())
            .ok_or_else(|| format!("{method} needs a {key}"))
    };
    let text = |key: &str| -> String {
        args.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string()
    };

    match method {
        "fleet" => session.fleet().await.map_err(|e| e.to_string()),

        "repositories" => {
            let items = session.repositories().await.map_err(|e| e.to_string())?;
            Ok(json!({
                "repositories": items.iter().map(|r| json!({
                    "id": uuid_of(&r.id).to_string(),
                    "short": crate::session::short(&r.id),
                    "displayName": r.display_name,
                    "remote": r.remote_summary,
                })).collect::<Vec<_>>()
            }))
        }

        "workspace.create" => {
            let base = match text("base") {
                // A phone's form leaves this blank; HEAD is what the CLI
                // defaults to, so both clients branch from the same place.
                b if b.is_empty() => "HEAD".to_string(),
                b => b,
            };
            let workspace = session
                .create_workspace(id("repository")?, &text("task"), &text("branch"), &base)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "id": uuid_of(&workspace.id).to_string() }))
        }

        "workspace.archive" => {
            session.archive_workspace(id("workspace")?).await.map_err(|e| e.to_string())?;
            Ok(json!({}))
        }
        "workspace.restore" => {
            session.restore_workspace(id("workspace")?).await.map_err(|e| e.to_string())?;
            Ok(json!({}))
        }

        "terminal.create" => {
            let terminal = session
                .create_terminal(
                    id("workspace")?,
                    &text("title"),
                    &text("preset"),
                    args.get("tile").and_then(|v| v.as_bool()).unwrap_or(false),
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "id": uuid_of(&terminal.id).to_string() }))
        }
        "terminal.stop" => {
            session.stop_terminal(id("terminal")?).await.map_err(|e| e.to_string())?;
            Ok(json!({}))
        }
        "terminal.restart" => {
            session.restart_terminal(id("terminal")?).await.map_err(|e| e.to_string())?;
            Ok(json!({}))
        }
        "terminal.dismiss_lost" => {
            session.dismiss_lost(id("terminal")?).await.map_err(|e| e.to_string())?;
            Ok(json!({}))
        }
        // The screen, base64 so it survives JSON.
        //
        // A capture carries escape sequences and arbitrary bytes; JSON carries
        // text. Encoding is the honest way through — the alternative is a lossy
        // string that would arrive already flattened, and the client has a real
        // emulator waiting for exactly these bytes.
        "terminal.screen" => {
            let known = args.get("knownRevision").and_then(|v| v.as_u64()).unwrap_or(0);
            let screen =
                session.screen(id("terminal")?, known).await.map_err(|e| e.to_string())?;
            Ok(json!({
                // Absent when unchanged, so an idle pane costs a few bytes on
                // the wire instead of a whole capture several times a second.
                "contents": base64(&screen.contents),
                "columns": screen.columns,
                "rows": screen.rows,
                "cursorColumn": screen.cursor_column,
                "cursorRow": screen.cursor_row,
                "revision": screen.revision,
                "unchanged": screen.unchanged,
                "modes": screen.modes,
            }))
        }

        // Input, as hex. A key is not always a character — arrows, Ctrl-C and a
        // bracketed paste are byte sequences — so nothing here re-encodes.
        "terminal.write" => {
            let hex = text("hex");
            let bytes = decode_hex(&hex).ok_or("input must be hex")?;
            session.write(id("terminal")?, bytes).await.map_err(|e| e.to_string())?;
            Ok(json!({}))
        }

        "terminal.resize" => {
            let columns = args.get("columns").and_then(|v| v.as_u64()).unwrap_or(80) as u32;
            let rows = args.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u32;
            session
                .resize_terminal(id("terminal")?, columns, rows)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({}))
        }

        // Refused rather than defaulted, so a typo in a client is a visible
        // error instead of a call that silently does nothing.
        other => Err(format!("unknown method: {other}")),
    }
}

fn parse_destination(config: &str) -> Result<Destination, String> {
    let value: Value = serde_json::from_str(config).map_err(|e| format!("bad config: {e}"))?;
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);

    let host_key = match text("host_fingerprint").as_deref() {
        Some("accept-any") => HostKeyPolicy::Accept,
        Some(fingerprint) => HostKeyPolicy::Pinned(fingerprint.to_string()),
        // No recorded key: report the fingerprint and refuse, so a human sees
        // it before anything trusts it.
        None => HostKeyPolicy::RequireApproval,
    };

    Ok(Destination {
        host: text("host").ok_or("config needs a host")?,
        port: value.get("port").and_then(|v| v.as_u64()).unwrap_or(22) as u16,
        user: text("user").ok_or("config needs a user")?,
        private_key: text("private_key").ok_or("config needs a private_key")?,
        passphrase: text("passphrase"),
        host_key,
    })
}

/// Queue a line that is not an answer to any request.
fn push_line(queue: &Arc<Mutex<VecDeque<String>>>, line: String) {
    queue.lock().expect("queue").push_back(line);
}

fn push(queue: &Arc<Mutex<VecDeque<String>>>, ticket: u64, outcome: Result<Value, String>) {
    let payload = match outcome {
        Ok(value) => json!({ "ticket": ticket, "ok": true, "result": value }),
        Err(message) => json!({ "ticket": ticket, "ok": false, "error": message }),
    };
    queue.lock().expect("queue").push_back(payload.to_string());
}

impl ClientHandle {
    fn take_ticket(&self) -> u64 {
        let mut next = self.next_ticket.lock().expect("ticket");
        let ticket = *next;
        *next += 1;
        ticket
    }
}

unsafe fn as_handle<'a>(handle: *mut c_void) -> Option<&'a mut ClientHandle> {
    if handle.is_null() {
        return None;
    }
    Some(unsafe { &mut *(handle as *mut ClientHandle) })
}

unsafe fn read_str(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(pointer) }.to_str().ok().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_fingerprint_means_ask_rather_than_trust() {
        // The default has to be the safe one: silently accepting an unknown key
        // is what makes an interception invisible.
        let destination = parse_destination(
            r#"{"host":"box","user":"me","private_key":"k"}"#,
        )
        .unwrap();
        assert!(matches!(destination.host_key, HostKeyPolicy::RequireApproval));
        assert_eq!(destination.port, 22);
    }

    #[test]
    fn a_recorded_fingerprint_is_pinned() {
        let destination = parse_destination(
            r#"{"host":"box","user":"me","private_key":"k","host_fingerprint":"SHA256:abc"}"#,
        )
        .unwrap();
        match destination.host_key {
            HostKeyPolicy::Pinned(f) => assert_eq!(f, "SHA256:abc"),
            other => panic!("expected a pin, got {other:?}"),
        }
    }

    #[test]
    fn accepting_any_key_has_to_be_spelled_out() {
        let destination = parse_destination(
            r#"{"host":"box","user":"me","private_key":"k","host_fingerprint":"accept-any"}"#,
        )
        .unwrap();
        assert!(matches!(destination.host_key, HostKeyPolicy::Accept));
    }

    #[test]
    fn an_incomplete_config_is_refused_by_name() {
        assert!(parse_destination(r#"{"user":"me","private_key":"k"}"#).unwrap_err().contains("host"));
        assert!(parse_destination(r#"{"host":"b","private_key":"k"}"#).unwrap_err().contains("user"));
        assert!(parse_destination("not json").unwrap_err().contains("bad config"));
    }

    #[test]
    fn results_come_back_in_order_and_only_once() {
        let handle = overnight_client_new();
        assert!(!handle.is_null());

        let h = unsafe { as_handle(handle) }.unwrap();
        push(&h.finished, 1, Ok(json!({"a": 1})));
        push(&h.finished, 2, Err("nope".into()));

        let first = unsafe { overnight_client_poll(handle) };
        let first = unsafe { CStr::from_ptr(first) }.to_str().unwrap().to_string();
        assert!(first.contains("\"ticket\":1") && first.contains("\"ok\":true"));

        let second = unsafe { overnight_client_poll(handle) };
        let second = unsafe { CStr::from_ptr(second) }.to_str().unwrap().to_string();
        assert!(second.contains("\"ticket\":2") && second.contains("\"ok\":false"));
        assert!(second.contains("nope"));

        // Drained.
        assert!(unsafe { overnight_client_poll(handle) }.is_null());
        unsafe { overnight_client_free(handle) };
    }

    #[test]
    fn a_generated_key_is_a_usable_openssh_pair() {
        let mut buffer = vec![0u8; 4096];
        let n = unsafe {
            overnight_client_generate_key(
                c"overnight-test".as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
            )
        };
        assert!(n > 0 && n <= buffer.len());

        let payload: Value = serde_json::from_slice(&buffer[..n]).expect("json");
        let private = payload["private_key"].as_str().unwrap();
        let public = payload["public_key"].as_str().unwrap();

        assert!(private.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(public.starts_with("ssh-ed25519 "));
        assert!(public.ends_with("overnight-test"), "the comment identifies the device");

        // The real assertion: the private key we hand out is one our own SSH
        // client can load. A key that generates but cannot be used would only
        // fail at the first connection attempt.
        assert!(crate::ssh::decode_key_for_test(private).is_ok());
    }

    #[test]
    fn two_generated_keys_differ() {
        let mut a = vec![0u8; 4096];
        let mut b = vec![0u8; 4096];
        let na = unsafe { overnight_client_generate_key(std::ptr::null(), a.as_mut_ptr(), a.len()) };
        let nb = unsafe { overnight_client_generate_key(std::ptr::null(), b.as_mut_ptr(), b.len()) };
        assert_ne!(a[..na], b[..nb]);
    }

    #[test]
    fn generating_into_a_short_buffer_reports_the_size_and_writes_nothing() {
        let mut tiny = [0u8; 4];
        let needed = unsafe {
            overnight_client_generate_key(std::ptr::null(), tiny.as_mut_ptr(), tiny.len())
        };
        assert!(needed > tiny.len());
        assert_eq!(tiny, [0, 0, 0, 0], "a truncated private key would be worse than none");
    }

    #[test]
    fn null_arguments_are_survivable() {
        // A UI bug must not take down the app.
        let null = std::ptr::null_mut();
        unsafe {
            assert_eq!(overnight_client_connect(null, std::ptr::null()), 0);
            assert_eq!(overnight_client_call(null, std::ptr::null(), std::ptr::null()), 0);
            assert!(overnight_client_poll(null).is_null());
            assert!(!overnight_client_connected(null));
            overnight_client_free(null);
        }

        let handle = overnight_client_new();
        unsafe {
            assert_eq!(overnight_client_call(handle, std::ptr::null(), std::ptr::null()), 0);
            overnight_client_free(handle);
        }
    }
}


/// Standard base64, without pulling in a crate for twenty lines.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - i * 6) & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod encoding_tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_including_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_survives_bytes_that_are_not_text() {
        // Which is the point: a screen is escape sequences, not a string.
        assert_eq!(base64(&[0x1b, 0x5b, 0x33, 0x31, 0x6d]), "G1szMW0=");
        assert_eq!(base64(&[0xff, 0x00, 0xfe]), "/wD+");
    }

    #[test]
    fn hex_round_trips_and_rejects_what_is_not_hex() {
        assert_eq!(decode_hex("00ff1b"), Some(vec![0x00, 0xff, 0x1b]));
        assert_eq!(decode_hex(""), Some(vec![]));
        assert_eq!(decode_hex("abc"), None, "odd length is not a byte string");
        assert_eq!(decode_hex("zz"), None);
    }
}

#[cfg(test)]
mod identity_tests {
    use std::ffi::CString;

    /// A generated key and the public key derived from it have to agree, or the
    /// device offers one identity and shows a human another.
    #[test]
    fn the_derived_public_key_matches_the_generated_one() {
        let comment = CString::new("test").unwrap();
        let mut buf = vec![0u8; 4096];
        let n = unsafe {
            super::overnight_client_generate_key(comment.as_ptr(), buf.as_mut_ptr(), buf.len())
        };
        assert!(n > 0 && n <= buf.len());

        let pair: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
        let private = pair["private_key"].as_str().unwrap();
        let generated = pair["public_key"].as_str().unwrap();

        let key = CString::new(private).unwrap();
        let mut out = vec![0u8; 2048];
        let n = unsafe {
            super::overnight_client_public_key(key.as_ptr(), out.as_mut_ptr(), out.len())
        };
        assert!(n > 0 && n <= out.len());
        let derived = String::from_utf8_lossy(&out[..n]).into_owned();

        // The comment is not part of the key, and `to_openssh` on a parsed key
        // keeps it, so compare the parts that identify it.
        let fields = |line: &str| -> String {
            line.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
        };
        assert_eq!(fields(&derived), fields(generated));
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed_at() {
        let junk = CString::new("not a key").unwrap();
        let mut out = vec![0u8; 256];
        assert_eq!(
            unsafe { super::overnight_client_public_key(junk.as_ptr(), out.as_mut_ptr(), out.len()) },
            0
        );
    }
}
