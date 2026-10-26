//! How soon a session notices a far end that went quiet, and what an idle one
//! sends while it waits (ov-781).
//!
//! A real russh server, in this process, on one end of an in-memory pipe; the
//! client on the other, through a valve that can stop every byte in both
//! directions the way a dead tunnel does: what the client writes vanishes
//! into a socket that still accepts it, and nothing ever comes back. No reset,
//! no EOF. That is the case the keepalive exists for, and the one a runner
//! crash or a relay drop produces on the tunnel.
//!
//! tokio's clock is paused, so the minutes these tests watch cost no wall
//! time and every timer fires exactly when it is due. The session is opened
//! through `Session::open_over`, which picks its settings from the
//! destination's reach exactly as `Session::open` does; only the stream is
//! this file's.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use russh::keys::ssh_key::{self, Algorithm, LineEnding, PrivateKey};
use russh::server::{self, Auth};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use tokio::time::Instant;

use super::{Destination, HostKeyPolicy, Reach, Session};
use farcooler_ceremony::Liveness;

/// Bytes each way, and the switch that kills the link.
#[derive(Default)]
struct Wire {
    sent: AtomicU64,
    received: AtomicU64,
    dead: AtomicBool,
}

impl Wire {
    fn bytes(&self) -> (u64, u64) {
        (self.sent.load(Ordering::SeqCst), self.received.load(Ordering::SeqCst))
    }
}

/// The client's end of the pipe, as a tunnel's socket behaves.
struct Valve {
    inner: DuplexStream,
    wire: Arc<Wire>,
}

impl AsyncRead for Valve {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.wire.dead.load(Ordering::SeqCst) {
            // Nothing will ever arrive, and nothing says so.
            return Poll::Pending;
        }
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        let read = (buf.filled().len() - before) as u64;
        self.wire.received.fetch_add(read, Ordering::SeqCst);
        polled
    }
}

impl AsyncWrite for Valve {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.wire.dead.load(Ordering::SeqCst) {
            // Accepted and lost, as a socketpair into a dead tunnel does.
            return Poll::Ready(Ok(buf.len()));
        }
        let polled = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &polled {
            self.wire.sent.fetch_add(*n as u64, Ordering::SeqCst);
        }
        polled
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A server that admits any key and answers a keepalive as OpenSSH does.
struct Runner;

impl server::Handler for Runner {
    type Error = russh::Error;

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        _key: &ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        _user: &str,
        _key: &ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }
}

fn random_key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("generate a key")
}

/// An authenticated session on `reach`, and the wire under it.
async fn connected(reach: Reach) -> (Session, Arc<Wire>) {
    let (client_end, server_end) = tokio::io::duplex(1 << 16);
    let config = Arc::new(server::Config {
        keys: vec![random_key()],
        // The server keeps its own counsel: no timer of its own may end the
        // session and be mistaken for the client's.
        inactivity_timeout: None,
        auth_rejection_time: Duration::ZERO,
        ..server::Config::default()
    });
    tokio::spawn(async move {
        if let Ok(running) = server::run_stream(config, server_end, Runner).await {
            let _ = running.await;
        }
    });

    let wire = Arc::new(Wire::default());
    let destination = Destination {
        reach,
        user: "u".into(),
        private_key: random_key().to_openssh(LineEnding::LF).expect("encode").to_string(),
        passphrase: None,
        host_key: HostKeyPolicy::Accept,
        derp_map: String::new(),
    };
    let valve = Valve { inner: client_end, wire: Arc::clone(&wire) };
    let session = match Session::open_over(&destination, valve).await {
        Ok(session) => session,
        Err(e) => panic!("the in-process server should admit the session: {e}"),
    };
    // Let the last of the handshake's bytes land before anything is counted.
    tokio::time::sleep(Duration::from_millis(1)).await;
    (session, wire)
}

fn tunnel() -> Reach {
    Reach::Tailcat { token: "tc-x".into(), client_key: String::new() }
}

fn direct() -> Reach {
    Reach::Direct { host: "box".into(), port: 22 }
}

/// How long until this session's link is over, watched in 100 ms steps, or
/// `None` if it outlives `limit`.
async fn ended_after(session: &Session, limit: Duration) -> Option<Duration> {
    let began = Instant::now();
    while began.elapsed() < limit {
        if session.handle.is_closed() {
            return Some(began.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

/// The card's first line: a tunnel that dies mid-session is noticed within
/// ten seconds, so the apps' redial starts then and not two minutes later.
///
/// Bounded below as well as above: the session must end because its probes
/// went unanswered, after the second one was given its chance, and not
/// because something else in this file closed it at once.
#[tokio::test(start_paused = true)]
async fn a_dead_tunnel_is_noticed_within_ten_seconds() {
    let (session, wire) = connected(tunnel()).await;
    assert!(!session.handle.is_closed(), "the session ended before anything happened");

    wire.dead.store(true, Ordering::SeqCst);
    let ended = ended_after(&session, Duration::from_secs(300))
        .await
        .expect("a session over a dead tunnel was still open after five minutes");
    eprintln!("a dead tunnel was noticed {ended:?} after it died");

    let tunnel = Liveness::TUNNEL;
    assert!(
        ended <= Duration::from_secs(10),
        "a dead tunnel went unnoticed for {ended:?}; the redial must start within 10 s"
    );
    assert!(
        ended >= tunnel.probe_after * tunnel.unanswered as u32,
        "the session ended after {ended:?}, before its probes had gone unanswered"
    );
}

/// The card's second line: an idle DIRECT session sends nothing it did not
/// send before this change. Nothing at all for the first 30 s, then one probe
/// and its answer, as it always had.
#[tokio::test(start_paused = true)]
async fn an_idle_direct_session_sends_nothing_new() {
    let (session, wire) = connected(direct()).await;
    let start = wire.bytes();

    tokio::time::sleep(Duration::from_millis(29_900)).await;
    assert_eq!(
        wire.bytes(),
        start,
        "an idle direct session sent or received bytes within its first 30 s"
    );

    // And the probe it always had is still there: this test can see one,
    // so the silence above is not a clock that never ran.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (sent, received) = wire.bytes();
    assert!(sent > start.0 && received > start.1, "no probe at 30 s on a direct session");
    assert!(!session.handle.is_closed());
}

/// What the tunnel's probe costs while idle, measured at the SSH layer: one
/// probe and its answer every 3 s, and nothing else. Printed for the report.
///
/// Bytes on the wire are more: each SSH packet rides a TCP segment inside
/// WireGuard inside the relay's own TCP, and the report adds that up.
#[tokio::test(start_paused = true)]
async fn an_idle_tunnel_probes_every_three_seconds_and_nothing_more() {
    let (session, wire) = connected(tunnel()).await;
    let start = wire.bytes();

    tokio::time::sleep(Duration::from_millis(3_050)).await;
    let first = wire.bytes();
    let probe = (first.0 - start.0, first.1 - start.1);
    assert!(probe.0 > 0 && probe.1 > 0, "no probe and answer at 3 s on the tunnel");

    tokio::time::sleep(Duration::from_secs(57)).await;
    let minute = wire.bytes();
    let spent = (minute.0 - start.0, minute.1 - start.1);
    eprintln!(
        "an idle tunnel's minute: {} probes, {} bytes sent and {} received at the SSH layer \
         ({} and {} per probe)",
        spent.0 / probe.0,
        spent.0,
        spent.1,
        probe.0,
        probe.1
    );
    assert_eq!(spent, (probe.0 * 20, probe.1 * 20), "not one probe and one answer every 3 s");
    assert!(!session.handle.is_closed(), "an answered probe ended the session");
}
