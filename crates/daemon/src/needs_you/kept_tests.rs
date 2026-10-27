use std::time::{Duration, UNIX_EPOCH};

use farcooler_protocol::v1::NeedsYouKind;
use farcooler_store::rulings::{Ruling, RulingState};

use super::*;
use crate::needs_you::assemble;

const DAY: i64 = 24 * 60 * 60 * 1000;
const NOW_MS: i64 = 10_000 * DAY;

fn ws() -> Uuid {
    Uuid::from_u128(7)
}

fn ruling(number: u32, decision: &str, by: &str, at: i64) -> Ruling {
    Ruling {
        id: Uuid::from_u128(u128::from(number)),
        workspace_id: ws(),
        number,
        decision: decision.into(),
        why: String::new(),
        reversal: String::new(),
        tasks: Vec::new(),
        task_keys: Vec::new(),
        theme_id: None,
        state: RulingState::Confirmed,
        note: String::new(),
        actor: "manager".into(),
        created_at: 0,
        settled_by: Some(by.into()),
        settled_at: Some(at),
        reversed_sha: None,
        resource_version: 1,
    }
}

fn names(handle: &str) -> Option<String> {
    Some(match handle {
        "bob" => "Bob Okafor".into(),
        "alice" => "Alice".into(),
        _ => return None,
    })
}

fn inputs(kept: Vec<Kept>, viewer: Option<Person>) -> Inputs {
    let mut inputs = Inputs { kept, viewer, ..Inputs::default() };
    inputs.workspaces.insert(ws(), "Billing".into());
    inputs
}

fn seen(inputs: &Inputs) -> Vec<pb::NeedsYouItem> {
    assemble(inputs, UNIX_EPOCH + Duration::from_millis(NOW_MS as u64))
}

fn bobs_keep(lead: Option<&str>) -> Vec<Kept> {
    group(vec![ruling(84, "Any member may Keep a ruling", "member:bob", NOW_MS - 1000)], |_| lead.map(str::to_string), names)
}

/// One Keep names the member and the ruling, and is the lead's alone: the
/// owner's while the owner leads, a member's once it is handed to them.
#[test]
fn a_keep_by_someone_else_is_the_leads_fyi() {
    let owner = seen(&inputs(bobs_keep(None), Some(Person::Owner)));
    assert_eq!(owner.len(), 1);
    assert_eq!(owner[0].kind(), NeedsYouKind::Fyi);
    assert_eq!(owner[0].question, "Bob Okafor kept R-84: Any member may Keep a ruling");
    assert_eq!(owner[0].workspace_name, "Billing");
    assert!(owner[0].actions.is_empty(), "an FYI asks nothing");

    assert!(seen(&inputs(bobs_keep(None), Some(Person::Member("alice".into())))).is_empty(), "not the lead");
    assert!(seen(&inputs(bobs_keep(None), Some(Person::Released))).is_empty());
    assert!(seen(&inputs(bobs_keep(None), None)).is_empty(), "the runner-wide count has none");

    let led = bobs_keep(Some("alice"));
    assert_eq!(seen(&inputs(led.clone(), Some(Person::Member("alice".into())))).len(), 1);
    assert!(seen(&inputs(led.clone(), Some(Person::Owner))).is_empty(), "the owner isn't the lead now");
    assert!(seen(&inputs(led, Some(Person::Member("carol".into())))).is_empty(), "a third member isn't the lead");
}

/// The lead's own Keep sends nothing, whoever the lead is.
#[test]
fn the_leads_own_keep_is_nobodys_fyi() {
    let own = group(vec![ruling(1, "Ship", "member:alice", NOW_MS)], |_| Some("alice".into()), names);
    assert!(own.is_empty());
    for viewer in [Person::Owner, Person::Member("alice".into()), Person::Member("bob".into())] {
        assert!(seen(&inputs(own.clone(), Some(viewer))).is_empty());
    }
}

/// A Keep All is one line naming the member and how many, not one per ruling,
/// and a long list says how many more.
#[test]
fn a_keep_all_is_one_line() {
    let all: Vec<Ruling> = (1..=6).map(|n| ruling(n, "Ship", "member:bob", NOW_MS - 500)).collect();
    let keeps = group(all, |_| None, names);
    assert_eq!(keeps.len(), 1);
    let items = seen(&inputs(keeps, Some(Person::Owner)));
    assert_eq!(items[0].question, "Bob Okafor kept 6 rulings: R-1, R-2, R-3, R-4 and 2 more");

    let two = group(vec![ruling(3, "A", "member:bob", 5), ruling(4, "B", "member:bob", 5)], |_| None, names);
    assert_eq!(question(&two[0]), "Bob Okafor kept 2 rulings: R-3, R-4");
}

/// Two moments are two lines, two keepers are two, and a handle with no name
/// on file is called by the handle.
#[test]
fn each_keeper_and_moment_is_its_own_line() {
    let keeps = group(
        vec![ruling(1, "A", "member:bob", 5), ruling(2, "B", "member:bob", 9), ruling(3, "C", "member:zed", 9)],
        |_| None,
        names,
    );
    assert_eq!(keeps.len(), 3);
    assert_eq!(keeps[2].name, "zed");
}

/// The owner's Keep and an agent's are not a member's: nothing is made.
#[test]
fn only_a_members_keep_counts() {
    let keeps = group(vec![ruling(1, "A", "user", 5), ruling(2, "B", "manager", 5)], |_| None, names);
    assert!(keeps.is_empty());
}

/// An FYI ages out, ranks after everything else, and a long decision is cut
/// to one row.
#[test]
fn it_ages_out_and_ranks_last() {
    let long = "x".repeat(200);
    let keeps = group(vec![ruling(1, &long, "member:bob", NOW_MS - WINDOW_MS + 1000)], |_| None, names);
    let items = seen(&inputs(keeps.clone(), Some(Person::Owner)));
    assert_eq!(items.len(), 1);
    assert!(items[0].question.ends_with('…') && items[0].question.chars().count() < 110, "{}", items[0].question);
    assert!(items[0].rank >= 6 * TIER_SPAN_FOR_TEST, "below every ask");

    let old = group(vec![ruling(1, "A", "member:bob", NOW_MS - WINDOW_MS - 1)], |_| None, names);
    assert!(seen(&inputs(old, Some(Person::Owner))).is_empty());
}

const TIER_SPAN_FOR_TEST: u32 = 100_000_000;
