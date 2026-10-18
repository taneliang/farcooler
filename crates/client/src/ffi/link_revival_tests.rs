//! A phone back from a pocket learns quickly whether its link lives, and a new
//! link answers before its event channel is open (ov-613).
//!
//! Against a stand-in runner on a Unix socket, served by the real
//! `serve_connection`, so only the client is under test. The stand-in can
//! leave `daemon.version` unanswered, which is what a half-open link looks
//! like from the phone: the request goes out and nothing comes back.

use std::sync::Arc;
use std::time::{Duration, Instant};

use farcooler_protocol::v1::{self as pb, Request, Response, Scope, response, result};
use farcooler_transport::{Connection, HandshakeConfig, Handler, Peer, serve_connection};

use super::concurrency_tests::{answer_for, call};
use super::*;

/// A runner that answers everything at once, unless told to leave the
/// probe's question unanswered.
struct Runner {
    silent: bool,
}

impl Handler for Runner {
    fn peer(&self) -> Peer {
        Peer { client_id: None, scope: Scope::HostAdmin }
    }

    async fn handle(&self, req: Request) -> Response {
        if self.silent && req.method == "daemon.version" {
            std::future::pending::<()>().await;
        }
        Response {
            request_id: req.request_id,
            outcome: Some(response::Outcome::Result(pb::Result {
                value: Some(result::Value::Empty(pb::Empty {})),
            })),
        }
    }
}

/// Serve every connection to `socket` as `Runner { silent }`, the second one
/// (the event channel `subscribe` opens) only once `second` is released.
fn serve(listener: tokio::net::UnixListener, silent: bool, second: Arc<tokio::sync::Notify>) {
    serve_as(listener, silent, 2, second, "stand-in");
}

/// The same, holding the `held`th connection until `hold` is released, and
/// saying it is `version` in its hello.
fn serve_as(
    listener: tokio::net::UnixListener, silent: bool, held: usize, hold: Arc<tokio::sync::Notify>,
    version: &'static str,
) {
    tokio::spawn(async move {
        let mut accepted = 0;
        while let Ok((stream, _)) = listener.accept().await {
            accepted += 1;
            let hold = (accepted == held).then(|| Arc::clone(&hold));
            tokio::spawn(async move {
                if let Some(hold) = hold {
                    hold.notified().await;
                }
                let (read, write) = stream.into_split();
                let mut conn = Connection::new(read, write);
                let cfg = HandshakeConfig { daemon_version: version.into() };
                let _ = serve_connection(&mut conn, &cfg, &Runner { silent }).await;
            });
        }
    });
}

/// A handle with a session to a `Runner`, opened by the test.
fn handle_on(silent: bool) -> (*mut c_void, tempfile::TempDir) {
    let handle = farcooler_client_new();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("runner.sock");
    let h = unsafe { as_handle(handle) }.unwrap();
    let session = h.runtime.block_on(async {
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        let open = Arc::new(tokio::sync::Notify::new());
        open.notify_one();
        serve(listener, silent, open);
        Session::connect_local(&socket).await.expect("connect")
    });
    h.put_session(session);
    (handle, dir)
}

/// **`connect` answers while the event channel is still opening.** The
/// stand-in holds the second connection, the one `subscribe` makes, until
/// the connect line has arrived, so a connect that waited for the
/// subscription is never answered.
#[test]
fn connect_answers_before_subscribe_finishes() {
    let handle = farcooler_client_new();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("runner.sock");
    let second = Arc::new(tokio::sync::Notify::new());
    let h = unsafe { as_handle(handle) }.unwrap();
    h.runtime.block_on(async {
        serve(tokio::net::UnixListener::bind(&socket).expect("bind"), false, Arc::clone(&second));
    });

    let config = std::ffi::CString::new(json!({ "socket": socket.display().to_string() }).to_string()).unwrap();
    let ticket = unsafe { farcooler_client_connect(handle, config.as_ptr()) };
    // A bound for a hang: an answer that waits on the held channel never comes.
    let answer = answer_for(handle, ticket, Instant::now() + Duration::from_secs(60))
        .expect("connect waited for the event channel");
    assert!(answer.contains("\"ok\":true"), "got {answer}");
    assert!(unsafe { farcooler_client_connected(handle) }, "answered before the slot was filled");
    assert!(!unsafe { farcooler_client_events_live(handle) }, "the channel is still held");

    // The channel still opens once the runner lets it, so nothing was traded
    // away for the early answer.
    second.notify_one();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !unsafe { farcooler_client_events_live(handle) } {
        assert!(Instant::now() < deadline, "the event channel never opened");
        std::thread::sleep(Duration::from_millis(5));
    }
    // And the channel's first word is a re-read, for whatever changed while
    // it opened, which nothing could push.
    let first = unsafe { farcooler_client_next_event(handle).as_ref() }
        .map(|line| unsafe { CStr::from_ptr(line) }.to_str().unwrap().to_string());
    assert_eq!(first.as_deref(), Some(r#"{"event":"resync"}"#));
    unsafe { farcooler_client_free(handle) };
    drop(dir);
}

/// **A connect that finishes after a newer one leaves the slot to the newer
/// one** (ov-613 review, M3). The app stopped waiting on it when it dialed
/// again; it is answered, as superseded, and its session is dropped.
#[test]
fn a_stale_connect_does_not_take_the_slot() {
    let handle = farcooler_client_new();
    let dir = tempfile::tempdir().unwrap();
    let (old, new) = (dir.path().join("old.sock"), dir.path().join("new.sock"));
    let hold = Arc::new(tokio::sync::Notify::new());
    let h = unsafe { as_handle(handle) }.unwrap();
    h.runtime.block_on(async {
        serve_as(tokio::net::UnixListener::bind(&old).unwrap(), false, 1, Arc::clone(&hold), "old");
        serve_as(tokio::net::UnixListener::bind(&new).unwrap(), false, 0, Arc::new(tokio::sync::Notify::new()), "new");
    });
    let connect = |socket: &std::path::Path| {
        let config = std::ffi::CString::new(json!({ "socket": socket.display().to_string() }).to_string()).unwrap();
        unsafe { farcooler_client_connect(handle, config.as_ptr()) }
    };
    let stale = connect(&old);
    let fresh = connect(&new);
    let answer = answer_for(handle, fresh, Instant::now() + Duration::from_secs(60)).expect("the new connect");
    assert!(answer.contains("\"ok\":true"), "got {answer}");
    hold.notify_one();
    let answer = answer_for(handle, stale, Instant::now() + Duration::from_secs(60)).expect("the old connect");
    assert!(answer.contains("\"trouble\":\"superseded\""), "got {answer}");
    let in_slot = locked(&h.session).as_ref().map(|s| s.daemon_version().to_string());
    assert_eq!(in_slot.as_deref(), Some("new"));
    unsafe { farcooler_client_free(handle) };
}

/// **A probe nobody answers ends the link** and says so, where any other
/// call that times out leaves the session in its slot.
#[test]
fn a_silent_probe_empties_the_slot_and_reports_a_drop() {
    let (handle, _dir) = handle_on(true);
    let ticket = call(handle, "probe", json!({}));
    let answer = answer_for(handle, ticket, Instant::now() + Duration::from_secs(60)).expect("the probe answered");
    let line: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(line["ok"], false, "{line}");
    assert_eq!(line["disconnected"], true, "{line}");
    assert!(!unsafe { farcooler_client_connected(handle) }, "the dead session is still in the slot");
    unsafe { farcooler_client_free(handle) };
}

/// A healthy link answers the probe, and keeps its session.
#[test]
fn a_probe_on_a_healthy_link_answers() {
    let (handle, _dir) = handle_on(false);
    let ticket = call(handle, "probe", json!({}));
    let answer = answer_for(handle, ticket, Instant::now() + Duration::from_secs(60)).expect("the probe answered");
    assert!(answer.contains("\"ok\":true"), "got {answer}");
    assert!(unsafe { farcooler_client_connected(handle) });
    unsafe { farcooler_client_free(handle) };
}

/// **On a paused clock, a silent link turns a probe into `timed_out` at the
/// probe's deadline**: not at the ordinary 30 s, and not at the keepalive's
/// two minutes. Connected in real time, then paused, so the handshake's own
/// clock never jumps.
#[tokio::test]
async fn a_silent_link_times_the_probe_out_at_its_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("runner.sock");
    let open = Arc::new(tokio::sync::Notify::new());
    serve(tokio::net::UnixListener::bind(&socket).expect("bind"), true, open);
    let session = Session::connect_local(&socket).await.expect("connect");

    tokio::time::pause();
    let began = tokio::time::Instant::now();
    let outcome = session.probe().await;
    let waited = began.elapsed();
    assert!(matches!(outcome, Err(SessionError::TimedOut { .. })), "{outcome:?}");
    assert!(
        waited >= crate::deadlines::PROBE && waited < crate::deadlines::ORDINARY,
        "the probe waited {waited:?}"
    );
}
