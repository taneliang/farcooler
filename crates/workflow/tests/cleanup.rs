//! A missing policy keeps old pins stable; explicit deletion policy is validated and pinned.
use farcooler_workflow::*;
use farcooler_workflow::cleanup::{DEFAULT_BUILD_FOLDERS, effects};
use std::collections::BTreeSet;

#[test]
fn build_cleanup_policy_defaults_preserve_old_definition_encoding() {
    let definition = Definition::coding();
    let raw = serde_json::to_value(&definition).unwrap();
    assert!(raw.get("build_folders").is_none());
    let old: Definition = serde_json::from_value(raw).unwrap();
    assert_eq!(old.pin(), definition.pin());
    assert_eq!(old.build_folders(), DEFAULT_BUILD_FOLDERS);
    let mut explicit = old.clone();
    explicit.build_folders = Some(vec!["artifacts/build".into()]);
    explicit.validate().unwrap();
    assert_ne!(explicit.pin(), old.pin());
    assert_eq!(explicit.build_folders(), ["artifacts/build"]);
    explicit.build_folders = Some(vec![]);
    explicit.validate().unwrap();
    assert!(explicit.build_folders().is_empty());
}
#[test]
fn build_cleanup_policy_refuses_paths_outside_the_worktree_or_git_metadata() {
    for path in ["", "/target", "../target", "a/../target", "a//build", ".git", ".git/objects", "a/.git/build", "*", "target/", "a/**/build", ".GIT", "a/.Git/objects"] {
        let mut definition = Definition::coding();
        definition.build_folders = Some(vec![path.into()]);
        assert_eq!(definition.validate(), Err(Refusal::Definition), "{path}");
    }
}
fn initial() -> State {
    let state = State::empty("w".into(), 1);
    let input = Input { schema: 1, id: "enqueue".into(), workspace: "w".into(), expected_revision: 0,
        epoch: 1, source: "test".into(), source_sequence: 1, now_ms: 1, grant: Grant::Controller,
        busy_resources: BTreeSet::new(), command: Command::Enqueue { item: "one".into(), definition: Definition::coding(),
            policy_pin: "policy".into(), resource: "resource".into(), dependencies: BTreeSet::new(), initial_candidate: None } };
    step(&state, &input).unwrap().state
}
#[test]
fn build_cleanup_selects_every_end_and_uncertainty_but_never_an_unchanged_build() {
    let mut before = initial();
    before.items.get_mut("one").unwrap().stage = 1;
    assert_eq!(before.items["one"].definition.stages[1].kind, StageKind::Build);
    assert!(effects(&before, &before).is_empty());
    for phase in [Phase::Done, Phase::Cancelled, Phase::Failed, Phase::Uncertain] {
        let mut after = before.clone();
        after.revision += 1;
        after.items.get_mut("one").unwrap().phase = phase.clone();
        let intents = effects(&before, &after);
        assert_eq!(intents.len(), 1, "{phase:?}");
        assert_eq!(intents[0].uncertain, phase == Phase::Uncertain);
        assert!(effects(&after, &after).is_empty());
    }
    let mut after = before.clone();
    after.revision += 1;
    after.items.clear();
    assert_eq!(effects(&before, &after).len(), 1, "archiving retains the intent");
}

#[test]
fn build_cleanup_wait_after_check_retains_the_runs_tree_and_published_policy() {
    for kind in [StageKind::Review(Reviewer::Person("owner".into())),
        StageKind::Join { group: "landing".into(), adopt: true }, StageKind::Land,
        StageKind::PushBranch, StageKind::OpenPr, StageKind::PrReady, StageKind::AwaitPr] {
        let mut before = initial();
        let old = before.items.get_mut("one").unwrap();
        old.stage = 2;
        assert_eq!(old.definition.stages[old.stage].kind, StageKind::Check);
        old.definition.build_folders = Some(vec!["custom-output".into()]);
        old.tree = Some(RunTree { name: "own-tree".into(), branch: "own-branch".into(),
            path: "/tmp/own-tree".into(), base: "a".repeat(40), adopted: false });
        old.definition.stages[3].kind = kind.clone();
        let mut after = before.clone();
        after.revision += 1;
        after.items.get_mut("one").unwrap().stage = 3;
        let intents = effects(&before, &after);
        assert_eq!(intents.len(), 1, "{kind:?}");
        let intent = &intents[0];
        assert_eq!(intent.tree, before.items["one"].tree);
        assert_eq!(intent.run, before.items["one"].run);
        assert_eq!(intent.folders, ["custom-output"]);
        assert_eq!(intent.revision, after.revision);
        assert_eq!(effects(&before, &after), intents, "replay keeps the effect identity");
        assert!(effects(&after, &after).is_empty(), "waiting does not repeat deletion");
    }
}


#[test]
fn build_cleanup_groups_select_transitions_ends_and_uncertainty_with_the_same_identity() {
    let mut before = initial();
    let group = before.items.get_mut("one").unwrap();
    group.group_of = Some(GroupOf { name: "landing".into(), pin: "published".into() });
    group.stage = 1;
    group.definition.build_folders = Some(vec!["group-output".into()]);
    assert!(effects(&before, &before).is_empty());
    for phase in [Phase::Done, Phase::Cancelled, Phase::Failed, Phase::Uncertain, Phase::WaitingReview] {
        let mut after = before.clone();
        after.revision += 1;
        let group = after.items.get_mut("one").unwrap();
        group.phase = phase.clone();
        if phase == Phase::WaitingReview { group.stage = 3; }
        let intents = effects(&before, &after);
        assert_eq!(intents.len(), 1, "{phase:?}");
        assert_eq!(intents[0].uncertain, phase == Phase::Uncertain);
        assert_eq!(intents[0].folders, ["group-output"]);
        assert_eq!(intents[0].run, before.items["one"].run);
        assert_eq!(effects(&before, &after), intents);
        assert!(effects(&after, &after).is_empty());
    }
}

#[test]
fn build_cleanup_group_definition_inherits_the_published_folder_policy() {
    let mut definition = Definition::coding();
    definition.groups.insert("landing".into(), GroupDef { limit: Some(1), exclusive: "landing".into(),
        steps: definition.stages.clone() });
    for folders in [None, Some(vec![]), Some(vec!["custom-output".into()])] {
        definition.build_folders = folders.clone();
        let group = definition.group_definition("landing").unwrap();
        assert_eq!(group.build_folders, folders);
        assert_eq!(group.build_folders(), definition.build_folders());
    }
}

#[test]
fn build_cleanup_keeps_build_output_for_the_following_check() {
    let mut before = initial();
    before.items.get_mut("one").unwrap().stage = 1;
    let mut after = before.clone();
    after.revision += 1;
    after.items.get_mut("one").unwrap().stage = 2;
    assert_eq!(after.items["one"].definition.stages[2].kind, StageKind::Check);
    assert!(effects(&before, &after).is_empty());
}

#[test]
fn build_cleanup_selects_a_rest_column_or_wait_without_a_step_change() {
    let mut before = initial();
    before.items.get_mut("one").unwrap().stage = 1;
    let mut after = before.clone();
    after.revision += 1;
    after.items.get_mut("one").unwrap().phase = Phase::WaitingApproval;
    assert_eq!(effects(&before, &after).len(), 1);
    assert!(effects(&after, &after).is_empty());

    let resting = after.items.get_mut("one").unwrap();
    resting.definition = Definition::ticket();
    resting.phase = Phase::Blocked;
    let intents = effects(&before, &after);
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].run, before.items["one"].run);
}
