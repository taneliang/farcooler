//! A train's phase, row by row (ov-503), the hold, and the history the moves
//! write. The pure rows are the table in `train_phase.rs`; the store rows read
//! a real board, with times moved by hand since the plan read uses the clock.

use rusqlite::params;

use super::*;
use crate::plan::{Lane, LaneUpdate, NewLane};
use crate::trains::{NewTrain, TrainUpdate};

const MIN: i64 = 60_000;
const NOW: i64 = 10_000 * MIN;

fn on(state: LaneState, since_min_ago: i64) -> LaneOn {
    LaneOn { id: Uuid::now_v7(), state, state_since: NOW - since_min_ago * MIN }
}

fn at(state: TrainState, since_min_ago: i64, lanes: &[LaneOn]) -> Derived {
    derive(&PhaseInput {
        state,
        state_since: NOW - since_min_ago * MIN,
        agent_open: false,
        agent_started_at: None,
        last_lane_on: None,
        lanes,
        now: NOW,
    })
}

#[test]
fn every_row_of_the_phase_table() {
    let ready = on(LaneState::Landing, 5);
    let building = on(LaneState::Building, 5);
    let fresh = |state, lanes: &[LaneOn]| at(state, 5, lanes).phase;
    // Waiting for lanes: a lane on it isn't ready.
    assert_eq!(fresh(TrainState::Integrating, &[ready, building]), TrainPhase::WaitingForLanes);
    // Integrating: every lane ready, no SHA.
    assert_eq!(fresh(TrainState::Integrating, &[ready]), TrainPhase::Integrating);
    assert_eq!(fresh(TrainState::Gating, &[ready]), TrainPhase::GatingLocally);
    assert_eq!(fresh(TrainState::Pushed, &[ready]), TrainPhase::WaitingForCi);
    assert_eq!(fresh(TrainState::Red, &[ready]), TrainPhase::CiFailed);
    assert_eq!(fresh(TrainState::Green, &[ready]), TrainPhase::ReadyToLand);
    assert_eq!(fresh(TrainState::Landed, &[]), TrainPhase::Landed);
    assert_eq!(fresh(TrainState::Dropped, &[]), TrainPhase::Dropped);
    // A closed lane never blocks: a train that carried a landed lane waits on no one.
    let landed = on(LaneState::Landed, 5);
    assert_eq!(fresh(TrainState::Integrating, &[ready, landed]), TrainPhase::Integrating);
}

#[test]
fn stalled_after_two_hours_without_an_open_agent_before_ci() {
    let ready = on(LaneState::Landing, 600);
    let building = on(LaneState::Building, 600);
    for state in [TrainState::Integrating, TrainState::Gating] {
        assert_eq!(at(state, 121, &[ready]).phase, TrainPhase::Stalled, "{state:?}");
        assert_eq!(at(state, 119, &[ready]).phase, if state == TrainState::Gating { TrainPhase::GatingLocally } else { TrainPhase::Integrating });
    }
    assert_eq!(at(TrainState::Integrating, 300, &[ready, building]).phase, TrainPhase::Stalled);
    // From Waiting for CI on, a long wait is CI's, not the train's.
    for state in [TrainState::Pushed, TrainState::Red, TrainState::Green] {
        assert!(!matches!(at(state, 600, &[ready]).phase, TrainPhase::Stalled), "{state:?}");
    }
    // An open agent keeps it from stalling; a finished one doesn't.
    let with_agent = |open| {
        derive(&PhaseInput {
            state: TrainState::Integrating,
            state_since: NOW - 300 * MIN,
            agent_open: open,
            agent_started_at: Some(NOW - 10 * MIN),
            last_lane_on: None,
            lanes: &[ready],
            now: NOW,
        })
    };
    assert_eq!(with_agent(true).phase, TrainPhase::Integrating);
    assert_eq!(with_agent(true).since, NOW - 10 * MIN, "integrating since its agent started");
    assert_eq!(with_agent(false).phase, TrainPhase::Stalled);
}

#[test]
fn held_after_forty_five_minutes_with_one_lane_ready_and_another_not() {
    let ready = on(LaneState::Landing, 600);
    let blocker = on(LaneState::Building, 46);
    let held = at(TrainState::Integrating, 46, &[ready, blocker]);
    assert_eq!(held.phase, TrainPhase::WaitingForLanes);
    assert_eq!(held.held_by, vec![blocker.id]);
    assert_eq!(held.since, NOW - 46 * MIN);
    // Not yet at 44 minutes.
    let soon = on(LaneState::Building, 44);
    assert!(at(TrainState::Integrating, 44, &[ready, soon]).held_by.is_empty());
    // Nothing ready, nothing to release: waiting, not held.
    let blocked = on(LaneState::Building, 600);
    assert!(at(TrainState::Integrating, 600, &[blocked]).held_by.is_empty());
    // A train that has pushed is nobody's to hold.
    assert!(at(TrainState::Pushed, 600, &[ready, blocker]).held_by.is_empty());
    // Both blocking lanes are named.
    let other = on(LaneState::Fixing, 60);
    assert_eq!(at(TrainState::Integrating, 60, &[ready, blocker, other]).held_by, vec![blocker.id, other.id]);
}

#[test]
fn the_wait_runs_from_the_last_lane_put_on_the_train() {
    let ready = on(LaneState::Landing, 600);
    let blocker = on(LaneState::Building, 600);
    let derived = |last_on_min_ago: i64| {
        derive(&PhaseInput {
            state: TrainState::Integrating,
            state_since: NOW - 600 * MIN,
            agent_open: true,
            agent_started_at: Some(NOW - 600 * MIN),
            last_lane_on: Some(NOW - last_on_min_ago * MIN),
            lanes: &[ready, blocker],
            now: NOW,
        })
    };
    assert!(!derived(50).held_by.is_empty());
    assert!(derived(10).held_by.is_empty(), "a lane just put on starts the wait over");
}

fn board() -> (Store, Uuid) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    (store, main)
}

fn lane(store: &Store, main: Uuid, name: &str, state: LaneState) -> Lane {
    let mut l = store.create_lane(main, &NewLane { name: name.into(), ..Default::default() }, &[], None, Actor::Manager).unwrap();
    let road = [LaneState::Building, LaneState::Review, LaneState::Landing];
    let stops = match state {
        LaneState::Queued => 0,
        LaneState::Building => 1,
        LaneState::Review => 2,
        _ => 3,
    };
    for s in &road[..stops] {
        l = store.update_lane(l.id, &LaneUpdate { state: Some(*s), ..Default::default() }, Actor::Manager).unwrap();
    }
    l
}

fn send_back(store: &Store, l: &Lane, state: LaneState) {
    store.update_lane(l.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager).unwrap();
}

/// A fixed lane, back through review to ready.
fn fixed(store: &Store, l: &Lane) {
    send_back(store, l, LaneState::Review);
    send_back(store, l, LaneState::Landing);
}

fn start(store: &Store, main: Uuid, name: &str, lanes: &[&Lane]) -> Train {
    let ids: Vec<Uuid> = lanes.iter().map(|l| l.id).collect();
    store.start_train(main, &NewTrain { name: name.into(), ..Default::default() }, &ids, Actor::Manager).unwrap()
}

/// The train's row and its lanes' clocks, all moved `minutes` into the past.
fn age(store: &Store, minutes: i64) {
    let ms = now_millis() - minutes * MIN;
    let conn = store.conn();
    conn.execute("UPDATE board_trains SET state_since = ?1, created_at = ?1", params![ms]).unwrap();
    conn.execute("UPDATE lanes SET state_since = ?1", params![ms]).unwrap();
    conn.execute("UPDATE train_events SET at = ?1", params![ms]).unwrap();
}

fn view(store: &Store, main: Uuid, name: &str) -> crate::plan_read::TrainView {
    store.plan(main, 0).unwrap().trains.into_iter().find(|t| t.train.name == name).unwrap()
}

#[test]
fn the_plan_read_derives_the_phase_of_a_real_train() {
    let (store, main) = board();
    let a = lane(&store, main, "a", LaneState::Landing);
    let b = lane(&store, main, "b", LaneState::Landing);
    let train = start(&store, main, "train-1", &[&a, &b]);
    assert_eq!(view(&store, main, "train-1").phase, TrainPhase::Integrating);

    send_back(&store, &b, LaneState::Fixing);
    let waiting = view(&store, main, "train-1");
    assert_eq!(waiting.phase, TrainPhase::WaitingForLanes);
    assert!(waiting.held_by.is_empty(), "not for 45 minutes");

    age(&store, 50);
    // The agent is recorded, so only the hold shows.
    let held = view(&store, main, "train-1");
    assert_eq!(held.held_by, vec![b.id]);
    assert_eq!(held.train.id, train.id);

    // Ready again, and it is no longer held.
    fixed(&store, &b);
    assert!(view(&store, main, "train-1").held_by.is_empty());
}

#[test]
fn a_move_writes_events_on_both_trains() {
    let (store, main) = board();
    let a = lane(&store, main, "a", LaneState::Landing);
    let b = lane(&store, main, "b", LaneState::Landing);
    let first = start(&store, main, "train-1", &[&a, &b]);
    let next = start(&store, main, "train-2", &[]);
    // Moving `b` is one write on the destination.
    store.set_train(next.id, &TrainUpdate { add_lanes: vec![b.id], ..Default::default() }, Actor::Manager).unwrap();
    let kinds = |train: Uuid| -> Vec<(String, Option<Uuid>)> {
        store.train_events(train).unwrap().into_iter().map(|e| (e.kind, e.lane_id)).collect()
    };
    assert_eq!(
        kinds(first.id),
        vec![("state".into(), None), ("lane_on".into(), Some(a.id)), ("lane_on".into(), Some(b.id)), ("lane_off".into(), Some(b.id))]
    );
    assert_eq!(kinds(next.id), vec![("state".into(), None), ("lane_on".into(), Some(b.id))]);
    let off = store.train_events(first.id).unwrap().into_iter().find(|e| e.kind == "lane_off").unwrap();
    assert_eq!(off.extra["to"], "train-2");
    let on = store.train_events(next.id).unwrap().into_iter().find(|e| e.kind == "lane_on").unwrap();
    assert_eq!(on.extra["from"], "train-1");
    // The lane's own history says where it was and went.
    assert_eq!(store.lane_train_events(b.id).unwrap().len(), 3);
    assert_eq!(view(&store, main, "train-1").lanes, vec![a.id]);

    // Back to Waiting: a lane taken off with nothing to go to.
    store.set_train(first.id, &TrainUpdate { remove_lanes: vec![a.id], ..Default::default() }, Actor::Manager).unwrap();
    let last = store.train_events(first.id).unwrap().pop().unwrap();
    assert_eq!((last.kind.as_str(), last.lane_id), ("lane_off", Some(a.id)));
    assert!(last.extra["to"].is_null());
}

#[test]
fn a_state_change_and_a_drop_write_events() {
    let (store, main) = board();
    let a = lane(&store, main, "a", LaneState::Landing);
    let train = start(&store, main, "train-1", &[&a]);
    store.set_train(train.id, &TrainUpdate { state: Some(TrainState::Gating), ..Default::default() }, Actor::Manager).unwrap();
    store.set_train(train.id, &TrainUpdate { state: Some(TrainState::Dropped), ..Default::default() }, Actor::Manager).unwrap();
    let events = store.train_events(train.id).unwrap();
    let words: Vec<_> = events.iter().map(|e| (e.kind.as_str(), e.extra["to"].as_str().map(str::to_string))).collect();
    assert_eq!(
        words,
        vec![
            ("state", Some("integrating".to_string())),
            ("lane_on", None),
            ("state", Some("gating".to_string())),
            ("state", Some("dropped".to_string())),
            ("lane_off", None),
        ]
    );
}

#[test]
fn a_hold_is_told_once_and_a_new_hold_is_told_again() {
    let (store, main) = board();
    let a = lane(&store, main, "a", LaneState::Landing);
    let b = lane(&store, main, "b", LaneState::Landing);
    start(&store, main, "train-1", &[&a, &b]);
    assert!(store.holds_untold(main).unwrap().is_empty(), "nothing is held yet");
    send_back(&store, &b, LaneState::Fixing);
    age(&store, 50);
    let held = store.holds_untold(main).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].names, vec!["b".to_string()]);
    assert!(store.record_hold_told(&held[0]).unwrap());
    assert!(!store.record_hold_told(&held[0]).unwrap(), "the same wait is recorded once");
    assert!(store.holds_untold(main).unwrap().is_empty(), "and then not listed");

    // The lane comes back and goes wrong again: a new wait, a new notice.
    fixed(&store, &b);
    assert!(store.holds_untold(main).unwrap().is_empty());
    send_back(&store, &b, LaneState::Fixing);
    store.conn().execute("UPDATE lanes SET state_since = ?1", params![now_millis() - 50 * MIN]).unwrap();
    let again = store.holds_untold(main).unwrap();
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].since, held[0].since);
}
