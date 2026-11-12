//! Owner-published automatic starts. Every transition still passes through the durable reducer.
use super::*;
use farcooler_workflow::scheduling::StartPolicy;
use std::collections::BTreeMap;
use farcooler_store::workflow_publication::Configuration;

#[derive(Default)]
pub(super) struct Scheduler {
    boards: BTreeMap<String, Board>,
    refused: BTreeMap<(String, String, bool), i64>,
    last_start_ms: Option<i64>,
    #[cfg(test)]
    state_reads: usize,
}
struct Board {
    revision: u64,
    policy_enabled: bool,
    state: Option<State>,
}

fn active(item: &Item) -> bool {
    !item.definition.is_ticket() && item.group_of.is_none()
        && !matches!(item.phase, Phase::Done | Phase::Cancelled | Phase::Failed | Phase::WaitingJoin | Phase::Joined)
}
fn finished(state: &State, key: &str) -> bool {
    state.items.get(key).is_some_and(|i| i.phase == Phase::Done)
        || state.archive.get(key).is_some_and(|i| i.phase == Phase::Done)
}
/// Only observed load is used, never guessed capacity. A failed sample holds load-based starts.
fn load_milli() -> Option<u64> {
    let mut load = 0.0;
    let read = unsafe { libc::getloadavg(&mut load, 1) };
    (read == 1 && load >= 0.0).then_some((load * 1000.0) as u64)
}
impl Runtime {
    pub(super) fn schedule_ready(&self, workspaces: &[String], now_ms: i64) -> Result<()> {
        self.schedule_with_load(workspaces, now_ms, load_milli())
    }
    fn schedule_with_load(&self, workspaces: &[String], now_ms: i64, load: Option<u64>) -> Result<()> {
        let mut scan = self.scheduling.lock().unwrap();
        // Inventory belongs to the runner, even when this pass was woken for only one board.
        let all = self.store.workflow_workspaces()?;
        scan.boards.retain(|w, _| all.contains(w));
        scan.refused.retain(|(w, _, _), _| all.contains(w));
        let mut configs = BTreeMap::new();
        for workspace in &all {
            let config = self.store.workflow_configuration(workspace)?;
            let enabled = config.as_ref().is_some_and(|c| c.definition.start_policy.is_some());
            let revision = self.store.workflow_state_revision(workspace)?.ok_or(Error::Refused("missing scheduling board"))?;
            if !scan.boards.get(workspace).is_some_and(|b| b.revision == revision && b.policy_enabled == enabled) {
                let state = if enabled || self.store.workflow_has_runs(workspace)? {
                    #[cfg(test)] { scan.state_reads += 1; }
                    self.state(workspace)?
                } else { None };
                scan.boards.insert(workspace.clone(), Board { revision, policy_enabled: enabled, state });
            }
            // Deleted boards' runs still consume capacity, but their publications cannot hold other boards paused.
            if !self.service_mode || !self.store.workflow_board_deleted(workspace)? {
                if let Some(config) = config { configs.insert(workspace.clone(), config); }
            }
        }
        let mut busy = scan.boards.values().filter_map(|b| b.state.as_ref())
            .flat_map(|s| s.items.values()).filter(|i| active(i)).count();
        let budgets: Vec<_> = configs.values().filter_map(|c| c.definition.start_policy.as_ref()).map(|p| &p.budget).collect();
        let start_gap_ms = budgets.iter().filter_map(|b| b.start_gap_ms()).max();
        for workspace in workspaces {
            if self.shutdown.as_ref().is_some_and(|s| *s.borrow()) { return Ok(()); }
            if self.service_mode && self.store.workflow_board_deleted(workspace)? { continue; }
            let Some(state) = scan.boards.get(workspace).and_then(|b| b.state.as_ref()) else { continue };
            let keys: Vec<_> = state.items.keys().cloned().collect();
            let config = configs.get(workspace);
            let policy = config.and_then(|c| c.definition.start_policy.as_ref());
            let removed_member = config.is_some_and(|c| crate::workflow_deciders::names_a_removed_member(&self.store, &c.definition).is_some());
            for key in keys {
                let state = scan.boards[workspace].state.as_ref().unwrap();
                let Some(item) = state.items.get(&key) else { continue };
                // Queue follows the admitted policy, including after publication removal.
                let queue = item.phase == Phase::WaitingJoin && item.definition.start_policy.as_ref().is_some_and(|p| p.auto_queue);
                let retry_key = (workspace.clone(), key.clone(), queue);
                if scan.refused.get(&retry_key).is_some_and(|until| now_ms < *until) { continue; }
                let command = if queue { Command::Queue { item: key.clone() } } else {
                    let (Some(config), Some(policy)) = (config, policy) else { continue };
                    if removed_member || !item.definition.is_ticket() || item.follows.is_some() || item.phase != Phase::WaitingApproval
                        || !policy.columns.contains(&item.definition.stages[item.stage].id) { continue; }
                    // All published machine constraints apply together: fixed capacity cannot bypass a load guard.
                    if start_gap_ms.is_some_and(|gap| scan.last_start_ms.is_some_and(|last|
                        now_ms < last || (now_ms.saturating_sub(last) as u64) < gap)) || !budgets.iter().all(|b| b.permits(busy, load))
                        || file_conflict(&scan.boards, &configs, workspace, &key, policy)
                        || !self.dependencies_ready(workspace, &key, policy, state)? { continue; }
                    let ordinal = state.items.values().filter(|i| !i.definition.is_ticket() && i.group_of.is_none()).count() + state.archive.len();
                    Command::Start { item: key.clone(), definition: config.definition.clone(), policy_pin: config.pin.clone(),
                        resource: Exec::item_resource(workspace, &key), dependencies: policy.dependencies.get(&key).cloned().unwrap_or_default(),
                        initial_candidate: matches!(config.definition.stages[0].kind, StageKind::Tracker).then(|| config.pin.clone()),
                        models: policy.models(&config.definition, ordinal), tree: None, with: vec![] }
                };
                let input = Input { schema: 1, id: format!("runtime:{}", Uuid::now_v7()), workspace: workspace.clone(),
                    expected_revision: state.revision, epoch: state.epoch, source: "runtime".into(),
                    source_sequence: state.source_sequences.get("runtime").copied().unwrap_or(0) + 1,
                    now_ms, grant: Grant::Controller, command, busy_resources: BTreeSet::new() };
                let before = state.items.values().filter(|i| active(i)).count();
                // The store supplies the state it committed: no per-item aggregate reload or speculative local reducer.
                let applied = self.store.workflow_apply_snapshot(&input);
                if let Some(wake) = &self.notice_wake { wake.notify_one(); }
                match applied {
                    Ok((receipt, next)) if receipt.result.is_ok() => {
                        let next = next.ok_or(Error::Refused("missing committed scheduling state"))?;
                        busy = busy - before + next.items.values().filter(|i| active(i)).count();
                        let board = scan.boards.get_mut(workspace).unwrap();
                        board.revision = next.revision;
                        board.state = Some(next);
                        scan.refused.remove(&retry_key);
                        if !queue { scan.last_start_ms = Some(now_ms); }
                    }
                    other => {
                        // Refusals do not change revision, and ownership can clear outside workflow state.
                        // Retry after a minute rather than creating 86,400 immutable receipts per day.
                        scan.refused.insert(retry_key, now_ms.saturating_add(60_000));
                        match other {
                            Ok((receipt, _)) => tracing::warn!(%workspace, item = %key, ?receipt.result, "automatic command refused"),
                            Err(error) => return Err(error.into()),
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn dependencies_ready(&self, workspace: &str, key: &str, policy: &StartPolicy, state: &State) -> Result<bool> {
        if policy.dependencies.get(key).is_some_and(|deps| deps.iter().any(|d| !finished(state, d))) { return Ok(false); }
        if !self.service_mode { return Ok(true); }
        let tasks = self.store.tasks_with_key(None, key).map_err(|_| Error::Refused("task blockers unreadable"))?;
        let Some(task) = tasks.iter().find(|t| t.workspace_id.to_string() == workspace) else { return Ok(false) };
        for block in self.store.blocks_for(task.id).map_err(|_| Error::Refused("task blockers unreadable"))? {
            let blocker = self.store.get_task(block.blocked_by).map_err(|_| Error::Refused("task blocker missing"))?;
            if blocker.status != farcooler_store::models::TaskStatus::Done { return Ok(false); }
        }
        Ok(true)
    }
}

/// File claims last through landing, even though landing releases build capacity.
fn file_conflict(boards: &BTreeMap<String, Board>, configs: &BTreeMap<String, Configuration>, workspace: &str,
    key: &str, policy: &StartPolicy) -> bool {
    let mine = configs.get(workspace);
    boards.iter().any(|(other_workspace, board)| {
        let other = configs.get(other_workspace);
        if workspace != other_workspace && mine.and_then(|c| c.adapter.as_ref()).zip(other.and_then(|c| c.adapter.as_ref()))
            .is_some_and(|(a, b)| match (a, b) {
                (farcooler_protocol::workflow_config::AdapterConfiguration::Coding(a),
                 farcooler_protocol::workflow_config::AdapterConfiguration::Coding(b)) => a.repository != b.repository,
            }) { return false; }
        board.state.as_ref().is_some_and(|state| state.items.iter().any(|(other_key, item)| {
            !item.definition.is_ticket() && item.group_of.is_none() && !matches!(item.phase, Phase::Done | Phase::Failed | Phase::Cancelled)
                && (policy.conflicts(key, other_key) || item.definition.start_policy.as_ref().is_some_and(|p|
                    p.conflicts(key, other_key) || policy.conflicts_with(key, p, other_key)))
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_workflow::scheduling::Budget;
    use farcooler_protocol::workflow_commands::{Action, Mutation};
    fn policy(budget: Budget) -> StartPolicy {
        StartPolicy { columns: ["todo".into()].into(), budget, files: Default::default(), dependencies: Default::default(),
            rotate_models: true, auto_queue: true }
    }
    fn publish(runtime: &Runtime, definition: Definition) { publish_on(runtime, "w", definition); }
    fn publish_on(runtime: &Runtime, workspace: &str, definition: Definition) {
        let state = runtime.state(workspace).unwrap();
        let action = Action::Publish { mutation: Mutation { id: format!("pub-{}", definition.pin()), revision: state.map_or(0, |s| s.revision),
            epoch: 1, sequence: 1 }, expected_pin: runtime.store.workflow_configuration(workspace).unwrap().map(|c| c.pin),
            definition, adapter: None, notifications: None };
        let answer = runtime.wire_command(workspace, "owner", action).unwrap();
        assert!(answer.pointer("/receipt/result/Ok").is_some(), "{answer}");
    }
    fn track(runtime: &Runtime, keys: &[&str]) {
        runtime.track_tickets(&keys.iter().map(|k| ("w".into(), (*k).into(), "todo".into())).collect::<Vec<_>>()).unwrap();
    }
    fn complete(runtime: &Runtime, key: &str, time: i64) {
        let effect = runtime.store.workflow_outbox("w").unwrap().into_iter()
            .find(|e| e.active && e.effect.item == key).unwrap().effect;
        let state = runtime.state("w").unwrap().unwrap();
        let claim = runtime.store.workflow_claim_effect("w", &effect.id, "fake-worker", state.revision,
            state.epoch, time, time + 1000).unwrap().unwrap();
        assert!(runtime.store.workflow_admit_effect(&claim, time).unwrap());
        let evidence = Evidence { candidate: "candidate".into(), manifest: "manifest".into(), receipt: "receipt".into(),
            policy_pin: effect.policy_pin.clone() };
        assert!(runtime.internal("w", Command::Observe { item: key.into(), effect: effect.id.clone(), run: effect.run.clone(),
            fence: effect.fence, outcome: Outcome::Passed(evidence) }, Grant::Operation { effect: effect.id,
                run: effect.run, fence: effect.fence }, time).unwrap().result.is_ok());
    }
    #[tokio::test]
    async fn fake_clock_and_load_start_three_hold_fourth_and_refill_on_finish() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Fixed(3)));
        publish(&runtime, d);
        track(&runtime, &["a", "b", "c", "d"]);
        runtime.schedule_with_load(&["w".into()], 100, Some(100)).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items.values().filter(|i| active(i)).count(), 3);
        assert!(state.items["d"].definition.is_ticket());
        complete(&runtime, "a", 200);
        runtime.schedule_with_load(&["w".into()], 201, Some(100)).unwrap();
        assert!(!runtime.state("w").unwrap().unwrap().items["d"].definition.is_ticket());
    }
    #[tokio::test]
    async fn load_and_known_files_and_dependencies_hold_starts_and_models_rotate() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.models.insert(d.stages[0].id.clone(), vec!["claude:opus".into(), "codex:gpt-5.5".into()]);
        let mut p = policy(Budget::Load { below_milli: 3000, min_start_gap_ms: 60_000 });
        p.files.insert("a".into(), ["shared.rs".into()].into());
        p.files.insert("b".into(), ["shared.rs".into()].into());
        p.dependencies.insert("c".into(), ["b".into()].into());
        d.start_policy = Some(p);
        publish(&runtime, d);
        track(&runtime, &["a", "b", "c", "d"]);
        for load in [None, Some(3000), Some(5000)] {
            runtime.schedule_with_load(&["w".into()], 100, load).unwrap();
            assert!(runtime.state("w").unwrap().unwrap().items.values().all(|i| i.definition.is_ticket()));
        }
        runtime.schedule_with_load(&["w".into()], 101, Some(2000)).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert!(active(&state.items["a"]) && state.items["d"].definition.is_ticket());
        runtime.schedule_with_load(&["w".into()], 60_101, Some(2000)).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert!(active(&state.items["d"]));
        assert!(state.items["b"].definition.is_ticket() && state.items["c"].definition.is_ticket());
        assert_ne!(state.items["a"].models, state.items["d"].models);
        complete(&runtime, "a", 60_102);
        runtime.schedule_with_load(&["w".into()], 120_101, Some(2000)).unwrap();
        assert!(active(&runtime.state("w").unwrap().unwrap().items["b"]));
        complete(&runtime, "b", 120_102);
        runtime.schedule_with_load(&["w".into()], 180_101, Some(2000)).unwrap();
        assert!(active(&runtime.state("w").unwrap().unwrap().items["c"]));
    }
    #[tokio::test]
    async fn published_policy_is_pumped_without_start_commands_and_can_pause_and_resume() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runtime");
        let mut runtime = Runtime::create(&root).await.unwrap();
        let mut d = coding_definition(false);
        d.start_policy = Some(policy(Budget::Fixed(0)));
        publish(&runtime, d.clone());
        track(&runtime, &["a", "b", "c", "d", "e"]);
        runtime.pump(100).await.unwrap();
        assert!(runtime.state("w").unwrap().unwrap().items.values().all(|i| i.definition.is_ticket()));
        d.start_policy.as_mut().unwrap().budget = Budget::Unlimited;
        publish(&runtime, d);
        // Restart reads the policy from the durable publication; no in-memory enable switch.
        drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        runtime.pump(200).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items.values().filter(|i| active(i)).count(), 5);
        assert!(state.items.values().all(|i| i.stage == 1), "all five provisions executed");
    }

    #[tokio::test]
    async fn republishing_keeps_the_active_runs_file_claims() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        let mut p = policy(Budget::Fixed(1));
        p.files.insert("a".into(), ["shared.rs".into()].into());
        d.start_policy = Some(p);
        publish(&runtime, d.clone());
        track(&runtime, &["a", "b"]);
        runtime.schedule_with_load(&["w".into()], 100, Some(0)).unwrap();
        let p = d.start_policy.as_mut().unwrap();
        p.budget = Budget::Unlimited;
        p.files.clear();
        p.files.insert("b".into(), ["shared.rs".into()].into());
        publish(&runtime, d);
        runtime.schedule_with_load(&["w".into()], 101, Some(0)).unwrap();
        assert!(runtime.state("w").unwrap().unwrap().items["b"].definition.is_ticket());
        complete(&runtime, "a", 102);
        runtime.schedule_with_load(&["w".into()], 103, Some(0)).unwrap();
        assert!(active(&runtime.state("w").unwrap().unwrap().items["b"]));
    }

    #[tokio::test]
    async fn file_claims_serialize_across_boards_without_holding_independent_work() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut first = coding_definition(false);
        first.stages.retain(|s| s.kind == StageKind::Build);
        let mut claims = policy(Budget::Unlimited);
        claims.files.insert("a".into(), ["shared.rs".into()].into());
        first.start_policy = Some(claims);
        publish(&runtime, first.clone());
        track(&runtime, &["a"]);
        runtime.schedule_with_load(&["w".into()], 100, Some(0)).unwrap();

        let mut second = first;
        let claims = second.start_policy.as_mut().unwrap();
        claims.files.clear();
        claims.files.insert("b".into(), ["shared.rs".into()].into());
        claims.files.insert("c".into(), ["independent.rs".into()].into());
        publish_on(&runtime, "other", second);
        runtime.track_tickets(&[("other".into(), "b".into(), "todo".into()),
            ("other".into(), "c".into(), "todo".into())]).unwrap();
        runtime.schedule_with_load(&["other".into()], 101, Some(0)).unwrap();
        let state = runtime.state("other").unwrap().unwrap();
        assert!(state.items["b"].definition.is_ticket(), "the first board still owns shared.rs");
        assert!(active(&state.items["c"]), "an independent file can build in parallel");

        complete(&runtime, "a", 102);
        runtime.schedule_with_load(&["other".into()], 103, Some(0)).unwrap();
        assert!(active(&runtime.state("other").unwrap().unwrap().items["b"]));
    }

    #[tokio::test]
    async fn removing_the_policy_holds_waiting_tickets_without_changing_admitted_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runtime");
        let runtime = Runtime::create(&root).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Fixed(1)));
        publish(&runtime, d.clone());
        track(&runtime, &["a", "b"]);
        runtime.schedule_with_load(&["w".into()], 100, Some(0)).unwrap();
        let admitted = runtime.state("w").unwrap().unwrap().items["a"].clone();

        d.start_policy = None;
        publish(&runtime, d.clone());
        runtime.schedule_with_load(&["w".into()], 101, Some(0)).unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items["a"], admitted);
        complete(&runtime, "a", 102);
        // Policy removal survives a restart and leaves the waiting ticket available for later admission.
        drop(runtime);
        let runtime = Runtime::open(&root).unwrap();
        runtime.schedule_with_load(&["w".into()], 103, Some(0)).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert!(finished(&state, "a"));
        assert!(state.items["b"].definition.is_ticket());

        d.start_policy = Some(policy(Budget::Unlimited));
        publish(&runtime, d);
        runtime.schedule_with_load(&["w".into()], 104, Some(0)).unwrap();
        assert!(active(&runtime.state("w").unwrap().unwrap().items["b"]));
    }

    #[tokio::test]
    async fn lowering_the_published_budget_preserves_runs_and_holds_new_starts() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Fixed(3)));
        publish(&runtime, d.clone());
        track(&runtime, &["a", "b", "c", "d"]);
        runtime.schedule_with_load(&["w".into()], 100, Some(0)).unwrap();
        d.start_policy.as_mut().unwrap().budget = Budget::Fixed(1);
        publish(&runtime, d);
        complete(&runtime, "a", 101);
        complete(&runtime, "b", 102);
        runtime.schedule_with_load(&["w".into()], 103, Some(0)).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert!(active(&state.items["c"]));
        assert!(state.items["d"].definition.is_ticket(), "the reduced budget is still full");
        assert_eq!(state.items["c"].definition.start_policy.as_ref().unwrap().budget, Budget::Fixed(3));
        complete(&runtime, "c", 104);
        runtime.schedule_with_load(&["w".into()], 105, Some(0)).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert!(active(&state.items["d"]));
        assert_eq!(state.items["d"].definition.start_policy.as_ref().unwrap().budget, Budget::Fixed(1));
    }

    #[tokio::test]
    async fn regression_load_reading_admits_only_one_run_across_boards() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Load { below_milli: 6000, min_start_gap_ms: 60_000 }));
        for board in ["w", "other"] {
            publish_on(&runtime, board, d.clone());
            runtime.track_tickets(&(0..20).map(|i| (board.into(), format!("t{i}"), "todo".into())).collect::<Vec<_>>()).unwrap();
        }
        runtime.schedule_with_load(&["w".into(), "other".into()], 100, Some(2000)).unwrap();
        let busy: usize = ["w", "other"].iter().map(|w| runtime.state(w).unwrap().unwrap().items.values().filter(|i| active(i)).count()).sum();
        assert_eq!(busy, 1);
        // Immediate pump re-entry and one-second service ticks must share the gate.
        for time in [100, 101, 1100, 2100, 10_100, 59_100, 60_099] {
            runtime.schedule_with_load(&["other".into()], time, Some(2000)).unwrap();
            let busy: usize = ["w", "other"].iter().map(|w| runtime.state(w).unwrap().unwrap().items.values().filter(|i| active(i)).count()).sum();
            assert_eq!(busy, 1, "load average still lags at {time}");
        }
        runtime.schedule_with_load(&["other".into(), "w".into()], 60_100, Some(2000)).unwrap();
        runtime.schedule_with_load(&["w".into()], 60_101, Some(2000)).unwrap();
        let busy: usize = ["w", "other"].iter().map(|w| runtime.state(w).unwrap().unwrap().items.values().filter(|i| active(i)).count()).sum();
        assert_eq!(busy, 2, "exactly one more start at the gap boundary");
        d.start_policy = Some(policy(Budget::Load { below_milli: 6000, min_start_gap_ms: 120_000 }));
        publish_on(&runtime, "other", d);
        for time in [60_102, 120_100, 180_099] {
            runtime.schedule_with_load(&["w".into()], time, Some(2000)).unwrap();
            let busy: usize = ["w", "other"].iter().map(|w| runtime.state(w).unwrap().unwrap().items.values().filter(|i| active(i)).count()).sum();
            assert_eq!(busy, 2, "republishing keeps the timer and the longest gap wins");
        }
        for load in [None, Some(6000)] {
            runtime.schedule_with_load(&["w".into()], 180_100, load).unwrap();
        }
        runtime.schedule_with_load(&["w".into()], 180_100, Some(2000)).unwrap();
        let busy: usize = ["w", "other"].iter().map(|w| runtime.state(w).unwrap().unwrap().items.values().filter(|i| active(i)).count()).sum();
        assert_eq!(busy, 3, "blocked scans do not move the deadline");
    }

    #[tokio::test]
    async fn regression_fixed_budget_counts_other_boards_even_without_policy() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Fixed(3)));
        publish(&runtime, d.clone());
        track(&runtime, &["a", "b", "c"]);
        runtime.schedule_with_load(&["w".into()], 100, Some(0)).unwrap();
        d.start_policy = None;
        publish(&runtime, d.clone());
        d.start_policy = Some(policy(Budget::Fixed(3)));
        publish_on(&runtime, "other", d);
        runtime.track_tickets(&[("other".into(), "d".into(), "todo".into())]).unwrap();
        runtime.schedule_with_load(&["other".into()], 101, Some(0)).unwrap();
        assert!(runtime.state("other").unwrap().unwrap().items["d"].definition.is_ticket());
        complete(&runtime, "a", 102);
        runtime.schedule_with_load(&["other".into()], 103, Some(0)).unwrap();
        assert!(active(&runtime.state("other").unwrap().unwrap().items["d"]));
    }

    #[tokio::test]
    async fn fixed_and_load_constraints_apply_together_on_the_runner() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Fixed(3)));
        publish(&runtime, d.clone());
        track(&runtime, &["a", "b", "c", "d"]);
        d.start_policy = Some(policy(Budget::Load { below_milli: 6000, min_start_gap_ms: 120_000 }));
        publish_on(&runtime, "other", d);
        runtime.track_tickets(&[("other".into(), "z".into(), "todo".into())]).unwrap();
        for load in [None, Some(6000)] {
            runtime.schedule_with_load(&["w".into(), "other".into()], 100, load).unwrap();
            assert!(runtime.state("w").unwrap().unwrap().items.values().all(|i| i.definition.is_ticket()));
        }
        for ordinal in 0..4 {
            let time = 101 + ordinal * 120_000;
            runtime.schedule_with_load(&["w".into(), "other".into()], time, Some(2000)).unwrap();
            let busy: usize = ["w", "other"].iter().map(|w| runtime.state(w).unwrap().unwrap().items.values().filter(|i| active(i)).count()).sum();
            assert_eq!(busy, usize::try_from(ordinal + 1).unwrap().min(3));
        }
        complete(&runtime, "a", 360_102);
        runtime.schedule_with_load(&["other".into()], 360_103, Some(2000)).unwrap();
        assert!(active(&runtime.state("other").unwrap().unwrap().items["z"]));
    }

    #[tokio::test]
    async fn regression_scan_skips_unconfigured_boards_and_reads_each_state_once() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        track(&runtime, &["a", "b", "c", "d"]);
        runtime.schedule_with_load(&["w".into()], 100, Some(0)).unwrap();
        assert_eq!(runtime.scheduling.lock().unwrap().state_reads, 0, "tracker-only board has no scheduling work");
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Fixed(0)));
        publish(&runtime, d);
        runtime.schedule_with_load(&["w".into()], 101, Some(0)).unwrap();
        assert_eq!(runtime.scheduling.lock().unwrap().state_reads, 1);
        runtime.schedule_with_load(&["w".into()], 102, Some(0)).unwrap();
        assert_eq!(runtime.scheduling.lock().unwrap().state_reads, 1, "unchanged idle state stays cached");
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Unlimited));
        publish(&runtime, d);
        runtime.schedule_with_load(&["w".into()], 103, Some(0)).unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items.values().filter(|i| active(i)).count(), 4);
        assert_eq!(runtime.scheduling.lock().unwrap().state_reads, 1, "starts and file checks reuse the scan snapshot");
        complete(&runtime, "a", 104);
        runtime.schedule_with_load(&["w".into()], 105, Some(0)).unwrap();
        assert_eq!(runtime.scheduling.lock().unwrap().state_reads, 2, "external transitions invalidate the snapshot once");
    }

    #[tokio::test]
    async fn regression_refused_start_backs_off_receipts() { refused_backoff(false).await; }
    #[tokio::test]
    async fn regression_refused_queue_backs_off_receipts() { refused_backoff(true).await; }
    async fn refused_backoff(queue: bool) {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.start_policy = Some(policy(Budget::Unlimited));
        publish(&runtime, d.clone());
        track(&runtime, &["a"]);
        // Simulate retained state that the reducer cannot admit: a tracker definition cannot Start,
        // or a WaitingJoin run with no candidate cannot Queue. Both refusals are durable receipts.
        let mut state = runtime.state("w").unwrap().unwrap();
        if queue {
            let item = state.items.get_mut("a").unwrap();
            item.definition = d;
            item.phase = Phase::WaitingJoin;
            item.stage = 0;
            item.definition.stages[0].kind = StageKind::Join { group: "landing".into(), adopt: false };
        } else {
            d = Definition::ticket_at("todo").unwrap();
            d.start_policy = Some(policy(Budget::Unlimited));
            publish(&runtime, d);
            state = runtime.state("w").unwrap().unwrap();
        }
        let json = serde_json::to_string(&state).unwrap().replace('\'', "''");
        farcooler_store::testing::exec_for_tests(&runtime.store, &format!("UPDATE workflow_states SET state='{json}' WHERE workspace='w'"));
        let count = || farcooler_store::testing::count_rows_for_tests(&runtime.store, "workflow_receipts");
        let before = count();
        runtime.schedule_with_load(&["w".into()], 100, Some(0)).unwrap();
        assert_eq!(count(), before + 1);
        for time in [1100, 2100, 10_100, 59_100] {
            runtime.schedule_with_load(&["w".into()], time, Some(0)).unwrap();
        }
        assert_eq!(count(), before + 1, "unchanged refusal must not write every second (queue={queue})");
        runtime.schedule_with_load(&["w".into()], 60_100, Some(0)).unwrap();
        assert_eq!(count(), before + 2, "retry eventually without a restart");
    }

}
