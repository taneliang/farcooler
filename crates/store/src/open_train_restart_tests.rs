//! What a restarted daemon is told about trains that left for their checks
//! (ov-845): nothing about a train that is past its checks, and nothing twice.
//! The store is reopened from its file between the steps, as a restart does.

use std::path::PathBuf;

use super::*;
use crate::plan::{Lane, LaneState, LaneUpdate, NewLane};
use crate::trains::{TrainState, TrainUpdate};

fn to(store: &Store, l: &Lane, state: LaneState) -> Lane {
    store.update_lane(l.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager).unwrap()
}

fn ready(store: &Store, main: Uuid, name: &str) -> Lane {
    let l = store.create_lane(main, &NewLane { name: name.into(), ..Default::default() }, &[], None, Actor::Manager).unwrap();
    to(store, &l, LaneState::Building);
    to(store, &l, LaneState::Review);
    to(store, &l, LaneState::Landing)
}

/// A file-backed board whose one ready lane sent train-1 off for its checks.
fn departed(tag: &str) -> (PathBuf, Store, Uuid, Train, Lane) {
    let dir = farcooler_core::scratch::dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");
    let store = Store::open(&path).unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let lane = ready(&store, main, "a");
    let train = store.train_named(main, "train-1").unwrap();
    assert_eq!(train.state, TrainState::Gating);
    assert_eq!(store.departures_untold(main).unwrap().len(), 1, "it is untold until someone is told");
    (path, store, main, train, lane)
}

/// A train nobody moved off `gating` whose lane has landed has nothing left to
/// check, so a daemon that comes up with an orchestrator at last says nothing.
#[test]
fn a_restart_tells_nothing_of_a_train_whose_lanes_have_landed() {
    let (path, store, main, train, lane) = departed("restart-landed");
    to(&store, &lane, LaneState::Landed);
    assert_eq!(store.train(train.id).unwrap().state, TrainState::Gating);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(store.departures_untold(main).unwrap().is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// The same for a train that is still `gating` with its SHA out.
#[test]
fn a_restart_tells_nothing_of_a_gating_train_with_a_sha_out() {
    let (path, store, main, train, _lane) = departed("restart-sha");
    store.set_train(train.id, &TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() }, Actor::Manager).unwrap();
    store.set_train(train.id, &TrainUpdate { state: Some(TrainState::Gating), ..Default::default() }, Actor::Manager).unwrap();
    assert_eq!(store.train(train.id).unwrap().state, TrainState::Gating);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(store.departures_untold(main).unwrap().is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// A train that landed or was dropped is never listed, restart or not.
#[test]
fn a_restart_tells_nothing_of_a_landed_or_dropped_train() {
    let (path, store, main, train, _lane) = departed("restart-settled");
    store.set_train(train.id, &TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() }, Actor::Manager).unwrap();
    store.set_train(train.id, &TrainUpdate { state: Some(TrainState::Landed), ..Default::default() }, Actor::Manager).unwrap();
    let second = ready(&store, main, "b");
    let next = store.train_named(main, "train-2").unwrap();
    assert_eq!(next.state, TrainState::Gating, "{second:?}");
    store.set_train(next.id, &TrainUpdate { state: Some(TrainState::Dropped), ..Default::default() }, Actor::Manager).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(store.departures_untold(main).unwrap().is_empty());
    assert!(store.workspaces_with_gating_trains().unwrap().is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// The told mark is in the file: a reopened store won't mark or list the same
/// departure again, and the next train that leaves is still new.
#[test]
fn a_departure_is_told_at_most_once_across_restarts() {
    let (path, store, main, train, _lane) = departed("restart-once");
    let told = store.departures_untold(main).unwrap();
    assert!(store.record_departure_told(&told[0]).unwrap());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(store.departures_untold(main).unwrap().is_empty(), "the reopened store remembers");
    assert!(!store.record_departure_told(&told[0]).unwrap(), "and refuses a second mark");
    // Once the train ahead lands, the next one leaves, and that is news.
    store.set_train(train.id, &TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() }, Actor::Manager).unwrap();
    store.set_train(train.id, &TrainUpdate { state: Some(TrainState::Landed), ..Default::default() }, Actor::Manager).unwrap();
    ready(&store, main, "b");
    let again = store.departures_untold(main).unwrap();
    assert_eq!(again.len(), 1, "{again:?}");
    assert_eq!(again[0].lanes, vec!["b".to_string()]);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}
