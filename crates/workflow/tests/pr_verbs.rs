//! Pull request verbs: push-branch, open-pr, pr-ready and await-pr are steps of an item, and the
//! validator keeps them in order and in bounds (ov-957). Private fixtures only.
use farcooler_workflow::*;
use std::collections::BTreeMap;

fn step(id: &str, kind: StageKind) -> Stage {
    Stage { id: id.into(), kind, progression: Progression::Automatic, max_attempts: 1,
        operation: OperationSpec { adapter_version: "fixture/v1".into(), arguments: vec![id.into()], environment: BTreeMap::new() },
        routes: BTreeMap::new(), limit: 0 }
}
fn route(mut s: Stage, label: &str, to: Target, limit: u8) -> Stage { s.routes.insert(label.into(), to); s.limit = limit; s }
/// build, push-branch, open-pr, await-pr (a requested change goes back to build), await-ci: the per-ticket pull request.
fn steps() -> Vec<Stage> {
    vec![step("build", StageKind::Action("build".into())),
        step("branch", StageKind::PushBranch),
        step("pr", StageKind::OpenPr),
        route(route(step("merged", StageKind::AwaitPr), "closed", Target::Fail, 0), "changes_requested", Target::Step("build".into()), 2),
        step("ci", StageKind::AwaitCi)]
}
fn definition(stages: Vec<Stage>) -> Definition {
    Definition { schema: 2, source: "pr/v1".into(), max_reworks: 0, rework_target: None, stages, groups: BTreeMap::new(), models: BTreeMap::new(), send_backs: None }
}
fn invalid(edit: impl FnOnce(&mut Vec<Stage>)) -> bool {
    let mut s = steps(); edit(&mut s); definition(s).validate() == Err(Refusal::Definition)
}

#[test]
fn the_per_ticket_pull_request_validates() { assert_eq!(definition(steps()).validate(), Ok(())); }
#[test]
fn the_ready_step_validates_between_the_pull_request_and_the_wait() {
    let mut s = steps(); s.insert(3, step("ready", StageKind::PrReady));
    assert_eq!(definition(s).validate(), Ok(()));
}
#[test]
fn the_verbs_are_routed_so_schema_one_refuses_them() {
    let mut d = definition(steps()); d.schema = 1;
    for stage in &mut d.stages { stage.routes.clear(); stage.limit = 0; }
    assert_eq!(d.validate(), Err(Refusal::Definition));
}
#[test]
fn each_step_needs_the_one_before_it() {
    // A pull request needs its branch pushed.
    assert!(invalid(|s| { s.remove(1); }));
    // Waiting on, or readying, a pull request nobody opened.
    assert!(invalid(|s| { s.remove(2); }));
    assert!(invalid(|s| { s.insert(1, step("ready", StageKind::PrReady)); }));
}
#[test]
fn they_are_not_group_steps() {
    let split = |to: &str| Target::Split { step: to.into(), otherwise: Box::new(Target::Fail) };
    let group = |extra: Option<Stage>| {
        let mut steps = vec![step("base", StageKind::AwaitBase),
            route(step("fold", StageKind::Fold), "conflict", split("base"), 3),
            route(step("gate", StageKind::Run), "rejected", split("base"), 3),
            route(step("push", StageKind::Push), "base_moved", Target::Step("base".into()), 2),
            step("ci", StageKind::AwaitCi)];
        if let Some(extra) = extra { steps.insert(1, extra); }
        let mut d = definition(vec![step("build", StageKind::Action("build".into())), step("join", StageKind::Join { group: "g".into(), adopt: true })]);
        d.groups.insert("g".into(), GroupDef { limit: 4, exclusive: "ref:r:main".into(), steps });
        d
    };
    assert_eq!(group(None).validate(), Ok(()), "the group is sound without it");
    assert_eq!(group(Some(step("branch", StageKind::PushBranch))).validate(), Err(Refusal::Definition));
}
#[test]
fn each_verb_emits_only_its_own_labels() {
    assert!(invalid(|s| { s[1].routes.insert("changes_requested".into(), Target::Fail); }));
    assert!(invalid(|s| { s[3].routes.insert("base_moved".into(), Target::Fail); }));
    assert!(invalid(|s| { s[4].routes.insert("changes_requested".into(), Target::Fail); }));
    // These are the labels a wait on a pull request has.
    for label in ["closed", "timeout", "changes_requested"] {
        let mut s = steps(); s[3].routes.insert(label.into(), Target::Fail);
        // changes_requested already goes back with a limit; replacing it with Fail removes the back edge and its limit.
        s[3].limit = if label == "changes_requested" { 0 } else { 2 };
        assert_eq!(definition(s).validate(), Ok(()), "{label}");
    }
    // A pull request that was closed is also what opening or readying one can find.
    let mut s = steps(); s[2].routes.insert("closed".into(), Target::Fail);
    assert_eq!(definition(s).validate(), Ok(()));
}
#[test]
fn a_requested_change_goes_back_to_where_a_new_candidate_is_made() {
    // Back to the push would push the same commit again.
    assert!(invalid(|s| { s[3].routes.insert("changes_requested".into(), Target::Step("branch".into())); }));
    // And it is bounded like every back edge.
    assert!(invalid(|s| { s[3].limit = 0; }));
}
#[test]
fn nothing_goes_back_across_a_merge() {
    // A failed run of CI after the merge may not rebuild and open a second pull request.
    assert!(invalid(|s| { s[4].routes.insert("rejected".into(), Target::Step("build".into())); s[4].limit = 1; }));
}
#[test]
fn the_merge_commit_is_the_candidate_the_next_step_waits_on() {
    assert!(makes_candidate(&StageKind::AwaitPr));
    assert!(!makes_candidate(&StageKind::OpenPr) && !makes_candidate(&StageKind::PushBranch));
    // CI needs a candidate on every path, and the wait on the merge supplies it.
    assert_eq!(definition(steps()).validate(), Ok(()));
}
