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
//! - The pinned runner connects (with an Ed25519 key standing in for the
//!   passkey, which this server cannot verify; see
//!   `a_passkey_signs_in_to_a_real_sshd.rs`), lists its
//!   terminals, and a pane's input goes on the control channel while its
//!   output arrives on a channel of its own (`crates/client/src/session.rs`'s
//!   layout).

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use farcooler_ceremony::{Reach, RunnerEntry};
use farcooler_web::enroll;
use farcooler_web::error::WebError;
use farcooler_web::ssh;
use farcooler_web::wire::Runner;
use support::{BROWSER_KEY, Ed25519InMemory, HOST_KEY, NODE_KEY, OTHER_KEY, Seen, TERMINAL, fingerprint, key, line};

/// A runner holding `host_key`, admitting only the browser's key, on one end
/// of an in-memory pipe. The other end is what a Tailcat connection would be.
fn runner(host_key: &str) -> (tokio::io::DuplexStream, Arc<Seen>) {
    let (browser_side, runner_side) = tokio::io::duplex(1 << 16);
    let seen = Arc::new(Seen::default());
    support::serve(runner_side, host_key, Some(key(BROWSER_KEY).public_key().clone()), seen.clone());
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
    let manifest = enroll::accept(&reply, &offer, false, 0.0, 0.0).expect("the browser accepts its own reply");
    enroll::granted(&manifest).remove(0)
}

async fn dial(stream: tokio::io::DuplexStream, granted: &enroll::Granted, client_key: &str) -> Result<Runner, WebError> {
    let mut signer = Ed25519InMemory(key(client_key));
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
    let granted = enroll::granted(&enroll::accept(&reply, &offer, false, 0.0, 0.0).unwrap());
    let why: Vec<(&str, &str, &str)> =
        granted.iter().map(|g| (g.id.as_str(), g.token.as_str(), g.why.as_str())).collect();
    assert_eq!(
        why,
        vec![("tunneled", "tc-token", ""), ("pending", "", "pending"), ("address", "", "address_only")]
    );
}
