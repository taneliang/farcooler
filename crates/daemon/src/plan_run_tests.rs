//! What a ticket's run says on the wire: phase, stage and who it waits on.

use super::*;
use farcooler_workflow::{step, Command, Definition, Grant, Input, Phase, Reviewer, StageKind};

/// A state holding one coding run for `item`; with `approval`, its first stage waits on `owner`.
fn state_with(item: &str, approval: bool) -> State {
    let mut definition = Definition::coding();
    if approval {
        definition.stages[0].progression = Progression::AwaitApproval { principal: "owner".into() };
    }
    step(
        &State::empty("workspace".into(), 1),
        &Input {
            schema: 1,
            id: "event-0".into(),
            workspace: "workspace".into(),
            expected_revision: 0,
            epoch: 1,
            source: "fixture".into(),
            source_sequence: 1,
            now_ms: 1,
            grant: Grant::Controller,
            command: Command::Enqueue {
                item: item.into(),
                definition,
                policy_pin: "policy".into(),
                resource: "resource".into(),
                dependencies: Default::default(),
                initial_candidate: None,
            },
            busy_resources: Default::default(),
        },
    )
    .unwrap()
    .state
}

#[test]
fn a_running_run_says_its_phase_and_stage_and_no_gate() {
    let run = run_of(&state_with("ov-310", false), "OV-310").expect("keys match without case");
    assert_eq!(run.phase, "Running");
    assert_eq!(run.stage_kind, "\"Provision\"");
    assert_eq!((run.gate.as_str(), run.gate_principal.as_str()), ("", ""));
}

#[test]
fn a_run_waiting_for_approval_names_who_decides() {
    let run = run_of(&state_with("ov-310", true), "ov-310").unwrap();
    assert_eq!(run.phase, "WaitingApproval");
    assert_eq!((run.gate.as_str(), run.gate_principal.as_str()), ("approval", "owner"));
}

#[test]
fn a_ticket_with_no_run_has_none() {
    assert!(run_of(&state_with("ov-310", false), "ov-999").is_none());
}

/// `state_with`'s one item, changed by `edit`.
fn edited(edit: impl FnOnce(&mut farcooler_workflow::Item)) -> State {
    let mut state = state_with("ov-310", false);
    edit(state.items.get_mut("ov-310").unwrap());
    state
}

#[test]
fn a_run_waiting_for_review_names_the_reviewer() {
    let state = edited(|item| {
        item.definition.stages[0].kind = StageKind::Review(Reviewer::Person("owner".into()));
        item.phase = Phase::WaitingReview;
    });
    let run = run_of(&state, "ov-310").unwrap();
    assert_eq!(run.phase, "WaitingReview");
    assert_eq!((run.gate.as_str(), run.gate_principal.as_str()), ("review", "owner"));
    assert_eq!(run.stage_kind, r#"{"Review":{"Person":"owner"}}"#);
}

#[test]
fn a_review_nobody_decides_has_no_gate() {
    let state = edited(|item| {
        item.definition.stages[0].kind = StageKind::Review(Reviewer::Agent);
        item.phase = Phase::WaitingReview;
    });
    assert_eq!(run_of(&state, "ov-310").unwrap().gate, "");
}

#[test]
fn a_failed_and_an_uncertain_run_say_so_with_no_gate() {
    for (phase, word) in [(Phase::Failed, "Failed"), (Phase::Uncertain, "Uncertain")] {
        let run = run_of(&edited(|item| item.phase = phase), "ov-310").unwrap();
        assert_eq!(run.phase, word);
        assert_eq!((run.gate.as_str(), run.gate_principal.as_str()), ("", ""));
        assert_eq!(run.stage_kind, "\"Provision\"", "still the stage it stopped on");
    }
}

#[test]
fn a_landed_run_is_past_its_last_stage() {
    let run = run_of(
        &edited(|item| {
            item.phase = Phase::Done;
            item.stage = item.definition.stages.len();
        }),
        "ov-310",
    )
    .unwrap();
    assert_eq!(run.phase, "Done");
    assert_eq!(run.stage_kind, "", "no step after the last");
}

#[test]
fn a_tracker_only_ticket_is_a_card_not_a_run() {
    assert!(run_of(&edited(|item| item.definition = Definition::ticket()), "ov-310").is_none());
}

/// A run that moves while the plan doesn't (ov-1040): the open plans are told to read it again, once per change.
#[tokio::test]
async fn a_run_that_moves_announces_the_plan_changed() {
    use farcooler_protocol::v1::event::Payload;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let board = svc.store.ensure_main_workspace(repo).unwrap();
    let watcher = crate::watch::Watcher::new(svc.clone());
    let mut events = watcher.subscribe();
    let workspace = board.id.to_string();
    let apply = |sequence: u64, expected: u64, grant: Grant, command: Command| {
        svc.store
            .workflow_apply(&Input {
                schema: 1,
                id: format!("event-{sequence}"),
                workspace: workspace.clone(),
                expected_revision: expected,
                epoch: 1,
                source: "fixture".into(),
                source_sequence: sequence,
                now_ms: sequence as i64,
                grant,
                command,
                busy_resources: Default::default(),
            })
            .unwrap()
    };
    let changed = |events: &mut tokio::sync::broadcast::Receiver<farcooler_protocol::v1::Event>| {
        std::iter::from_fn(|| events.try_recv().ok())
            .filter(|e| matches!(e.payload, Some(Payload::PlanChanged(_))))
            .count()
    };
    let mut definition = Definition::coding();
    definition.stages[0].progression = Progression::AwaitApproval { principal: "owner".into() };
    apply(
        1,
        0,
        Grant::Controller,
        Command::Enqueue {
            item: "ov-310".into(),
            definition,
            policy_pin: "policy".into(),
            resource: "resource".into(),
            dependencies: Default::default(),
            initial_candidate: None,
        },
    );
    assert_eq!(changed(&mut events), 1, "a run that appears is news");
    let before = svc.store.workflow_state(&workspace).unwrap().unwrap();
    assert_eq!(before.items["ov-310"].phase, Phase::WaitingApproval);

    // An app's Approve arrives through `workflow.command`, which writes through the wire path, not `workflow_apply`.
    let input = Input {
        schema: 1,
        id: "event-2".into(),
        workspace: workspace.clone(),
        expected_revision: before.revision,
        epoch: 1,
        source: "fixture".into(),
        source_sequence: 2,
        now_ms: 2,
        grant: Grant::Decision { principal: "owner".into() },
        command: Command::Approve { item: "ov-310".into(), run: before.items["ov-310"].run.clone(), pin: before.items["ov-310"].pin.clone(), candidate: None },
        busy_resources: Default::default(),
    };
    let payload = serde_json::to_string(&input).unwrap();
    let receipt = svc.store.workflow_wire_write_by(&workspace, "wire-approve", &payload, None, farcooler_store::workflow_publication::Write::Apply(&input)).unwrap();
    assert!(receipt.result.is_ok(), "{:?}", receipt.result);
    let after = svc.store.workflow_state(&workspace).unwrap().unwrap();
    assert_ne!(after.items["ov-310"].phase, Phase::WaitingApproval, "the approval moved the run");
    assert_eq!(changed(&mut events), 1, "a run that moved while the plan did not is news");
    let again = svc.store.workflow_wire_write_by(&workspace, "wire-approve", &payload, None, farcooler_store::workflow_publication::Write::Apply(&input)).unwrap();
    assert!(again.result.is_ok());
    assert_eq!(changed(&mut events), 0, "a receipt answered from the cache is not news");
}
