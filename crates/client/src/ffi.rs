//! The C ABI, for Swift and later Kotlin.
//!
//! Everything here is asynchronous underneath and synchronous at the boundary,
//! because a UI toolkit's idea of async is not a Rust runtime's and bridging
//! the two through callbacks means one of them is always wrong about which
//! thread it is on.
//!
//! The model instead is a **request queue with polling**:
//!
//! 1. `farcooler_call` submits work and returns immediately with a ticket.
//! 2. The runtime, on its own thread, does the work.
//! 3. `farcooler_poll` returns finished results, oldest first.
//!
//! A UI already has a frame loop or a timer, so polling costs it nothing and
//! removes every question about callback threading, re-entrancy, and what
//! happens when a view disappears mid-flight. The same shape as the VT core's
//! revision counter, for the same reason.
//!
//! Every answer is JSON. The wire stays protobuf; this is the boundary, and a
//! protobuf runtime in Swift and again in Kotlin — to describe messages this
//! crate has already decoded — would be work with nothing to show for it.

//! ## Safety, once rather than per function
//!
//! Every function here is `unsafe` under the same contract, so
//! `clippy::missing_safety_doc` is silenced at the module level rather than
//! answered one function at a time — copies of one paragraph are copies to
//! drift out of date.
//!
//! The contract: pointers are either null or valid for the call's duration, a
//! handle came from `farcooler_client_new` and has not been freed, and strings
//! passed in are NUL-terminated UTF-8. Null is checked everywhere.
#![allow(clippy::missing_safety_doc)]

use std::collections::VecDeque;
use std::ffi::{CStr, c_char, c_void};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::session::{Session, SessionError, uuid_of};
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

/// Create a client. Free with `farcooler_client_free`.
#[unsafe(no_mangle)]
pub extern "C" fn farcooler_client_new() -> *mut c_void {
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
pub unsafe extern "C" fn farcooler_client_free(handle: *mut c_void) {
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
pub unsafe extern "C" fn farcooler_client_connect(
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
pub unsafe extern "C" fn farcooler_client_call(
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
            None => Err(Lost::Already),
            Some(session) => dispatch(session, &method, &parsed).await.map_err(Lost::Call),
        };

        // A transport failure is not this request's problem; it is every
        // future request's problem.
        //
        // Emptying the slot is what makes `farcooler_client_connected` an
        // answer about now rather than about history: it reported whether a
        // session had ever been put here, and nothing ever took one out, so a
        // session whose ssh transport died an hour ago still read as
        // connected. It is also what makes the next call fail in microseconds
        // with "not connected" rather than spending another TCP timeout
        // learning the same thing.
        let lost = match &outcome {
            Err(Lost::Call(e)) => e.is_disconnect(),
            // An empty slot is reported as a drop too, and it has to be.
            //
            // The first call to notice a dead link is whichever one the user
            // happened to make — a keystroke, a resize — and it empties the
            // slot on its way out. If this answered `false`, the poll that
            // arrives a moment later would be told "not connected" with
            // nothing to mark it as a disconnection, and a client whose only
            // detector is that poll would sit at `connected` forever with
            // every call failing. Saying so twice costs nothing: both clients
            // ignore a drop reported while they are already reconnecting.
            Err(Lost::Already) => true,
            Ok(_) => false,
        };
        if lost {
            *guard = None;
        }
        // Released before pushing, so a client that reconnects the instant it
        // reads this answer is not queued behind this call's own lock.
        drop(guard);

        push_call(&finished, ticket, outcome, lost);
    });

    ticket
}

/// Paste an image into a terminal.
///
/// Its own function rather than a `farcooler_client_call` method because the
/// payload is megabytes of binary: routing it through the JSON boundary would
/// mean base64 in and base64 out, a third more bytes and two more copies, to
/// describe something neither Swift nor Kotlin needs to look at.
///
/// Returns a ticket. Progress arrives on `farcooler_client_poll` as
///
/// ```text
/// {"ticket": 7, "progress": {"sent": 262144, "total": 1048576}}
/// ```
///
/// and the answer, once, as the usual `{"ticket": 7, "ok": true, "result":
/// {"path": "..."}}`. The bytes are copied before this returns, so the caller
/// may free them immediately.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_paste_file(
    handle: *mut c_void,
    terminal: *const c_char,
    name: *const c_char,
    mime: *const c_char,
    data: *const u8,
    len: usize,
) -> u64 {
    let Some(h) = (unsafe { as_handle(handle) }) else { return 0 };
    let Some(terminal) = (unsafe { read_str(terminal) }) else { return 0 };
    let name = unsafe { read_str(name) }.unwrap_or_default();
    let mime = unsafe { read_str(mime) }.unwrap_or_else(|| "application/octet-stream".into());
    if data.is_null() || len == 0 {
        return 0;
    }
    let image = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();

    let ticket = h.take_ticket();
    let session = Arc::clone(&h.session);
    let finished = Arc::clone(&h.finished);

    h.runtime.spawn(async move {
        let mut guard = session.lock().await;
        let outcome = match (guard.as_mut(), terminal.parse::<uuid::Uuid>().ok()) {
            (None, _) => Err(Lost::Already),
            (Some(_), None) => {
                Err(Lost::Call(SessionError::Protocol("that is not a terminal id".into())))
            }
            (Some(session), Some(id)) => {
                let queue = Arc::clone(&finished);
                session
                    .paste_file(id, &name, &mime, &image, |sent, total| {
                        queue.lock().expect("queue").push_back(
                            json!({"ticket": ticket, "progress": {"sent": sent, "total": total}})
                                .to_string(),
                        );
                    })
                    .await
                    .map(|path| json!({ "path": path }))
                    .map_err(Lost::Call)
            }
        };

        // Same rule as `farcooler_client_call`: a dead link is every future
        // call's problem, not just this one's.
        let lost = match &outcome {
            Err(Lost::Call(e)) => e.is_disconnect(),
            Err(Lost::Already) => true,
            Ok(_) => false,
        };
        if lost {
            *guard = None;
        }
        drop(guard);

        push_call(&finished, ticket, outcome, lost);
    });

    ticket
}

/// Why a call produced no answer, kept apart from its message just long enough
/// to decide whether the session is still worth keeping.
enum Lost {
    /// There was no session to call on — either nothing ever connected, or a
    /// call before this one found the link dead and emptied the slot.
    Already,
    Call(SessionError),
}

impl Lost {
    fn message(&self) -> String {
        match self {
            Lost::Already => "not connected".to_string(),
            Lost::Call(e) => e.to_string(),
        }
    }
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
///  "public_key": "ssh-ed25519 AAAA... farcooler"}
/// ```
///
/// Returns the number of bytes needed. If that exceeds `capacity`, nothing is
/// written; call again with a larger buffer. 2048 bytes is ample.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_generate_key(
    comment: *const c_char,
    out: *mut u8,
    capacity: usize,
) -> usize {
    use russh::keys::ssh_key::{Algorithm, LineEnding, PrivateKey};

    let comment = unsafe { read_str(comment) }.unwrap_or_else(|| "farcooler".into());

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
pub unsafe extern "C" fn farcooler_client_stream_start(
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
                    json!({ "stream": key, "chunk": farcooler_core::base64::encode(&buf[..n]) }).to_string(),
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
pub unsafe extern "C" fn farcooler_client_stream_stop(
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
/// human trying to authorize it.
///
/// Returns the number of bytes needed, as `farcooler_client_generate_key` does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_public_key(
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

/// The themes compiled into this build, as JSON, with no session required.
///
/// Session-free on purpose. A phone that has never reached a host still needs
/// a theme to render with — the whole point of the built-ins being built in —
/// and every other call here needs a live ssh connection. Whatever a host adds
/// arrives separately, through the `themes` method, and is merged by the
/// client on top of these.
///
/// Writes into `out` and returns the number of bytes. If `out` is null or too
/// small, nothing is written and the needed size is returned — the same
/// contract `farcooler_client_generate_key` uses.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_builtin_themes(out: *mut u8, capacity: usize) -> usize {
    let items: Vec<Value> = farcooler_core::theme::built_in()
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "dark": t.dark,
                "background": t.background,
                "foreground": t.foreground,
                "cursor": t.cursor,
                "ansi": t.ansi,
            })
        })
        .collect();
    let payload =
        json!({ "themes": items, "default": farcooler_core::theme::DEFAULT_THEME }).to_string();

    let bytes = payload.as_bytes();
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
pub unsafe extern "C" fn farcooler_client_poll(handle: *mut c_void) -> *const c_char {
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

/// Whether there is a live session right now.
///
/// Now, not ever: `farcooler_client_call` empties the slot the moment a call
/// fails for a transport reason, so this goes false when the link dies rather
/// than staying true until the handle is freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_connected(handle: *mut c_void) -> bool {
    let Some(h) = (unsafe { as_handle(handle) }) else { return false };
    // try_lock rather than blocking: this is called from a UI thread, and a
    // call in flight holds the session for as long as the host takes to answer.
    h.session.try_lock().map(|guard| guard.is_some()).unwrap_or(true)
}

async fn dispatch(
    session: &mut Session,
    method: &str,
    args: &Value,
) -> Result<Value, SessionError> {
    let id = |key: &str| -> Result<uuid::Uuid, SessionError> {
        args.get(key)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<uuid::Uuid>().ok())
            .ok_or_else(|| SessionError::Protocol(format!("{method} needs a {key}")))
    };
    let text = |key: &str| -> String {
        args.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string()
    };
    // An absent array and an empty one are the same thing here, and that is
    // right: the adapter writer always writes all four detection arrays, so a
    // client clearing one sends `[]` and a client that never had one sends
    // nothing — both mean "no strings".
    let strings = |key: &str| -> Vec<String> {
        args.get(key)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    let numbers = |key: &str| -> Vec<u32> {
        args.get(key)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
            .unwrap_or_default()
    };

    match method {
        "fleet" => session.fleet().await,

        // What this machine is, and whether it was built from the same source
        // as the client asking.
        //
        // Exposed because the phone had no way to find out. The Mac reads it
        // from `farcooler status --json`, and a mismatch there is the thing
        // that makes a fix you already shipped go on reproducing — so a client
        // that cannot see it is a client that cannot explain itself.
        "host" => {
            let facts = session.host().await?;
            Ok(json!({
                "daemonVersion": facts.daemon_version,
                "clientVersion": farcooler_protocol::BUILD,
                "buildsMatch": facts.daemon_version == farcooler_protocol::BUILD,
                "platform": facts.platform,
                "livePanes": facts.live_terminal_count,
                "healthy": facts.self_health
                    != farcooler_protocol::v1::SelfHealth::Degraded as i32,
                // What this machine says a derived branch name starts with. The
                // client applies it, because the composer shows you the branch
                // it is about to create and a prefix added on the far side would
                // make that preview a lie.
                "branchPrefix": facts.settings
                    .as_ref()
                    .map(|s| s.branch_prefix.as_str())
                    .unwrap_or_default(),
            }))
        }

        // The host's own themes. The built-ins are compiled into each client,
        // so only what this machine adds crosses the wire.
        "themes" => {
            let items = session.themes().await?;
            Ok(json!({
                "themes": items.iter().map(|t| json!({
                    "name": t.name,
                    "dark": t.dark,
                    "background": t.background,
                    "foreground": t.foreground,
                    "cursor": t.cursor,
                    "ansi": t.ansi,
                })).collect::<Vec<_>>()
            }))
        }

        // MARK: - Machine settings
        //
        // Editing what this machine's config.toml holds. Every write answers
        // with the file's new state read back, not with what was sent, so a
        // value the writer normalized is what the caller ends up holding.

        "settings.set_branch_prefix" => {
            let host = session.set_branch_prefix(&text("prefix")).await?;
            Ok(json!({
                "branchPrefix": host.settings
                    .as_ref()
                    .map(|s| s.branch_prefix.as_str())
                    .unwrap_or_default(),
            }))
        }

        "theme.upsert" => {
            let ansi = numbers("ansi");
            if ansi.len() != 16 {
                return Err(SessionError::Protocol(
                    "a theme needs exactly sixteen ANSI colours".into(),
                ));
            }
            let themes = session
                .upsert_theme(farcooler_protocol::v1::Theme {
                    name: text("name"),
                    dark: args.get("dark").and_then(|v| v.as_bool()).unwrap_or(true),
                    background: args.get("background").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    foreground: args.get("foreground").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    cursor: args.get("cursor").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    ansi,
                })
                .await?;
            Ok(theme_json(&themes))
        }

        "theme.delete" => Ok(theme_json(&session.delete_theme(&text("name")).await?)),

        "adapters" => Ok(adapter_json(&session.adapters().await?)),

        "adapter.upsert" => {
            let adapter = wire_adapter(&text("preset"), &text("program"), &strings, args);
            Ok(adapter_json(&session.upsert_adapter(adapter).await?))
        }

        "adapter.delete" => {
            Ok(adapter_json(&session.delete_adapter(&text("preset")).await?))
        }

        "adapter.test" => {
            let adapter = wire_adapter(&text("preset"), &text("program"), &strings, args);
            let outcome = session.test_adapter(adapter).await?;
            Ok(json!({
                "ok": outcome.ok,
                "reported": outcome.reported,
                "failure": outcome.failure,
            }))
        }

        "repositories" => {
            let items = session.repositories().await?;
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
            // A worktree with nothing running in it is a directory, so the
            // manual form asks for a shell. A caller creating its own agent
            // terminal — the quick-task flow — passes an empty string, which is
            // also what an older client that never sends the key gets.
            // Adopt an existing branch rather than create one. Absent from an
            // older client, and absent means create — the behavior every caller
            // that predates the key already had.
            let adopt = args.get("adopt").and_then(|v| v.as_bool()).unwrap_or(false);
            let workspace = session
                .create_workspace(
                    id("repository")?,
                    &text("task"),
                    &text("branch"),
                    &base,
                    &text("terminal"),
                    adopt,
                )
                .await?;
            Ok(json!({ "id": uuid_of(&workspace.id).to_string() }))
        }

        "workspace.hide" => {
            session.hide_workspace(id("workspace")?).await?;
            Ok(json!({}))
        }
        "workspace.unhide" => {
            session.unhide_workspace(id("workspace")?).await?;
            Ok(json!({}))
        }

        "workspace.remove_worktree" => {
            use crate::actions::RemoveWorktreeOutcome;
            match session
                .remove_worktree(id("workspace")?, &text("confirm"))
                .await?
            {
                RemoveWorktreeOutcome::Removed => Ok(json!({ "ok": true })),
                RemoveWorktreeOutcome::ConfirmationRequired => {
                    Ok(json!({ "confirmationRequired": true }))
                }
            }
        }

        // ---- changes ----
        //
        // The review surface, reachable from a phone at last. The Mac gets all
        // of this by shelling out to `farcooler changes … --json`; a phone has
        // no CLI to shell out to, so every one of these was unreachable and the
        // whole feature was a desktop feature by accident rather than by design.

        "changes.change_set" => {
            let fresh = args.get("fresh").and_then(|v| v.as_bool()).unwrap_or(false);
            Ok(session.change_set(id("workspace")?, fresh).await?)
        }

        "changes.file_diff" => {
            let context = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            Ok(session
                .file_diff(id("workspace")?, &text("path"), &text("scope"), context)
                .await?)
        }

        "changes.commit_files" => {
            Ok(session.commit_files(id("workspace")?, &text("sha")).await?)
        }

        "changes.inbox" => Ok(session.changes_inbox().await?),

        "changes.mark_read" => {
            session.changes_mark_read(id("workspace")?).await?;
            Ok(json!({}))
        }

        "changes.set_base" => {
            Ok(session.changes_set_base(id("workspace")?, &text("baseRef")).await?)
        }

        // ---- branches, stacks and PRs ----

        "branch.list" => Ok(session.branches(id("repository")?).await?),

        "stack.get" => Ok(session.stack(id("repository")?, &text("branch")).await?),

        "pr.refresh" => Ok(session.pr_refresh(id("repository")?).await?),

        // ---- the machine itself ----

        "daemon.version" => Ok(session.daemon_capabilities().await?),

        "host.health" => {
            let host = session.host().await?;
            let degraded =
                host.self_health == farcooler_protocol::v1::SelfHealth::Degraded as i32;
            Ok(json!({
                "platform": host.platform,
                "daemonVersion": host.daemon_version,
                "protocolVersion": host.protocol_version,
                "healthy": !degraded,
                // Why it is degraded, in the daemon's own words. Shown rather
                // than summarized: the client cannot know which of these
                // matters, and "something is wrong" is not an actionable
                // sentence.
                "reasons": host.self_health_reasons,
                "livePanes": host.live_terminal_count,
            }))
        }

        "repository_root.list" => {
            let roots = session.roots().await?;
            Ok(json!({
                "roots": roots.iter().map(|r| json!({
                    "id": uuid_of(&r.id).to_string(),
                    // Absent unless this client holds `host_admin`; a phone with
                    // read scope sees the root exists without seeing where it is.
                    "displayPath": r.display_path,
                })).collect::<Vec<_>>()
            }))
        }

        "repository_root.remove" => {
            session.remove_repository_root(id("root")?).await?;
            Ok(json!({}))
        }

        "repository_root.add" => {
            let root = session
                .add_repository_root(&text("path"))
                .await?;
            Ok(json!({
                "id": uuid_of(&root.id).to_string(),
                "displayPath": root.display_path,
            }))
        }

        "repository.register" => {
            let repo = session
                .register_repository(&text("path"))
                .await?;
            Ok(json!({
                "id": uuid_of(&repo.id).to_string(),
                "displayName": repo.display_name,
            }))
        }

        "terminal.create" => {
            let terminal = session
                .create_terminal(
                    id("workspace")?,
                    &text("title"),
                    &text("preset"),
                    args.get("tile").and_then(|v| v.as_bool()).unwrap_or(false),
                )
                .await?;
            Ok(json!({ "id": uuid_of(&terminal.id).to_string() }))
        }
        "terminal.stop" => {
            session.stop_terminal(id("terminal")?).await?;
            Ok(json!({}))
        }
        "terminal.restart" => {
            session.restart_terminal(id("terminal")?).await?;
            Ok(json!({}))
        }
        "terminal.dismiss_lost" => {
            session.dismiss_lost(id("terminal")?).await?;
            Ok(json!({}))
        }
        // Answers `{}` for the reason the agent calls below do: what a client
        // redraws from is the fleet it polls, or the pushed change, not an echo
        // of the row taken at the instant of the write.
        "terminal.seen" => {
            session.mark_seen(id("terminal")?).await?;
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
                session.screen(id("terminal")?, known).await?;
            Ok(json!({
                // Absent when unchanged, so an idle pane costs a few bytes on
                // the wire instead of a whole capture several times a second.
                "contents": farcooler_core::base64::encode(&screen.contents),
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
            let bytes = decode_hex(&hex)
                .ok_or_else(|| SessionError::Protocol("input must be hex".into()))?;
            session.write(id("terminal")?, bytes).await?;
            Ok(json!({}))
        }

        "terminal.resize" => {
            let columns = args.get("columns").and_then(|v| v.as_u64()).unwrap_or(80) as u32;
            let rows = args.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u32;
            session
                .resize_terminal(id("terminal")?, columns, rows)
                .await?;
            Ok(json!({}))
        }

        // ---- agent channel ----
        //
        // The mutating calls discard the `Terminal` the daemon hands back and
        // answer `{}`, the same as `terminal.stop` and `terminal.restart`: the
        // pushed `terminal_changed` event is what a client actually redraws
        // from, so echoing the row here would be a second, staler copy of it.
        "terminal.set_pane_mode" => {
            let mode = match text("paneMode").as_str() {
                "agent" => farcooler_protocol::v1::PaneMode::Agent,
                "terminal" => farcooler_protocol::v1::PaneMode::Terminal,
                // Refused rather than guessed at: a client asking to switch to
                // a mode that does not exist has a bug worth surfacing, not a
                // default worth picking for it.
                other => {
                    return Err(SessionError::Protocol(format!("unknown pane mode: {other}")));
                }
            };
            let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
            session
                .set_pane_mode(id("terminal")?, mode, force)
                .await?;
            Ok(json!({}))
        }

        // Answers `{"events": [{"seq": ..., "payloadJson": "..."}]}` — a
        // client's whole cursor state is `fromSeq`, taken from the highest
        // `seq` it has already applied, never a counter kept beside this call.
        "terminal.agent_subscribe" => {
            let from_seq = args.get("fromSeq").and_then(|v| v.as_u64()).unwrap_or(0);
            let batch = session
                .agent_subscribe(id("terminal")?, from_seq, args.get("epoch").and_then(|v| v.as_u64()).unwrap_or(0))
                .await?;
            Ok(json!({
                // The epoch goes back with the batch, not just in with the
                // request. A client that cannot see it change cannot know its
                // cursor has stopped meaning anything — which is the bug that
                // took four attempts to fix on the Mac, and this is the path
                // the phone uses.
                "epoch": batch.epoch,
                "events": batch.events.iter().map(|e| json!({
                    "seq": e.seq,
                    "payloadJson": e.payload_json,
                })).collect::<Vec<_>>()
            }))
        }

        "terminal.agent_prompt" => {
            // `[{ "mime": "image/png", "base64": "..." }]`, decoded here so the
            // protocol carries bytes and only this boundary deals in text.
            let images: Vec<(String, Vec<u8>)> = args
                .get("images")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let mime = item.get("mime")?.as_str()?.to_string();
                            let data =
                                farcooler_core::base64::decode(item.get("base64")?.as_str()?)?;
                            Some((mime, data))
                        })
                        .collect()
                })
                .unwrap_or_default();
            session
                .agent_prompt(id("terminal")?, &text("text"), &images)
                .await?;
            Ok(json!({}))
        }

        "terminal.agent_answer" => {
            session
                .agent_answer(id("terminal")?, &text("requestId"), &text("optionId"))
                .await?;
            Ok(json!({}))
        }

        "terminal.agent_set_mode" => {
            session
                .agent_set_mode(id("terminal")?, &text("mode"))
                .await?;
            Ok(json!({}))
        }

        "terminal.agent_edit_queued" => {
            session
                .agent_edit_queued(id("terminal")?, &text("queuedId"), &text("text"))
                .await?;
            Ok(json!({}))
        }

        "terminal.agent_cancel_queued" => {
            session
                .agent_cancel_queued(id("terminal")?, &text("queuedId"))
                .await?;
            Ok(json!({}))
        }

        "terminal.agent_steer_queued" => {
            session
                .agent_steer_queued(id("terminal")?, &text("queuedId"))
                .await?;
            Ok(json!({}))
        }

        "terminal.agent_cancel" => {
            session.agent_cancel(id("terminal")?).await?;
            Ok(json!({}))
        }

        "worktree.file_search" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
            let paths = session
                .search_worktree_files(id("workspace")?, &text("query"), limit)
                .await?;
            Ok(json!({ "paths": paths }))
        }

        // Refused rather than defaulted, so a typo in a client is a visible
        // error instead of a call that silently does nothing.
        other => Err(SessionError::Protocol(format!("unknown method: {other}"))),
    }
}

/// Themes as every client already decodes them, so a write's answer and
/// `themes`'s answer are the same shape.
fn theme_json(items: &[farcooler_protocol::v1::Theme]) -> Value {
    json!({
        "themes": items.iter().map(|t| json!({
            "name": t.name,
            "dark": t.dark,
            "background": t.background,
            "foreground": t.foreground,
            "cursor": t.cursor,
            "ansi": t.ansi,
        })).collect::<Vec<_>>()
    })
}

/// Adapters, with the origin as a word rather than an enum number.
///
/// A word because the clients are Swift and Kotlin and neither has the
/// generated enum: a number here would be three copies of a mapping table, and
/// the third one would be wrong.
fn adapter_json(items: &[farcooler_protocol::v1::Adapter]) -> Value {
    json!({
        "adapters": items.iter().map(|a| json!({
            "preset": a.preset,
            "program": a.program,
            "args": a.args,
            "env": a.env,
            "commands": a.commands,
            "identity": a.identity,
            "blocked": a.blocked,
            "working": a.working,
            "origin": match farcooler_protocol::v1::AdapterOrigin::try_from(a.origin) {
                Ok(farcooler_protocol::v1::AdapterOrigin::BuiltIn) => "builtIn",
                Ok(farcooler_protocol::v1::AdapterOrigin::Override) => "override",
                Ok(farcooler_protocol::v1::AdapterOrigin::User) => "user",
                _ => "unknown",
            },
            // Derived here rather than left to each client: an adapter with no
            // program is a recognized agent Far Cooler cannot host as a chat,
            // which is a real state and the one thing a picker has to know.
            "chatCapable": !a.program.is_empty(),
        })).collect::<Vec<_>>()
    })
}

/// One adapter out of a client's JSON.
fn wire_adapter(
    preset: &str,
    program: &str,
    strings: &dyn Fn(&str) -> Vec<String>,
    args: &Value,
) -> farcooler_protocol::v1::Adapter {
    farcooler_protocol::v1::Adapter {
        preset: preset.to_string(),
        program: program.to_string(),
        args: strings("args"),
        env: args
            .get("env")
            .and_then(|v| v.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        commands: strings("commands"),
        identity: strings("identity"),
        blocked: strings("blocked"),
        working: strings("working"),
        // Set by the daemon on the way back out; a client claiming one would be
        // claiming something only the daemon can know.
        origin: 0,
        // Unlike `origin`, this IS the client's to say: it is what the form is
        // configured for, and it is what `adapter.test` has to exercise.
        // Absent means acp, which is what every adapter that says nothing gets.
        backend: args
            .get("backend")
            .and_then(|v| v.as_str())
            .map(|name| {
                farcooler_core::activity::AdapterBackend::parse(name).to_proto() as i32
            })
            .unwrap_or_default(),
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

/// The same envelope, plus the one thing a client cannot work out from the
/// message: whether the link is gone.
///
/// Both phone apps recover meaning from error strings by matching substrings.
/// That is the right call on the connect path, where the message genuinely is
/// all there is, and the wrong thing to extend to every call on a live
/// session. Rust still has the type at the moment the error is produced, so it
/// is answered once here instead of guessed separately in Swift and Kotlin.
fn push_call(
    queue: &Arc<Mutex<VecDeque<String>>>,
    ticket: u64,
    outcome: Result<Value, Lost>,
    lost: bool,
) {
    let payload = match outcome {
        Ok(value) => json!({ "ticket": ticket, "ok": true, "result": value }),
        Err(reason) => json!({
            "ticket": ticket,
            "ok": false,
            "error": reason.message(),
            "disconnected": lost,
        }),
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
    fn a_call_on_a_handle_that_never_connected_reports_a_dropped_link() {
        // The hole this closes: the first call to notice a dead link is
        // whichever one the user happened to make, and it empties the session
        // slot on its way out. Every call after it lands here — so if this
        // said nothing, the poll that follows would be told "not connected"
        // with no way to tell it apart from a request being refused, and a
        // client whose only detector is that poll would sit at "connected"
        // forever with everything failing.
        let handle = farcooler_client_new();
        let ticket = unsafe { farcooler_client_call(handle, c"fleet".as_ptr(), c"{}".as_ptr()) };
        assert_ne!(ticket, 0);

        // The call is answered on the runtime's thread, so wait for it rather
        // than assuming it has landed by now.
        let answer = loop {
            if let Some(line) = unsafe { farcooler_client_poll(handle).as_ref() } {
                break unsafe { CStr::from_ptr(line) }.to_str().unwrap().to_string();
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };

        assert!(answer.contains("\"ok\":false"));
        assert!(answer.contains("\"disconnected\":true"), "got {answer}");
        assert!(!unsafe { farcooler_client_connected(handle) });
        unsafe { farcooler_client_free(handle) };
    }

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
        let handle = farcooler_client_new();
        assert!(!handle.is_null());

        let h = unsafe { as_handle(handle) }.unwrap();
        push(&h.finished, 1, Ok(json!({"a": 1})));
        push(&h.finished, 2, Err("nope".into()));

        let first = unsafe { farcooler_client_poll(handle) };
        let first = unsafe { CStr::from_ptr(first) }.to_str().unwrap().to_string();
        assert!(first.contains("\"ticket\":1") && first.contains("\"ok\":true"));

        let second = unsafe { farcooler_client_poll(handle) };
        let second = unsafe { CStr::from_ptr(second) }.to_str().unwrap().to_string();
        assert!(second.contains("\"ticket\":2") && second.contains("\"ok\":false"));
        assert!(second.contains("nope"));

        // Drained.
        assert!(unsafe { farcooler_client_poll(handle) }.is_null());
        unsafe { farcooler_client_free(handle) };
    }

    #[test]
    fn a_generated_key_is_a_usable_openssh_pair() {
        let mut buffer = vec![0u8; 4096];
        let n = unsafe {
            farcooler_client_generate_key(
                c"farcooler-test".as_ptr(),
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
        assert!(public.ends_with("farcooler-test"), "the comment identifies the device");

        // The real assertion: the private key we hand out is one our own SSH
        // client can load. A key that generates but cannot be used would only
        // fail at the first connection attempt.
        assert!(crate::ssh::decode_key_for_test(private).is_ok());
    }

    #[test]
    fn two_generated_keys_differ() {
        let mut a = vec![0u8; 4096];
        let mut b = vec![0u8; 4096];
        let na = unsafe { farcooler_client_generate_key(std::ptr::null(), a.as_mut_ptr(), a.len()) };
        let nb = unsafe { farcooler_client_generate_key(std::ptr::null(), b.as_mut_ptr(), b.len()) };
        assert_ne!(a[..na], b[..nb]);
    }

    #[test]
    fn generating_into_a_short_buffer_reports_the_size_and_writes_nothing() {
        let mut tiny = [0u8; 4];
        let needed = unsafe {
            farcooler_client_generate_key(std::ptr::null(), tiny.as_mut_ptr(), tiny.len())
        };
        assert!(needed > tiny.len());
        assert_eq!(tiny, [0, 0, 0, 0], "a truncated private key would be worse than none");
    }

    #[test]
    fn null_arguments_are_survivable() {
        // A UI bug must not take down the app.
        let null = std::ptr::null_mut();
        unsafe {
            assert_eq!(farcooler_client_connect(null, std::ptr::null()), 0);
            assert_eq!(farcooler_client_call(null, std::ptr::null(), std::ptr::null()), 0);
            assert!(farcooler_client_poll(null).is_null());
            assert!(!farcooler_client_connected(null));
            farcooler_client_free(null);
        }

        let handle = farcooler_client_new();
        unsafe {
            assert_eq!(farcooler_client_call(handle, std::ptr::null(), std::ptr::null()), 0);
            farcooler_client_free(handle);
        }
    }
}


fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
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
    fn base64_survives_bytes_that_are_not_text() {
        // Kept after the local encoder was deleted in favour of
        // `farcooler_core::base64`: the shared module proves it matches the
        // RFC, and this proves the thing THIS crate depends on — a screen is
        // escape sequences and high bytes, not a string.
        assert_eq!(farcooler_core::base64::encode(&[0x1b, 0x5b, 0x33, 0x31, 0x6d]), "G1szMW0=");
        assert_eq!(farcooler_core::base64::encode(&[0xff, 0x00, 0xfe]), "/wD+");
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
            super::farcooler_client_generate_key(comment.as_ptr(), buf.as_mut_ptr(), buf.len())
        };
        assert!(n > 0 && n <= buf.len());

        let pair: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
        let private = pair["private_key"].as_str().unwrap();
        let generated = pair["public_key"].as_str().unwrap();

        let key = CString::new(private).unwrap();
        let mut out = vec![0u8; 2048];
        let n = unsafe {
            super::farcooler_client_public_key(key.as_ptr(), out.as_mut_ptr(), out.len())
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
            unsafe { super::farcooler_client_public_key(junk.as_ptr(), out.as_mut_ptr(), out.len()) },
            0
        );
    }
}
