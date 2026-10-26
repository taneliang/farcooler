//! `move_theme` (ov-755): one call renumbers the ranking 1..N in one
//! transaction and records who moved the theme.

use super::*;

use crate::models::{Actor, Handle};

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn themes(names: &[&str]) -> (Store, Vec<Uuid>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let ids = names
        .iter()
        .map(|n| store.create_theme(main, &NewTheme { name: n.to_string(), outcome: String::new() }, &[], Actor::Manager).unwrap().id)
        .collect();
    (store, ids)
}

fn places(store: &Store, ids: &[Uuid]) -> Vec<i64> {
    ids.iter().map(|id| store.board_theme(*id).unwrap().ordinal).collect()
}

/// One call yields exactly 1..N, whatever the ordinals were: ties and gaps
/// (a dropped theme's number left behind) are made whole, and a dropped
/// theme takes no place.
#[test]
fn one_move_leaves_the_ranking_one_to_n() {
    let (store, ids) = themes(&["Alpha", "Beta", "Gamma", "Delta"]);
    store.update_theme(ids[1], &ThemeUpdate { state: Some(ThemeState::Dropped), ..Default::default() }, Actor::Manager).unwrap();
    // A tie and a gap, as a pair of interleaved writes would leave.
    store.update_theme(ids[2], &ThemeUpdate { ordinal: Some(1), ..Default::default() }, Actor::Manager).unwrap();
    store.update_theme(ids[3], &ThemeUpdate { ordinal: Some(9), ..Default::default() }, Actor::Manager).unwrap();
    store.move_theme(ids[3], 1, Actor::User).unwrap();
    // Delta first, then Alpha and Gamma (tied at 1, the older first); Beta is out.
    assert_eq!(places(&store, &[ids[3], ids[0], ids[2]]), [1, 2, 3]);
    store.move_theme(ids[3], 3, Actor::User).unwrap();
    assert_eq!(places(&store, &[ids[0], ids[2], ids[3]]), [1, 2, 3]);
    assert_eq!(refused(store.move_theme(ids[0], 4, Actor::User)), "place");
    assert_eq!(refused(store.move_theme(ids[0], 0, Actor::User)), "place");
    assert_eq!(refused(store.move_theme(ids[1], 1, Actor::User)), "place", "a dropped theme has no place");
}

/// One `rank` event names who moved it, a rep's move as its member's; a
/// move to where it already is says nothing.
#[test]
fn the_move_is_recorded_under_who_made_it() {
    let (store, ids) = themes(&["Alpha", "Beta"]);
    let bob = Actor::Member { handle: Handle::new("bob").unwrap() };
    store.move_theme(ids[1], 1, bob).unwrap();
    let events = store.plan_events(Subject::Theme(ids[1]), 0).unwrap();
    let ranked: Vec<_> = events.iter().filter(|e| e.kind == "rank").collect();
    assert_eq!(ranked.len(), 1);
    assert_eq!((ranked[0].actor.as_str(), ranked[0].body.as_str()), ("member:bob", "Moved to next up."));
    store.move_theme(ids[1], 1, bob).unwrap();
    assert_eq!(store.plan_events(Subject::Theme(ids[1]), 0).unwrap().iter().filter(|e| e.kind == "rank").count(), 1);
}
