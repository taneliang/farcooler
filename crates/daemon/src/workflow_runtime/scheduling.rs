//! Owner-published automatic starts. Every transition still passes through the durable reducer.
use super::*;
use farcooler_workflow::scheduling::StartPolicy;

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
        for workspace in workspaces {
            if self.shutdown.as_ref().is_some_and(|s| *s.borrow()) { return Ok(()); }
            // A removed board must never acquire new work.
            if self.service_mode && self.store.workflow_board_deleted(workspace)? { continue; }
            let config = self.store.workflow_configuration(workspace)?;
            let Some(state) = self.state(workspace)? else { continue };
            let keys: Vec<_> = state.items.keys().cloned().collect();
            for key in keys {
                let Some(state) = self.state(workspace)? else { continue };
                let Some(item) = state.items.get(&key) else { continue };
                // Queue is idempotent by phase, and uses the admitted run's policy, not a later publication.
                if item.phase == Phase::WaitingJoin && item.definition.start_policy.as_ref().is_some_and(|p| p.auto_queue) {
                    self.internal(workspace, Command::Queue { item: key }, Grant::Controller, now_ms)?;
                    continue;
                }
                let Some(config) = &config else { continue };
                let Some(policy) = config.definition.start_policy.as_ref() else { continue };
                if crate::workflow_deciders::names_a_removed_member(&self.store, &config.definition).is_some() { continue; }
                if !item.definition.is_ticket() || item.follows.is_some() || item.phase != Phase::WaitingApproval
                    || !policy.columns.contains(&item.definition.stages[item.stage].id) { continue; }
                let busy = state.items.values().filter(|i| active(i)).count();
                if !policy.budget.permits(busy, load)
                    || self.file_conflict(workspace, &key, policy)?
                    || !self.dependencies_ready(workspace, &key, policy, &state)? { continue; }
                let ordinal = state.items.values().filter(|i| !i.definition.is_ticket() && i.group_of.is_none()).count()
                    + state.archive.len();
                let receipt = self.internal(workspace, Command::Start { item: key.clone(), definition: config.definition.clone(),
                    policy_pin: config.pin.clone(), resource: Exec::item_resource(workspace, &key),
                    dependencies: policy.dependencies.get(&key).cloned().unwrap_or_default(),
                    initial_candidate: matches!(config.definition.stages[0].kind, StageKind::Tracker).then(|| config.pin.clone()),
                    models: policy.models(&config.definition, ordinal), tree: None, with: vec![] }, Grant::Controller, now_ms)?;
                if let Err(reason) = receipt.result { tracing::warn!(%workspace, item = %key, ?reason, "automatic start refused"); }
            }
        }
        Ok(())
    }
    /// Keep file claims until landing finishes, even though landing releases the agent budget.
    fn file_conflict(&self, workspace: &str, key: &str, policy: &StartPolicy) -> Result<bool> {
        let mine = self.store.workflow_configuration(workspace)?;
        for other_workspace in self.store.workflow_workspaces()? {
            let other = self.store.workflow_configuration(&other_workspace)?;
            if workspace != other_workspace && mine.as_ref().and_then(|c| c.adapter.as_ref())
                .zip(other.as_ref().and_then(|c| c.adapter.as_ref()))
                .is_some_and(|(a, b)| match (a, b) {
                    (farcooler_protocol::workflow_config::AdapterConfiguration::Coding(a),
                     farcooler_protocol::workflow_config::AdapterConfiguration::Coding(b)) => a.repository != b.repository,
                }) { continue; }
            let Some(state) = self.state(&other_workspace)? else { continue };
            for (other_key, item) in &state.items {
                if item.definition.is_ticket() || item.group_of.is_some()
                    || matches!(item.phase, Phase::Done | Phase::Failed | Phase::Cancelled) { continue; }
                if policy.conflicts(key, other_key)
                    || item.definition.start_policy.as_ref().is_some_and(|p| p.conflicts(key, other_key)
                        || policy.conflicts_with(key, p, other_key)) { return Ok(true); }
            }
        }
        Ok(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_workflow::scheduling::Budget;
    use farcooler_protocol::workflow_commands::{Action, Mutation};
    fn policy(budget: Budget) -> StartPolicy {
        StartPolicy { columns: ["todo".into()].into(), budget, files: Default::default(), dependencies: Default::default(),
            rotate_models: true, auto_queue: true }
    }
    fn publish(runtime: &Runtime, definition: Definition) {
        let state = runtime.state("w").unwrap();
        let action = Action::Publish { mutation: Mutation { id: format!("pub-{}", definition.pin()), revision: state.map_or(0, |s| s.revision),
            epoch: 1, sequence: 1 }, expected_pin: runtime.store.workflow_configuration("w").unwrap().map(|c| c.pin),
            definition, adapter: None, notifications: None };
        let answer = runtime.wire_command("w", "owner", action).unwrap();
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
        let mut p = policy(Budget::Load { below_milli: 3000 });
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
        assert!(active(&state.items["a"]) && active(&state.items["d"]));
        assert!(state.items["b"].definition.is_ticket() && state.items["c"].definition.is_ticket());
        assert_ne!(state.items["a"].models, state.items["d"].models);
        complete(&runtime, "a", 102);
        runtime.schedule_with_load(&["w".into()], 103, Some(2000)).unwrap();
        assert!(active(&runtime.state("w").unwrap().unwrap().items["b"]));
        complete(&runtime, "b", 104);
        runtime.schedule_with_load(&["w".into()], 105, Some(2000)).unwrap();
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

}
