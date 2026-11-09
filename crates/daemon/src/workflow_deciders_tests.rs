use super::*;

#[test]
fn a_decider_is_the_owner_a_member_or_the_maintainer_role() {
    for good in ["owner", "member:alice", "member:a-1", "role:maintainer"] { assert!(well_formed(good), "{good}"); }
    for bad in ["", "alice", "orchestrator", "member:", "member:Alice", "member:alice smith", "role:", "role:admin", "role:Maintainer"] {
        assert!(!well_formed(bad), "{bad}");
    }
}

#[test]
fn a_gate_decides_as_its_name_only_if_the_caller_holds_it() {
    let held = ["member:alice".to_string(), MAINTAINER.to_string()];
    assert_eq!(decide_as(Some("member:alice"), &held, "member:alice"), "member:alice");
    assert_eq!(decide_as(Some(MAINTAINER), &held, "member:alice"), MAINTAINER);
    assert_eq!(decide_as(Some("member:bob"), &held, "member:alice"), "member:alice", "not hers: as herself, which the reducer refuses");
    assert_eq!(decide_as(Some(MAINTAINER), &["member:bob".into()], "member:bob"), "member:bob");
    assert_eq!(decide_as(None, &held, "member:alice"), "member:alice");
}

#[test]
fn every_named_decider_is_listed_once() {
    let mut definition = Definition::tracker("member:alice");
    let mut second = definition.stages[0].clone(); second.id = "second".into();
    second.progression = Progression::AwaitApproval { principal: "owner".into() };
    let mut third = definition.stages[0].clone(); third.id = "third".into();
    definition.stages.extend([second, third]);
    assert_eq!(named(&definition), ["member:alice", "owner"]);
}

#[tokio::test]
async fn a_definition_naming_a_stranger_or_a_malformed_name_is_refused() {
    let (_dir, svc, _repo) = crate::test_support::fixture().await;
    svc.store.set_member("alice", "Alice", &[]).unwrap();
    for ok in ["owner", "member:alice", MAINTAINER] { assert_eq!(refuse_unknown(&svc.store, &Definition::tracker(ok)), Ok(()), "{ok}"); }
    for bad in ["member:bob", "alice", "role:admin"] { assert_eq!(refuse_unknown(&svc.store, &Definition::tracker(bad)), Err(bad.into())); }
}

/// An approval step may wait on `orchestrator`, as a ticket's does; an orchestrator's review gate names only
/// `orchestrator`, so no member slips in through one (review F4, F5).
#[tokio::test]
async fn an_orchestrators_gate_names_only_the_orchestrator() {
    let (_dir, svc, _repo) = crate::test_support::fixture().await;
    svc.store.set_member("alice", "Alice", &[]).unwrap();
    assert_eq!(refuse_unknown(&svc.store, &Definition::tracker("orchestrator")), Ok(()));
    let mut definition = Definition::tracker("owner");
    for (named, refused) in [("orchestrator", None), ("member:alice", Some("member:alice")), ("member:ghost", Some("member:ghost"))] {
        definition.stages[0].kind = StageKind::Review(Reviewer::Orchestrator(named.into()));
        assert_eq!(refuse_unknown(&svc.store, &definition), refused.map_or(Ok(()), |r| Err(r.into())), "{named}");
    }
    definition.stages[0].kind = StageKind::Review(Reviewer::Person("orchestrator".into()));
    assert_eq!(refuse_unknown(&svc.store, &definition), Err("orchestrator".into()), "a person gate names a person");
}

#[tokio::test]
async fn a_removed_member_is_found_at_any_step() {
    let (_dir, svc, _repo) = crate::test_support::fixture().await;
    svc.store.set_member("alice", "Alice", &[]).unwrap();
    let mut definition = Definition::tracker("owner");
    assert_eq!(names_a_removed_member(&svc.store, &definition), None);
    let mut later = definition.stages[0].clone(); later.id = "later".into();
    later.progression = Progression::Automatic;
    later.kind = StageKind::Review(Reviewer::Person("member:bob".into()));
    definition.stages.push(later);
    assert_eq!(names_a_removed_member(&svc.store, &definition), Some("member:bob"));
    definition.stages[1].kind = StageKind::Review(Reviewer::Person("member:alice".into()));
    assert_eq!(names_a_removed_member(&svc.store, &definition), None);
}
