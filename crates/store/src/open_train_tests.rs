//! The collecting train (ov-520, ov-523): a lane that passes review joins it, a
//! lane that leaves landing comes off, it departs by itself when it can and
//! keeps trying, a lane taken off by hand stays off, and the board always has
//! exactly one.
//!
//! A ready lane departs at once when nothing is ahead, so most tests first put
//! a train ahead: `hold` starts one by hand, which stays integrating.

use rusqlite::params;

use super::*;
use crate::board_ci::CiRead;
use crate::plan::{Lane, LaneState, LaneUpdate, NewLane, Subject};
use crate::train_phase::{TrainPhase, last_lane_on};
use crate::trains::{NewTrain, TrainAgentRecord, TrainState, TrainUpdate};

fn board() -> (Store, Uuid) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    (store, main)
}

fn to(store: &Store, l: &Lane, state: LaneState) -> Lane {
    store.update_lane(l.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager).unwrap()
}

/// A lane through review to landing: the write that makes it ready.
fn ready(store: &Store, main: Uuid, name: &str) -> Lane {
    let l = store.create_lane(main, &NewLane { name: name.into(), ..Default::default() }, &[], None, Actor::Manager).unwrap();
    to(store, &l, LaneState::Building);
    to(store, &l, LaneState::Review);
    to(store, &l, LaneState::Landing)
}

/// A train started by hand with no lanes: ahead of the collecting train, and
/// never departing by itself.
fn hold(store: &Store, main: Uuid, name: &str) -> Train {
    store.start_train(main, &NewTrain { name: name.into(), ..Default::default() }, &[], Actor::Manager).unwrap()
}

fn on_train(store: &Store, l: &Lane) -> Option<String> {
    store.lane(l.id).unwrap().train
}

fn collecting_count(store: &Store, main: Uuid) -> i64 {
    store
        .conn()
        .query_row(
            "SELECT count(*) FROM board_trains WHERE workspace_id = ?1 AND state = 'integrating' AND collecting = 1",
            params![uuid_blob(main)],
            |r| r.get(0),
        )
        .unwrap()
}

fn set(store: &Store, train: &Train, update: TrainUpdate) -> Train {
    store.set_train(train.id, &update, Actor::Manager).unwrap()
}

fn state(store: &Store, train: &Train, to: TrainState) -> Train {
    set(store, train, TrainUpdate { state: Some(to), ..Default::default() })
}

fn named(store: &Store, main: Uuid, name: &str) -> Train {
    store.train_named(main, name).unwrap()
}

fn phase(store: &Store, main: Uuid, t: &Train) -> Option<TrainPhase> {
    store.plan(main, 0).unwrap().trains.into_iter().find(|v| v.train.id == t.id).map(|v| v.phase)
}

fn read(subject: &str, status: CiStatus) -> CiRead {
    CiRead {
        subject: subject.into(),
        sha: "1a1b3275f00d".into(),
        status,
        url: String::new(),
        jobs: Vec::new(),
        fetched_at: 0,
        changed_at: 0,
        asked_at: 0,
    }
}

#[test]
fn a_lane_that_passes_review_joins_the_collecting_train_at_once() {
    let (store, main) = board();
    assert!(store.plan(main, 0).unwrap().trains.is_empty(), "a board that has never had a lane ready has no train");
    hold(&store, main, "ahead");
    let a = ready(&store, main, "a");
    assert_eq!(on_train(&store, &a).as_deref(), Some("train-1"), "the collecting train was made for it");
    let b = ready(&store, main, "b");
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-1"), "and the next lane rides the same one");
    let open = named(&store, main, "train-1");
    assert_eq!((open.state, open.collecting, collecting_count(&store, main)), (TrainState::Integrating, true, 1));
    let kinds: Vec<_> = store.train_events(open.id).unwrap().into_iter().map(|e| (e.kind, e.lane_id)).collect();
    assert_eq!(kinds, vec![("state".to_string(), None), ("lane_on".into(), Some(a.id)), ("lane_on".into(), Some(b.id))]);
    assert_eq!(phase(&store, main, &open), Some(TrainPhase::WaitingForPreviousTrain));
    // A lane that has not passed review is on no train.
    let c = store.create_lane(main, &NewLane { name: "c".into(), ..Default::default() }, &[], None, Actor::Manager).unwrap();
    to(&store, &c, LaneState::Building);
    to(&store, &c, LaneState::Review);
    assert_eq!(on_train(&store, &c), None);
}

#[test]
fn an_empty_collecting_train_is_not_drawn() {
    let (store, main) = board();
    hold(&store, main, "ahead");
    let a = ready(&store, main, "a");
    let open = named(&store, main, "train-1");
    assert!(phase(&store, main, &open).is_some());
    set(&store, &open, TrainUpdate { remove_lanes: vec![a.id], ..Default::default() });
    assert_eq!(phase(&store, main, &open), None, "the plan read leaves the empty standing train out");
    assert_eq!(collecting_count(&store, main), 1, "but it is still there to collect");
}

#[test]
fn a_lane_that_leaves_landing_comes_off_in_the_same_write() {
    let (store, main) = board();
    hold(&store, main, "ahead");
    let (a, b, c, d) = (ready(&store, main, "a"), ready(&store, main, "b"), ready(&store, main, "c"), ready(&store, main, "d"));
    let train = named(&store, main, "train-1");
    assert_eq!(to(&store, &a, LaneState::Fixing).train, None, "the write that moved it also took it off");
    assert_eq!(to(&store, &b, LaneState::Fixing).train, None);
    assert_eq!(to(&store, &c, LaneState::Dropped).train, None);
    let view = store.plan(main, 0).unwrap().trains.into_iter().find(|t| t.train.id == train.id).unwrap();
    assert_eq!(view.lanes, vec![d.id], "the train goes on with the lane still ready");
    let history: Vec<_> = store.train_events(train.id).unwrap().into_iter().filter(|e| e.kind == "lane_off").map(|e| e.lane_id).collect();
    assert_eq!(history, vec![Some(a.id), Some(b.id), Some(c.id)]);
    let said: Vec<String> = store.plan_events(Subject::Lane(a.id), 0).unwrap().into_iter().filter(|e| e.kind == "train").map(|e| e.body).collect();
    assert_eq!(said, vec!["Taken off Train 1: it left landing.".to_string()]);
    // Fixed and reviewed again, it rides the collecting train again.
    to(&store, &a, LaneState::Review);
    assert_eq!(to(&store, &a, LaneState::Landing).train.as_deref(), Some("train-1"));
}

#[test]
fn a_lane_comes_off_a_train_that_is_checking_but_stays_on_one_that_is_pushed() {
    let (store, main) = board();
    let a = ready(&store, main, "a");
    let first = named(&store, main, "train-1");
    assert_eq!(first.state, TrainState::Gating, "nothing was ahead, so it left at once");
    let b = ready(&store, main, "b");
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-2"));
    // Checking: no SHA is out, so the lane comes off, as `train_moving` never allowed by hand.
    assert_eq!(to(&store, &b, LaneState::Fixing).train, None);
    // Pushed: the push carries its commits, so it stays and says so.
    set(&store, &first, TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() });
    let after = to(&store, &a, LaneState::Fixing);
    assert_eq!(after.train.as_deref(), Some("train-1"));
    let said: Vec<String> = store.plan_events(Subject::Lane(a.id), 0).unwrap().into_iter().filter(|e| e.kind == "train").map(|e| e.body).collect();
    assert_eq!(said, vec!["Stays on Train 1: its commits are in the train's push.".to_string()]);
}

#[test]
fn a_lane_on_a_train_that_landed_stays_on_it_as_its_record() {
    let (store, main) = board();
    let a = ready(&store, main, "a");
    let train = named(&store, main, "train-1");
    set(&store, &train, TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() });
    state(&store, &train, TrainState::Landed);
    assert_eq!(store.lane(a.id).unwrap().state, LaneState::Landed);
    assert_eq!(on_train(&store, &a).as_deref(), Some("train-1"));
}

#[test]
fn with_nothing_ahead_a_ready_lane_sends_the_collecting_train_off_at_once() {
    let (store, main) = board();
    let a = ready(&store, main, "a");
    let train = named(&store, main, "train-1");
    assert_eq!((train.state, train.collecting), (TrainState::Gating, false));
    assert_eq!(on_train(&store, &a).as_deref(), Some("train-1"));
    let behind = named(&store, main, "train-2");
    assert_eq!((behind.state, behind.collecting, collecting_count(&store, main)), (TrainState::Integrating, true, 1));
    assert!(store.train_events(train.id).unwrap().iter().any(|e| e.extra["departed"] == true));
    // The next lane waits for it.
    let b = ready(&store, main, "b");
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-2"));
    assert_eq!(phase(&store, main, &behind), Some(TrainPhase::WaitingForPreviousTrain));
}

#[test]
fn the_collecting_train_departs_when_the_train_ahead_lands() {
    let (store, main) = board();
    let a = ready(&store, main, "a");
    let ahead = named(&store, main, "train-1");
    let b = ready(&store, main, "b");
    let open = named(&store, main, "train-2");
    set(&store, &ahead, TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() });
    state(&store, &ahead, TrainState::Green);
    assert_eq!(store.train(open.id).unwrap().state, TrainState::Integrating, "still waiting while the train ahead isn't on main");
    state(&store, &ahead, TrainState::Landed);
    assert_eq!(store.lane(a.id).unwrap().state, LaneState::Landed);
    let after = store.train(open.id).unwrap();
    assert_eq!((after.state, after.collecting), (TrainState::Gating, false), "the collecting train starts its checks");
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-2"));
    assert_eq!(phase(&store, main, &after), Some(TrainPhase::GatingLocally));
    let behind = named(&store, main, "train-3");
    assert_eq!((behind.state, collecting_count(&store, main)), (TrainState::Integrating, 1));
}

#[test]
fn departure_retries_when_main_turns_green_a_lane_comes_late_or_a_fix_main_train_lands() {
    // (a) Main is red when the lane is ready, then green.
    let (store, main) = board();
    store.record_ci(main, &read("main", CiStatus::Failed)).unwrap();
    let a = ready(&store, main, "a");
    let open = named(&store, main, "train-1");
    assert_eq!(open.state, TrainState::Integrating, "main is red, so it waits");
    assert_eq!(phase(&store, main, &open), Some(TrainPhase::WaitingForMain));
    let write = store.record_ci(main, &read("main", CiStatus::Passed)).unwrap();
    assert!(write.changed);
    assert_eq!(store.train(open.id).unwrap().state, TrainState::Gating, "main turned green");
    assert_eq!(on_train(&store, &a).as_deref(), Some("train-1"));

    // (b) The train ahead was dropped with no lane ready; a lane that comes later departs it.
    let (store, main) = board();
    let ahead = hold(&store, main, "ahead");
    let open = named(&store, main, "train-1");
    state(&store, &ahead, TrainState::Dropped);
    assert_eq!(store.train(open.id).unwrap().state, TrainState::Integrating, "no lane to carry");
    ready(&store, main, "late");
    assert_eq!(store.train(open.id).unwrap().state, TrainState::Gating);

    // (c) A fix-main train lands while main still reads red; the next read of main departs it.
    let (store, main) = board();
    ready(&store, main, "fix-main");
    let fix = named(&store, main, "train-1");
    let b = ready(&store, main, "b");
    let open = named(&store, main, "train-2");
    store.record_ci(main, &read("main", CiStatus::Failed)).unwrap();
    set(&store, &fix, TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() });
    state(&store, &fix, TrainState::Landed);
    assert_eq!(store.train(open.id).unwrap().state, TrainState::Integrating, "main still reads red");
    store.record_ci(main, &read("main", CiStatus::Passed)).unwrap();
    assert_eq!(store.train(open.id).unwrap().state, TrainState::Gating);
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-2"));

    // (d) The runner's scan retries it too.
    let (store, main) = board();
    store.record_ci(main, &read("main", CiStatus::Failed)).unwrap();
    ready(&store, main, "a");
    let open = named(&store, main, "train-1");
    store.conn().execute("UPDATE board_ci SET status = 'passed'", []).unwrap();
    assert_eq!(store.workspaces_to_collect().unwrap(), vec![main]);
    assert_eq!(store.collect_ready_lanes(main, Actor::Runner).unwrap().departed, Some(open.id));
}

#[test]
fn main_running_or_unread_does_not_hold_the_departure() {
    let (store, main) = board();
    store.record_ci(main, &read("main", CiStatus::Running)).unwrap();
    ready(&store, main, "a");
    assert_eq!(named(&store, main, "train-1").state, TrainState::Gating, "main is running");
    let (store, main) = board();
    assert!(store.ci_read(main, "main").unwrap().is_none(), "main was never read");
    ready(&store, main, "a");
    assert_eq!(named(&store, main, "train-1").state, TrainState::Gating, "and unread");
}

#[test]
fn a_train_that_waits_for_a_train_still_ahead_or_for_a_lane_does_not_depart() {
    let (store, main) = board();
    ready(&store, main, "a");
    let first = named(&store, main, "train-1");
    ready(&store, main, "b");
    let second = named(&store, main, "train-2");
    state(&store, &second, TrainState::Gating);
    ready(&store, main, "c");
    let third = named(&store, main, "train-3");
    state(&store, &first, TrainState::Dropped);
    assert_eq!(store.train(third.id).unwrap().state, TrainState::Integrating, "train-2 is still ahead");
    state(&store, &second, TrainState::Dropped);
    assert_eq!(store.train(third.id).unwrap().state, TrainState::Gating);
    // Nothing on it is ready.
    let (store, main) = board();
    ready(&store, main, "a");
    let first = named(&store, main, "train-1");
    let empty = named(&store, main, "train-2");
    state(&store, &first, TrainState::Dropped);
    assert_eq!(store.train(empty.id).unwrap().state, TrainState::Integrating, "no lane to carry");
}

#[test]
fn a_train_started_by_hand_never_departs_on_its_own() {
    let (store, main) = board();
    let g = hold(&store, main, "g");
    let x = ready(&store, main, "x");
    let mine = store.start_train(main, &NewTrain { name: "integ-5".into(), ..Default::default() }, &[x.id], Actor::Manager).unwrap();
    assert!(!mine.collecting);
    state(&store, &g, TrainState::Landed);
    assert_eq!(store.train(mine.id).unwrap().state, TrainState::Integrating, "still the coordinator's to start");
    assert_eq!(on_train(&store, &x).as_deref(), Some("integ-5"));
    // An integrating train landed by hand lets the collecting one go (the guard no longer needs the train to be past integrating).
    let (store, main) = board();
    let g = hold(&store, main, "g");
    ready(&store, main, "a");
    let open = named(&store, main, "train-1");
    state(&store, &g, TrainState::Landed);
    assert_eq!(store.train(open.id).unwrap().state, TrainState::Gating);
}

#[test]
fn giving_a_train_an_agent_claims_it() {
    let (store, main) = board();
    hold(&store, main, "ahead");
    ready(&store, main, "a");
    let open = named(&store, main, "train-1");
    let agent = TrainAgentRecord { harness: "claude".into(), agent_id: "a1".into(), model: None, ended: false };
    let claimed = set(&store, &open, TrainUpdate { agent: Some(agent), ..Default::default() });
    assert!(!claimed.collecting);
    assert_eq!(collecting_count(&store, main), 1, "a new collecting train opened");
    let b = ready(&store, main, "b");
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-2"), "new lanes ride the new one");
}

#[test]
fn a_lane_taken_off_by_hand_stays_off_until_it_comes_back_through_review() {
    let (store, main) = board();
    hold(&store, main, "ahead");
    let a = ready(&store, main, "a");
    let b = ready(&store, main, "b");
    let open = named(&store, main, "train-1");
    set(&store, &open, TrainUpdate { remove_lanes: vec![a.id], ..Default::default() });
    assert_eq!(on_train(&store, &a), None);
    assert!(store.workspaces_to_collect().unwrap().contains(&main), "b is still on the train, but the scan has nothing to move for a");
    assert!(store.collect_ready_lanes(main, Actor::Runner).unwrap().joined.is_empty(), "the scan leaves a off");
    assert_eq!(on_train(&store, &a), None);
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-1"));
    // Back through fixing and review, it joins again.
    to(&store, &a, LaneState::Fixing);
    to(&store, &a, LaneState::Review);
    assert_eq!(to(&store, &a, LaneState::Landing).train.as_deref(), Some("train-1"));
    // Put back by hand, it is on again, and off again clears nothing it shouldn't.
    set(&store, &open, TrainUpdate { remove_lanes: vec![b.id], ..Default::default() });
    set(&store, &open, TrainUpdate { add_lanes: vec![b.id], ..Default::default() });
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-1"));
}

#[test]
fn a_train_started_by_hand_carries_exactly_the_lanes_listed() {
    let (store, main) = board();
    hold(&store, main, "ahead");
    let (a, b, c) = (ready(&store, main, "a"), ready(&store, main, "b"), ready(&store, main, "c"));
    let open = named(&store, main, "train-1");
    let new = NewTrain { name: "integ-9".into(), base: "origin/main".into(), title: "Phones".into(), ..Default::default() };
    let mine = store.start_train(main, &new, &[a.id], Actor::Manager).unwrap();
    assert_eq!(mine.id, open.id, "the same train, not a second");
    assert_eq!((mine.name.as_str(), mine.base.as_str(), mine.state, mine.collecting), ("integ-9", "origin/main", TrainState::Integrating, false));
    assert_eq!(on_train(&store, &a).as_deref(), Some("integ-9"));
    // The lanes it was not asked to carry never ride along: they wait on a fresh collecting train.
    let fresh = named(&store, main, "train-10");
    assert!(fresh.collecting);
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-10"));
    assert_eq!(on_train(&store, &c).as_deref(), Some("train-10"));
    assert_eq!(collecting_count(&store, main), 1);
    // Landing it lands a, and only a.
    set(&store, &mine, TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() });
    state(&store, &mine, TrainState::Landed);
    let states: Vec<_> = [&a, &b, &c].iter().map(|l| store.lane(l.id).unwrap().state).collect();
    assert_eq!(states, vec![LaneState::Landed, LaneState::Landing, LaneState::Landing]);
    let moved: Vec<_> = store.train_events(open.id).unwrap().into_iter().filter(|e| e.kind == "lane_off").map(|e| (e.lane_id, e.extra["to"].clone())).collect();
    assert_eq!(moved, vec![(Some(b.id), serde_json::json!("train-10")), (Some(c.id), serde_json::json!("train-10"))]);
}

#[test]
fn a_train_moved_back_to_integrating_by_hand_is_claimed_and_never_leaves_again() {
    let (store, main) = board();
    let a = ready(&store, main, "a");
    let gating = named(&store, main, "train-1");
    let behind = named(&store, main, "train-2");
    assert_eq!(gating.state, TrainState::Gating);
    let back = state(&store, &gating, TrainState::Integrating);
    assert_eq!((back.state, back.collecting), (TrainState::Integrating, false), "claimed, not collecting");
    // Its lane is ready and nothing else is ahead, but it never leaves on its own,
    // and neither the scan nor a later lane changes that.
    assert_eq!(store.collect_ready_lanes(main, Actor::Runner).unwrap().departed, None);
    let b = ready(&store, main, "b");
    assert_eq!(store.train(gating.id).unwrap().state, TrainState::Integrating);
    assert_eq!(on_train(&store, &a).as_deref(), Some("train-1"));
    // The collecting train waits behind it, and new lanes ride that one: still one collecting.
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-2"));
    assert_eq!(store.train(behind.id).unwrap().state, TrainState::Integrating);
    assert_eq!(collecting_count(&store, main), 1);
    assert_eq!(phase(&store, main, &store.train(behind.id).unwrap()), Some(TrainPhase::WaitingForPreviousTrain));
    // Its own checks are the coordinator's: gate it, push it, land it.
    state(&store, &back, TrainState::Gating);
    set(&store, &back, TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() });
    state(&store, &back, TrainState::Landed);
    assert_eq!(store.train(behind.id).unwrap().state, TrainState::Gating, "and then the collecting train goes");
}

#[test]
fn a_train_gated_by_hand_that_loses_its_lane_is_not_dropped() {
    let (store, main) = board();
    hold(&store, main, "ahead");
    let a = ready(&store, main, "a");
    let mine = store.start_train(main, &NewTrain { name: "integ-6".into(), ..Default::default() }, &[a.id], Actor::Manager).unwrap();
    state(&store, &mine, TrainState::Gating);
    // The gate fails and the lane goes back for a fix.
    assert_eq!(to(&store, &a, LaneState::Fixing).train, None);
    assert_eq!(store.train(mine.id).unwrap().state, TrainState::Gating, "it is the coordinator's train");
    // Pushed after the lane left: the SHA is still taken.
    let pushed = set(&store, &mine, TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() });
    assert_eq!((pushed.state, pushed.pushed_sha.as_deref()), (TrainState::Pushed, Some("1a1b3275")));
}

#[test]
fn told_once_holds_when_callers_race() {
    let (store, main) = board();
    ready(&store, main, "a");
    let departed = store.departures_untold(main).unwrap().remove(0);
    let store = std::sync::Arc::new(store);
    let wins = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                if store.record_departure_told(&departed).unwrap() {
                    wins.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(wins.load(std::sync::atomic::Ordering::SeqCst), 1, "one caller sends");
    let told = store.train_events(departed.train.id).unwrap().into_iter().filter(|e| e.kind == "told").count();
    assert_eq!(told, 1);
}

#[test]
fn a_collecting_train_with_a_ready_lane_and_a_red_main_waits_for_main() {
    let (store, main) = board();
    store.record_ci(main, &read("main", CiStatus::Failed)).unwrap();
    ready(&store, main, "a");
    let open = named(&store, main, "train-1");
    assert_eq!(phase(&store, main, &open), Some(TrainPhase::WaitingForMain));
    // With no lane ready it is only waiting for lanes (it isn't drawn at all while empty).
    store.record_ci(main, &read("main", CiStatus::Passed)).unwrap();
    assert_eq!(store.train(open.id).unwrap().state, TrainState::Gating);
}

#[test]
fn the_orchestrator_is_told_of_each_departure_once() {
    let (store, main) = board();
    ready(&store, main, "a");
    let train = named(&store, main, "train-1");
    let told = store.departures_untold(main).unwrap();
    assert_eq!(told.len(), 1);
    assert_eq!((told[0].train.id, told[0].lanes.clone()), (train.id, vec!["a".to_string()]));
    assert!(store.record_departure_told(&told[0]).unwrap());
    assert!(!store.record_departure_told(&told[0]).unwrap(), "recorded once");
    assert!(store.departures_untold(main).unwrap().is_empty());
    // A train the coordinator sent off by hand is not announced.
    hold(&store, main, "mine");
    let mine = named(&store, main, "mine");
    state(&store, &mine, TrainState::Gating);
    assert!(store.departures_untold(main).unwrap().is_empty());
    assert_eq!(store.workspaces_with_gating_trains().unwrap(), vec![main]);
}

#[test]
fn there_is_always_exactly_one_collecting_train() {
    let (store, main) = board();
    let a = ready(&store, main, "a");
    assert_eq!(collecting_count(&store, main), 1);
    let first = named(&store, main, "train-1");
    let second = named(&store, main, "train-2");
    state(&store, &second, TrainState::Dropped);
    assert_eq!(collecting_count(&store, main), 1, "after the collecting one was dropped");
    set(&store, &first, TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() });
    state(&store, &first, TrainState::Landed);
    assert_eq!(collecting_count(&store, main), 1, "after landing");
    let by_hand = hold(&store, main, "integ-20");
    assert_eq!(collecting_count(&store, main), 1, "a train started by hand is not the collecting one");
    state(&store, &by_hand, TrainState::Gating);
    assert_eq!(collecting_count(&store, main), 1);
    assert_eq!(store.lane(a.id).unwrap().state, LaneState::Landed);
}

#[test]
fn lanes_a_build_from_before_this_left_ready_are_collected() {
    let (store, main) = board();
    hold(&store, main, "ahead");
    let l = store.create_lane(main, &NewLane { name: "old".into(), ..Default::default() }, &[], None, Actor::Manager).unwrap();
    crate::testing::strand_in_landing(&store, l.id);
    assert_eq!(store.workspaces_to_collect().unwrap(), vec![main]);
    assert_eq!(store.collect_ready_lanes(main, Actor::Runner).unwrap().joined, vec![l.id]);
    assert_eq!(on_train(&store, &l).as_deref(), Some("train-1"));
    assert!(store.collect_ready_lanes(main, Actor::Runner).unwrap().joined.is_empty(), "nothing the second time");
}

#[test]
fn a_rename_is_a_history_row_the_readers_ignore() {
    let (store, main) = board();
    hold(&store, main, "ahead");
    ready(&store, main, "a");
    let open = named(&store, main, "train-1");
    let before = last_lane_on(&store.conn(), open.id).unwrap();
    let mine = store.start_train(main, &NewTrain { name: "integ-7".into(), ..Default::default() }, &[], Actor::Manager).unwrap();
    assert!(store.train_events(mine.id).unwrap().iter().any(|e| e.kind == "renamed"));
    assert_eq!(last_lane_on(&store.conn(), open.id).unwrap(), before, "the phase clock reads lane_on rows only");
    assert!(store.plan(main, 0).unwrap().trains.iter().any(|t| t.train.name == "integ-7"));
}

#[test]
fn a_lane_moves_between_two_trains_and_each_history_says_where() {
    let (store, main) = board();
    ready(&store, main, "a");
    let first = named(&store, main, "train-1");
    let b = ready(&store, main, "b");
    let second = named(&store, main, "train-2");
    set(&store, &first, TrainUpdate { add_lanes: vec![b.id], ..Default::default() });
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-1"));
    let off = store.train_events(second.id).unwrap().into_iter().find(|e| e.kind == "lane_off").unwrap();
    assert_eq!((off.lane_id, off.extra["to"].clone()), (Some(b.id), serde_json::json!("train-1")));
    let on = store.train_events(first.id).unwrap().into_iter().rfind(|e| e.kind == "lane_on").unwrap();
    assert_eq!((on.lane_id, on.extra["from"].clone()), (Some(b.id), serde_json::json!("train-2")));
    assert_eq!(store.lane_train_events(b.id).unwrap().len(), 3, "on train-2, off it, on train-1");
    // Moved by a train write, not taken off by hand: the scan doesn't undo it.
    assert!(store.collect_ready_lanes(main, Actor::Runner).unwrap().joined.is_empty());
}

#[test]
fn taking_a_lane_off_the_wrong_train_is_a_no_op_and_a_swap_is_one_write() {
    let (store, main) = board();
    hold(&store, main, "ahead");
    let (a, b) = (ready(&store, main, "a"), ready(&store, main, "b"));
    let mine = store.start_train(main, &NewTrain { name: "integ-4".into(), ..Default::default() }, &[a.id], Actor::Manager).unwrap();
    let behind = named(&store, main, "train-5");
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-5"));
    // `integ-4` can't take b off a train b isn't on, and neither can the other take a off.
    set(&store, &mine, TrainUpdate { remove_lanes: vec![b.id], ..Default::default() });
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-5"));
    set(&store, &behind, TrainUpdate { remove_lanes: vec![a.id], ..Default::default() });
    assert_eq!(on_train(&store, &a).as_deref(), Some("integ-4"));
    // Add one lane and remove another in one write.
    set(&store, &mine, TrainUpdate { add_lanes: vec![b.id], remove_lanes: vec![a.id], ..Default::default() });
    assert_eq!((on_train(&store, &a), on_train(&store, &b).as_deref().map(str::to_string)), (None, Some("integ-4".to_string())));
    assert!(store.collect_ready_lanes(main, Actor::Runner).unwrap().joined.is_empty(), "a was taken off by hand and stays off");
}

#[test]
fn a_train_that_left_and_lost_every_lane_is_dropped_so_it_is_not_ahead_for_good() {
    // The coordinator takes its only lane onto a train of its own.
    let (store, main) = board();
    let a = ready(&store, main, "a");
    let left = named(&store, main, "train-1");
    assert_eq!(left.state, TrainState::Gating);
    let mine = store.start_train(main, &NewTrain { name: "integ-2".into(), ..Default::default() }, &[a.id], Actor::Manager).unwrap();
    assert_eq!(store.train(left.id).unwrap().state, TrainState::Dropped, "it had nothing left to check");
    assert_eq!(on_train(&store, &a).as_deref(), Some(mine.name.as_str()));

    // Its only lane goes back for a fix, and the lane waiting behind it can go.
    let (store, main) = board();
    let a = ready(&store, main, "a");
    let left = named(&store, main, "train-1");
    let b = ready(&store, main, "b");
    let behind = named(&store, main, "train-2");
    assert_eq!(behind.state, TrainState::Integrating);
    to(&store, &a, LaneState::Fixing);
    assert_eq!(store.train(left.id).unwrap().state, TrainState::Dropped);
    assert_eq!(store.train(behind.id).unwrap().state, TrainState::Gating, "nothing is ahead of it now");
    assert_eq!(on_train(&store, &b).as_deref(), Some("train-2"));
    assert_eq!(collecting_count(&store, main), 1);

    // A train the coordinator started by hand may be empty on purpose.
    let (store, main) = board();
    let mine = hold(&store, main, "integ-3");
    assert_eq!(store.train(mine.id).unwrap().state, TrainState::Integrating);
}
