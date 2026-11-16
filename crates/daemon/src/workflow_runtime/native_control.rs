//! Freeze the retained native process tree without changing the engine's run identity or phase.
use super::*;
use crate::engine_steering::{self, Control, Order};
use farcooler_tmux::TmuxServer;
use std::sync::Mutex;

/// Resumed on every exit path, including a cancelled future, before the host reaps the server.
#[derive(Default)]
pub(super) struct Frozen(Vec<i32>);
impl Frozen { pub(super) fn quiescent(mut self) { self.0.clear(); } }
impl Drop for Frozen {
    fn drop(&mut self) { for &pid in &self.0 { unsafe { libc::kill(pid, libc::SIGCONT); } } }
}
async fn freeze(tty: &str, supervisor: i32) -> Result<Frozen> {
    let mut frozen = Frozen::default();
    // Stopping parents first closes the fork race; repeat until no new descendants appear.
    for _ in 0..100 {
        let table = super::coding_native::snapshot()?;
        let mut current = table.under_tty(tty.trim_start_matches("/dev/"));
        if !current.contains(&supervisor) { return Err(Error::Refused("native supervisor no longer owns its tty")); }
        current.retain(|pid| *pid != supervisor);
        if current.is_empty() { return Err(Error::Refused("native pause has no process facts")); }
        let old = frozen.0.len();
        for &pid in &current {
            if pid <= 1 || pid == std::process::id() as i32 { return Err(Error::Refused("invalid native pid")); }
            if unsafe { libc::kill(pid, libc::SIGSTOP) } != 0 {
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) { continue; }
                return Err(Error::Refused("native pause failed"));
            }
            if !frozen.0.contains(&pid) { frozen.0.push(pid); }
        }
        if frozen.0.len() == old && current.iter().all(|&pid| stopped(pid)) { return Ok(frozen); }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(Error::Refused("native pause did not settle"))
}

// A successful signal only requests a stop. Confirm the kernel observed it before acknowledging a
// pause: otherwise a shell handling its child's exit can still execute after the caller sees Paused.
#[cfg(target_os = "macos")]
fn stopped(pid: i32) -> bool {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (got == size && matches!(info.pbi_status, libc::SSTOP | libc::SZOMB))
        || (got <= 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH))
}
#[cfg(not(target_os = "macos"))]
fn stopped(pid: i32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat.rsplit_once(')').and_then(|(_, s)| s.trim_start().chars().next())
            .is_some_and(|state| matches!(state, 'T' | 't' | 'Z' | 'X')),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
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
    pane: &str, tty: &str, supervisor: i32, id: Uuid, clock: &ActiveClock, timeout: Duration, mut orders: mpsc::Receiver<Order>,
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
            Some((op, reply)) = orders.recv() => {
                let result = match op {
                    Control::Pause if frozen.is_none() => {
                        // Validate the tagged pane again; never signal a tty from a stale capability.
                        match tmux.pane_life(pane, id).await {
                            Ok(farcooler_tmux::PaneLife::Live) => {
                                // Recorded first, so a gate that ends meanwhile cannot continue what is about to stop.
                                if let Some(y) = yielded { y.manual(true); }
                                match freeze(tty, supervisor).await {
                                Ok(stopped) => {
                                    remaining = remaining.saturating_sub(active_since.elapsed());
                                    *frozen = Some(stopped); clock.pause(); engine_steering::paused(id, true); Ok(())
                                },
                                Err(error) => {
                                    tracing::warn!(%error, "native pause refused");
                                    if let Some(y) = yielded { y.manual(false); }
                                    Err(farcooler_core::DomainError::OperationFailed)
                                },
                                }
                            },
                            _ => Err(farcooler_core::DomainError::NotFound),
                        }
                    },
                    Control::Resume if frozen.is_some() => {
                        // A builder a landing gate also holds stays stopped until the gate ends.
                        match (frozen.take(), yielded) {
                            (Some(stopped), Some(y)) => { stopped.quiescent(); y.manual(false); }
                            (stopped, _) => drop(stopped),
                        }
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
        let program = "exec /bin/sh -c 'sh -c \"while :; do echo tick >> ticks; sleep 0.02; done\"; status=$?; exit \"$status\"'";
        let window = tmux.create_terminal_window(id, id, "pause", dir.to_str().unwrap(), program).await.unwrap();
        let pane = tmux.list_tagged_panes().await.unwrap().into_iter().find(|p| p.pane_id == window.pane_id).unwrap();
        let ticks = dir.join("ticks");
        for _ in 0..200 { if ticks.exists() { break; } tokio::time::sleep(Duration::from_millis(10)).await; }
        assert!(ticks.exists());
        let (send, orders) = mpsc::channel(8);
        let (_cancel, cancel) = tokio::sync::watch::channel(false);
        let clock = ActiveClock::new();
        let mut stop_reply = None;
        let mut frozen = None;
        let turn = controlled(std::future::pending::<Result<()>>(), &tmux, &pane.pane_id, &pane.tty, pane.stamp.pid as i32, id,
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

    /// A builder a landing gate holds stopped is not timed out for it, and its quiet periods don't run (ov-1301): the
    /// Canary failed six builds, with no reason, whose limit ran out during a 40-minute gate.
    #[tokio::test]
    async fn a_gate_holding_a_builder_past_its_limit_does_not_time_it_out() {
        let tmux = TmuxServer::in_dir("gate-held", Uuid::now_v7(), &farcooler_core::scratch::dir("gate-held"));
        // Nothing runs on this tty, so stopping and continuing it signals nothing.
        let yielding = crate::yielding::Yielding::default();
        let builder = yielding.builder(crate::yielding::Target::Tty("/dev/ttys-gate-held-none".into()));
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
                let (outcome, ()) = tokio::join!(controlled(turn, tmux, "%0", "", 0, Uuid::now_v7(), &clock,
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
