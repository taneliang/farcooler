use super::*;
use farcooler_store::workflow_effects::EffectClaim;

async fn fixture() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    (dir, runtime)
}
pub(super) fn enqueue(runtime: &mut Runtime, workspace: &str, item: &str, definition: Definition) {
    let controller = runtime.controller();
    let revision = runtime.state(workspace).unwrap().map_or(0, |s| s.revision);
    let sequence = runtime.state(workspace).unwrap().and_then(|s| s.source_sequences.get("controller").copied())
        .unwrap_or(0) + 1;
    let request = Request { id: format!("enqueue-{item}"), workspace: workspace.into(), expected_revision: revision,
        sequence, command: Command::Enqueue { item: item.into(), definition, policy_pin: "policy/v1".into(),
            resource: runtime.resource().into(), dependencies: BTreeSet::new(), initial_candidate: None } };
    let receipt = runtime.submit(&controller, request, 1).unwrap(); assert!(receipt.result.is_ok(), "{:?}", receipt.result);
}
pub(super) async fn finish(runtime: &mut Runtime, workspace: &str) -> State {
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
            expected_revision: revision, sequence: 2, command: Command::Cancel { item: "coding".into() , reason: None } }, 3)
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
    let (_other_dir, other) = fixture().await;
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
    let receipt = runtime.submit(&controller, request, 1).unwrap(); assert!(receipt.result.is_ok(), "{:?}", receipt.result);
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
        sequence: 1, command: Command::Review { item: "coding".into(), run: item.run.clone(), evidence, passed: true, reason: None } }, 8)
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

#[tokio::test]
async fn workflow_runtime_owned_inbox_approval_cancel_and_idle() {
    for cancel in [false, true] {
        let (_dir, mut runtime) = fixture().await;
        let mut definition = coding_definition(true);
        for stage in &mut definition.stages {
            stage.progression = Progression::AwaitApproval { principal: "owner".into() };
        }
        enqueue(&mut runtime, "w", "coding", definition);
        let controller = runtime.controller();
        let owner = runtime.decision_session(&controller, "owner").unwrap();
        let intruder = runtime.decision_session(&controller, "intruder").unwrap();
        let (_foreign_dir, foreign) = fixture().await;
        let foreign_controller = foreign.controller();
        let handle = runtime.command_handle(&controller).unwrap();
        let (stop, shutdown) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move { runtime.run(shutdown).await.unwrap(); runtime });
        tokio::time::timeout(Duration::from_secs(15), async {
            let mut sequence = 0;
            loop {
                let state = handle.state("w").await.unwrap().unwrap();
                let item = &state.items["coding"];
                if matches!(item.phase, Phase::Done | Phase::Cancelled) { break; }
                if item.phase != Phase::WaitingApproval { tokio::task::yield_now().await; continue; }
                let request = Request { id: format!("inbox-{sequence}"), workspace: "w".into(),
                    expected_revision: state.revision, sequence: sequence + 1,
                    command: Command::Approve { item: "coding".into(), run: item.run.clone(),
                        pin: item.pin.clone(), candidate: item.candidate.clone() } };
                assert_eq!(handle.decide(&intruder, request.clone(), 20).await.unwrap().result,
                    Err(Refusal::Unauthorized));
                let mut stale = request.clone();
                stale.id = format!("stale-inbox-{sequence}");
                stale.expected_revision -= 1;
                assert_eq!(handle.decide(&owner, stale, 20).await.unwrap().result, Err(Refusal::Stale));
                if cancel {
                    let request = Request { id: "inbox-cancel".into(), sequence: 2,
                        command: Command::Cancel { item: "coding".into() , reason: None }, ..request };
                    assert!(handle.submit(&foreign_controller, request.clone(), 20).await.is_err());
                    assert!(handle.submit(&controller, request, 20).await.unwrap().result.is_ok());
                } else {
                    let mut request = request;
                    request.id = format!("valid-inbox-{sequence}");
                    assert!(handle.decide(&owner, request, 20).await.unwrap().result.is_ok());
                    sequence += 1;
                }
            }
        }).await.unwrap();
        // Reads and time passing must not trigger further full-history sweeps.
        let (send, receive) = oneshot::channel();
        handle.send.send(Inbox::State("w".into(), send)).await.unwrap();
        receive.await.unwrap().unwrap();
        stop.send(true).unwrap();
        let mut runtime = task.await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items["coding"].phase, if cancel { Phase::Cancelled } else { Phase::Done });
        let before = runtime.sweeps;
        let (stop, shutdown) = tokio::sync::watch::channel(false);
        let handle = runtime.command_handle(&controller).unwrap();
        let task = tokio::spawn(async move { runtime.run(shutdown).await.unwrap(); runtime });
        for _ in 0..5 {
            handle.state("w").await.unwrap();
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        stop.send(true).unwrap();
        let runtime = task.await.unwrap();
        assert_eq!(runtime.sweeps, before + 1, "idle reads/timer rescanned history");
    }
}

#[tokio::test]
async fn workflow_runtime_second_coding_item_pins_new_base_and_recovers() {
    let (dir, mut runtime) = fixture().await;
    enqueue(&mut runtime, "w", "first", coding_definition(true));
    let first = finish(&mut runtime, "w").await.items["first"].candidate.clone().unwrap();
    enqueue(&mut runtime, "w", "second", coding_definition(true));
    runtime.pump(100).await.unwrap(); // Second Provision pins the landed first candidate.
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .find(|s| s.active).unwrap().effect;
    assert_eq!(runtime.item_base("w", &effect).unwrap(), first);
    let state = runtime.state("w").unwrap().unwrap();
    let claim = runtime.store.workflow_claim_effect("w", &effect.id, "crash", state.revision,
        state.epoch, 101, 106).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 101).unwrap());
    let binding = Binding::new("w", &effect, claim.resource_generation,
        state.items["second"].iteration, &first);
    let outcome = runtime.execute("w", &binding).await.unwrap();
    runtime.write_journal(&Journal { binding: binding.clone(), outcome }).unwrap();
    let candidate = adapters::git(&runtime.tree(&binding), &["rev-parse", "HEAD"]).await.unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
    runtime.pump(200).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["second"].candidate, Some(candidate));
    let state = finish(&mut runtime, "w").await;
    let second = state.items["second"].candidate.clone().unwrap();
    assert_eq!(state.items["second"].phase, Phase::Done);
    assert_ne!(first, second);
    assert_eq!(adapters::git(&runtime.root.join("repo"), &["rev-parse", "refs/heads/landed"])
        .await.unwrap(), second);
    assert!(crate::git::git(&runtime.root.join("repo"), &["merge-base", "--is-ancestor", &first, &second])
        .await.unwrap().ok);
}

#[tokio::test]
async fn workflow_runtime_owned_lease_deadline_and_explicit_reconciliation() {
    for admitted in [false, true] {
        let (_dir, mut runtime) = fixture().await;
        enqueue_tracker(&mut runtime, "w", "item", &[]);
        let (binding, claim) = claim(&runtime, "w", now() + 150);
        let journal = if admitted {
            assert!(runtime.store.workflow_admit_effect(&claim, now()).unwrap());
            Some(Journal { outcome: runtime.execute("w", &binding).await.unwrap(), binding })
        } else { None };
        let controller = runtime.controller();
        let handle = runtime.command_handle(&controller).unwrap();
        let path = journal.as_ref().map(|j| runtime.journal_path(&j.binding.effect));
        let (stop, shutdown) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move { runtime.run(shutdown).await.unwrap(); runtime });
        tokio::time::timeout(Duration::from_secs(5), async {
            if let Some(journal) = journal {
                loop {
                    if handle.state("w").await.unwrap().unwrap().items["item"].phase == Phase::Uncertain { break; }
                    tokio::task::yield_now().await;
                }
                // Only the trusted runner writes a delayed bound receipt; wake contains no result.
                durable(&path.unwrap(), &serde_json::to_vec(&journal).unwrap()).unwrap();
                handle.reconcile().await.unwrap();
            }
            loop {
                if handle.state("w").await.unwrap().unwrap().items["item"].phase == Phase::Done { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.unwrap();
        stop.send(true).unwrap();
        let runtime = task.await.unwrap();
        assert!(runtime.sweeps <= 4, "deadline/reconcile unexpectedly polled");
    }
}

#[tokio::test]
async fn workflow_runtime_interrupted_base_publication_recovers_before_admission() {
    for partial in [Some(""), Some("\"truncated"), None] {
        let (dir, mut runtime) = fixture().await;
        enqueue(&mut runtime, "w", "first", coding_definition(true));
        let first = finish(&mut runtime, "w").await.items["first"].candidate.clone().unwrap();
        enqueue(&mut runtime, "w", "second", coding_definition(true));
        let state = runtime.state("w").unwrap().unwrap();
        let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
        runtime.store.workflow_claim_effect("w", &effect.id, "crashed-publisher", state.revision,
            state.epoch, 2, 7).unwrap().unwrap();
        let path = runtime.base_path("w", &effect).unwrap();
        if let Some(bytes) = partial { durable(&path, bytes.as_bytes()).unwrap(); }
        // A crash during the new writer's temporary publication is ignored too.
        durable(&runtime.root.join("base-publish-orphan.tmp"), b"\"unfinished").unwrap();
        drop(runtime);
        let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
        runtime.pump(100).await.unwrap();
        assert_eq!(runtime.item_base("w", &effect).unwrap(), first);
        assert_eq!(runtime.state("w").unwrap().unwrap().items["second"].stage, 1);
        let pin = std::fs::read(&path).unwrap();
        drop(runtime);
        let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
        assert_eq!(finish(&mut runtime, "w").await.items["second"].phase, Phase::Done);
        assert_eq!(std::fs::read(&path).unwrap(), pin, "valid selected pin changed after reopen");
    }
}

#[tokio::test]
async fn workflow_runtime_admitted_base_is_never_recreated() {
    let (dir, mut runtime) = fixture().await;
    enqueue(&mut runtime, "w", "coding", coding_definition(true));
    let (binding, claim) = claim(&runtime, "w", 2);
    runtime.pin_base("w", &binding.effect).await.unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 2).unwrap());
    let path = runtime.base_path("w", &binding.effect).unwrap();
    // Corrupt legacy admitted pins are refused, never resampled or automatically repaired.
    std::fs::write(&path, b"").unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
    runtime.pump(100).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["coding"].phase, Phase::Uncertain);
    assert!(runtime.pin_base("w", &binding.effect).await.is_err());
    assert!(std::fs::read(path).unwrap().is_empty());
    assert_eq!(runtime.store.workflow_outbox("w").unwrap()[0].dispatch_state, "uncertain");
    assert!(!runtime.tree(&binding).exists());
}

/// ov-1047: nobody can decide a run on a board that was deleted, so the next sweep cancels it and says why on the run.
/// A fixture's key (not a board id) is never read as a deleted board.
#[tokio::test]
async fn workflow_runtime_cancels_runs_on_a_deleted_board_with_a_reason() {
    let (_dir, mut runtime) = fixture().await;
    // A store that has boards: one real board, so a board id with no row is a deleted one.
    let host = Uuid::now_v7();
    let root = runtime.store.create_repository_root(host, "/repos/ov-1047", 0).unwrap();
    let repo = runtime.store.create_repository(host, root.id, "r", "/repos/ov-1047/.git", "").unwrap();
    let live = runtime.store.ensure_main_workspace(repo.id).unwrap().id.to_string();
    let gone = Uuid::now_v7().to_string();
    enqueue(&mut runtime, &gone, "coding", coding_definition(true));
    // A run on a board that still exists: its key is a real board id, so only the row lookup keeps it.
    let controller = runtime.controller();
    let request = Request { id: "enqueue-kept".into(), workspace: live.clone(), expected_revision: 0, sequence: 1,
        command: Command::Enqueue { item: "coding".into(), definition: coding_definition(true), policy_pin: "policy/v1".into(),
            resource: Exec::item_resource(&live, "coding"), dependencies: BTreeSet::new(), initial_candidate: None } };
    assert!(runtime.submit(&controller, request, 1).unwrap().result.is_ok());
    runtime.pump(2).await.unwrap();
    let item = runtime.state(&gone).unwrap().unwrap().items["coding"].clone();
    assert!(matches!(item.phase, Phase::Cancelled | Phase::Cancelling), "{:?}", item.phase);
    assert_eq!(item.cancel_reason.as_deref(), Some("This run was cancelled because its board was deleted."));
    let kept = runtime.state(&live).unwrap().unwrap().items["coding"].clone();
    assert!(kept.cancel_reason.is_none() && !matches!(kept.phase, Phase::Cancelled | Phase::Cancelling), "{:?}", kept.phase);
    // A person reads the run through the workflow read, which carries the reason.
    let read = runtime.wire_command(&gone, "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    assert_eq!(read["state"]["items"]["coding"]["cancel_reason"], "This run was cancelled because its board was deleted.", "{read}");
    assert!(read["state"]["items"]["coding"].get("cancel_reason").is_some());
    let live_read = runtime.wire_command(&live, "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    assert!(live_read["state"]["items"]["coding"].get("cancel_reason").is_none(), "{live_read}");
}

/// ov-1047 review: the engine always refuses a follower, and a group whose push landed, and each refusal is a
/// permanent receipt, so the sweep must not ask for them.
#[tokio::test]
async fn the_sweep_never_asks_to_cancel_what_the_engine_always_refuses() {
    let (_dir, mut runtime) = fixture().await;
    enqueue(&mut runtime, "w", "coding", coding_definition(true));
    let plain = runtime.state("w").unwrap().unwrap().items["coding"].clone();
    assert!(super::cancellable_on_deleted_board(&plain));
    let mut follower = plain.clone();
    follower.follows = Some("lead".into());
    assert!(!super::cancellable_on_deleted_board(&follower));
    let mut done = plain.clone();
    done.phase = Phase::Cancelled;
    assert!(!super::cancellable_on_deleted_board(&done));
    let mut pushed = plain;
    pushed.members = vec![Member { item: "t".into(), candidate: "c".into() }];
    pushed.pushed = Some("abc".into());
    assert!(!super::cancellable_on_deleted_board(&pushed));
}

/// ov-1047 review: the sweep itself leaves a follower and a pushed group alone, and asks once about a run the engine
/// refuses though the rule allowed it. Every refusal is a receipt that stays, so the receipts count how often it asked.
#[tokio::test]
async fn the_sweep_skips_refused_runs_and_asks_once_about_the_rest() {
    let (_dir, mut runtime) = fixture().await;
    let host = Uuid::now_v7();
    let root = runtime.store.create_repository_root(host, "/repos/ov-1047-sweep", 0).unwrap();
    let repo = runtime.store.create_repository(host, root.id, "r", "/repos/ov-1047-sweep/.git", "").unwrap();
    runtime.store.ensure_main_workspace(repo.id).unwrap();
    let gone = Uuid::now_v7().to_string();
    let controller = runtime.controller();
    for (n, item) in ["plain", "follower", "pushed"].into_iter().enumerate() {
        let request = Request { id: format!("enqueue-{item}"), workspace: gone.clone(), expected_revision: n as u64, sequence: n as u64 + 1,
            command: Command::Enqueue { item: item.into(), definition: coding_definition(true), policy_pin: "policy/v1".into(),
                resource: Exec::item_resource(&gone, item), dependencies: BTreeSet::new(), initial_candidate: None } };
        assert!(runtime.submit(&controller, request, 1).unwrap().result.is_ok());
    }
    // Shape the three runs on the stored state: a follower, a group whose push landed, and a run the engine refuses
    // for a reason the rule cannot see (a stop already named on it).
    let mut state = runtime.state(&gone).unwrap().unwrap();
    state.items.get_mut("follower").unwrap().follows = Some("lead".into());
    let pushed = state.items.get_mut("pushed").unwrap();
    pushed.members = vec![Member { item: "t".into(), candidate: "c".into() }];
    pushed.pushed = Some("abc".into());
    state.items.get_mut("plain").unwrap().run = "cancel:earlier".into();
    let json = serde_json::to_string(&state).unwrap().replace('\'', "''");
    farcooler_store::testing::exec_for_tests(&runtime.store, &format!("UPDATE workflow_states SET state='{json}' WHERE workspace='{gone}'"));
    let receipts = |runtime: &Runtime| farcooler_store::testing::workflow_cancel_receipt_count(&runtime.store, &gone);
    let before = receipts(&runtime);
    for time in 2..6 { runtime.pump(time).await.unwrap(); }
    let after = receipts(&runtime);
    let items = runtime.state(&gone).unwrap().unwrap().items;
    assert!(items["follower"].cancel_reason.is_none() && items["pushed"].cancel_reason.is_none());
    assert_eq!(after - before, 1, "four sweeps: the refused run is asked about once, the follower and the pushed group never");
}

/// Tools join a tree's folders into PATH-like lists: cargo puts `<tree>/target/debug/deps` in
/// DYLD_FALLBACK_LIBRARY_PATH, npm puts `<tree>/node_modules/.bin` on PATH. A `:` in any folder the engine
/// makes splits that entry in two, and every gate run there failed (ov-1081).
#[tokio::test]
async fn workflow_runtime_engine_folders_join_into_a_path_list() {
    let (_dir, mut runtime) = fixture().await;
    enqueue(&mut runtime, "w", "coding", coding_definition(true));
    assert_eq!(finish(&mut runtime, "w").await.items["coding"].phase, Phase::Done);
    let tree = runtime.item_tree("w", "coding");
    assert!(tree.is_dir(), "the run made its tree");
    let deps = tree.join("target/debug/deps");
    let joined = std::env::join_paths([deps.clone(), PathBuf::from("/usr/lib")]).expect("cargo's join of the tree's deps");
    assert_eq!(std::env::split_paths(&joined).next(), Some(deps));
    let mut folders = vec![runtime.root.clone()];
    while let Some(folder) = folders.pop() {
        assert!(!folder.to_string_lossy().contains(':'), "{} holds a PATH separator", folder.display());
        // Inside a repository or worktree, the names are git's and the project's.
        if folder.join(".git").exists() || folder.join("HEAD").is_file() { continue; }
        for entry in std::fs::read_dir(&folder).unwrap().flatten() {
            if entry.file_type().unwrap().is_dir() { folders.push(entry.path()); }
        }
    }
}

/// A tree made before ov-1081 still answers to its old name: an agent or gate already running in it reports
/// and reads that path, so the run finishes there rather than in a second, empty tree.
#[tokio::test]
async fn workflow_runtime_a_tree_made_under_the_old_name_is_kept_until_made_again() {
    let (_dir, mut runtime) = fixture().await;
    enqueue(&mut runtime, "w", "coding", coding_definition(true));
    runtime.pump(2).await.unwrap(); // provision
    let tree = runtime.item_tree("w", "coding");
    let legacy = runtime.root.join(format!("tree-{}", digest(&("w", "coding"))));
    assert_ne!(tree, legacy, "a new tree has the new name");
    adapters::git(&runtime.root.join("repo"), &["worktree", "move", tree.to_str().unwrap(), legacy.to_str().unwrap()]).await.unwrap();
    assert_eq!(runtime.item_tree("w", "coding"), legacy);
    assert_eq!(finish(&mut runtime, "w").await.items["coding"].phase, Phase::Done);
    assert!(!tree.exists(), "no second tree was made");
}
