//! Owner-published automatic starts. Every transition still passes through the durable reducer.
use super::*;
use farcooler_workflow::scheduling::{Budget, StartPolicy};
use std::collections::BTreeMap;
use farcooler_store::workflow_publication::Configuration;

const REFUSED_RETRY_MS: i64 = 60_000;

#[derive(Default)]
pub(super) struct Scheduler {
    boards: BTreeMap<String, Board>,
    refused: BTreeMap<(String, String, bool), i64>,
    last_start_ms: Option<i64>,
    recovered_start: bool,
    #[cfg(test)]
    state_reads: usize,
    #[cfg(test)]
    load_override_milli: Option<u64>,
}
struct Board {
    revision: Option<u64>,
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
    pub(super) fn automatic_queue_deferred(&self, workspace: &str, item: &str, now_ms: i64) -> bool {
        self.scheduling.lock().unwrap().refused.get(&(workspace.into(), item.into(), true))
            .is_some_and(|until| now_ms < *until)
    }
    pub(super) fn defer_automatic_queue(&self, workspace: &str, item: &str, now_ms: i64) {
        self.scheduling.lock().unwrap().refused.insert((workspace.into(), item.into(), true), now_ms.saturating_add(REFUSED_RETRY_MS));
    }
    pub(super) fn schedule_ready(&self, workspaces: &[String], now_ms: i64, ready_queues: &BTreeSet<(String, String)>) -> Result<()> {
        #[cfg(test)]
        let load = {
            let fake = self.scheduling.lock().unwrap().load_override_milli;
            fake.or_else(load_milli)
        };
        #[cfg(not(test))]
        let load = load_milli();
        self.schedule(workspaces, now_ms, load, Some(ready_queues))
    }
    #[cfg(test)]
    fn schedule_with_load(&self, workspaces: &[String], now_ms: i64, load: Option<u64>) -> Result<()> {
        self.schedule(workspaces, now_ms, load, None)
    }
    fn schedule(&self, workspaces: &[String], now_ms: i64, load: Option<u64>,
        ready_queues: Option<&BTreeSet<(String, String)>>) -> Result<()>
    {
        let mut scan = self.scheduling.lock().unwrap();
        // Admission history outlives both completed runs and runtime restarts. Read it once,
        // rather than parsing the durable event log on every one-second service scan.
        if !scan.recovered_start {
            scan.last_start_ms = self.store.workflow_last_automatic_start_ms()?;
            scan.recovered_start = true;
        }
        // Inventory belongs to the runner, even when this pass was woken for only one board.
        let all = self.store.workflow_scheduling_workspaces()?;
        scan.boards.retain(|w, _| all.contains(w));
        scan.refused.retain(|(w, _, _), _| all.contains(w));
        let mut configs = BTreeMap::new();
        for workspace in &all {
            let config = self.store.workflow_configuration(workspace)?;
            let enabled = config.as_ref().is_some_and(|c| c.definition.start_policy.is_some());
            let revision = self.store.workflow_state_revision(workspace)?;
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
            let mut keys: Vec<_> = state.items.keys().cloned().collect();
            let config = configs.get(workspace);
            let policy = config.and_then(|c| c.definition.start_policy.as_ref());
            if let Some(policy) = policy {
                let startable = |key: &String| {
                    let item = &state.items[key];
                    item.definition.is_ticket() && item.phase == Phase::WaitingApproval
                        && policy.columns.contains(&item.definition.stages[item.stage].id)
                };
                let mut starts: Vec<_> = keys.iter().filter(|key| startable(key)).cloned().collect();
                if starts.len() > 1 {
                    let order = self.board_order(state)?;
                    starts.sort_by_key(|key| order.get(key).copied().unwrap_or(usize::MAX));
                    let mut ordered = starts.into_iter();
                    // Keep the existing scan order of auto-queue commands.
                    for key in keys.iter_mut().filter(|key| startable(key)) {
                        *key = ordered.next().expect("one ticket per start candidate");
                    }
                }
            }
            let removed_member = config.is_some_and(|c| crate::workflow_deciders::names_a_removed_member(&self.store, &c.definition).is_some());
            for key in keys {
                let state = scan.boards[workspace].state.as_ref().unwrap();
                let Some(item) = state.items.get(&key) else { continue };
                // Queue follows the admitted policy, including after publication removal.
                let queue = item.phase == Phase::WaitingJoin && (item.queue_hold.is_some()
                    || item.definition.start_policy.as_ref().is_some_and(|p| p.auto_queue));
                if queue && ready_queues.is_some_and(|ready| !ready.contains(&(workspace.clone(), key.clone()))) { continue; }
                let retry_key = (workspace.clone(), key.clone(), queue);
                if scan.refused.get(&retry_key).is_some_and(|until| now_ms < *until) { continue; }
                let command = if queue { Command::Queue { item: key.clone() } } else {
                    let (Some(config), Some(policy)) = (config, policy) else { continue };
                    if removed_member || !item.definition.is_ticket() || item.follows.is_some() || item.phase != Phase::WaitingApproval
                        || !policy.columns.contains(&item.definition.stages[item.stage].id) { continue; }
                    // Active runs consume budget even while waiting for an execution slot.
                    // All published machine constraints apply together.
                    if start_gap_ms.is_some_and(|gap| scan.last_start_ms.is_some_and(|last|
                        now_ms < last || (now_ms.saturating_sub(last) as u64) < gap))
                        || !budgets.iter().all(|b| b.permits(busy, load))
                        || file_conflict(&scan.boards, &configs, workspace, &key, policy)
                        || !self.dependencies_ready(workspace, &key, policy, state)? { continue; }
                    let ordinal = state.items.values().filter(|i| !i.definition.is_ticket() && i.group_of.is_none()).count() + state.archive.len();
                    Command::Start { start_load: Some(farcooler_workflow::StartLoad { sample_milli: load,
                        below_milli: budgets.iter().filter_map(|b| match b { Budget::Load { below_milli, .. } => Some(*below_milli), _ => None }).min() }), item: key.clone(), definition: config.definition.clone(), policy_pin: config.pin.clone(),
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
                        board.revision = Some(next.revision);
                        board.state = Some(next);
                        scan.refused.remove(&retry_key);
                        if !queue { scan.last_start_ms = Some(now_ms); }
                    }
                    other => {
                        // Refusals do not change revision, and ownership can clear outside workflow state.
                        // Retry after a minute rather than creating 86,400 immutable receipts per day.
                        scan.refused.insert(retry_key, now_ms.saturating_add(REFUSED_RETRY_MS));
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
        let Some(task) = self.store.workflow_item_task(workspace, key)? else { return Ok(false) };
        for block in self.store.blocks_for(task).map_err(|_| Error::Refused("task blockers unreadable"))? {
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
    use farcooler_protocol::workflow_commands::{Action, Mutation};
    fn policy(budget: Budget) -> StartPolicy {
        StartPolicy { columns: ["todo".into()].into(), budget, files: Default::default(), dependencies: Default::default(),
            rotate_models: true, auto_queue: true }
    }
    fn publish(runtime: &Runtime, definition: Definition) { publish_on(runtime, "w", definition); }
    fn publish_on(runtime: &Runtime, workspace: &str, definition: Definition) {
        let state = runtime.state(workspace).unwrap();
        let action = Action::Publish { reason: None, mutation: Mutation { id: format!("pub-{}", definition.pin()), revision: state.map_or(0, |s| s.revision),
            epoch: 1, sequence: 1 }, expected_pin: runtime.store.workflow_configuration(workspace).unwrap().map(|c| c.pin),
            definition, adapter: None, notifications: None };
        let answer = runtime.wire_command(workspace, "owner", action).unwrap();
        assert!(answer.pointer("/receipt/result/Ok").is_some(), "{answer}");
    }
    fn track(runtime: &Runtime, keys: &[&str]) {
        runtime.track_tickets(&keys.iter().map(|k| ("w".into(), (*k).into(), "todo".into())).collect::<Vec<_>>()).unwrap();
    }
    fn complete(runtime: &Runtime, key: &str, time: i64) {
        let effect = runtime.store.workflow_dispatch_effects("w", &[]).unwrap().into_iter()
            .find(|e| e.effect.item == key).unwrap().effect;
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
    fn todo_cards(runtime: &Runtime, titles: &[&str]) -> (String, Vec<farcooler_store::models::Task>) {
        use farcooler_store::models::{Actor, TaskStatus};
        let host = Uuid::now_v7();
        let root = runtime.store.create_repository_root(host, "/repos/ordered", 0).unwrap();
        let repo = runtime.store.create_repository(host, root.id, "ordered", "/repos/ordered/.git", "").unwrap();
        let workspace = runtime.store.ensure_main_workspace(repo.id).unwrap().id;
        let tasks = titles.iter().map(|title| {
            let task = runtime.store.create_task(workspace, title, Actor::User).unwrap();
            runtime.store.set_task_status(task.id, TaskStatus::Todo, Actor::User).unwrap()
        }).collect();
        (workspace.to_string(), tasks)
    }

    fn build_only(budget: Budget) -> Definition {
        let mut definition = coding_definition(false);
        definition.stages.retain(|stage| stage.kind == StageKind::Build);
        definition.start_policy = Some(policy(budget));
        definition
    }

    #[tokio::test]
    async fn automatic_starts_follow_the_board_order_out_of_key_order() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let (workspace, tasks) = todo_cards(&runtime, &["first filed", "second filed", "third filed"]);
        publish_on(&runtime, &workspace, build_only(Budget::Fixed(3)));
        runtime.track_tickets(&tasks.iter().map(|task| (workspace.clone(), task.key.clone(), "todo".into())).collect::<Vec<_>>()).unwrap();
        let id = Uuid::parse_str(&workspace).unwrap();
        runtime.store.set_line(id, farcooler_store::waits::TaskLine::Agent,
            &[tasks[1].id, tasks[2].id], farcooler_store::models::Actor::Manager).unwrap();
        runtime.store.reorder_task(tasks[2].id, tasks[1].id, false).unwrap();

        runtime.schedule_with_load(std::slice::from_ref(&workspace), 100, Some(0)).unwrap();
        let started: Vec<_> = runtime.events(&workspace).unwrap().into_iter().filter_map(|input| {
            if let Command::Start { item, .. } = input.command { Some(item) } else { None }
        }).collect();
        assert_eq!(started, [2, 1, 0].map(|index| tasks[index].key.clone()));
    }

    #[tokio::test]
    async fn full_pools_do_not_return_start_allowances_on_later_scans() {
        for (stage, pool, cap) in [(StageKind::Build, Pool::Agent, 4), (StageKind::Provision, Pool::Work, 2)] {
            let dir = tempfile::tempdir().unwrap();
            let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
            runtime.set_effect_cap(4);
            runtime.places().work = 2;
            let mut definition = coding_definition(false);
            definition.stages.retain(|s| s.kind == stage);
            let budget = cap + 1;
            definition.start_policy = Some(policy(Budget::Fixed(budget)));
            publish(&runtime, definition);
            let tickets = ["a", "b", "c", "d", "e", "f", "g", "h"];
            track(&runtime, &tickets);
            let boards = ["w".into()];
            let mut running = Vec::new();
            let mut futures = Vec::new();
            let mut count = 0;
            runtime.launch_ready(&boards, 100, &mut running, &mut futures, &mut count).await.unwrap();
            assert_eq!(running.len(), cap);
            assert!(running.iter().all(|job| job.pool == Some(pool)));
            assert_eq!(runtime.places().waiting.len(), 1);

            for time in [1100, 61_100, 121_100] {
                runtime.launch_ready(&boards, time, &mut running, &mut futures, &mut count).await.unwrap();
                let state = runtime.state("w").unwrap().unwrap();
                assert_eq!(state.items.values().filter(|item| active(item)).count(), budget,
                    "{pool:?} waiters still count against the fixed budget at {time}");
                for key in &tickets[budget..] {
                    assert!(state.items[*key].definition.is_ticket(), "{key} should still be in To Do");
                }
                assert_eq!(runtime.places().waiting.len(), 1);
            }
        }
    }

    #[tokio::test]
    async fn a_reordered_waiting_build_gets_the_next_slot_without_preempting_the_holder() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        runtime.set_effect_cap(1);
        let (workspace, tasks) = todo_cards(&runtime, &["holder", "old waiter", "new waiter", "still To Do"]);
        let [holder, old, new, todo] = tasks.as_slice() else { unreachable!() };
        let boards = std::slice::from_ref(&workspace);
        let definition = build_only(Budget::Fixed(3));
        let new_item = new.id.to_string();
        for item in [&holder.key, &old.key, &new_item] {
            super::super::concurrency_tests::enqueue_item(&mut runtime, &workspace, item, definition.clone());
        }
        let mut running = Vec::new();
        let mut futures = Vec::new();
        let mut count = 0;
        runtime.launch_ready(boards, 100, &mut running, &mut futures, &mut count).await.unwrap();
        assert_eq!(running[0].effect.item, holder.key);
        publish_on(&runtime, &workspace, definition);
        runtime.track_tickets(&[(workspace.clone(), todo.key.clone(), "todo".into())]).unwrap();

        let position = |runtime: &Runtime, key: &str| {
            let read = runtime.read_get(&workspace).unwrap();
            let waiting = read["places"]["waiting"].as_array().unwrap().iter().find(|row| row["item"] == key).unwrap();
            assert_eq!(read["state"]["items"][key]["execution"]["position"], waiting["position"]);
            waiting["position"].as_u64().unwrap()
        };
        assert_eq!(position(&runtime, &old.key), 1);
        assert_eq!(position(&runtime, &new_item), 2);
        let revision = runtime.state(&workspace).unwrap().unwrap().revision;
        let id = Uuid::parse_str(&workspace).unwrap();
        runtime.store.set_line(id, farcooler_store::waits::TaskLine::Build, &[new.id, old.id],
            farcooler_store::models::Actor::Manager).unwrap();
        assert_eq!(position(&runtime, &new_item), 1, "the existing build line command reorders live runs");
        assert!(runtime.store.get_task(new.id).unwrap().wait.is_none(), "the engine supplies the live wait");
        runtime.store.reorder_task(old.id, new.id, false).unwrap();
        assert_eq!(position(&runtime, &old.key), 1);
        runtime.store.reorder_task(new.id, old.id, false).unwrap();
        assert_eq!(position(&runtime, &new_item), 1);
        assert_eq!(position(&runtime, &old.key), 2);
        runtime.launch_ready(boards, 101, &mut running, &mut futures, &mut count).await.unwrap();
        let state = runtime.state(&workspace).unwrap().unwrap();
        assert_eq!(state.revision, revision);
        assert!(state.items[&todo.key].definition.is_ticket());
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].effect.item, holder.key);
        assert!(!*running[0].cancel.borrow());

        let finished = running.pop().unwrap();
        runtime.activities.lock().unwrap().remove(&(workspace.clone(), finished.effect.id.clone()));
        futures.clear();
        let evidence = Evidence {
            candidate: "candidate".into(), manifest: "manifest".into(), receipt: "receipt".into(),
            policy_pin: finished.effect.policy_pin.clone(),
        };
        assert!(runtime.observe(&workspace, &finished.effect, Outcome::Passed(evidence), false, 102).unwrap());
        runtime.launch_ready(boards, 103, &mut running, &mut futures, &mut count).await.unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].effect.item, new_item, "the later-started build wins the freed slot");
        assert_eq!(position(&runtime, &old.key), 1);
        assert!(!runtime.state(&workspace).unwrap().unwrap().items[&todo.key].definition.is_ticket(),
            "the holder's completion freed one start allowance");
    }

    #[tokio::test]
    async fn continuous_sweep_advances_load_gap_while_an_effect_stays_running() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        super::super::concurrency_tests::enqueue_item(&mut runtime, "w", "gate", coding_definition(false));
        let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
        let binding = Binding::new("w", &effect, 1, 0, &runtime.base);
        let (cancel, _) = tokio::sync::watch::channel(false);
        // Keep the actual continuous sweep open, as a long landing gate does. No external process runs.
        let tree = runtime.tree_lock(&runtime.tree(&binding)).try_read_owned().unwrap();
        let job = Running { workspace: "w".into(), effect, binding, cancel, pool: None, _tree: tree, _landing: None, reconciling: false };
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.stages[0].progression = Progression::AwaitApproval { principal: "owner".into() };
        // Isolate the admission timer from host load; near-limit fake samples are tested separately.
        d.start_policy = Some(policy(Budget::Load { below_milli: u64::MAX, min_start_gap_ms: 60_000 }));
        publish(&runtime, d);
        track(&runtime, &["a", "b", "c"]);
        runtime.continuous = true;
        let handle = runtime.command_handle(&runtime.controller()).unwrap();
        let (finish, finished) = oneshot::channel();
        tokio::time::pause();
        let task = tokio::spawn(async move {
            let (_, placeholder) = mpsc::channel(1);
            let mut inbox = std::mem::replace(&mut runtime.inbox, placeholder);
            runtime.sweep_loop(100, &mut inbox, &mut vec![job],
                &mut vec![Some(Box::pin(async move {
                    finished.await.unwrap();
                    Ok(Outcome::SafeFailure)
                }))]).await.unwrap();
            runtime
        });
        let first = handle.state("w").await.unwrap().unwrap();
        assert!(!first.items["a"].definition.is_ticket());
        assert!(first.items["b"].definition.is_ticket());
        // Reads and scans before the gap cannot spend another allowance.
        for seconds in [1, 1, 57] {
            tokio::time::advance(Duration::from_secs(seconds)).await;
            handle.state("w").await.unwrap();
            assert!(handle.state("w").await.unwrap().unwrap().items["b"].definition.is_ticket());
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        handle.state("w").await.unwrap();
        let after_gap = handle.state("w").await.unwrap().unwrap();
        tokio::time::advance(Duration::from_secs(60)).await;
        handle.state("w").await.unwrap();
        let after_second_gap = handle.state("w").await.unwrap().unwrap();
        finish.send(()).unwrap();
        let runtime = task.await.unwrap();
        assert!(!after_gap.items["b"].definition.is_ticket(), "a running effect must not freeze admission time");
        assert!(after_gap.items["c"].definition.is_ticket(), "one allowance per gap");
        assert!(!after_second_gap.items["c"].definition.is_ticket(), "later gaps keep advancing in the same sweep");
        let observation = runtime.events("w").unwrap().into_iter().find(|event|
            matches!(&event.command, Command::Observe { item, .. } if item == "gate")).unwrap();
        assert!(observation.now_ms >= 120_100, "settlement must use the advanced clock too");
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
        assert_eq!(state.items["a"].start_load, Some(farcooler_workflow::StartLoad { sample_milli: Some(100), below_milli: None }));
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
        const FAKE_LOAD_MILLI: u64 = 100;
        let tickets = ["a", "b", "c", "d", "e"];
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runtime");
        let mut runtime = Runtime::create(&root).await.unwrap();
        runtime.scheduling.get_mut().unwrap().load_override_milli = Some(FAKE_LOAD_MILLI);
        let mut d = coding_definition(false);
        d.stages[1].progression = Progression::AwaitApproval { principal: "owner".into() };
        d.start_policy = Some(policy(Budget::Fixed(0)));
        publish(&runtime, d.clone());
        track(&runtime, &tickets);
        runtime.pump(100).await.unwrap();
        assert!(runtime.state("w").unwrap().unwrap().items.values().all(|i| i.definition.is_ticket()));
        d.start_policy.as_mut().unwrap().budget = Budget::Unlimited;
        publish(&runtime, d);
        // Restart reads the policy from the durable publication; no in-memory enable switch.
        drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        runtime.scheduling.get_mut().unwrap().load_override_milli = Some(FAKE_LOAD_MILLI);
        // Provision one worktree per sweep to avoid concurrent Git writes in the fixture repository.
        runtime.places().work = 1;
        for _ in &tickets { runtime.pump(200).await.unwrap(); }
        let state = runtime.state("w").unwrap().unwrap();
        let expected_load = Some(farcooler_workflow::StartLoad {
            sample_milli: Some(FAKE_LOAD_MILLI), below_milli: None,
        });
        for key in tickets {
            let item = &state.items[key];
            assert_eq!(item.stage, 1, "{key} reached Build after provisioning (phase: {:?})", item.phase);
            assert_eq!(item.phase, Phase::WaitingApproval, "{key} is waiting to build");
            assert_eq!(item.start_load, expected_load, "{key}'s automatic start recorded the test's load sample");
        }
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
    async fn near_limit_burst_keeps_load_allowance_across_scans_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runtime");
        let runtime = Runtime::create(&root).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Load { below_milli: 6000, min_start_gap_ms: 60_000 }));
        publish(&runtime, d.clone());
        track(&runtime, &["a", "b", "c", "d"]);
        runtime.schedule_with_load(&["w".into()], 100, Some(5999)).unwrap();
        for time in [100, 101, 1100, 2100] {
            runtime.schedule_with_load(&["w".into()], time, Some(5999)).unwrap();
            assert_eq!(runtime.state("w").unwrap().unwrap().items.values().filter(|i| active(i)).count(), 1);
        }
        // Completion and restart cannot turn the same lagging load into another allowance.
        complete(&runtime, "a", 2101);
        drop(runtime);
        let runtime = Runtime::open(&root).unwrap();
        publish_on(&runtime, "other", d);
        runtime.track_tickets(&[("other".into(), "z".into(), "todo".into())]).unwrap();
        for time in [99, 2102, 3100, 59_100, 60_099] {
            runtime.schedule_with_load(&["other".into(), "w".into()], time, Some(5999)).unwrap();
            for board in ["w", "other"] {
                assert!(runtime.state(board).unwrap().unwrap().items.values().all(|i| !active(i)),
                    "restart must retain the runner-wide allowance at {time}");
            }
        }
        for load in [None, Some(6000), Some(7000)] {
            runtime.schedule_with_load(&["w".into()], 60_100, load).unwrap();
            assert!(runtime.state("w").unwrap().unwrap().items["b"].definition.is_ticket());
        }
        runtime.schedule_with_load(&["w".into(), "other".into()], 60_100, Some(5999)).unwrap();
        runtime.schedule_with_load(&["other".into()], 60_100, Some(5999)).unwrap();
        let busy: usize = ["w", "other"].iter().map(|w|
            runtime.state(w).unwrap().unwrap().items.values().filter(|i| active(i)).count()).sum();
        assert_eq!(busy, 1, "a fresh acceptable sample after the gap permits exactly one more start");
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
    async fn automatic_start_records_effective_load_across_boards_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runtime");
        let runtime = Runtime::create(&root).await.unwrap();
        let mut d = coding_definition(false);
        d.start_policy = Some(policy(Budget::Fixed(3)));
        publish(&runtime, d.clone());
        track(&runtime, &["a"]);
        d.start_policy = Some(policy(Budget::Load { below_milli: 6000, min_start_gap_ms: 60_000 }));
        publish_on(&runtime, "other", d);
        runtime.schedule_with_load(&["w".into()], 100, Some(4100)).unwrap();
        let expected = farcooler_workflow::StartLoad { sample_milli: Some(4100), below_milli: Some(6000) };
        let event = runtime.events("w").unwrap().into_iter().find(|e| matches!(e.command, Command::Start { .. })).unwrap();
        assert!(matches!(event.command, Command::Start { start_load: Some(ref load), .. } if load == &expected));
        assert_eq!(runtime.state("w").unwrap().unwrap().items["a"].start_load, Some(expected.clone()));
        drop(runtime);
        let runtime = Runtime::open(&root).unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items["a"].start_load, Some(expected));
    }

    #[tokio::test]
    async fn boundary_sized_automatic_start_keeps_load_and_archives_it() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Load { below_milli: 6000, min_start_gap_ms: 60_000 }));
        publish(&runtime, d.clone());
        let deps: Vec<String> = (0..60).map(|n| format!("dep-{n:03}-{}", "x".repeat(90))).collect();
        let refs: Vec<&str> = deps.iter().map(String::as_str).collect();
        track(&runtime, &refs);
        track(&runtime, &["a"]);
        for key in &deps {
            assert!(runtime.internal("w", Command::Move { item: key.clone(), to: "done".into() }, Grant::Controller, 1).unwrap().result.is_ok());
        }
        let p = d.start_policy.as_mut().unwrap();
        p.dependencies.insert("a".into(), deps.into_iter().collect());
        p.files.insert("a".into(), ["x".into()].into());
        let state = runtime.state("w").unwrap().unwrap();
        let sample = |definition: Definition| Input { schema: 1, id: format!("runtime:{}", Uuid::nil()), workspace: "w".into(),
            expected_revision: state.revision + 1, epoch: state.epoch, source: "runtime".into(),
            source_sequence: state.source_sequences.get("runtime").copied().unwrap_or(0) + 1,
            now_ms: 100, grant: Grant::Controller, busy_resources: BTreeSet::new(),
            command: Command::Start { start_load: None, item: "a".into(), policy_pin: definition.pin(),
                resource: Exec::item_resource("w", "a"), dependencies: definition.start_policy.as_ref().unwrap().dependencies["a"].clone(),
                initial_candidate: None, models: Default::default(), tree: None, with: vec![], definition } };
        let base = serde_json::to_vec(&sample(d.clone())).unwrap().len();
        d.start_policy.as_mut().unwrap().files.insert("a".into(), ["x".repeat(1 + 65536 - base)].into());
        assert_eq!(serde_json::to_vec(&sample(d.clone())).unwrap().len(), 65536);
        publish(&runtime, d);
        runtime.schedule_with_load(&["w".into()], 100, Some(4100)).unwrap();
        let expected = Some(farcooler_workflow::StartLoad { sample_milli: Some(4100), below_milli: Some(6000) });
        let event = runtime.events("w").unwrap().into_iter().find(|e| matches!(e.command, Command::Start { .. })).unwrap();
        assert!(serde_json::to_vec(&event).unwrap().len() > 65536);
        assert_eq!(runtime.state("w").unwrap().unwrap().items["a"].start_load, expected);
        complete(&runtime, "a", 200);
        runtime.internal("w", Command::Advance, Grant::Controller, 201).unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().archive["a"].start_load, expected);
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
    async fn completion_refills_fixed_capacity_only_after_load_gap_and_good_sample() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let mut d = coding_definition(false);
        d.stages.retain(|s| s.kind == StageKind::Build);
        d.start_policy = Some(policy(Budget::Fixed(3)));
        publish(&runtime, d.clone());
        track(&runtime, &["a", "b", "c", "d"]);
        d.start_policy = Some(policy(Budget::Load { below_milli: 6000, min_start_gap_ms: 60_000 }));
        publish_on(&runtime, "load-policy", d);
        assert!(runtime.store.workflow_configuration("load-policy").unwrap().is_some());
        assert!(runtime.state("load-policy").unwrap().is_none(), "publication needs no aggregate");
        for time in [100, 60_100, 120_100] {
            runtime.schedule_with_load(&["w".into()], time, Some(2000)).unwrap();
        }
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items.values().filter(|i| active(i)).count(), 3);
        assert!(state.items["d"].definition.is_ticket());
        complete(&runtime, "a", 120_101);
        // A backwards clock and a freed build slot must not bypass the admission timer.
        for time in [99, 120_101, 180_099] {
            runtime.schedule_with_load(&["w".into()], time, Some(2000)).unwrap();
            assert!(runtime.state("w").unwrap().unwrap().items["d"].definition.is_ticket(), "gap holds at {time}");
        }
        for load in [None, Some(6000), Some(8000)] {
            runtime.schedule_with_load(&["w".into()], 180_100, load).unwrap();
            assert!(runtime.state("w").unwrap().unwrap().items["d"].definition.is_ticket(), "load holds at {load:?}");
        }
        runtime.schedule_with_load(&["w".into()], 180_100, Some(2000)).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert!(active(&state.items["d"]));
        assert_eq!(state.items.values().filter(|i| active(i)).count(), 3);
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
