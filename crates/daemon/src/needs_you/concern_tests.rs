//! The routing rule (ov-633): whose a pane is, what each reader is told, and
//! who is pushed.

use super::*;

fn member(h: &str) -> Option<Whose> {
    Some(Whose::Member(h.into()))
}

fn names() -> HashMap<String, String> {
    HashMap::from([("alice".into(), "Alice Chen".into()), ("bob".into(), "Bob Okafor".into())])
}

#[test]
fn a_pane_is_its_reps_then_its_orchestrators_lead_then_its_login() {
    let rep = PaneFacts { rep: Some("bob".into()), login: Some(("alice".into(), true)), ..PaneFacts::default() };
    assert_eq!(whose_pane(&rep), member("bob"), "a rep is its member's, whatever login it records");
    let led = PaneFacts { orchestrator_lead: Some(Some("alice".into())), ..PaneFacts::default() };
    assert_eq!(whose_pane(&led), member("alice"), "an orchestrator is its lead's");
    let owner_led = PaneFacts { orchestrator_lead: Some(None), login: None, ..PaneFacts::default() };
    assert_eq!(whose_pane(&owner_led), Some(Whose::Owner), "an orchestrator the owner leads is the owner's");
    let lane = PaneFacts { login: Some(("alice".into(), true)), ..PaneFacts::default() };
    assert_eq!(whose_pane(&lane), member("alice"), "a pane on a member's login is theirs");
    assert_eq!(whose_pane(&PaneFacts::default()), Some(Whose::Owner), "the runner's own login is the owner's");
}

#[test]
fn a_removed_members_pane_is_nobodys() {
    let gone = PaneFacts { login: Some(("alice".into(), false)), ..PaneFacts::default() };
    assert_eq!(whose_pane(&gone), None);
}

#[test]
fn a_task_with_no_agent_is_its_desks_member_or_nobodys() {
    let (desk, other) = (Uuid::now_v7(), Uuid::now_v7());
    let people = People { desks: HashMap::from([(desk, "bob".to_string())]), ..People::default() };
    assert_eq!(people.whose(None, Some(desk)), member("bob"));
    assert_eq!(people.whose(None, Some(other)), None, "a task nobody's desk is nobody's");
    let pane = Uuid::now_v7();
    let people = People { panes: HashMap::from([(pane, member("alice"))]), ..people };
    assert_eq!(people.whose(Some(pane), Some(desk)), member("alice"), "the agent on it wins over the desk");
}

#[test]
fn a_member_reads_their_own_as_yours_and_anothers_as_others() {
    let alice = Person::Member("alice".into());
    let mine = concern(member("alice").as_ref(), Some(&alice), &names()).unwrap();
    assert!(mine.yours);
    assert_eq!((mine.handle.as_str(), mine.name.as_str()), ("alice", "Alice Chen"));
    let bobs = concern(member("bob").as_ref(), Some(&alice), &names()).unwrap();
    assert!(!bobs.yours);
    assert_eq!(bobs.name, "Bob Okafor");
    let owners = concern(Some(&Whose::Owner), Some(&alice), &names()).unwrap();
    assert!(!owners.yours, "the owner's item is someone else's to a member");
    assert_eq!(owners.handle, "", "the owner has no handle");
}

#[test]
fn the_owner_an_admin_reads_members_items_as_others_and_their_own_as_yours() {
    let owner = Person::Owner;
    assert!(concern(Some(&Whose::Owner), Some(&owner), &names()).unwrap().yours);
    assert!(!concern(member("alice").as_ref(), Some(&owner), &names()).unwrap().yours);
}

#[test]
fn nobodys_item_carries_no_concern_and_files_with_yours() {
    assert_eq!(concern(None, Some(&Person::Member("alice".into())), &names()), None);
    let item = pb::NeedsYouItem::default();
    assert!(!is_others(&item), "nobody's is everyone's, so never filed as someone else's");
}

#[test]
fn a_released_device_owns_nothing() {
    assert!(!concern(member("alice").as_ref(), Some(&Person::Released), &names()).unwrap().yours);
    assert!(!concern(Some(&Whose::Owner), Some(&Person::Released), &names()).unwrap().yours);
}

#[test]
fn a_members_item_pushes_that_member_and_only_quietly_moves_the_owners_card() {
    assert_eq!(push_to(member("alice").as_ref()), PushTo { owner_alerts: false, member: Some("alice".into()) });
}

#[test]
fn nobodys_and_the_owners_items_push_the_owner_as_before() {
    assert_eq!(push_to(None), PushTo { owner_alerts: true, member: None }, "nobody's goes to the admin");
    assert_eq!(push_to(Some(&Whose::Owner)), PushTo { owner_alerts: true, member: None });
}

#[test]
fn a_solo_owner_gets_no_people_and_no_concern() {
    let store = farcooler_store::Store::open_in_memory().unwrap();
    assert!(gather(&store, &[]).unwrap().is_none(), "a runner with no members reads no people");
}
