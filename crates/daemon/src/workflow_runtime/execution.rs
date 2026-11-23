//! Runtime observations expire with the owned future; nothing here is durable authority.
use super::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use farcooler_workflow::{Dispatch, execution_view};

#[derive(Default)]
pub(crate) struct Activity {
    process: AtomicU32,
    started: AtomicBool,
    settling: AtomicBool,
    pub(super) session: AtomicBool,
    pub(super) lost: AtomicBool,
}
impl Activity {
    fn alive(&self) -> bool {
        let pid = self.process.load(Ordering::SeqCst);
        self.settling.load(Ordering::SeqCst) || self.session.load(Ordering::SeqCst) || (pid > 0 && crate::effect_runner::present(pid))
    }
    /// Supervision handed the result to the still-owned adapter for verification and receipt writing.
    pub(super) fn settle(&self) {
        self.settling.store(true, Ordering::SeqCst);
    }
}
pub(super) type Activities = Arc<Mutex<BTreeMap<(String, String), Arc<Activity>>>>;
tokio::task_local! { pub(crate) static ACTIVITY: Arc<Activity>; }

/// Pins a real child until its supervisor stops observing it, including cancellation.
pub(crate) struct Process(Option<Arc<Activity>>);
pub(crate) fn process(pid: u32) -> Process {
    let activity = ACTIVITY.try_with(Arc::clone).ok();
    if let Some(activity) = &activity {
        activity.process.store(pid, Ordering::SeqCst);
        activity.settling.store(false, Ordering::SeqCst);
        activity.started.store(true, Ordering::SeqCst);
    }
    Process(activity)
}
impl Process {
    pub(crate) fn settle(&self) {
        if let Some(activity) = &self.0 { activity.settle(); }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(activity) = &self.0 { activity.process.store(0, Ordering::SeqCst); }
    }
}

impl Exec {
    /// Same launch order and slot holders across boards. A missing runtime owner never means Running.
    pub(super) fn execution_state(&self, workspace: &str) -> Result<Option<State>> {
        let Some(mut state) = self.state(workspace)? else { return Ok(None) };
        let activities = self.activities.lock().map_err(|_| Error::Refused("execution observations unavailable"))?.clone();
        let now_ms = self.clock_ms();
        let mut queued = Vec::new();
        let mut holders = Vec::new();
        let mut effects = BTreeMap::new();
        let mut landing_groups = std::collections::HashSet::new();
        let mut landing_processes = Vec::new();
        for board in self.store.workflow_workspaces()? {
            let running: Vec<_> = activities.keys().filter(|(workspace, _)| workspace == &board)
                .map(|(_, effect)| effect.as_str()).collect();
            let effects_in_board = self.store.workflow_dispatch_effects(&board, &running)?;
            if effects_in_board.is_empty() { continue; }
            let other = if board == workspace { None } else { self.state(&board)? };
            let board_state = if board == workspace { Some(&state) } else { other.as_ref() };
            for stored in effects_in_board {
                let key = (board.clone(), stored.effect.id.clone());
                let pool = board_state.as_ref().and_then(|s| s.items.get(&stored.effect.item))
                    .and_then(|item| Pool::of(&stored.effect, item));
                // A live pre-admission claim is launching, so it holds a slot rather than a place in line.
                let claimed = stored.active && stored.dispatch_state == "claimed"
                    && stored.lease_until.is_some_and(|t| t > now_ms);
                if (activities.contains_key(&key) || claimed) && pool.is_some() {
                    holders.push((pool, format!("{}/{}", board, stored.effect.item)));
                } else if stored.active && pool.is_some()
                    && matches!(stored.dispatch_state.as_str(), "pending" | "claimed") {
                    queued.push((key.clone(), pool));
                }
                if stored.active && board == workspace { effects.insert(stored.effect.id.clone(), stored); }
            }
        }
        for (id, item) in state.items.iter_mut().filter(|(_, i)| matches!(i.phase, Phase::Running | Phase::WaitingSlot | Phase::Uncertain)) {
            let key = (workspace.to_owned(), item.pending.clone().unwrap_or_default());
            let activity = activities.get(&key);
            let stored = effects.get(&key.1);
            // A kept uncertainty is an outcome, not evidence that a live session disappeared.
            if item.phase == Phase::Uncertain && activity.is_none() {
                if item.stopping.is_some() && item.failure.is_none() {
                    item.execution = Some(execution_view(Dispatch::UncertainBecause(
                        "The cancellation receipt could not be verified. The original process and its external effects must be checked before this run can be closed."),
                        None, Vec::new()));
                }
                continue;
            }
            let dispatch = match stored.map(|s| s.dispatch_state.as_str()) {
                Some("pending") => Dispatch::Pending,
                Some("claimed") if stored.and_then(|s| s.lease_until).is_some_and(|t| t > now_ms) =>
                    Dispatch::Claimed { alive: false, lost: None },
                Some("claimed") => Dispatch::Pending,
                Some("admitted") if activity.is_some() => {
                    let activity = activity.unwrap();
                    let alive = activity.alive();
                    Dispatch::Claimed { alive, lost: (activity.lost.load(Ordering::SeqCst)
                        || (!alive && activity.started.load(Ordering::SeqCst)))
                        .then_some("The claimed process or session was lost without a receipt.") }
                }
                _ => Dispatch::Uncertain,
            };
            let pool = stored.and_then(|s| Pool::of(&s.effect, item));
            let position = queued.iter().filter(|(_, p)| *p == pool).position(|(k, _)| k == &key).map(|i| i + 1);
            let holding = holders.iter().filter(|(p, _)| *p == pool).map(|(_, h)| h.clone()).collect();
            let view = execution_view(dispatch, position, holding);
            if view.phase == Phase::Running && item.group_of.is_some()
                && matches!(stored.map(|s| &s.effect.kind), Some(EffectKind::Execute(StageKind::Run)))
                && let Some(activity) = activity
            {
                let group = activity.process.load(Ordering::SeqCst) as i32;
                if group > 1 {
                    landing_groups.insert(group);
                    landing_processes.push((id.clone(), group));
                }
            }
            item.phase = view.phase.clone();
            item.execution = Some(view);
        }
        let mut waits = self.yielding.heavy_lock_waits(&landing_groups);
        for (id, group) in landing_processes {
            if let Some(view) = state.items.get_mut(&id).and_then(|item| item.execution.as_mut()) {
                view.heavy_lock_wait = waits.remove(&group);
            }
        }
        Ok(Some(state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::concurrency_tests::enqueue_item;

    fn admit(runtime: &Runtime) -> Effect {
        let state = runtime.state("w").unwrap().unwrap();
        let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
        let now = runtime.clock_ms();
        let claim = runtime.store.workflow_claim_effect("w", &effect.id, "test-owner", state.revision,
            state.epoch, now, now + 30_000).unwrap().unwrap();
        assert!(runtime.store.workflow_admit_effect(&claim, now).unwrap());
        effect
    }

    #[tokio::test]
    async fn workflow_execution_normal_child_exit_stays_present_until_adapter_returns() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        enqueue_item(&mut runtime, "w", "one", coding_definition(false));
        let effect = admit(&runtime);
        let activity = Arc::new(Activity::default());
        runtime.activities.lock().unwrap().insert(("w".into(), effect.id.clone()), activity.clone());
        ACTIVITY.scope(activity, async {
            assert!(crate::effect_runner::fixture(dir.path(), "exit 0", &dir.path().join("child.log"),
                Duration::from_secs(5), &[]).await.unwrap());
            // The real child is reaped, but this adapter still has receipt verification to do.
            let state = runtime.execution_state("w").unwrap().unwrap();
            assert_eq!(state.items["one"].phase, Phase::Running);
            assert!(!state.items["one"].execution.as_ref().unwrap().text.contains("lost"));
            tokio::task::yield_now().await;
            assert_eq!(runtime.execution_state("w").unwrap().unwrap().items["one"].phase, Phase::Running);
        }).await;
        runtime.activities.lock().unwrap().clear();
        assert_eq!(runtime.execution_state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    }

    #[tokio::test]
    async fn workflow_execution_exit_before_supervision_distinguishes_completion_from_loss() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        enqueue_item(&mut runtime, "w", "one", coding_definition(false));
        let effect = admit(&runtime);
        for killed in [false, true] {
            let activity = Arc::new(Activity::default());
            runtime.activities.lock().unwrap().insert(("w".into(), effect.id.clone()), activity.clone());
            ACTIVITY.scope(activity, async {
                let mut child = tokio::process::Command::new("/bin/sh")
                    .args(["-c", if killed { "exec sleep 30" } else { "exit 0" }]).spawn().unwrap();
                let pid = child.id().unwrap();
                let presence = process(pid);
                if killed { unsafe { libc::kill(pid as i32, libc::SIGKILL); } }
                crate::effect_runner::exited(pid).await.unwrap();
                // Read before supervision observes or reaps the result: a normal exit is not loss.
                let state = runtime.execution_state("w").unwrap().unwrap();
                assert_eq!(state.items["one"].phase, if killed { Phase::Uncertain } else { Phase::Running });
                child.wait().await.unwrap();
                drop(presence);
            }).await;
        }
    }

    #[tokio::test]
    async fn workflow_execution_stored_uncertainty_is_not_replaced_with_session_loss() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        enqueue_item(&mut runtime, "w", "one", coding_definition(false));
        let effect = admit(&runtime);
        runtime.observe("w", &effect, Outcome::Uncertain, false, runtime.clock_ms()).unwrap();
        let state = runtime.execution_state("w").unwrap().unwrap();
        assert_eq!(state.items["one"].phase, Phase::Uncertain);
        assert!(state.items["one"].execution.is_none(), "no runtime loss was observed");
        assert_eq!(Some(state), runtime.state("w").unwrap());
    }

    #[tokio::test]
    async fn workflow_execution_handle_never_exposes_unclaimed_running_intent() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        enqueue_item(&mut runtime, "w", "one", coding_definition(false));
        let controller = runtime.controller();
        let handle = runtime.command_handle(&controller).unwrap();
        let (_, state) = tokio::join!(async {
            let message = runtime.inbox.recv().await.unwrap();
            runtime.receive(message).await;
        }, handle.state("w"));
        let state = state.unwrap().unwrap();
        assert_eq!(state.items["one"].phase, Phase::WaitingSlot);
        assert_eq!(state.items["one"].execution.as_ref().unwrap().position, Some(1));
        assert_eq!(Some(state), runtime.execution_state("w").unwrap());
    }

    #[tokio::test]
    async fn workflow_execution_live_claim_holds_a_slot_instead_of_a_queue_place() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        for id in ["one", "two", "three"] { enqueue_item(&mut runtime, "w", id, coding_definition(false)); }
        let state = runtime.state("w").unwrap().unwrap();
        let effect = state.items["one"].pending.as_ref().unwrap();
        let at = runtime.clock_ms();
        runtime.store.workflow_claim_effect("w", effect, "test-owner", state.revision,
            state.epoch, at, at + 30_000).unwrap().unwrap();
        let state = runtime.execution_state("w").unwrap().unwrap();
        assert_eq!(state.items["one"].phase, Phase::Starting);
        for (id, position) in [("two", 1), ("three", 2)] {
            let view = state.items[id].execution.as_ref().unwrap();
            assert_eq!(view.phase, Phase::WaitingSlot);
            assert_eq!(view.position, Some(position));
            assert_eq!(view.holders, vec!["w/one"]);
        }
        // A claim that expires before admission returns to its original place in line.
        runtime.exec.launched = Some((at + 30_001, std::time::Instant::now()));
        let state = runtime.execution_state("w").unwrap().unwrap();
        for (id, position) in [("one", 1), ("two", 2), ("three", 3)] {
            let view = state.items[id].execution.as_ref().unwrap();
            assert_eq!(view.phase, Phase::WaitingSlot);
            assert_eq!(view.position, Some(position));
            assert!(view.holders.is_empty());
        }
    }
}
