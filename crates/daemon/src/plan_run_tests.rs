//! What a ticket's run says on the wire: phase, stage and who it waits on.

use super::*;
use farcooler_workflow::{step, Command, Definition, Grant, Input};

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
