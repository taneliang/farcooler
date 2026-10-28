//! A handshake over a tunnel that goes silent is given up on, and a person's
//! slow passkey touch is not what runs it out (ov-834).
//!
//! The tunnel is the dead-tunnel test's valve (`support::Valve`): shut, it
//! swallows what the page writes and delivers nothing. The allowance is read
//! on a `ManualClock` that only the test moves, one second at a time, with
//! tokio's own clock paused so settling costs no wall time.
//!
//! Each test goes red without the code it names:
//! - `Budget::run` (no bound): the silent-tunnel tests never end;
//! - `Metered::auth_sign`'s pause: the slow-touch test is cut short at the
//!   allowance, and the silent-after-touch test gives up early;

mod support;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use farcooler_ceremony::Liveness;
use farcooler_web::error::WebError;
use farcooler_web::handshake::{self, Budget, HANDSHAKE};
use farcooler_web::liveness::{Clock, Link};
use farcooler_web::ssh::SignError;
use farcooler_web::wire::Runner;
use russh::keys::HashAlg;
use russh::keys::agent::AgentIdentity;
use support::{BROWSER_KEY, Ed25519InMemory, HOST_KEY, ManualClock, Seen, Tunnel, Valve, key, line};

/// Signs as the Ed25519 key does, after `touch` of the page's clock (the
/// person's wait), and says when it finished by shutting the tunnel when
/// `then_dies` is set.
struct Touching {
    inner: Ed25519InMemory,
    clock: Arc<ManualClock>,
    touch: u64,
    then_dies: Option<Arc<Tunnel>>,
}

impl russh::Signer for Touching {
    type Error = SignError;

    fn auth_sign(
        &mut self,
        key: &AgentIdentity,
        hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, SignError>> + Send {
        let wait = self.clock.sleep(self.touch);
        let signed = self.inner.auth_sign(key, hash_alg, to_sign);
        let dies = self.then_dies.clone();
        async move {
            wait.await;
            let signed = signed.await;
            if let Some(tunnel) = dies {
                tunnel.dead.store(true, Ordering::SeqCst);
            }
            signed
        }
    }
}

struct Rig {
    clock: Arc<ManualClock>,
    tunnel: Arc<Tunnel>,
    seen: Arc<Seen>,
}

impl Rig {
    fn new() -> Rig {
        Rig { clock: Arc::new(ManualClock::default()), tunnel: Arc::new(Tunnel::default()), seen: Arc::new(Seen::default()) }
    }

    /// Start signing in with a signer that takes `touch` ms to sign, and
    /// say how it ended once it has. `then_dies` shuts the tunnel the moment
    /// the signature is made.
    async fn sign_in(&self, touch: u64, then_dies: bool) -> tokio::task::JoinHandle<Result<Runner, WebError>> {
        let (browser_side, runner_side) = tokio::io::duplex(1 << 16);
        support::serve(runner_side, HOST_KEY, Some(key(BROWSER_KEY).public_key().clone()), self.seen.clone());
        let clock = self.clock.clone();
        let link = Link::new(clock.clone(), Liveness::TUNNEL);
        let stream = link.tap(Valve { inner: browser_side, tunnel: self.tunnel.clone() });
        let signer = Touching {
            inner: Ed25519InMemory(key(BROWSER_KEY)),
            clock: self.clock.clone(),
            touch,
            then_dies: then_dies.then(|| self.tunnel.clone()),
        };
        let signing = tokio::task::spawn_local(async move {
            handshake::sign_in(
                clock,
                HANDSHAKE,
                stream,
                &support::fingerprint(HOST_KEY),
                "owner",
                &line(BROWSER_KEY),
                signer,
                link,
                || {},
            )
            .await
        });
        // Started before the clock moves: the allowance counts from the
        // first poll, and a test that moved time first would be counting
        // from a later start than it thinks.
        tokio::time::sleep(Duration::from_millis(50)).await;
        signing
    }

    /// One second of the page's clock, with every task given its turn.
    async fn second(&self) {
        self.clock.advance(1_000);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    async fn seconds(&self, n: u64) {
        for _ in 0..n {
            self.second().await;
        }
    }
}

/// Under a paused clock a test that waits for something that never comes
/// does not hang: the runtime has nothing to do, so time moves on to this
/// bound and the test fails.
async fn local<F: Future>(f: F) -> F::Output {
    let bound = Duration::from_secs(3_600);
    tokio::time::timeout(bound, tokio::task::LocalSet::new().run_until(f)).await.expect("the test waited for something that never came")
}

/// The stated time. Every other test here reads the constant, so this is the
/// one that holds its value: fifteen seconds of waiting on the runner, which
/// with the Use Passkey window (60 s, services/web passkey-prompt.ts) still
/// ends inside sshd's default `LoginGraceTime` of 120 s.
#[test]
fn the_allowance_is_fifteen_seconds_and_fits_beside_the_passkey_window() {
    assert_eq!(HANDSHAKE, Duration::from_secs(15));
    assert!(HANDSHAKE + Duration::from_secs(60) < Duration::from_secs(120));
}

/// ov-834 acceptance 1: a tunnel dialed and then silent. The handshake's first
/// read never comes, and the sign-in ends at the allowance, in the word the
/// page words as "didn't answer", not a second before.
#[tokio::test(start_paused = true)]
async fn a_tunnel_that_is_silent_from_the_start_ends_the_handshake_at_the_allowance() {
    local(async {
        let rig = Rig::new();
        rig.tunnel.dead.store(true, Ordering::SeqCst);
        let signing = rig.sign_in(0, false).await;
        rig.seconds(HANDSHAKE.as_secs() - 1).await;
        assert!(!signing.is_finished(), "inside the allowance it is still waiting");
        rig.second().await;
        let ended = signing.await.unwrap().err().expect("a silent tunnel does not sign in");
        assert_eq!(ended, WebError::NoAnswer);
        assert_eq!(ended.word(), "no_answer");
        assert_eq!(rig.seen.auths.load(Ordering::SeqCst), 0);
    })
    .await;
}

/// The login can finish and the runner's daemon still never say hello: that
/// is not left hanging either. The client core bounds the hello on tokio's own
/// clock (`SessionError::DaemonMissing`), which a page has too, so it ends in
/// that error rather than waiting for this allowance. Pinned here so the
/// sign-in as a whole is known to end, whichever bound is first.
#[tokio::test(start_paused = true)]
async fn a_daemon_that_never_says_hello_ends_the_sign_in_with_an_error() {
    local(async {
        let rig = Rig::new();
        rig.seen.mute_hello.store(true, Ordering::SeqCst);
        let signing = rig.sign_in(0, false).await;
        rig.seconds(HANDSHAKE.as_secs()).await;
        let ended = signing.await.unwrap().err().expect("a daemon that never says hello does not sign in");
        assert!(matches!(ended, WebError::Exec(_) | WebError::NoAnswer), "{ended:?}");
        assert_eq!(rig.seen.hellos.lock().await.len(), 1, "the login worked and the hello went unanswered");
    })
    .await;
}

/// The failure is retryable: the same rig, its tunnel mended, signs in. What
/// the stuck attempt left behind blocks nothing.
#[tokio::test(start_paused = true)]
async fn a_new_attempt_after_the_silent_one_signs_in() {
    local(async {
        let rig = Rig::new();
        rig.tunnel.dead.store(true, Ordering::SeqCst);
        let stuck = rig.sign_in(0, false).await;
        rig.seconds(HANDSHAKE.as_secs()).await;
        assert!(stuck.await.unwrap().is_err());

        let healthy = Rig::new();
        let again = healthy.sign_in(0, false).await;
        healthy.seconds(1).await;
        assert!(again.await.unwrap().is_ok());
    })
    .await;
}

/// ov-834 acceptance 2: a person who takes 50 seconds over the passkey (inside
/// the 60 s Use Passkey window) is not cut short. The allowance's clock is
/// stopped while they touch, so the 15 seconds are for the runner alone.
#[tokio::test(start_paused = true)]
async fn a_slow_touch_inside_the_window_is_not_cut_short() {
    local(async {
        let rig = Rig::new();
        let signing = rig.sign_in(50_000, false).await;
        // Well past the allowance on the page's clock, all of it the touch.
        rig.seconds(49).await;
        assert!(!signing.is_finished(), "the person is still touching");
        rig.seconds(1).await;
        let runner = signing.await.unwrap().expect("the touch finished inside the window, so it signs in");
        assert!(!runner.lost());
    })
    .await;
}

/// The other edge of the same rule: the touch is not free time for the
/// runner. A tunnel that goes quiet right after the touch is given up on a
/// full allowance later, however long the touch took.
#[tokio::test(start_paused = true)]
async fn a_tunnel_that_goes_silent_after_a_slow_touch_gets_a_full_allowance_after_it() {
    local(async {
        let rig = Rig::new();
        let signing = rig.sign_in(40_000, true).await;
        rig.seconds(40).await;
        assert!(!signing.is_finished());
        rig.seconds(HANDSHAKE.as_secs() - 1).await;
        assert!(!signing.is_finished(), "inside the allowance, counted from the end of the touch");
        rig.seconds(1).await;
        assert_eq!(signing.await.unwrap().err(), Some(WebError::NoAnswer));
    })
    .await;
}

/// The allowance itself, with no tunnel: waiting counts, a pause does not,
/// and a pause that ends starts counting again.
#[tokio::test(start_paused = true)]
async fn a_pause_stops_the_allowances_clock_and_its_end_starts_it_again() {
    local(async {
        let clock = Arc::new(ManualClock::default());
        let budget = Budget::new(clock.clone(), Duration::from_secs(10));
        let waiting = {
            let budget = budget.clone();
            tokio::task::spawn_local(async move { budget.run(std::future::pending::<()>()).await })
        };
        let tick = |n: u64| {
            let clock = clock.clone();
            async move {
                for _ in 0..n {
                    clock.advance(1_000);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        };
        tick(4).await;
        let pause = budget.pause();
        tick(100).await;
        assert!(!waiting.is_finished(), "a hundred seconds of pause is not waiting");
        drop(pause);
        tick(5).await;
        assert!(!waiting.is_finished(), "nine counted seconds so far");
        tick(1).await;
        assert_eq!(waiting.await.unwrap(), None);
    })
    .await;
}

/// `Metered` is the only thing that pauses: a signer wrapped in it is what the
/// tests above rely on, and one that is not wrapped is cut at the allowance.
#[tokio::test(start_paused = true)]
async fn an_unmetered_slow_signer_would_be_cut_short() {
    local(async {
        let rig = Rig::new();
        let (browser_side, runner_side) = tokio::io::duplex(1 << 16);
        support::serve(runner_side, HOST_KEY, Some(key(BROWSER_KEY).public_key().clone()), rig.seen.clone());
        let clock: Arc<dyn Clock> = rig.clock.clone();
        let budget = Budget::new(clock.clone(), HANDSHAKE);
        let mut slow = Touching { inner: Ed25519InMemory(key(BROWSER_KEY)), clock: rig.clock.clone(), touch: 50_000, then_dies: None };
        let signing = tokio::task::spawn_local(async move {
            budget
                .run(farcooler_web::ssh::connect(
                    browser_side,
                    &support::fingerprint(HOST_KEY),
                    "owner",
                    &line(BROWSER_KEY),
                    &mut slow,
                ))
                .await
                .is_none()
        });
        rig.seconds(HANDSHAKE.as_secs()).await;
        assert!(signing.await.unwrap(), "with no pause the touch runs the allowance out");
    })
    .await;
}
