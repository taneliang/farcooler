use super::*;
use farcooler_store::workflow_effects::EffectClaim;

async fn fixture() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    (dir, runtime)
}
fn enqueue(runtime: &mut Runtime, workspace: &str, item: &str, definition: Definition) {
    let controller = runtime.controller();
    let revision = runtime.state(workspace).unwrap().map_or(0, |s| s.revision);
    let sequence = runtime.state(workspace).unwrap().and_then(|s| s.source_sequences.get("controller").copied())
        .unwrap_or(0) + 1;
    let request = Request { id: format!("enqueue-{item}"), workspace: workspace.into(), expected_revision: revision,
        sequence, command: Command::Enqueue { item: item.into(), definition, policy_pin: "policy/v1".into(),
            resource: runtime.resource().into(), dependencies: BTreeSet::new(), initial_candidate: None } };
    assert!(runtime.submit(&controller, request, 1).unwrap().result.is_ok());
}
async fn finish(runtime: &mut Runtime, workspace: &str) -> State {
    for time in 2..40 {
        runtime.pump(time).await.unwrap();
        let state = runtime.state(workspace).unwrap().unwrap();
        if state.items.values().all(|i| matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled)) {
            return state;
        }
    }
    panic!("runtime failed to settle: {:?}", runtime.state(workspace).unwrap());
}
fn claim(runtime: &Runtime, workspace: &str, now: i64) -> (Binding, EffectClaim) {
    let state = runtime.state(workspace).unwrap().unwrap();
    let stored = runtime.store.workflow_outbox(workspace).unwrap().into_iter().find(|s| s.active).unwrap();
    let claim = runtime.store.workflow_claim_effect(workspace, &stored.effect.id, "test-owner", state.revision,
        state.epoch, now, now + 5).unwrap().unwrap();
    (Binding::new(workspace, &stored.effect, claim.resource_generation,
        state.items[&stored.effect.item].iteration, &runtime.base), claim)
}

#[tokio::test]
async fn workflow_runtime_offline_rework_exact_candidate_and_replay() {
    let (_dir, mut runtime) = fixture().await;
    enqueue(&mut runtime, "w", "coding", coding_definition(true));
    let state = finish(&mut runtime, "w").await;
    let item = &state.items["coding"];
    assert_eq!(item.phase, Phase::Done);
    assert_eq!(item.iteration, 1);
    let landed = adapters::git(&runtime.root.join("repo"), &["rev-parse", "refs/heads/landed"]).await.unwrap();
    assert_eq!(item.candidate.as_ref(), Some(&landed));
    assert_eq!(item.last_evidence.as_ref().unwrap().candidate, landed);
    let mut replay = State::empty("w".into(), 1);
    for event in runtime.events("w").unwrap() { replay = step(&replay, &event).unwrap().state; }
    assert_eq!(replay, state);
    let outbox = runtime.store.workflow_outbox("w").unwrap();
    assert_eq!(outbox.len(), 9);
    assert!(outbox.iter().all(|s| s.dispatch_state == "settled" && !s.active));
    let journals = std::fs::read_dir(runtime.root.join("receipts")).unwrap().filter(|e|
        e.as_ref().unwrap().path().extension().is_some_and(|s| s == "json")).count();
    assert_eq!(journals, 9);
    let notices = runtime.notices("w", 0).unwrap();
    assert_eq!(notices.len(), 10);
    assert_eq!(runtime.notices("w", 0).unwrap(), notices);
    assert!(runtime.notices("w", state.revision).unwrap().is_empty());
    assert_eq!(runtime.pump(100).await.unwrap(), 0);
}

#[tokio::test]
async fn workflow_runtime_approval_policies_and_authentication() {
    for mode in 0..4 {
        let (_dir, mut runtime) = fixture().await;
        let mut definition = coding_definition(mode != 0);
        for (index, stage) in definition.stages.iter_mut().enumerate() {
            if mode == 3 || (mode == 2 && (index == 0 || stage.kind == StageKind::Land)) {
                stage.progression = Progression::AwaitApproval { principal: "owner".into() };
            }
        }
        enqueue(&mut runtime, "w", "coding", definition);
        let controller = runtime.controller();
        let owner = runtime.decision_session(&controller, "owner").unwrap();
        let other = runtime.decision_session(&controller, "intruder").unwrap();
        let mut decisions = 0;
        for time in 2..60 {
            runtime.pump(time).await.unwrap();
            let state = runtime.state("w").unwrap().unwrap();
            let item = &state.items["coding"];
            if item.phase == Phase::Done { break; }
            if item.phase != Phase::WaitingApproval { continue; }
            let before = state.clone();
            assert_eq!(runtime.pump(time).await.unwrap(), 0);
            assert_eq!(runtime.state("w").unwrap().unwrap(), before);
            let command = Command::Approve { item: "coding".into(), run: item.run.clone(), pin: item.pin.clone(),
                candidate: item.candidate.clone() };
            let request = Request { id: format!("decision-{time}"), workspace: "w".into(),
                expected_revision: state.revision, sequence: decisions + 1, command };
            assert_eq!(runtime.decide(&other, request.clone(), time).unwrap().result, Err(Refusal::Unauthorized));
            let mut stale = request.clone();
            stale.id = format!("stale-{time}");
            stale.expected_revision -= 1;
            assert_eq!(runtime.decide(&owner, stale, time).unwrap().result, Err(Refusal::Stale));
            let mut valid = request;
            valid.id = format!("valid-{time}");
            assert!(runtime.decide(&owner, valid, time).unwrap().result.is_ok());
            decisions += 1;
        }
        assert_eq!(runtime.state("w").unwrap().unwrap().items["coding"].phase, Phase::Done);
        assert_eq!(decisions, match mode { 2 => 2, 3 => 9, _ => 0 });
    }
}

#[tokio::test]
async fn workflow_runtime_restart_pre_admit_and_admitted_are_distinct() {
    for admitted in [false, true] {
        let (dir, mut runtime) = fixture().await;
        enqueue(&mut runtime, "w", "coding", coding_definition(true));
        let (binding, claim) = claim(&runtime, "w", 2);
        if admitted { assert!(runtime.store.workflow_admit_effect(&claim, 2).unwrap()); }
        drop(runtime);
        let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
        runtime.pump(100).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        if admitted {
            assert_eq!(state.items["coding"].phase, Phase::Uncertain);
            assert!(!runtime.tree(&binding).exists());
            assert_eq!(runtime.pump(200).await.unwrap(), 0);
        } else {
            assert!(runtime.tree(&binding).exists());
            assert_eq!(finish(&mut runtime, "w").await.items["coding"].phase, Phase::Done);
        }
    }
}

#[tokio::test]
async fn workflow_runtime_accepted_result_before_commit_is_reconciled_once() {
    let (dir, mut runtime) = fixture().await;
    enqueue(&mut runtime, "w", "coding", coding_definition(true));
    runtime.pump(2).await.unwrap(); // provision
    let (binding, claim) = claim(&runtime, "w", 3);
    assert!(runtime.store.workflow_admit_effect(&claim, 3).unwrap());
    let outcome = runtime.execute("w", &binding).await.unwrap(); // commit accepted, workflow not committed
    runtime.write_journal(&Journal { binding: binding.clone(), outcome }).unwrap();
    let before = adapters::git(&runtime.tree(&binding), &["rev-parse", "HEAD"]).await.unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
    runtime.pump(100).await.unwrap();
    assert_eq!(adapters::git(&runtime.tree(&binding), &["rev-parse", "HEAD"]).await.unwrap(), before);
    assert_eq!(runtime.state("w").unwrap().unwrap().items["coding"].candidate, Some(before));
    assert_eq!(finish(&mut runtime, "w").await.items["coding"].phase, Phase::Done);
}

#[tokio::test]
async fn workflow_runtime_cancel_requires_original_settlement() {
    for settled in [false, true] {
        let (dir, mut runtime) = fixture().await;
        enqueue(&mut runtime, "w", "coding", coding_definition(true));
        let (binding, claim) = claim(&runtime, "w", 2);
        assert!(runtime.store.workflow_admit_effect(&claim, 2).unwrap());
        if settled {
            let outcome = runtime.execute("w", &binding).await.unwrap();
            runtime.write_journal(&Journal { binding, outcome }).unwrap();
        }
        let controller = runtime.controller();
        let revision = runtime.state("w").unwrap().unwrap().revision;
        assert!(runtime.submit(&controller, Request { id: "cancel".into(), workspace: "w".into(),
            expected_revision: revision, sequence: 2, command: Command::Cancel { item: "coding".into() } }, 3)
            .unwrap().result.is_ok());
        drop(runtime);
        let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
        runtime.pump(100).await.unwrap();
        let item = runtime.state("w").unwrap().unwrap().items["coding"].clone();
        assert_eq!(item.phase, if settled { Phase::Cancelled } else { Phase::Uncertain });
        assert_eq!(owns_resource(&item), !settled);
    }
}

#[tokio::test]
async fn workflow_runtime_capabilities_specs_and_writer_are_local() {
    let (dir, runtime) = fixture().await;
    assert!(Runtime::open(&dir.path().join("runtime")).is_err());
    let (_other_dir, mut other) = fixture().await;
    let controller = runtime.controller();
    let request = Request { id: "forged".into(), workspace: "w".into(), expected_revision: 0, sequence: 1,
        command: Command::Advance };
    assert!(other.submit(&controller, request, 1).is_err());
    let mut definition = coding_definition(true);
    definition.stages[1].operation.arguments.push("arbitrary command".into());
    assert!(adapters::validate(&definition).is_err());
    assert!(serde_json::from_str::<Request>(r#"{"grant":"Controller"}"#).is_err());
}

#[tokio::test]
async fn workflow_runtime_owned_loop_finishes_without_controller() {
    let (_dir, mut runtime) = fixture().await;
    enqueue(&mut runtime, "w", "coding", coding_definition(true));
    let (send, receive) = tokio::sync::watch::channel(false);
    let root = runtime.root.clone();
    let task = tokio::spawn(async move { runtime.run(receive).await.unwrap(); runtime });
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let store = Store::open(root.join("workflow.sqlite")).unwrap();
            if store.workflow_state("w").unwrap().unwrap().items["coding"].phase == Phase::Done { break; }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.unwrap();
    send.send(true).unwrap();
    let runtime = task.await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["coding"].phase, Phase::Done);
}

fn enqueue_tracker(runtime: &mut Runtime, workspace: &str, item: &str, dependencies: &[&str]) {
    let state = runtime.state(workspace).unwrap();
    let controller = runtime.controller();
    let mut definition = Definition::tracker("owner");
    definition.stages[0].progression = Progression::Automatic;
    let request = Request { id: format!("enqueue-{item}"), workspace: workspace.into(),
        expected_revision: state.as_ref().map_or(0, |s| s.revision),
        sequence: state.as_ref().and_then(|s| s.source_sequences.get("controller")).copied().unwrap_or(0) + 1,
        command: Command::Enqueue { item: item.into(), definition, policy_pin: "policy/v1".into(),
            resource: runtime.resource().into(), dependencies: dependencies.iter().map(|d| (*d).into()).collect(),
            initial_candidate: Some("checklist:exact-revision".into()) } };
    assert!(runtime.submit(&controller, request, 1).unwrap().result.is_ok());
}

#[tokio::test]
async fn workflow_runtime_release_and_recovery_advance_blocked_work_without_churn() {
    let (dir, mut runtime) = fixture().await;
    // Seed a legal second canonical resource in the store only. The runtime never
    // dispatches on it; this models a previously completed cross-resource dependency.
    let mut definition = Definition::tracker("owner");
    definition.stages[0].progression = Progression::Automatic;
    let seed = Input { schema: 1, id: "dependency".into(), workspace: "b".into(), expected_revision: 0,
        epoch: 1, source: "fixture-seed".into(), source_sequence: 1, now_ms: 1, grant: Grant::Controller,
        busy_resources: BTreeSet::new(), command: Command::Enqueue { item: "dependency".into(),
            definition: definition.clone(), policy_pin: "policy/v1".into(), resource: "fixture:dependency".into(),
            dependencies: BTreeSet::new(), initial_candidate: Some("checklist:exact-revision".into()) } };
    assert!(runtime.store.workflow_apply(&seed).unwrap().result.is_ok());
    enqueue_tracker(&mut runtime, "b", "waiter", &["dependency"]);
    enqueue_tracker(&mut runtime, "a", "holder", &[]);
    let b = runtime.state("b").unwrap().unwrap();
    let effect = runtime.store.workflow_outbox("b").unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect("b", &effect.id, "fixture-owner", b.revision, 1, 2, 10)
        .unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 2).unwrap());
    let complete = Input { schema: 1, id: "complete-dependency".into(), workspace: "b".into(),
        expected_revision: b.revision, epoch: 1, source: "fixture-seed".into(), source_sequence: 2, now_ms: 3,
        grant: Grant::Operation { effect: effect.id.clone(), run: effect.run.clone(), fence: effect.fence },
        busy_resources: BTreeSet::new(), command: Command::Observe { item: effect.item, effect: effect.id,
            run: effect.run, fence: effect.fence, outcome: Outcome::Passed(Evidence {
                candidate: "checklist:exact-revision".into(), manifest: "fixture:manifest".into(),
                receipt: "fixture:receipt".into(), policy_pin: "policy/v1".into() }) } };
    assert!(runtime.store.workflow_apply(&complete).unwrap().result.is_ok());
    assert_eq!(runtime.state("b").unwrap().unwrap().items["waiter"].phase, Phase::Blocked);
    // Recovery discovers the wait and A's completion releases the reservation.
    drop(runtime);
    let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
    runtime.pump(100).await.unwrap();
    runtime.pump(101).await.unwrap();
    assert_eq!(runtime.state("a").unwrap().unwrap().items["holder"].phase, Phase::Done);
    assert_eq!(runtime.state("b").unwrap().unwrap().items["waiter"].phase, Phase::Done);
    let before = runtime.state("b").unwrap().unwrap().revision;
    runtime.pump(102).await.unwrap();
    assert_eq!(runtime.state("b").unwrap().unwrap().revision, before);
}

#[tokio::test]
async fn workflow_runtime_generated_crash_receipt_and_replay_properties() {
    let (dir, mut runtime) = fixture().await;
    let mut accepted = 0;
    for seed in 0..64u64 {
        let workspace = format!("seed-{seed}");
        enqueue_tracker(&mut runtime, &workspace, "item", &[]);
        let (binding, claim) = claim(&runtime, &workspace, 2);
        assert!(runtime.store.workflow_admit_effect(&claim, 2).unwrap());
        let result = runtime.execute(&workspace, &binding).await.unwrap();
        if seed & 1 != 0 {
            runtime.write_journal(&Journal { binding: binding.clone(), outcome: result.clone() }).unwrap();
        }
        drop(runtime);
        runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
        runtime.pump(100).await.unwrap();
        if seed & 1 == 0 {
            assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["item"].phase, Phase::Uncertain);
            let mut wrong = binding.clone();
            wrong.generation += 1;
            runtime.write_journal(&Journal { binding: wrong, outcome: result.clone() }).unwrap();
            assert_eq!(runtime.pump(101).await.unwrap(), 0);
            assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["item"].phase, Phase::Uncertain);
            // This is an explicit test reconciliation by the trusted runner. Production
            // exposes no API for supplying an external serialized result or Grant.
            std::fs::remove_file(runtime.journal_path(&binding.effect)).unwrap();
            runtime.write_journal(&Journal { binding, outcome: result }).unwrap();
            runtime.pump(102).await.unwrap();
        }
        let state = runtime.state(&workspace).unwrap().unwrap();
        assert_eq!(state.items["item"].phase, Phase::Done);
        let mut replay = State::empty(workspace.clone(), 1);
        for event in runtime.events(&workspace).unwrap() {
            let first = step(&replay, &event).unwrap();
            assert_eq!(first, step(&replay, &event).unwrap());
            replay = first.state;
            accepted += 1;
        }
        assert_eq!(replay, state);
    }
    println!("runtime generated recovery/replay: seeds=64 accepted_events={accepted}; stale generation rejected=32");
}

#[tokio::test]
async fn workflow_runtime_semantic_review_and_corrupt_journal_are_not_success() {
    let (_dir, mut runtime) = fixture().await;
    let mut definition = coding_definition(true);
    definition.stages[3].kind = StageKind::Review(Reviewer::Person("owner".into()));
    enqueue(&mut runtime, "w", "coding", definition);
    for t in 2..6 { runtime.pump(t).await.unwrap(); }
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["coding"].phase, Phase::WaitingReview);
    assert_eq!(runtime.pump(7).await.unwrap(), 0);
    let controller = runtime.controller();
    let owner = runtime.decision_session(&controller, "owner").unwrap();
    let item = &state.items["coding"];
    let evidence = Evidence { candidate: item.candidate.clone().unwrap(), manifest: "review:exact".into(),
        receipt: "human:decision".into(), policy_pin: item.policy_pin.clone() };
    assert!(runtime.decide(&owner, Request { id: "review".into(), workspace: "w".into(), expected_revision: state.revision,
        sequence: 1, command: Command::Review { item: "coding".into(), run: item.run.clone(), evidence, passed: true } }, 8)
        .unwrap().result.is_ok());
    let (binding, claim) = claim(&runtime, "w", 9);
    assert!(runtime.store.workflow_admit_effect(&claim, 9).unwrap());
    durable(&runtime.journal_path(&binding.effect), b"partial result").unwrap();
    runtime.pump(100).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["coding"].phase, Phase::Uncertain);
}

#[tokio::test]
async fn workflow_runtime_every_accepted_adapter_result_reopens_without_reexecution() {
    let (dir, mut runtime) = fixture().await;
    enqueue(&mut runtime, "w", "coding", coding_definition(true));
    for index in 0..9 {
        let (binding, claim) = claim(&runtime, "w", 10 + index);
        assert!(runtime.store.workflow_admit_effect(&claim, 10 + index).unwrap());
        let outcome = runtime.execute("w", &binding).await.unwrap();
        runtime.write_journal(&Journal { binding: binding.clone(), outcome }).unwrap();
        let log = runtime.root.join("receipts").join(format!("{}.git.jsonl", binding.effect.id));
        let raw_before = std::fs::read(&log).ok();
        drop(runtime);
        runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
        assert_eq!(runtime.pump(100 + index).await.unwrap(), 1);
        assert_eq!(std::fs::read(&log).ok(), raw_before);
    }
    assert_eq!(runtime.state("w").unwrap().unwrap().items["coding"].phase, Phase::Done);
}
