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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use farcooler_ceremony::Liveness;
use farcooler_protocol::link_ending;
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
    /// When the next probe is due: a probe interval after the last byte, or
    /// after the last probe, as russh re-arms its timer when it fires.
    due: u64,
    /// Probes sent since, none of them answered.
    unanswered: usize,
}

impl Watch {
    /// A link that opened at `now` (milliseconds on the caller's clock).
    pub fn new(liveness: Liveness, now: u64) -> Self {
        Self { liveness, due: now + liveness.probe_after.as_millis() as u64, unanswered: 0 }
    }

    /// Something arrived. Resets the clock and the count: a link carrying
    /// output sends no probes at all.
    pub fn heard(&mut self, now: u64) {
        self.due = now + self.liveness.probe_after.as_millis() as u64;
        self.unanswered = 0;
    }

    /// What is due at `now`. A `Probe` answer has been counted: asking again
    /// at the same instant says `Wait`.
    pub fn step(&mut self, now: u64) -> Step {
        let every = self.liveness.probe_after.as_millis() as u64;
        if now < self.due {
            return Step::Wait(self.due - now);
        }
        // Re-armed from now, not from when it was due: a timer that fired late
        // (a hidden tab's, a laptop that slept) gives the probe it sends its
        // whole interval to be answered before the next look.
        self.due = now + every;
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

/// Why a link was given up on (ov-1513), and how long it had been quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ended {
    /// One of `farcooler_protocol::link_ending`'s words.
    pub cause: &'static str,
    /// Milliseconds since the last byte from the runner.
    pub idle_ms: u64,
}

/// The liveness of one connection: what the page has heard from the runner,
/// and whether it has given up on it.
pub struct Link {
    clock: Arc<dyn Clock>,
    watch: Mutex<Watch>,
    /// When the last byte arrived, or when the link opened.
    heard_at: AtomicU64,
    /// How many times something has arrived.
    heard: AtomicU64,
    /// Set once, by whoever gives the link up first.
    ended: OnceLock<Ended>,
    /// Foreground checks under way (`Link::checking`).
    checks: AtomicUsize,
    lost_tx: tokio::sync::watch::Sender<bool>,
}

/// A foreground check under way: see [`Link::checking`].
pub struct Checking<'a>(&'a Link);

impl Drop for Checking<'_> {
    fn drop(&mut self) {
        self.0.checks.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Link {
    pub fn new(clock: Arc<dyn Clock>, liveness: Liveness) -> Arc<Self> {
        let now = clock.now_ms();
        let (lost_tx, _) = tokio::sync::watch::channel(false);
        Arc::new(Self {
            clock,
            watch: Mutex::new(Watch::new(liveness, now)),
            heard_at: AtomicU64::new(now),
            heard: AtomicU64::new(0),
            ended: OnceLock::new(),
            checks: AtomicUsize::new(0),
            lost_tx,
        })
    }

    /// Something arrived from the runner.
    pub fn heard(&self) {
        let now = self.clock.now_ms();
        self.heard_at.store(now, Ordering::SeqCst);
        self.heard.fetch_add(1, Ordering::SeqCst);
        self.watch.lock().unwrap_or_else(|p| p.into_inner()).heard(now);
    }

    /// How many times something has arrived: a change means the runner has
    /// been heard from since.
    pub fn heard_count(&self) -> u64 {
        self.heard.load(Ordering::SeqCst)
    }

    /// Milliseconds since the last byte from the runner.
    pub fn idle_ms(&self) -> u64 {
        self.clock.now_ms().saturating_sub(self.heard_at.load(Ordering::SeqCst))
    }

    /// Say that a foreground check is asking the runner. While one is, the
    /// quiet-link rule probes but does not give the link up: the check is
    /// waiting longer, on purpose, and will try again before it decides
    /// (`Runner::check`). A connection that closes still ends it at once.
    pub fn checking(&self) -> Checking<'_> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        Checking(self)
    }

    /// Whether the link has been given up on.
    pub fn is_lost(&self) -> bool {
        self.ended.get().is_some()
    }

    /// Why the link was given up on, once it has been.
    pub fn ended(&self) -> Option<Ended> {
        self.ended.get().copied()
    }

    /// Give up on the link, for `cause`. True for the call that did, false for
    /// any later one, whose cause is dropped.
    pub fn lose(&self, cause: &'static str) -> bool {
        let first = self.ended.set(Ended { cause, idle_ms: self.idle_ms() }).is_ok();
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
                self.lose(link_ending::SOCKET_CLOSE);
                return;
            }
            let now = self.clock.now_ms();
            let step = self.watch.lock().unwrap_or_else(|p| p.into_inner()).step(now);
            match step {
                Step::Dead if self.checks.load(Ordering::SeqCst) == 0 => {
                    self.lose(link_ending::SILENCE);
                    return;
                }
                // A check is under way, and its verdict is the one that
                // counts. The rule is looked at again an interval on.
                Step::Dead => {}
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
