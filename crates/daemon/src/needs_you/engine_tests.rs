//! A stuck engine build is a Block in its own workspace, with what the pane asks.
use super::super::*;
use crate::engine_stuck::Stuck;

#[test]
fn a_stuck_build_is_a_block_naming_its_question_in_its_workspace() {
    let workspace = Uuid::now_v7();
    let mut inputs = Inputs::default();
    inputs.workspaces.insert(workspace, "Overnight".into());
    inputs.engine_stuck = vec![Stuck {
        workspace: workspace.to_string(),
        item: "ov-9".into(),
        since_ms: 1_000,
        question: "Do you trust the files in this folder?".into(),
    }];
    let items = assemble(&inputs, UNIX_EPOCH + std::time::Duration::from_secs(10));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind(), NeedsYouKind::Blocked);
    assert_eq!(items[0].question, "Do you trust the files in this folder?");
    assert_eq!(items[0].detail.as_deref(), Some("ov-9"));
    assert_eq!(items[0].workspace_name, "Overnight");
}

#[test]
fn tickets_stopped_at_one_providers_limit_are_one_item() {
    use farcooler_workflow::*;
    let workspace = Uuid::now_v7().to_string();
    let mut state = State::empty(workspace.clone(), 1);
    let run = |state: &mut State, command: Command, grant: Grant| {
        let input = Input { schema: 1, id: format!("c{}", state.revision), workspace: state.workspace.clone(),
            expected_revision: state.revision, epoch: 1, source: "t".into(), source_sequence: state.revision + 1,
            now_ms: 1_000 + state.revision as i64, grant, command, busy_resources: Default::default() };
        let done = step(state, &input).unwrap();
        *state = done.state;
        done.effects
    };
    for (key, harness, name) in [("ov-1", "codex", "Codex"), ("ov-2", "codex", "Codex"), ("ov-3", "claude", "Claude")] {
        let mut definition = Definition::coding();
        definition.stages = definition.stages[1..2].to_vec();
        definition.max_reworks = 0;
        definition.rework_target = None;
        let effect = run(&mut state, Command::Enqueue { item: key.into(), definition, policy_pin: "p".into(),
            resource: format!("r-{key}"), dependencies: Default::default(), initial_candidate: None }, Grant::Controller).remove(0);
        let reason = format!("usage limit until Dec 1st, 2026 1:23 AM ({name})");
        run(&mut state, Command::Observe { item: key.into(), effect: effect.id.clone(), run: effect.run.clone(),
            fence: effect.fence, outcome: Outcome::UsageLimit { harness: harness.into(), reason } },
            Grant::Operation { effect: effect.id, run: effect.run, fence: effect.fence });
    }
    let failures: Vec<_> = state.items.iter().map(|(item, run)| farcooler_store::workflow::WorkflowFailure {
        workspace: workspace.clone(), item: item.clone(), since_ms: run.since_ms, reason: run.failure.clone().unwrap() }).collect();
    let mut inputs = Inputs { engine_stuck: crate::engine_stuck::provider_limits(&failures), ..Inputs::default() };
    inputs.workspaces.insert(Uuid::parse_str(&workspace).unwrap(), "Overnight".into());
    let items = assemble(&inputs, UNIX_EPOCH + std::time::Duration::from_secs(10));
    assert_eq!(items.len(), 2, "one per provider: {items:#?}");
    let codex = items.iter().find(|i| i.detail.as_deref() == Some("Codex")).unwrap();
    assert!(codex.question.contains("usage limit until Dec 1st, 2026 1:23 AM"), "{}", codex.question);
    assert!(codex.question.contains("ov-1, ov-2"), "{}", codex.question);
    assert!(codex.question.contains("farcooler workflow model"), "{}", codex.question);
    assert_eq!(codex.workspace_name, "Overnight");
}
