//! A bound on signing in (ov-834): a handshake over a half-dead tunnel gives
//! up, and a person's passkey touch is never what runs it out.
//!
//! The connection watch (`liveness`) starts once the runner is up, so until
//! then a tunnel that goes silent after it dialed (the runner restarted, or
//! another device's revocation dropped the relay's state) left the SSH
//! handshake waiting on a read that never comes. Nothing else ends it: russh
//! has no timer here (see `ssh::connect`), and sshd's `LoginGraceTime` closes
//! a connection it can hear, not one it can't.
//!
//! **The allowance is the wait for the runner, not for the person.** Signing
//! in has one step a person does: the passkey touch, which the page allows
//! 60 s for (`USE_PASSKEY_WINDOW_MS`, services/web/src/client/passkey-prompt.ts).
//! [`Metered`] wraps the signer and stops the allowance's clock while the
//! signature is being made, so a slow touch costs the runner's side nothing
//! and a tunnel that goes silent after it is still given up on a full
//! allowance later. Time is handed in through [`Clock`], as the liveness
//! watch's is, so a test moves it by hand.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::keys::HashAlg;
use russh::keys::agent::AgentIdentity;

use crate::error::WebError;
use crate::liveness::{Clock, Link};
use crate::ssh::{self, Connection, SignError};
use crate::wire::Runner;

/// How long signing in may wait on the runner, the person's touch not
/// counted: the key exchange, the login's round trips, and the runner's
/// hello. A healthy tunnel does all of that in well under two seconds; fifteen
/// leaves a slow relay its time and still ends a silent one before the
/// person gives up looking at the screen.
pub const HANDSHAKE: Duration = Duration::from_secs(15);

/// While paused, the clock is looked at this often, to see the pause end.
const WHILE_PAUSED_MS: u64 = 1_000;

#[derive(Default)]
struct Spent {
    /// Milliseconds the allowance's clock was stopped, in finished pauses.
    paused: u64,
    /// When the pause now under way began.
    since: Option<u64>,
}

/// An allowance of waiting, whose clock a [`Pause`] stops.
pub struct Budget {
    clock: Arc<dyn Clock>,
    allowance_ms: u64,
    started: u64,
    spent: Mutex<Spent>,
}

/// The allowance's clock is stopped for as long as this is held.
pub struct Pause(Arc<Budget>);

impl Drop for Pause {
    fn drop(&mut self) {
        let now = self.0.clock.now_ms();
        let mut spent = self.0.spent.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(since) = spent.since.take() {
            spent.paused += now.saturating_sub(since);
        }
    }
}

impl Budget {
    pub fn new(clock: Arc<dyn Clock>, allowance: Duration) -> Arc<Self> {
        let started = clock.now_ms();
        Arc::new(Self { clock, allowance_ms: allowance.as_millis() as u64, started, spent: Mutex::default() })
    }

    /// Stop the clock until the guard is dropped. Not nested: one thing at a
    /// time is waiting on the person.
    pub fn pause(self: &Arc<Self>) -> Pause {
        let now = self.clock.now_ms();
        self.spent.lock().unwrap_or_else(|p| p.into_inner()).since.get_or_insert(now);
        Pause(self.clone())
    }

    /// Waiting that counts so far, and whether the clock is stopped.
    fn counted(&self) -> (u64, bool) {
        let now = self.clock.now_ms();
        let spent = self.spent.lock().unwrap_or_else(|p| p.into_inner());
        let ongoing = spent.since.map_or(0, |s| now.saturating_sub(s));
        ((now - self.started).saturating_sub(spent.paused + ongoing), spent.since.is_some())
    }

    /// Run `work`, and give up (`None`, dropping it) once the allowance of
    /// counted waiting is used.
    pub async fn run<T>(&self, work: impl Future<Output = T>) -> Option<T> {
        let mut work = std::pin::pin!(work);
        loop {
            let (counted, paused) = self.counted();
            if counted >= self.allowance_ms {
                return None;
            }
            let wait = if paused { WHILE_PAUSED_MS } else { self.allowance_ms - counted };
            match futures::future::select(work.as_mut(), self.clock.sleep(wait)).await {
                futures::future::Either::Left((done, _)) => return Some(done),
                futures::future::Either::Right(_) => {}
            }
        }
    }
}

/// A signer that stops the [`Budget`]'s clock while it signs.
pub struct Metered<G> {
    pub signer: G,
    pub budget: Arc<Budget>,
}

impl<G: russh::Signer<Error = SignError>> russh::Signer for Metered<G> {
    type Error = SignError;

    fn auth_sign(
        &mut self,
        key: &AgentIdentity,
        hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, SignError>> + Send {
        // Held from here, not from the first poll: the person's wait begins
        // when the prompt is asked for.
        let pause = self.budget.pause();
        let signing = self.signer.auth_sign(key, hash_alg, to_sign);
        async move {
            let signed = signing.await;
            drop(pause);
            signed
        }
    }
}

/// Sign in to a runner and start its control session, all inside
/// [`HANDSHAKE`] of waiting on the runner (`Budget`). A silent tunnel ends in
/// `no_answer`, the word the page already words as "didn't answer", and the
/// connection made so far is dropped with the future.
///
/// When the connection is up it is watched by `link` (ov-772), and `on_lost`
/// is called if the watch gives up on it.
#[allow(clippy::too_many_arguments)]
pub async fn sign_in<S, G>(
    clock: Arc<dyn Clock>,
    allowance: Duration,
    stream: S,
    host_key: &str,
    user: &str,
    public_key: &str,
    signer: G,
    link: Arc<Link>,
    on_lost: impl FnOnce() + 'static,
) -> Result<Runner, WebError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    G: russh::Signer<Error = SignError>,
{
    let budget = Budget::new(clock, allowance);
    let mut metered = Metered { signer, budget: budget.clone() };
    let work = async {
        let connection: Connection = ssh::connect(stream, host_key, user, public_key, &mut metered).await?;
        Runner::start_watched(connection, link, on_lost).await
    };
    budget.run(work).await.unwrap_or(Err(WebError::NoAnswer))
}
