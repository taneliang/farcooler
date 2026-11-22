//! Freeze the retained native process tree without changing the engine's run identity or phase.
use super::*;
use crate::engine_steering::{self, Control, Order};
use farcooler_tmux::TmuxServer;
use std::sync::Mutex;

/// Resumed on every exit path, including a cancelled future, before the host reaps the server. Only the agent's hold
/// signals it (ov-1330): this names the hold, never a process.
#[derive(Default)]
pub(super) struct Frozen(Option<PathBuf>);
impl Frozen { pub(super) fn quiescent(mut self) { self.0 = None; } }
impl Drop for Frozen {
    fn drop(&mut self) { if let Some(hold) = self.0.take() { crate::native_hold::tell(&hold, "cont"); } }
}
/// Ask the hold to stop the agent's group. It answers once the kernel has stopped every member.
async fn freeze(hold: &Path, wait: Duration) -> Result<Frozen> {
    match crate::native_hold::pause(hold, wait).await {
        crate::native_hold::Answer::Said(word) if word == "stopped" => Ok(Frozen(Some(hold.to_path_buf()))),
        crate::native_hold::Answer::Gone => Err(Error::Refused("native pause has no hold to ask")),
        _ => Err(Error::Refused("native hold did not confirm pause")),
    }
}

fn resume(stopped: Frozen, builder: Option<&crate::yielding::BuilderGuard>) {
    if let Some(builder) = builder {
        stopped.quiescent();
        builder.manual(false);
    }
}

/// Quiet-period accounting excludes time the orchestrator paused the step, and time a landing gate held its builder
/// stopped (ov-1301).
struct ClockState { started: tokio::time::Instant, paused_for: Duration, paused_at: Option<tokio::time::Instant>,
    gates: Option<crate::yielding::HeldClock> }
#[derive(Clone)]
pub(super) struct ActiveClock(std::sync::Arc<Mutex<ClockState>>);
impl ActiveClock {
    #[cfg(test)]
    pub(super) fn new() -> Self { Self::held_by(None) }
    /// A clock that also stops while a gate holds `builder` stopped.
    pub(super) fn held_by(builder: Option<&crate::yielding::BuilderGuard>) -> Self {
        Self(std::sync::Arc::new(Mutex::new(ClockState { started: tokio::time::Instant::now(), paused_for: Duration::ZERO,
            paused_at: None, gates: builder.map(crate::yielding::BuilderGuard::held_clock) })))
    }
    pub(super) fn now(&self) -> Duration {
        let clock = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let held = clock.gates.as_ref().map_or(Duration::ZERO, crate::yielding::HeldClock::held);
        clock.paused_at.unwrap_or_else(tokio::time::Instant::now).duration_since(clock.started)
            .saturating_sub(clock.paused_for).saturating_sub(held)
    }
    fn pause(&self) { self.0.lock().unwrap_or_else(|e| e.into_inner()).paused_at = Some(tokio::time::Instant::now()); }
    fn resume(&self) {
        let mut clock = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = clock.paused_at.take() { clock.paused_for += at.elapsed(); }
    }
}

/// The turn and its deadline are polled only while active. A pause therefore suspends the step's
/// timeout as well as its processes; Stop still works while paused and settles through the engine. Time a landing
/// gate held the builder stopped is given back to the deadline too (ov-1301): the Canary failed six builds, with no
/// reason, whose limit ran out while a 40-minute gate held them.
#[allow(clippy::too_many_arguments)]
pub(super) async fn controlled<T>(turn: impl std::future::Future<Output = Result<T>>, tmux: &TmuxServer,
    pane: &str, hold: &Path, id: Uuid, clock: &ActiveClock, timeout: Duration, mut orders: mpsc::Receiver<Order>,
    mut cancel: tokio::sync::watch::Receiver<bool>, frozen: &mut Option<Frozen>, yielded: Option<&crate::yielding::BuilderGuard>, stop_reply: &mut Option<oneshot::Sender<farcooler_core::Result<()>>>) -> Result<Option<T>> {
    tokio::pin!(turn);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut remaining = timeout;
    let mut active_since = tokio::time::Instant::now();
    let mut credited = Duration::ZERO;
    loop {
        tokio::select! {
            result = &mut turn, if frozen.is_none() => return result.map(Some),
            _ = &mut deadline, if frozen.is_none() => {
                let (held, holding) = yielded.map_or((Duration::ZERO, false), crate::yielding::BuilderGuard::held);
                let owed = held.saturating_sub(credited);
                if owed.is_zero() && !holding { return Ok(None); }
                // What is left is what the gate took, counted from now, so a later hand pause reckons from it.
                credited = held;
                active_since = tokio::time::Instant::now();
                remaining = owed.max(Duration::from_secs(1));
                deadline.as_mut().reset(active_since + remaining);
            }
            _ = async { while !*cancel.borrow_and_update() { if cancel.changed().await.is_err() { break; } } } => return Ok(None),
            Some((op, mut reply)) = orders.recv() => {
                let result = match op {
                    Control::Pause if frozen.is_none() => {
                        // Validate the tagged pane again before asking its hold to pause.
                        match tmux.pane_life(pane, id).await {
                            Ok(farcooler_tmux::PaneLife::Live) => {
                                let mut latch = match crate::yielding::manual_pause_latch(yielded, hold) {
                                    Ok(latch) => latch,
                                    Err(error) => {
                                        tracing::warn!(%error, "native pause deferred by the heavy-job queue");
                                        let _ = reply.send(Err(farcooler_core::DomainError::OperationFailed));
                                        continue;
                                    }
                                };
                                // Recorded first, so a gate that ends meanwhile cannot continue what is about to stop.
                                if let Some(y) = yielded { y.manual(true); }
                                let attempt = tokio::select! {
                                    biased;
                                    _ = reply.closed() => None,
                                    _ = cancel.wait_for(|&cancelled| cancelled) => None,
                                    result = freeze(hold, Duration::from_secs(5)) => Some(result),
                                };
                                // A wrapper may have opened a legacy queue while the group was stopping.
                                let attempt = attempt.map(|result| result.and_then(|stopped| {
                                    latch.check(hold)?;
                                    Ok(stopped)
                                }));
                                drop(latch);
                                match attempt {
                                    Some(Ok(stopped)) => {
                                        clock.pause();
                                        engine_steering::paused(id, true);
                                        if reply.send(Ok(())).is_ok() {
                                            remaining = remaining.saturating_sub(active_since.elapsed());
                                            *frozen = Some(stopped);
                                        } else {
                                            resume(stopped, yielded);
                                            clock.resume();
                                            engine_steering::paused(id, false);
                                        }
                                        continue;
                                    }
                                    Some(Err(error)) => tracing::warn!(%error, "native pause refused"),
                                    None => {}
                                }
                                if let Some(y) = yielded { y.manual(false); }
                                Err(farcooler_core::DomainError::OperationFailed)
                            },
                            _ => Err(farcooler_core::DomainError::NotFound),
                        }
                    },
                    Control::Resume if frozen.is_some() => {
                        // A builder a landing gate also holds stays stopped until the gate ends.
                        if let Some(stopped) = frozen.take() { resume(stopped, yielded); }
                        clock.resume();
                        active_since = tokio::time::Instant::now();
                        deadline.as_mut().reset(active_since + remaining);
                        engine_steering::paused(id, false); Ok(())
                    },
                    Control::Stop => { *stop_reply = Some(reply); return Ok(None); },
                    Control::Pause | Control::Resume => Ok(()),
                };
                let _ = reply.send(result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn a_native_pause_freezes_processes_and_the_steps_deadline_then_resumes_the_same_pane() {
        let dir = farcooler_core::scratch::dir("native-pause");
        let id = Uuid::now_v7();
        let tmux = TmuxServer::in_dir(&format!("pause-{id}"), id, &dir);
        struct Reap<'a>(&'a TmuxServer);
        impl Drop for Reap<'_> { fn drop(&mut self) { farcooler_tmux::reap_server(self.0.socket()); } }
        let _reap = Reap(&tmux);
        let hold = dir.join("p.hold.sock");
        let program = crate::native_hold::pane_command(&crate::native_hold::test_binary::daemon_binary().unwrap(), &hold,
            "sh -c 'while :; do echo tick >> ticks; sleep 0.02; done'");
        let window = tmux.create_terminal_window(id, id, "pause", dir.to_str().unwrap(), &program).await.unwrap();
        let pane = tmux.list_tagged_panes().await.unwrap().into_iter().find(|p| p.pane_id == window.pane_id).unwrap();
        let ticks = dir.join("ticks");
        for _ in 0..200 { if ticks.exists() { break; } tokio::time::sleep(Duration::from_millis(10)).await; }
        assert!(ticks.exists());
        let (send, orders) = mpsc::channel(8);
        let (_cancel, cancel) = tokio::sync::watch::channel(false);
        let clock = ActiveClock::new();
        let mut stop_reply = None;
        let mut frozen = None;
        let turn = controlled(std::future::pending::<Result<()>>(), &tmux, &pane.pane_id, &hold, id,
            &clock, Duration::from_millis(250), orders, cancel, &mut frozen, None, &mut stop_reply);
        let steer = async {
            let order = |op| { let (reply, answer) = oneshot::channel(); let send = send.clone(); async move {
                send.send((op, reply)).await.unwrap(); answer.await.unwrap().unwrap();
            }};
            order(Control::Pause).await;
            let paused = std::fs::read(&ticks).unwrap();
            let at = clock.now();
            tokio::time::sleep(Duration::from_millis(400)).await;
            assert_eq!(std::fs::read(&ticks).unwrap(), paused, "the native process and its descendants are frozen");
            assert_eq!(clock.now(), at, "quiet-period accounting is frozen too");
            order(Control::Pause).await; // idempotent; doesn't reset the clock
            order(Control::Resume).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(std::fs::read(&ticks).unwrap().len() > paused.len(), "the same process resumed");
        };
        let (outcome, ()) = tokio::join!(turn, steer);
        farcooler_tmux::reap_server(tmux.socket());
        assert!(outcome.unwrap().is_none(), "only active time expires the step");
    }

    #[tokio::test]
    async fn a_reviewer_keeps_running_after_its_queued_pause_times_out() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;

        let dir = farcooler_core::scratch::dir("native-pause-timeout");
        let hold = dir.join("review.sock");
        let mut reviewer = std::process::Command::new(crate::native_hold::test_binary::daemon_binary().unwrap())
            .args([crate::native_hold::FLAG, hold.to_str().unwrap(), "--", "/bin/sh", "-c",
                "while [ ! -f proceed ]; do sleep 0.02; done; echo running > result"])
            .current_dir(&dir).spawn().unwrap();
        assert!(crate::workflow_runtime::wait_until(tokio::time::Instant::now() + Duration::from_secs(5),
            Duration::from_millis(10), || hold.exists()).await);
        let mut first = UnixStream::connect(&hold).unwrap();
        let mut second = UnixStream::connect(&hold).unwrap();
        // Both earlier connections keep their requests incomplete until the reviewer's pause is refused.
        assert!(freeze(&hold, Duration::from_millis(250)).await.is_err());
        first.write_all(b"ready\n").unwrap();
        second.write_all(b"ready\n").unwrap();
        let state = crate::native_hold::ask(&hold, "stopped", Duration::from_secs(5)).await;
        std::fs::write(dir.join("proceed"), "").unwrap();
        let ran = crate::workflow_runtime::wait_until(tokio::time::Instant::now() + Duration::from_secs(2),
            Duration::from_millis(10), || dir.join("result").exists()).await;
        if !ran { crate::native_hold::ask(&hold, "end", Duration::from_secs(5)).await; }
        reviewer.wait().unwrap();
        assert_eq!(state, crate::native_hold::Answer::Said("unsettled".into()));
        assert!(ran, "the reviewer must run after the queue has drained");
    }

    /// A builder a landing gate holds stopped is not timed out for it, and its quiet periods don't run (ov-1301): the
    /// Canary failed six builds, with no reason, whose limit ran out during a 40-minute gate.
    #[tokio::test]
    async fn a_gate_holding_a_builder_past_its_limit_does_not_time_it_out() {
        let tmux = TmuxServer::in_dir("gate-held", Uuid::now_v7(), &farcooler_core::scratch::dir("gate-held"));
        let yielding = crate::yielding::Yielding::default();
        // No hold listens here, so stopping and continuing it signals nothing.
        let builder = yielding.builder(crate::yielding::Target::Hold("/nonexistent/gate-held.hold.sock".into()));
        let run = |held: bool| {
            let (tmux, builder, yielding) = (&tmux, &builder, &yielding);
            async move {
                let (_send, orders) = mpsc::channel(8);
                let (_cancel, cancel) = tokio::sync::watch::channel(false);
                let clock = ActiveClock::held_by(Some(builder));
                let (mut frozen, mut stop_reply) = (None, None);
                let gate = held.then(|| yielding.gate());
                // The agent's turn needs 300 ms of its own time; its limit is 200 ms, and a gate holds it for 400.
                let turn = async { tokio::time::sleep(Duration::from_millis(300)).await; Ok(()) };
                let release = async move { tokio::time::sleep(Duration::from_millis(400)).await; drop(gate); };
                let quiet = clock.clone();
                let (outcome, ()) = tokio::join!(controlled(turn, tmux, "%0", Path::new("/nonexistent/gate-held.hold.sock"), Uuid::now_v7(), &clock,
                    Duration::from_millis(200), orders, cancel, &mut frozen, Some(builder), &mut stop_reply), release);
                (outcome.unwrap(), quiet.now())
            }
        };
        let (outcome, active) = run(false).await;
        assert!(outcome.is_none(), "with no gate, the limit ends it");
        assert!(active >= Duration::from_millis(200), "{active:?}");
        let (outcome, active) = run(true).await;
        assert!(outcome.is_some(), "a gate's hold is not the agent's time");
        assert!(active < Duration::from_millis(250), "quiet periods leave the gate's hold out: {active:?}");
    }
}
