//! A run that moves announces the plan changed, once.

use farcooler_workflow::{Command, Definition, Grant, Input, Phase, Progression};

use crate::watch::Watcher;

/// A run that moves while the plan doesn't (ov-1040): the open plans are told to read it again, once per change.
#[tokio::test]
async fn a_run_that_moves_announces_the_plan_changed() {
    use farcooler_protocol::v1::event::Payload;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let board = svc.store.ensure_main_workspace(repo).unwrap();
    let watcher = Watcher::new(svc.clone());
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
