//! How the page notices a tunnel that died without a word (ov-772).
//!
//! A runner restart, or another device being revoked, drops the tunnel's
//! relay state and sends this browser nothing: its TCP lives inside the
//! tunnel, so no reset ever arrives and the terminal just stops. The native
//! client has russh's keepalive for this, driven by the numbers in
//! [`farcooler_ceremony::Liveness`] (`Reach::liveness`, ov-781). russh's timer
//! is tokio's, which a browser does not have, so the page runs the same rule on
//! its own clock: [`Watch`] is that rule, and [`Link::run`] is the loop that
//! drives it.
//!
//! **One definition of the numbers.** `Watch` reads `Liveness::TUNNEL`; it
//! never spells 3 seconds or 2 probes itself, so changing the tunnel's setting
//! moves the phones, the Mac and the browser together. Its counting is
//! russh's (`client/mod.rs`, the keepalive arm): the timer fires every
//! `probe_after` of quiet, each firing counts one unanswered probe, and the
//! firing that takes the count past `unanswered` ends the link. Any byte
//! received resets both.
//!
//! **Time is handed in.** [`Watch`] takes the current time as an argument and
//! [`Link`] reads and sleeps through a [`Clock`], so a test moves time by hand
//! and nothing in here depends on how fast the machine running it is.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use farcooler_ceremony::Liveness;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// What a link should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Nothing is due; look again in this many milliseconds.
    Wait(u64),
    /// Send a probe that asks for an answer, then look again in
    /// `Liveness::probe_after`.
    Probe,
    /// More probes have gone unanswered than `Liveness::unanswered` allows.
    Dead,
}

/// The quiet-link rule, with no clock of its own.
#[derive(Debug, Clone)]
pub struct Watch {
    liveness: Liveness,
    /// When something was last received, or when the link was opened.
    heard_at: u64,
    /// Probes sent since, none of them answered.
    unanswered: usize,
}

impl Watch {
    /// A link that opened at `now` (milliseconds on the caller's clock).
    pub fn new(liveness: Liveness, now: u64) -> Self {
        Self { liveness, heard_at: now, unanswered: 0 }
    }

    /// Something arrived. Resets the clock and the count: a link carrying
    /// output sends no probes at all.
    pub fn heard(&mut self, now: u64) {
        self.heard_at = now;
        self.unanswered = 0;
    }

    /// What is due at `now`. A `Probe` answer has been counted: asking again
    /// at the same instant says `Wait`.
    pub fn step(&mut self, now: u64) -> Step {
        let every = self.liveness.probe_after.as_millis() as u64;
        let due = self.heard_at + every * (self.unanswered as u64 + 1);
        if now < due {
            return Step::Wait(due - now);
        }
        self.unanswered += 1;
        if self.unanswered > self.liveness.unanswered { Step::Dead } else { Step::Probe }
    }
}

/// Time, as the link reads and waits on it.
pub trait Clock: Send + Sync + 'static {
    /// Milliseconds since some fixed point. Only differences matter.
    fn now_ms(&self) -> u64;
    /// Resolve after `ms` milliseconds.
    fn sleep(&self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// The page's own time: `Date.now` and `setTimeout` in a browser, and the
/// process's monotonic clock and tokio's timer natively.
pub struct PageClock;

impl Clock for PageClock {
    fn now_ms(&self) -> u64 {
        #[cfg(target_arch = "wasm32")]
        {
            js_sys::Date::now() as u64
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
            START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
        }
    }

    fn sleep(&self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let _ = farcooler_transport::rt::timeout(Duration::from_millis(ms), std::future::pending::<()>()).await;
        })
    }
}

/// The liveness of one connection: what the page has heard from the runner,
/// and whether it has given up on it.
pub struct Link {
    clock: Arc<dyn Clock>,
    watch: Mutex<Watch>,
    lost: AtomicBool,
    lost_tx: tokio::sync::watch::Sender<bool>,
}

impl Link {
    pub fn new(clock: Arc<dyn Clock>, liveness: Liveness) -> Arc<Self> {
        let now = clock.now_ms();
        let (lost_tx, _) = tokio::sync::watch::channel(false);
        Arc::new(Self { clock, watch: Mutex::new(Watch::new(liveness, now)), lost: AtomicBool::new(false), lost_tx })
    }

    /// Something arrived from the runner.
    pub fn heard(&self) {
        let now = self.clock.now_ms();
        self.watch.lock().unwrap_or_else(|p| p.into_inner()).heard(now);
    }

    /// Whether the link has been given up on.
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    /// Give up on the link. True for the call that did, false for any later.
    pub fn lose(&self) -> bool {
        let first = !self.lost.swap(true, Ordering::SeqCst);
        if first {
            self.lost_tx.send_replace(true);
        }
        first
    }

    /// Resolves once the link is lost, however it was.
    pub async fn lost(&self) {
        let mut rx = self.lost_tx.subscribe();
        let _ = rx.wait_for(|lost| *lost).await;
    }

    /// Count everything `stream` delivers as heard from the runner.
    pub fn tap<S>(self: &Arc<Self>, stream: S) -> Tapped<S> {
        Tapped { inner: stream, link: self.clone() }
    }

    /// Probe a quiet link and give up on a dead one, until the link is lost
    /// (here or elsewhere). `probe` sends one probe and returns at once: its
    /// answer is whatever arrives, which `tap` counts. `ended` says the
    /// connection itself closed, which needs no waiting to be believed.
    pub async fn run(self: &Arc<Self>, probe: impl Fn(), ended: impl Fn() -> bool) {
        while !self.is_lost() {
            if ended() {
                self.lose();
                return;
            }
            let now = self.clock.now_ms();
            let step = self.watch.lock().unwrap_or_else(|p| p.into_inner()).step(now);
            match step {
                Step::Dead => {
                    self.lose();
                    return;
                }
                Step::Probe => {
                    probe();
                    // The next look is a probe interval on, never later than
                    // the one the rule gives.
                }
                Step::Wait(ms) => {
                    futures::future::select(self.clock.sleep(ms), Box::pin(self.lost())).await;
                }
            }
        }
    }
}

/// A stream that tells its [`Link`] whenever bytes arrive on it.
pub struct Tapped<S> {
    inner: S,
    link: Arc<Link>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Tapped<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(polled, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.link.heard();
        }
        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Tapped<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
