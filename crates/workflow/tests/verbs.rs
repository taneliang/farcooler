//! Landing verbs: await-base, fold, run, push and await-ci are steps of a group, and the validator
//! keeps them safe. Private fixtures only.
use farcooler_workflow::*;
use std::collections::BTreeMap;

fn step(id: &str, kind: StageKind) -> Stage {
    Stage { id: id.into(), kind, progression: Progression::Automatic, max_attempts: 1,
        operation: OperationSpec { adapter_version: "fixture/v1".into(), arguments: vec![id.into()], environment: BTreeMap::new() },
        routes: BTreeMap::new(), limit: 0 }
}
fn route(mut s: Stage, label: &str, to: Target, limit: u8) -> Stage { s.routes.insert(label.into(), to); s.limit = limit; s }
fn split(to: &str) -> Target { Target::Split { step: to.into(), otherwise: Box::new(Target::Fail) } }
/// await-base, fold, run, push, await-ci: the batched push, as a group.
fn steps() -> Vec<Stage> {
    vec![step("base", StageKind::AwaitBase),
        route(step("fold", StageKind::Fold), "conflict", split("base"), 3),
        route(step("gate", StageKind::Run), "rejected", split("base"), 3),
        route(step("push", StageKind::Push), "base_moved", Target::Step("base".into()), 2),
        route(route(step("ci", StageKind::AwaitCi), "rejected", Target::Fail, 0), "timeout", Target::Fail, 0)]
}
fn definition(steps: Vec<Stage>) -> Definition {
    let mut join = step("join", StageKind::Join { group: "landing".into(), adopt: true });
    join.operation.arguments.clear();
    Definition { schema: 2, source: "landing/v1".into(), max_reworks: 0, rework_target: None,
        stages: vec![step("build", StageKind::Action("build".into())), join],
        groups: [("landing".to_string(), GroupDef { limit: 8, exclusive: "ref:origin:main".into(), steps })].into() }
}
fn invalid(edit: impl FnOnce(&mut Vec<Stage>)) -> bool {
    let mut s = steps(); edit(&mut s); definition(s).validate() == Err(Refusal::Definition)
}

#[test]
fn the_batched_push_validates() { assert_eq!(definition(steps()).validate(), Ok(())); }
#[test]
fn a_push_retried_from_the_base_is_the_one_back_edge_across_a_push() {
    // base_moved means the remote refused: nothing landed, so going back to the base is safe.
    assert_eq!(definition(steps()).validate(), Ok(()));
    // The same edge for any other label would repeat a push that may have landed.
    assert!(invalid(|s| { s[3].routes.clear(); s[3].routes.insert("rejected".into(), Target::Step("base".into())); }));
    // And no later step may go back across the push.
    assert!(invalid(|s| { s[4].routes.insert("rejected".into(), Target::Step("base".into())); s[4].limit = 1; }));
}
#[test]
fn a_split_never_follows_a_push() {
    // run after push: its split would push the second half again.
    assert!(invalid(|s| { s.push(route(step("late", StageKind::Run), "rejected", split("base"), 1)); }));
}
#[test]
fn a_push_needs_a_check_after_the_fold() {
    assert!(invalid(|s| { s.remove(2); s[1].routes.clear(); s[1].limit = 0; }));
    // A rejected gate may not fall forward to the push, or finish the group with what failed.
    let forward = |to: Target| { let mut s = steps(); s[2].routes.insert("rejected".into(), to); s[2].limit = 0; definition(s).validate() };
    assert_eq!(forward(Target::Step("push".into())), Err(Refusal::Definition));
    assert_eq!(forward(Target::Done), Err(Refusal::Definition));
    assert_eq!(forward(Target::Fail), Ok(()), "failing is what a rejected gate does by default");
}
#[test]
fn each_verb_emits_only_its_own_labels() {
    assert!(invalid(|s| { s[0].routes.insert("base_moved".into(), Target::Fail); }));
    assert!(invalid(|s| { s[1].routes.insert("base_moved".into(), Target::Fail); }));
    assert!(invalid(|s| { s[2].routes.insert("conflict".into(), Target::Fail); }));
    assert!(invalid(|s| { s[3].routes.insert("timeout".into(), Target::Fail); }));
    assert!(invalid(|s| { s[4].routes.insert("conflict".into(), Target::Fail); }));
    // Waiting verbs time out; CI can also be canceled.
    let mut s = steps();
    s[0].routes.insert("timeout".into(), Target::Fail);
    s[4].routes.insert("cancelled".into(), Target::Fail);
    assert_eq!(definition(s).validate(), Ok(()));
}
#[test]
fn group_verbs_are_for_groups_and_all_verbs_need_schema_2() {
    let mut d = definition(steps());
    d.stages.insert(1, step("fold", StageKind::Fold));
    assert_eq!(d.validate(), Err(Refusal::Definition), "a ticket has no members to fold");
    for kind in [StageKind::Run, StageKind::AwaitCi] {
        let mut d = Definition::coding(); d.stages.push(step("v", kind));
        assert_eq!(d.validate(), Err(Refusal::Definition), "schema 1 has no verbs");
    }
}
#[test]
fn a_fold_needs_the_base_a_step_before_it_and_a_split_restarts_at_the_wait_for_it() {
    // The split target must make its candidate anew: waiting for the base does, folding does not.
    assert!(invalid(|s| { s[2].routes.insert("rejected".into(), Target::Split { step: "fold".into(), otherwise: Box::new(Target::Fail) }); }));
    // Fold with no await-base ahead has no base to fold onto.
    assert!(invalid(|s| { s.remove(0); s[0].routes.clear(); s[0].limit = 0; s[1].routes.clear(); s[1].limit = 0; s[2].routes.clear(); s[2].limit = 0; }));
}

#[test]
fn no_route_reaches_a_push_without_a_passing_gate_after_the_last_change() {
    // fold passed -> push: every ordinary fold would push unchecked.
    assert!(invalid(|s| { s[1].routes.insert("passed".into(), Target::Step("push".into())); s[1].routes.insert("conflict".into(), Target::Step("gate".into())); s[1].limit = 0; }));
    // The same through a route that is not a rejected gate: a conflict going to the push.
    assert!(invalid(|s| { s[1].routes.insert("conflict".into(), Target::Step("push".into())); s[1].limit = 0; }));
    // A gate whose rejection splits, with the fallback pushing or finishing, would push what failed.
    for otherwise in [Target::Step("push".into()), Target::Done] {
        assert!(invalid(|s| { s[2].routes.insert("rejected".into(), Target::Split { step: "base".into(), otherwise: Box::new(otherwise.clone()) }); }), "{otherwise:?}");
    }
    // Failing is still fine, and so is a gate that is passed on every path.
    assert_eq!(definition(steps()).validate(), Ok(()));
}
#[test]
fn a_gate_before_a_change_does_not_count() {
    // run, then another fold-like change (await-base) before the push: the gate judged something else.
    let s = vec![step("base", StageKind::AwaitBase), step("fold", StageKind::Fold), step("gate", StageKind::Run),
        step("base2", StageKind::AwaitBase), step("push", StageKind::Push)];
    assert_eq!(definition(s).validate(), Err(Refusal::Definition));
}
