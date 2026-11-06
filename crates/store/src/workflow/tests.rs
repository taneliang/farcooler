use super::*;
use farcooler_workflow::*;
use std::collections::BTreeSet;

fn enqueue(workspace: &str, resource: &str) -> Input {
    Input { busy_resources: BTreeSet::new(), schema: 1, id: "enqueue".into(), workspace: workspace.into(), expected_revision: 0,
        epoch: 1, source: "controller".into(), source_sequence: 1, now_ms: 1000, grant: Grant::Controller,
        command: Command::Enqueue { item: "item".into(), definition: Definition::coding(),
            policy_pin: "policy-1".into(), resource: resource.into(), dependencies: BTreeSet::new(), initial_candidate: None } }
}
fn temp() -> std::path::PathBuf {
    let path = farcooler_core::scratch::dir("workflow");
    std::fs::create_dir_all(&path).unwrap();
    path.join("fixture.db")
}
fn observation(state: &State, outcome: Outcome) -> Input {
    let item = &state.items["item"];
    let effect = item.pending.clone().unwrap();
    Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("result-{}", state.revision), workspace: state.workspace.clone(),
        expected_revision: state.revision, epoch: state.epoch, source: "adapter".into(),
        source_sequence: state.revision, now_ms: 2000,
        grant: Grant::Operation { effect: effect.clone(), run: item.run.clone(), fence: item.fence },
        command: Command::Observe { item: "item".into(), effect, run: item.run.clone(), fence: item.fence, outcome } }
}
fn admit(store: &Store, state: &State) {
    let effect = state.items["item"].pending.as_ref().unwrap();
    let claim = store.workflow_claim_effect(&state.workspace, effect, "fixture-runtime", state.revision,
        state.epoch, 1000, 2000).unwrap().unwrap();
    assert!(store.workflow_admit_effect(&claim, 1001).unwrap());
}
#[test]
fn workflow_atomic_reopen_duplicate_payload_and_replay() {
    let path = temp();
    let command = enqueue("workspace", "resource");
    let receipt;
    {
        let store = Store::open(&path).unwrap();
        receipt = store.workflow_apply(&command).unwrap();
        assert_eq!(receipt.result, Ok(1));
        let state = store.workflow_state("workspace").unwrap().unwrap();
        let e = Evidence { candidate: "candidate".into(), manifest: "manifest".into(),
            receipt: "receipt".into(), policy_pin: "policy-1".into() };
        admit(&store, &state);
    store.workflow_apply(&observation(&state, Outcome::Passed(e))).unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.workflow_apply(&command).unwrap(), receipt);
    let mut changed = command.clone(); changed.now_ms += 1;
    assert!(matches!(store.workflow_apply(&changed), Err(WorkflowError::IdentityConflict)));
    let events = store.workflow_events("workspace").unwrap();
    assert_eq!(events.len(), 2);
    let mut replay = State::empty("workspace".into(), 1);
    for e in events { replay = step(&replay, &e).unwrap().state; }
    assert_eq!(Some(replay), store.workflow_state("workspace").unwrap());
    let effects = store.workflow_outbox("workspace").unwrap();
    assert_eq!(effects.len(), 2);
    assert_eq!(effects.iter().filter(|e| e.active).count(), 1);
    drop(store);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
#[test]
fn workflow_sql_failure_rolls_back_every_boundary() {
    // Inject faults at each write after the reducer, including the final receipt.
    for table in ["workflow_events", "workflow_states", "workflow_resources", "workflow_outbox", "workflow_receipts"] {
        let store = Store::open_in_memory().unwrap();
        store.conn().execute_batch(&format!(
            "CREATE TRIGGER injected BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'injected'); END;"
        )).unwrap();
        assert!(store.workflow_apply(&enqueue("workspace", "resource")).is_err(), "{table}");
        for table in ["workflow_events", "workflow_states", "workflow_resources", "workflow_outbox", "workflow_receipts"] {
            let count: u64 = store.conn().query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap();
            assert_eq!(count, 0, "{table}");
        }
        store.conn().execute_batch("DROP TRIGGER injected").unwrap();
        assert_eq!(store.workflow_apply(&enqueue("workspace", "resource")).unwrap().result, Ok(1));
    }
}
#[test]
fn workflow_refusals_deduplicated_without_revision() {
    let store = Store::open_in_memory().unwrap();
    let good = enqueue("workspace", "resource");
    store.workflow_apply(&good).unwrap();
    let mut stale = good.clone(); stale.id = "stale".into();
    let receipt = store.workflow_apply(&stale).unwrap();
    assert_eq!(receipt.result, Err(Refusal::Stale));
    assert_eq!(store.workflow_apply(&stale).unwrap(), receipt);
    assert_eq!(store.workflow_state("workspace").unwrap().unwrap().revision, 1);
    assert_eq!(store.workflow_events("workspace").unwrap().len(), 1);
    assert_eq!(store.workflow_outbox("workspace").unwrap().len(), 1);
}
#[test]
fn workflow_concurrent_resource_admission_across_connections() {
    let path = temp();
    drop(Store::open(&path).unwrap());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles = vec![];
    for workspace in ["one", "two"] {
        let path = path.clone(); let barrier = barrier.clone();
        handles.push(std::thread::spawn(move || {
            let store = Store::open(path).unwrap();
            barrier.wait();
            store.workflow_apply(&enqueue(workspace, "shared-resource")).unwrap().result
        }));
    }
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|r| r == &&Ok(1)).count(), 1);
    assert_eq!(outcomes.iter().filter(|r| r == &&Err(Refusal::Resource)).count(), 1);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
#[test]
fn workflow_concurrent_same_revision_and_duplicate_command() {
    for duplicate in [false, true] {
        let path = temp();
        let store = Store::open(&path).unwrap();
        store.workflow_apply(&enqueue("workspace", "resource")).unwrap();
        let state = store.workflow_state("workspace").unwrap().unwrap();
        admit(&store, &state);
        let mut command = observation(&state, Outcome::Uncertain);
        command.id = "same".into();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = vec![];
        for n in 0..2 {
            let path = path.clone(); let barrier = barrier.clone(); let mut c = command.clone();
            if !duplicate { c.id = format!("different-{n}"); }
            handles.push(std::thread::spawn(move || {
                let store = Store::open(path).unwrap(); barrier.wait(); store.workflow_apply(&c).unwrap()
            }));
        }
        let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(outcomes.iter().filter(|r| r.result == Ok(2)).count(), if duplicate { 2 } else { 1 });
        assert_eq!(store.workflow_events("workspace").unwrap().len(), 2);
        drop(store);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
#[test]
fn workflow_uncertain_boundary_reopen_and_cancellation_holds_resource() {
    let path = temp();
    let store = Store::open(&path).unwrap();
    store.workflow_apply(&enqueue("workspace", "resource")).unwrap();
    let state = store.workflow_state("workspace").unwrap().unwrap();
    admit(&store, &state);
    store.workflow_apply(&observation(&state, Outcome::Uncertain)).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let state = store.workflow_state("workspace").unwrap().unwrap();
    assert_eq!(state.items["item"].phase, Phase::Uncertain);
    assert_eq!(store.workflow_apply(&enqueue("other", "resource")).unwrap().result, Err(Refusal::Resource));
    let mut cancel = enqueue("workspace", "resource");
    cancel.id = "cancel".into(); cancel.expected_revision = 2; cancel.source_sequence = 2;
    cancel.command = Command::Cancel { item: "item".into() };
    store.workflow_apply(&cancel).unwrap();
    let state = store.workflow_state("workspace").unwrap().unwrap();
    assert_eq!(state.items["item"].phase, Phase::Cancelling);
    assert_eq!(store.workflow_outbox("workspace").unwrap().iter().filter(|e| e.active).count(), 1);
    admit(&store, &state);
    store.workflow_apply(&observation(&state, stopped(&state))).unwrap();
    let mut other = enqueue("other", "resource"); other.id = "retry-admission".into();
    assert_eq!(store.workflow_apply(&other).unwrap().result, Ok(1));
    drop(store);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
#[test]
fn workflow_immutable_history_and_refusing_registry() {
    let store = Store::open_in_memory().unwrap();
    store.workflow_apply(&enqueue("workspace", "resource")).unwrap();
    for sql in ["DELETE FROM workflow_events", "UPDATE workflow_events SET payload = '{}'",
        "DELETE FROM workflow_receipts", "UPDATE workflow_receipts SET receipt = '{}'",
        "DELETE FROM workflow_outbox", "UPDATE workflow_outbox SET payload = '{}'",
        "INSERT OR REPLACE INTO workflow_receipts SELECT * FROM workflow_receipts"] {
        assert!(store.conn().execute(sql, []).is_err(), "{sql}");
    }
    let schema = store.schema().unwrap();
    assert_eq!(schema.compatible_down_to, Some(schema.version));
    // An older binary's open_newer predicate cannot accept this floor.
    assert!(schema.compatible_down_to.unwrap() > schema.version - 1);
}
#[test]
fn workflow_migration_from_every_registered_source_schema() {
    for version in 0..crate::migrate::CURRENT_SCHEMA_VERSION {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON; CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
        crate::migrate::migrate_only_to(&mut conn, version);
        crate::migrate::migrate(&mut conn, version).unwrap();
        assert_eq!(crate::migrate::read_schema_version(&conn).unwrap(), crate::migrate::CURRENT_SCHEMA_VERSION);
        let errors: u64 = conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
        assert_eq!(errors, 0, "source schema {version}");
    }
    println!("migration source schemas tested={}", crate::migrate::CURRENT_SCHEMA_VERSION);
}
#[test]
fn workflow_claim_dispatch_crash_boundaries_and_generation() {
    let path = temp();
    let store = Store::open(&path).unwrap();
    store.workflow_apply(&enqueue("workspace", "resource")).unwrap();
    let state = store.workflow_state("workspace").unwrap().unwrap();
    let effect = state.items["item"].pending.clone().unwrap();
    // Before admission, no observation can manufacture external success.
    assert_eq!(store.workflow_apply(&observation(&state, Outcome::Uncertain)).unwrap().result, Err(Refusal::Phase));
    let first = store.workflow_claim_effect("workspace", &effect, "old-runtime", 1, 1, 1000, 1100).unwrap().unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(store.workflow_claim_effect("workspace", &effect, "new-runtime", 1, 1, 1050, 1200).unwrap().is_none());
    let second = store.workflow_claim_effect("workspace", &effect, "new-runtime", 1, 1, 1100, 1200).unwrap().unwrap();
    assert!(second.lease_generation > first.lease_generation);
    assert!(!store.workflow_admit_effect(&first, 1101).unwrap());
    assert!(store.workflow_admit_effect(&second, 1101).unwrap());
    assert!(!store.workflow_admit_effect(&second, 1102).unwrap());
    drop(store); // Simulates crash before I/O, or after I/O but before its result transaction.
    let store = Store::open(&path).unwrap();
    assert!(store.workflow_claim_effect("workspace", &effect, "reconciler", 1, 1, 5000, 6000).unwrap().is_none());
    assert_eq!(store.workflow_outbox("workspace").unwrap()[0].dispatch_state, "admitted");
    let mut uncertainty = observation(&state, Outcome::Uncertain);
    uncertainty.id = "recovery-uncertain".into(); // Rejected identity remains a receipt; use a new command.
    assert_eq!(store.workflow_apply(&uncertainty).unwrap().result, Ok(2));
    assert!(store.workflow_claim_effect("workspace", &effect, "reconciler", 2, 1, 5000, 6000).unwrap().is_none());
    let mut cancel = enqueue("workspace", "resource");
    cancel.id = "cancel".into(); cancel.expected_revision = 2; cancel.source_sequence = 2;
    cancel.command = Command::Cancel { item: "item".into() };
    store.workflow_apply(&cancel).unwrap();
    let state = store.workflow_state("workspace").unwrap().unwrap();
    let stop = state.items["item"].pending.clone().unwrap();
    let stop_claim = store.workflow_claim_effect("workspace", &stop, "runtime", 3, 1, 5000, 6000).unwrap().unwrap();
    assert!(stop_claim.resource_generation > second.resource_generation);
    assert!(store.workflow_admit_effect(&stop_claim, 5001).unwrap());
    store.workflow_apply(&observation(&state, stopped(&state))).unwrap();
    assert_eq!(store.workflow_apply(&enqueue("other-workspace", "resource")).unwrap().result, Ok(1));
    let other = store.workflow_state("other-workspace").unwrap().unwrap();
    let new_claim = store.workflow_claim_effect("other-workspace", other.items["item"].pending.as_ref().unwrap(),
        "runtime", 1, 1, 7000, 8000).unwrap().unwrap();
    assert!(new_claim.resource_generation > stop_claim.resource_generation);
    assert!(!store.workflow_admit_effect(&second, 7000).unwrap());
    drop(store);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

fn stopped(state: &State) -> Outcome {
    Outcome::Stopped { operation: state.items["item"].stopping.clone().unwrap(), receipt: "drained-reconciled-receipt".into() }
}
#[test]
fn workflow_tracker_exact_approval_duplicate_and_reopen_done() {
    let path = temp();
    let store = Store::open(&path).unwrap();
    let mut command = enqueue("workspace", "checklist-resource");
    if let Command::Enqueue { definition, initial_candidate, .. } = &mut command.command {
        *definition = Definition::tracker("owner"); *initial_candidate = Some("checklist-v1".into());
    }
    assert_eq!(store.workflow_apply(&command).unwrap().result, Ok(1));
    let state = store.workflow_state("workspace").unwrap().unwrap();
    assert!(store.workflow_outbox("workspace").unwrap().is_empty());
    let mut approval = command.clone();
    approval.id = "decision".into(); approval.expected_revision = 1; approval.source_sequence = 2;
    approval.grant = Grant::Decision { principal: "owner".into() };
    approval.command = Command::Approve { item: "item".into(), run: state.items["item"].run.clone(),
        pin: state.items["item"].pin.clone(), candidate: Some("checklist-v1".into()) };
    let receipt = store.workflow_apply(&approval).unwrap();
    assert_eq!(receipt.result, Ok(2));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.workflow_apply(&approval).unwrap(), receipt);
    let state = store.workflow_state("workspace").unwrap().unwrap();
    admit(&store, &state);
    let proof = Evidence { candidate: "checklist-v1".into(), manifest: "manifest-v1".into(),
        receipt: "tracker-noop-receipt".into(), policy_pin: "policy-1".into() };
    assert_eq!(store.workflow_apply(&observation(&state, Outcome::Passed(proof))).unwrap().result, Ok(3));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.workflow_state("workspace").unwrap().unwrap().items["item"].phase, Phase::Done);
    assert!(store.workflow_outbox("workspace").unwrap().iter().all(|e| !e.active && e.dispatch_state == "settled"));
    drop(store);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
#[test]
fn workflow_complete_coding_trace_reopens_between_every_effect() {
    let path = temp();
    Store::open(&path).unwrap().workflow_apply(&enqueue("workspace", "resource")).unwrap();
    let mut steps = 0;
    let mut reworked = false;
    loop {
        let store = Store::open(&path).unwrap();
        let state = store.workflow_state("workspace").unwrap().unwrap();
        let item = &state.items["item"];
        if item.phase == Phase::Done { break; }
        admit(&store, &state);
        let proof = Evidence { candidate: item.candidate.clone().unwrap_or("candidate-v1".into()),
            manifest: format!("manifest-{}", state.revision), receipt: format!("receipt-{}", state.revision),
            policy_pin: item.policy_pin.clone() };
        let outcome = if matches!(item.definition.stages[item.stage].kind, StageKind::Review(_)) && !reworked {
            reworked = true; Outcome::Rejected(proof)
        } else { Outcome::Passed(proof) };
        assert!(store.workflow_apply(&observation(&state, outcome)).unwrap().result.is_ok());
        steps += 1;
    }
    assert_eq!(steps, 9);
    let store = Store::open(&path).unwrap();
    let mut replay = State::empty("workspace".into(), 1);
    for e in store.workflow_events("workspace").unwrap() { replay = step(&replay, &e).unwrap().state; }
    assert_eq!(store.workflow_state("workspace").unwrap(), Some(replay));
    assert_eq!(store.workflow_outbox("workspace").unwrap().len(), 9);
    assert!(store.workflow_outbox("workspace").unwrap().iter().all(|e| e.dispatch_state == "settled"));
    drop(store);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn workflow_blocked_shared_resource_reopen_and_generation_handoff() {
    let path = temp();
    let store = Store::open(&path).unwrap();
    store.workflow_apply(&enqueue("workspace", "resource")).unwrap();
    let mut blocked = enqueue("workspace", "resource");
    blocked.id = "dependent".into(); blocked.expected_revision = 1; blocked.source_sequence = 2;
    if let Command::Enqueue { item, dependencies, .. } = &mut blocked.command {
        *item = "dependent".into(); dependencies.insert("item".into());
    }
    assert_eq!(store.workflow_apply(&blocked).unwrap().result, Ok(2));
    drop(store);
    let store = Store::open(&path).unwrap();
    let state = store.workflow_state("workspace").unwrap().unwrap();
    assert_eq!(state.items["dependent"].phase, Phase::Blocked);
    let reservations: u64 = store.conn().query_row("SELECT count(*) FROM workflow_resources", [], |r| r.get(0)).unwrap();
    assert_eq!(reservations, 1);
    let original_generation: u64 = store.conn().query_row("SELECT generation FROM workflow_resources", [], |r| r.get(0)).unwrap();
    let mut state = state;
    while state.items["item"].phase != Phase::Done {
        admit(&store, &state);
        let item = &state.items["item"];
        let evidence = Evidence { candidate: item.candidate.clone().unwrap_or("candidate".into()),
            manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: item.policy_pin.clone() };
        assert!(store.workflow_apply(&observation(&state, Outcome::Passed(evidence))).unwrap().result.is_ok());
        state = store.workflow_state("workspace").unwrap().unwrap();
    }
    assert_eq!(state.items["dependent"].phase, Phase::Running);
    let (owner, generation): (String, u64) = store.conn().query_row(
        "SELECT item, generation FROM workflow_resources", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(owner, "dependent");
    assert!(generation > original_generation);
    let mut replay = State::empty("workspace".into(), 1);
    for event in store.workflow_events("workspace").unwrap() { replay = step(&replay, &event).unwrap().state; }
    assert_eq!(state, replay);
    drop(store);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn workflow_cross_workspace_dependents_cannot_block_completion_and_replay() {
    let path = temp();
    let store = Store::open(&path).unwrap();
    for (workspace, resource, needed) in [("one", "rA", "rB"), ("two", "rB", "rA")] {
        let mut first = enqueue(workspace, resource);
        if let Command::Enqueue { definition, .. } = &mut first.command {
            definition.stages = vec![definition.stages[1].clone()];
            definition.max_reworks = 0; definition.rework_target = None;
        }
        assert_eq!(store.workflow_apply(&first).unwrap().result, Ok(1));
        let mut dependent = first;
        dependent.id = "dependent".into(); dependent.expected_revision = 1; dependent.source_sequence = 2;
        if let Command::Enqueue { item, resource, dependencies, .. } = &mut dependent.command {
            *item = "dependent".into(); *resource = needed.into(); dependencies.insert("item".into());
        }
        assert_eq!(store.workflow_apply(&dependent).unwrap().result, Ok(2));
    }
    drop(store);
    let store = Store::open(&path).unwrap();
    for workspace in ["one", "two"] {
        let state = store.workflow_state(workspace).unwrap().unwrap();
        admit(&store, &state);
        let evidence = Evidence { candidate: "candidate".into(), manifest: "manifest".into(),
            receipt: "receipt".into(), policy_pin: "policy-1".into() };
        let mut result = observation(&state, Outcome::Passed(evidence));
        // Caller-provided occupancy is not trusted; store derives its own snapshot.
        result.busy_resources.insert("forged-resource".into());
        let receipt = store.workflow_apply(&result).unwrap();
        assert_eq!(receipt.result, Ok(3));
        assert_eq!(store.workflow_apply(&result).unwrap(), receipt);
        assert_eq!(store.workflow_state(workspace).unwrap().unwrap().items["item"].phase, Phase::Done);
    }
    let state = store.workflow_state("one").unwrap().unwrap();
    assert_eq!(state.items["dependent"].phase, Phase::Blocked);
    let mut advance = enqueue("one", "unused");
    advance.workspace = "one".into(); advance.id = "advance".into();
    advance.expected_revision = state.revision; advance.source = "controller".into();
    advance.source_sequence = 3; advance.grant = Grant::Controller; advance.command = Command::Advance;
    assert_eq!(store.workflow_apply(&advance).unwrap().result, Ok(4));
    drop(store);
    let store = Store::open(&path).unwrap();
    for workspace in ["one", "two"] {
        let state = store.workflow_state(workspace).unwrap().unwrap();
        assert_eq!(state.items["dependent"].phase, Phase::Running);
        let mut replay = State::empty(workspace.into(), 1);
        for event in store.workflow_events(workspace).unwrap() {
            assert!(!event.busy_resources.contains("forged-resource"));
            replay = step(&replay, &event).unwrap().state;
        }
        assert_eq!(state, replay);
    }
    drop(store);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
