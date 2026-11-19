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
    observe_item(state, "item", outcome)
}
fn observe_item(state: &State, key: &str, outcome: Outcome) -> Input {
    let item = &state.items[key];
    let effect = item.pending.clone().unwrap();
    Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("result-{}", state.revision), workspace: state.workspace.clone(),
        expected_revision: state.revision, epoch: state.epoch, source: "adapter".into(),
        source_sequence: state.revision, now_ms: 2000,
        grant: Grant::Operation { effect: effect.clone(), run: item.run.clone(), fence: item.fence },
        command: Command::Observe { item: key.into(), effect, run: item.run.clone(), fence: item.fence, outcome } }
}
fn admit(store: &Store, state: &State) {
    admit_item(store, state, "item");
}
fn admit_item(store: &Store, state: &State, key: &str) {
    let effect = state.items[key].pending.as_ref().unwrap();
    let claim = store.workflow_claim_effect(&state.workspace, effect, "fixture-runtime", state.revision,
        state.epoch, 1000, 2000).unwrap().unwrap();
    assert!(store.workflow_admit_effect(&claim, 1001).unwrap());
}

#[test]
fn a_landing_group_review_keeps_findings_and_briefs_for_rework() {
    use crate::models::{Actor, NoteKind};

    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("Landing review");
    let board = store.ensure_main_workspace(repo).unwrap();
    let workspace = board.id.to_string();
    let tickets = ["First fix", "Second fix"].map(|title| store.create_task(board.id, title, Actor::User).unwrap());

    let mut definition = Definition::coding();
    definition.schema = 2;
    definition.max_reworks = 0;
    definition.rework_target = None;
    let build = definition.stages[1].clone();
    let mut review = definition.stages[3].clone();
    review.id = "group-review".into();
    review.kind = StageKind::Review(Reviewer::Agent);
    review.routes.insert("rejected".into(), Target::Step(build.id.clone()));
    review.limit = 1;
    let mut join = definition.stages[2].clone();
    join.id = "join".into();
    join.kind = StageKind::Join { group: "landing".into(), adopt: false };
    join.routes.insert("rejected".into(), Target::Step(build.id.clone()));
    join.limit = 1;
    definition.stages.truncate(2);
    definition.stages.push(join);
    definition.groups.insert("landing".into(), GroupDef {
        limit: None, exclusive: "landing-target".into(), steps: vec![build, review],
    });
    definition.validate().unwrap();

    let current = || store.workflow_state(&workspace).unwrap().unwrap();
    let control = |command| {
        let revision = store.workflow_state(&workspace).unwrap().map_or(0, |state| state.revision);
        let receipt = store.workflow_apply(&Input {
            id: format!("control-{revision}"), command, expected_revision: revision, source_sequence: revision + 1,
            ..enqueue(&workspace, "unused")
        }).unwrap();
        assert!(receipt.result.is_ok(), "{receipt:?}");
    };
    let settle = |state: &State, key, outcome| {
        admit_item(&store, state, key);
        let input = observe_item(state, key, outcome);
        let receipt = store.workflow_apply(&input).unwrap();
        assert!(receipt.result.is_ok(), "{receipt:?}");
        assert_eq!(store.workflow_apply(&input).unwrap(), receipt, "replaying the verdict must not duplicate notes");
        receipt
    };
    let evidence = |candidate: &str, receipt: &str| Evidence {
        candidate: candidate.into(), manifest: "manifest".into(), receipt: receipt.into(), policy_pin: "policy-1".into(),
    };
    // Hold the landing resource while both tickets join the same group.
    assert!(store.workflow_apply(&enqueue("other", "landing-target")).unwrap().result.is_ok());
    for ticket in &tickets {
        control(Command::Enqueue { item: ticket.key.clone(), definition: definition.clone(), policy_pin: "policy-1".into(),
            resource: ticket.key.clone(), dependencies: BTreeSet::new(), initial_candidate: None });
        for _ in 0..2 {
            settle(&current(), &ticket.key, Outcome::Passed(evidence(&ticket.key, "ticket-step")));
        }
        control(Command::Queue { item: ticket.key.clone() });
    }
    for _ in 0..2 {
        let holder = store.workflow_state("other").unwrap().unwrap();
        settle(&holder, "item", Outcome::SafeFailure);
    }
    control(Command::Advance);
    let group = current().items[&tickets[0].key].grouped_in.clone().unwrap();
    assert_eq!(current().items[&group].members.len(), 2);

    for (index, (candidate, receipt_name, finding)) in [
        ("candidate-1", "review-1.json", "Keep the caller's deadline."),
        ("candidate-2", "review-2.json", "Handle the exhausted deadline."),
    ].into_iter().enumerate() {
        settle(&current(), &group, Outcome::Passed(evidence(candidate, "group-build")));
        let reviewing = current();
        let run = &reviewing.items[&group].run;
        let receipt = settle(&reviewing, &group, Outcome::ReviewRejected {
            evidence: evidence(candidate, receipt_name), summary: finding.into(), receipt: receipt_name.into(),
        });
        let outbox = store.workflow_outbox(&workspace).unwrap();
        let rebuilds: Vec<_> = outbox.iter().filter(|stored| receipt.effects.contains(&stored.effect.id)
            && stored.effect.kind == EffectKind::Execute(StageKind::Build)).collect();
        let expected: Vec<_> = if index == 0 { vec![group.as_str()] } else { tickets.iter().map(|ticket| ticket.key.as_str()).collect() };
        assert_eq!(rebuilds.len(), expected.len());
        for item in expected {
            let build = rebuilds.iter().find(|stored| stored.effect.item == item).unwrap();
            assert_eq!(build.effect.brief.as_deref(), Some(finding));
        }
        for ticket in &tickets {
            let notes = store.notes_for(ticket.id, Some(NoteKind::Finding)).unwrap();
            assert_eq!(notes.len(), index + 1, "{notes:?}");
            assert!(notes.iter().any(|note| note.body == finding && note.extra["run"] == *run
                && note.extra["receipt"] == receipt_name), "{notes:?}");
        }
        if index == 1 {
            let reason = current().items[&group].failed_reason().unwrap();
            assert!(reason.starts_with("group-review:") && reason.contains(finding) && reason.contains(receipt_name), "{reason}");
        }
    }
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
fn ov1379_repin_and_admission_cannot_both_consume_an_effect() {
    use crate::workflow_publication::Write;

    for admitted in [false, true] {
        let store = Store::open_in_memory().unwrap();
        let definition = Definition::coding();
        let old_pin = definition.pin();
        store.workflow_wire_write("workspace", "publish", "publish", Write::Publish {
            expected_pin: None, definition: &definition, adapter: None, notifications: None, revision: 0,
        }).unwrap();
        let mut input = enqueue("workspace", "resource");
        let Command::Enqueue { policy_pin, .. } = &mut input.command else { unreachable!() };
        *policy_pin = old_pin.clone();
        store.workflow_apply(&input).unwrap();
        let state = store.workflow_state("workspace").unwrap().unwrap();
        let item = &state.items["item"];
        let effect = item.pending.clone().unwrap();
        let claim = store.workflow_claim_effect("workspace", &effect, "runner", state.revision,
            state.epoch, 1000, 2000).unwrap().unwrap();
        if admitted { assert!(store.workflow_admit_published_effect(&claim, 1001).unwrap()); }

        let mut definition = definition;
        definition.send_backs = Some(1);
        let published = store.workflow_wire_write("workspace", "publish-next", "publish-next", Write::Publish {
            expected_pin: Some(&old_pin), definition: &definition, adapter: None, notifications: None, revision: state.revision,
        }).unwrap();
        assert_eq!(published.result, Ok(state.revision));
        if !admitted {
            assert!(!store.workflow_admit_published_effect(&claim, 1001).unwrap(),
                "a publication between claim and admission fences the old step even before the runtime reclassifies it");
        }

        let receipt = store.workflow_apply(&Input {
            id: "repin".into(), source: "runtime".into(), source_sequence: 1, expected_revision: state.revision,
            command: Command::RepinPending { item: "item".into(), effect, policy_pin: definition.pin(),
                definition: Some(Box::new(definition)) },
            ..enqueue("workspace", "resource")
        }).unwrap();
        let mut replay = State::empty("workspace".into(), 1);
        for input in store.workflow_events("workspace").unwrap() { replay = step(&replay, &input).unwrap().state; }
        assert_eq!(Some(&replay), store.workflow_state("workspace").unwrap().as_ref());
        if admitted {
            assert_eq!(receipt.result, Err(Refusal::Phase));
            assert_eq!(replay, state);
        } else {
            assert_eq!(receipt.result, Ok(state.revision + 1));
            assert!(!store.workflow_admit_published_effect(&claim, 1001).unwrap());
            let replacement = &replay.items["item"];
            assert_ne!(replacement.pending, item.pending);
            assert_eq!(replacement.run, item.run);
        }
    }
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
fn workflow_load_gap_recovers_only_accepted_automatic_starts() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.workflow_last_automatic_start_ms().unwrap(), None);
    let mut ticket = enqueue("workspace", "resource");
    if let Command::Enqueue { definition, initial_candidate, .. } = &mut ticket.command {
        *definition = Definition::ticket_at("todo").unwrap();
        *initial_candidate = Some("policy-1".into());
    }
    assert!(store.workflow_apply(&ticket).unwrap().result.is_ok());
    assert_eq!(store.workflow_last_automatic_start_ms().unwrap(), None);
    let start = Input { id: "start".into(), source: "runtime".into(), source_sequence: 1,
        expected_revision: 1, now_ms: 100, command: Command::Start { start_load: None, item: "item".into(),
            definition: Definition::coding(), policy_pin: "policy-1".into(), resource: "resource".into(),
            dependencies: Default::default(), initial_candidate: None, models: Default::default(),
            tree: None, with: vec![] }, ..ticket };
    assert!(store.workflow_apply(&start).unwrap().result.is_ok());
    assert_eq!(store.workflow_last_automatic_start_ms().unwrap(), Some(100));
    let refused = Input { id: "refused".into(), now_ms: 2000, ..start.clone() };
    assert!(store.workflow_apply(&refused).unwrap().result.is_err());
    let advance = Input { id: "advance".into(), expected_revision: 2, source_sequence: 2,
        now_ms: 3000, command: Command::Advance, ..start };
    assert!(store.workflow_apply(&advance).unwrap().result.is_ok());
    assert_eq!(store.workflow_last_automatic_start_ms().unwrap(), Some(100),
        "refused starts and unrelated runtime events cannot move the load deadline");
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
    cancel.command = Command::Cancel { item: "item".into() , reason: None };
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
    cancel.command = Command::Cancel { item: "item".into() , reason: None };
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

/// ov-1047 review: only a board id with no row, in a store that has boards, reads as deleted.
#[test]
fn a_board_is_deleted_only_when_the_store_has_boards_and_not_this_one() {
    let store = Store::open_in_memory().unwrap();
    let unknown = uuid::Uuid::now_v7().to_string();
    // No boards at all: a runtime opened alone, so nothing is deleted.
    assert!(!store.workflow_board_deleted(&unknown).unwrap());
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    assert!(!store.workflow_board_deleted(&main.to_string()).unwrap(), "a board that exists");
    assert!(store.workflow_board_deleted(&unknown).unwrap(), "a board id with no row");
    assert!(!store.workflow_board_deleted("w").unwrap(), "a fixture's key");
}

#[test]
fn workflow_join_checks_the_published_landing_resource_in_other_workspaces() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.workflow_apply(&enqueue("other", "shared-target")).unwrap().result.is_ok());
    let mut definition = Definition::coding();
    definition.schema = 2; definition.max_reworks = 0; definition.rework_target = None;
    definition.stages.truncate(2);
    let mut join = definition.stages[0].clone(); join.id = "join".into();
    join.kind = StageKind::Join { group: "landing".into(), adopt: false };
    definition.stages.push(join);
    definition.groups.insert("landing".into(), GroupDef { limit: None, exclusive: "old-target".into(),
        steps: vec![definition.stages[0].clone()] });
    let mut input = enqueue("workspace", "build-resource");
    if let Command::Enqueue { definition: d, .. } = &mut input.command { *d = definition.clone(); }
    assert!(store.workflow_apply(&input).unwrap().result.is_ok());
    loop {
        let state = store.workflow_state("workspace").unwrap().unwrap();
        if state.items["item"].phase == Phase::WaitingJoin { break; }
        admit(&store, &state);
        let evidence = Evidence { candidate: "candidate".into(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: "policy-1".into() };
        assert!(store.workflow_apply(&observation(&state, Outcome::Passed(evidence))).unwrap().result.is_ok());
    }
    definition.groups.get_mut("landing").unwrap().exclusive = "shared-target".into();
    for command in [Command::RefreshGroups { definition, policy_pin: "published-policy".into() }, Command::Queue { item: "item".into() }] {
        let state = store.workflow_state("workspace").unwrap().unwrap();
        input.id = format!("command-{}", state.revision); input.expected_revision = state.revision;
        input.source_sequence = state.revision + 1; input.command = command;
        assert!(store.workflow_apply(&input).unwrap().result.is_ok());
    }
    let state = store.workflow_state("workspace").unwrap().unwrap();
    let group = &state.items[state.items["item"].grouped_in.as_ref().unwrap()];
    assert_eq!(group.resource, "shared-target"); assert_eq!(group.phase, Phase::Blocked);
    let mut replay = State::empty("workspace".into(), 1);
    for event in store.workflow_events("workspace").unwrap() { replay = step(&replay, &event).unwrap().state; }
    assert_eq!(state, replay);
}

/// ov-1220: a Start refused for its resource says which resource and what holds it: another item's run in this
/// workspace or another, or the engine cutover's quarantine with its reason. ov-1187 read only `Resource`.
#[test]
fn a_start_refused_for_its_resource_names_it_and_what_holds_it() {
    let path = temp();
    let store = Store::open(&path).unwrap();
    assert_eq!(store.workflow_apply(&enqueue("workspace", "shared")).unwrap().result, Ok(1), "a coding run holds `shared`");
    let ticket = Definition::ticket();
    let ticket_pin = store.workflow_ticket_bundle(&ticket).unwrap();
    let apply = |workspace: &str, id: &str, command: Command| {
        let revision = store.workflow_state(workspace).unwrap().map_or(0, |s| s.revision);
        let mut input = enqueue(workspace, "unused");
        input.id = id.into(); input.expected_revision = revision; input.source_sequence = revision + 1; input.command = command;
        store.workflow_apply(&input).unwrap().result
    };
    let rest = |workspace: &str| assert!(apply(workspace, "rest", Command::Enqueue { item: "ticket".into(), definition: ticket.clone(),
        policy_pin: ticket_pin.clone(), resource: format!("own-{workspace}"), dependencies: BTreeSet::new(),
        initial_candidate: Some(ticket.pin()) }).is_ok());
    let tracker = Definition::tracker("owner");
    let start = |workspace: &str, id: &str, resource: &str| apply(workspace, id, Command::Start { start_load: None, item: "ticket".into(),
        policy_pin: tracker.pin(), resource: resource.into(), dependencies: Default::default(), initial_candidate: Some(tracker.pin()),
        definition: tracker.clone(), models: Default::default(), tree: None, with: vec![] });
    rest("workspace");
    rest("other");
    let Err(Refusal::Held(why)) = start("workspace", "start-1", "shared") else { panic!("the same workspace's holder is named") };
    assert!(why.contains("ticket's resource shared is held by item's run in this workspace"), "{why}");
    assert!(why.contains("`farcooler workflow get workspace`"), "{why}");
    let Err(Refusal::Held(why)) = start("other", "start-2", "shared") else { panic!("another workspace's holder is named") };
    assert!(why.contains("ticket's resource shared is held by item's run in the workspace workspace"), "{why}");
    store.conn().execute("INSERT INTO engine_cutover_quarantine VALUES('fenced','held for the test')", []).unwrap();
    let Err(Refusal::Held(why)) = start("other", "start-3", "fenced") else { panic!("a quarantine is named") };
    assert!(why.contains("ticket's resource fenced is quarantined since the engine's cutover (held for the test)"), "{why}");
    assert!(start("other", "start-4", "free").is_ok(), "a free resource starts");
    drop(store);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
