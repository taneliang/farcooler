//! The browser notices a tunnel that died without a word (ov-772, plan-763
//! slice 9) by the rule every other client uses: `Liveness::TUNNEL`.
//!
//! A tunnel that dies sends nothing — no reset, no end of stream — so the page
//! sees a connection that is open and silent. Here that is a valve between the
//! page's SSH and an in-process runner: shut, it swallows every write and
//! delivers nothing, as the real tunnel does after a runner restart or another
//! device's revocation drops its relay state.
//!
//! **No test here depends on how fast the machine is.** The watch reads and
//! sleeps on a `ManualClock` that only the test moves, one second at a time;
//! tokio's own clock is paused, which makes "let everything settle" a step that
//! costs no wall time and cannot be too short.
//!
//! Each test goes red without the code it names:
//! - `Watch::step` counting (a probe that does not count, or a reset that does
//!   not reset): the unit tests and the end-to-end one;
//! - `Link::tap` (the page never hears the runner): the healthy-link test sees
//!   a loss on a link that is answering every probe;
//! - `Runner::start_watched`'s probe: the quiet-link test sees no probe sent;
//! - `Pane`'s `link.lost()` arm: the pane test never sees `ended`.

mod support;

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use farcooler_ceremony::Liveness;
use farcooler_web::liveness::{Clock, Link, Step, Watch};
use farcooler_web::ssh;
use farcooler_web::wire::Runner;
use support::{BROWSER_KEY, Ed25519InMemory, HOST_KEY, Seen, TERMINAL, key, line};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

// ---- the rule, on its own ----

#[test]
fn a_silent_link_is_probed_at_each_interval_and_dead_on_the_one_past_the_allowance() {
    let l = Liveness::TUNNEL;
    let every = l.probe_after.as_millis() as u64;
    let mut watch = Watch::new(l, 1_000);
    assert_eq!(watch.step(1_000), Step::Wait(every));
    assert_eq!(watch.step(1_000 + every - 1), Step::Wait(1));
    let mut at = 1_000;
    for n in 1..=l.unanswered {
        at += every;
        assert_eq!(watch.step(at), Step::Probe, "probe {n} of {}", l.unanswered);
        assert_eq!(watch.step(at), Step::Wait(every), "a probe is counted once");
    }
    at += every;
    assert_eq!(watch.step(at), Step::Dead);
    assert_eq!(at - 1_000, l.noticed_within().as_millis() as u64);
}

#[test]
fn anything_heard_resets_the_clock_and_the_count() {
    let l = Liveness::TUNNEL;
    let every = l.probe_after.as_millis() as u64;
    let mut watch = Watch::new(l, 0);
    assert_eq!(watch.step(every), Step::Probe);
    assert_eq!(watch.step(2 * every), Step::Probe);
    watch.heard(2 * every + 500);
    // The allowance starts over from the byte, not from the link's opening.
    assert_eq!(watch.step(3 * every), Step::Wait(500));
    assert_eq!(watch.step(3 * every + 500), Step::Probe);
    assert_eq!(watch.step(4 * every + 500), Step::Probe);
    assert_eq!(watch.step(5 * every + 500), Step::Dead);
}

/// The browser has no figure of its own: whatever `Liveness` says is what the
/// watch does, so a change to the tunnel's setting moves every client.
#[test]
fn the_watch_follows_the_numbers_it_is_given() {
    let quick = Liveness { probe_after: Duration::from_millis(500), unanswered: 1 };
    let mut watch = Watch::new(quick, 0);
    assert_eq!(watch.step(500), Step::Probe);
    assert_eq!(watch.step(1_000), Step::Dead);
    assert_eq!(quick.noticed_within(), Duration::from_millis(1_000));
}

/// Acceptance 1's twelve seconds is met by the tunnel's own setting with room
/// for the page to draw the word.
#[test]
fn the_tunnels_setting_leaves_the_page_inside_twelve_seconds() {
    assert!(Liveness::TUNNEL.noticed_within() <= Duration::from_secs(12));
}

// ---- time, by hand ----

#[derive(Default)]
struct ManualClock {
    now: AtomicU64,
    sleepers: Mutex<Vec<(u64, tokio::sync::oneshot::Sender<()>)>>,
}

impl ManualClock {
    fn advance(&self, ms: u64) {
        let now = self.now.fetch_add(ms, Ordering::SeqCst) + ms;
        let mut sleepers = self.sleepers.lock().unwrap();
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut *sleepers).into_iter().partition(|(t, _)| *t <= now);
        *sleepers = later;
        for (_, wake) in due {
            let _ = wake.send(());
        }
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
    fn sleep(&self, ms: u64) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let (wake, woken) = tokio::sync::oneshot::channel();
        self.sleepers.lock().unwrap().push((self.now_ms() + ms, wake));
        Box::pin(async move {
            let _ = woken.await;
        })
    }
}

// ---- a tunnel that can die ----

#[derive(Default)]
struct Tunnel {
    dead: AtomicBool,
    /// Writes swallowed since it died: what the page sent into the dark.
    lost_writes: AtomicUsize,
}

struct Valve {
    inner: DuplexStream,
    tunnel: Arc<Tunnel>,
}

impl AsyncRead for Valve {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        if self.tunnel.dead.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Valve {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        if self.tunnel.dead.load(Ordering::SeqCst) {
            self.tunnel.lost_writes.fetch_add(1, Ordering::SeqCst);
            return Poll::Ready(Ok(buf.len()));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct Rig {
    runner: Runner,
    clock: Arc<ManualClock>,
    tunnel: Arc<Tunnel>,
    seen: Arc<Seen>,
    lost: Arc<AtomicUsize>,
}

impl Rig {
    async fn up() -> Rig {
        let (browser_side, runner_side) = tokio::io::duplex(1 << 16);
        let seen = Arc::new(Seen::default());
        support::serve(runner_side, HOST_KEY, Some(key(BROWSER_KEY).public_key().clone()), seen.clone());
        let tunnel = Arc::new(Tunnel::default());
        let clock = Arc::new(ManualClock::default());
        let link = Link::new(clock.clone(), Liveness::TUNNEL);
        let stream = link.tap(Valve { inner: browser_side, tunnel: tunnel.clone() });
        let mut signer = Ed25519InMemory(key(BROWSER_KEY));
        let conn = ssh::connect(stream, &support::fingerprint(HOST_KEY), "owner", &line(BROWSER_KEY), &mut signer)
            .await
            .expect("connects");
        let lost = Arc::new(AtomicUsize::new(0));
        let counted = lost.clone();
        let runner = Runner::start_watched(conn, link, move || {
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .expect("starts");
        let rig = Rig { runner, clock, tunnel, seen, lost };
        rig.settle().await;
        rig
    }

    /// Let every task run until all are waiting. Tokio's clock is paused, so
    /// this takes no wall time, and it ends only when nothing can move.
    async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    /// One second of the page's clock.
    async fn second(&self) {
        self.clock.advance(1_000);
        self.settle().await;
    }

    fn kill(&self) {
        self.tunnel.dead.store(true, Ordering::SeqCst);
    }

    fn probes_sent_into_the_dark(&self) -> usize {
        self.tunnel.lost_writes.load(Ordering::SeqCst)
    }
}

async fn local<F: std::future::Future>(f: F) -> F::Output {
    tokio::task::LocalSet::new().run_until(f).await
}

/// ov-772 acceptance 1, the half the page can see: a tunnel that goes silent
/// is given up on at nine seconds from the last byte, having probed at three
/// and six, and not a second earlier.
#[tokio::test(start_paused = true)]
async fn a_tunnel_that_dies_is_given_up_on_at_the_ninth_second_after_two_probes() {
    local(async {
        let rig = Rig::up().await;
        rig.kill();
        let before = rig.probes_sent_into_the_dark();

        for _ in 0..2 {
            rig.second().await;
        }
        assert_eq!(rig.probes_sent_into_the_dark(), before, "nothing is due before three seconds");
        rig.second().await;
        let first = rig.probes_sent_into_the_dark();
        assert!(first > before, "the first probe goes at three seconds");
        for _ in 0..3 {
            rig.second().await;
        }
        let second = rig.probes_sent_into_the_dark();
        assert!(second > first, "the second goes at six");
        for _ in 0..2 {
            rig.second().await;
        }
        assert_eq!(rig.lost.load(Ordering::SeqCst), 0, "eight seconds is inside the allowance");
        assert!(!rig.runner.lost());
        rig.second().await;
        assert_eq!(rig.lost.load(Ordering::SeqCst), 1, "the ninth second ends it, once");
        assert!(rig.runner.lost());
        assert_eq!(rig.probes_sent_into_the_dark(), second, "the verdict is not another probe");

        for _ in 0..30 {
            rig.second().await;
        }
        assert_eq!(rig.lost.load(Ordering::SeqCst), 1, "told once");
    })
    .await;
}

/// The cost side: a healthy idle link is probed every three seconds, every
/// probe is answered by the runner, and the answers keep it alive. If the page
/// did not count what it hears, it would call this link dead at nine.
#[tokio::test(start_paused = true)]
async fn a_healthy_idle_link_is_probed_and_stays_up() {
    local(async {
        let rig = Rig::up().await;
        for _ in 0..60 {
            rig.second().await;
        }
        assert_eq!(rig.lost.load(Ordering::SeqCst), 0);
        assert!(!rig.runner.lost());
        let probes = rig.seen.probes.load(Ordering::SeqCst);
        assert!((18..=21).contains(&probes), "sixty seconds is twenty probes, saw {probes}");
    })
    .await;
}

/// A terminal that is open when the tunnel dies is told it ended, which is
/// what the page turns into "Reconnecting". A stream over a dead tunnel never
/// ends by itself.
#[tokio::test(start_paused = true)]
async fn an_open_pane_ends_when_the_link_is_given_up_on() {
    local(async {
        let rig = Rig::up().await;
        let ended = Arc::new(AtomicBool::new(false));
        let flag = ended.clone();
        let _pane = rig.runner.attach(TERMINAL, |_| {}, move || flag.store(true, Ordering::SeqCst)).await.unwrap();
        rig.settle().await;
        assert!(!ended.load(Ordering::SeqCst));
        rig.kill();
        for _ in 0..8 {
            rig.second().await;
        }
        assert!(!ended.load(Ordering::SeqCst), "still inside the allowance");
        rig.second().await;
        assert!(ended.load(Ordering::SeqCst));
    })
    .await;
}

/// Back from a background tab or a network change, the page asks at once
/// rather than waiting out the interval: silence for the probe's own deadline
/// is the verdict (`Session::probe`, 2.5 s).
#[tokio::test(start_paused = true)]
async fn checking_a_dead_link_says_so_without_waiting_for_the_interval() {
    local(async {
        let rig = Rig::up().await;
        assert!(rig.runner.check().await, "a link that answers says so");
        assert_eq!(rig.lost.load(Ordering::SeqCst), 0);

        rig.kill();
        // The page's clock has not moved: the interval has not come round.
        assert!(!rig.runner.check().await);
        rig.settle().await;
        assert!(rig.runner.lost());
        assert_eq!(rig.lost.load(Ordering::SeqCst), 1, "a check that fails tells the page, once");
        assert!(!rig.runner.check().await, "and a lost link stays lost");
    })
    .await;
}

/// A runner the page let go of is not a loss: reconnecting drops the old one
/// and must not be told the old one died.
#[tokio::test(start_paused = true)]
async fn a_runner_the_page_dropped_is_never_reported_lost() {
    local(async {
        let rig = Rig::up().await;
        let Rig { runner, clock, tunnel, lost, .. } = rig;
        tunnel.dead.store(true, Ordering::SeqCst);
        drop(runner);
        for _ in 0..30 {
            clock.advance(1_000);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(lost.load(Ordering::SeqCst), 0);
    })
    .await;
}

/// ov-772 acceptance 2's runner half: after the tunnel dies and is given up
/// on, a fresh connection attaches the same terminal, gets the runner's replay
/// (which carries the pane's scrollback) and types into it. Nothing of the old
/// connection is needed, and none of it can answer for the new one.
#[tokio::test(start_paused = true)]
async fn a_new_connection_attaches_the_same_terminal_and_types_after_a_loss() {
    local(async {
        let old = Rig::up().await;
        let _pane = old.runner.attach(TERMINAL, |_| {}, || {}).await.unwrap();
        old.kill();
        for _ in 0..9 {
            old.second().await;
        }
        assert!(old.runner.lost());
        drop(_pane);
        let Rig { runner, .. } = old;
        drop(runner);

        let fresh = Rig::up().await;
        let (shown_tx, mut shown) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let pane = fresh
            .runner
            .attach(TERMINAL, move |bytes| { let _ = shown_tx.send(bytes.to_vec()); }, || {})
            .await
            .unwrap();
        assert_eq!(shown.recv().await.unwrap(), b"replay$ ", "the runner replays the pane on attach");
        pane.write(b"ls").await.unwrap();
        let mut echoed = Vec::new();
        while echoed.len() < 2 {
            echoed.extend(shown.recv().await.unwrap());
        }
        assert_eq!(echoed, b"ls");
        assert!(!fresh.runner.lost());
    })
    .await;
}
