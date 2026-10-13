//! What a train says is true (ov-503): landing lands its lanes, only ready
//! lanes board, and the repair migration for what the hand-kept states left
//! wrong. Beside `trains_tests.rs`, which holds the rest.

use rusqlite::params;

use super::*;
use crate::plan::{Lane, LaneState, LaneUpdate, NewLane};

fn board() -> (Store, Uuid) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    (store, main)
}

fn lane(store: &Store, main: Uuid, name: &str) -> Lane {
    store.create_lane(main, &NewLane { name: name.into(), ..Default::default() }, &[], None, Actor::Manager).unwrap()
}

fn to(store: &Store, l: &Lane, state: LaneState) -> Lane {
    store.update_lane(l.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager).unwrap()
}

/// A lane taken as far as `state` along the road a real lane takes.
fn lane_at(store: &Store, main: Uuid, name: &str, state: LaneState) -> Lane {
    let l = lane(store, main, name);
    let road = [LaneState::Building, LaneState::Review, LaneState::Landing];
    let stops = match state {
        LaneState::Queued => 0,
        LaneState::Building => 1,
        LaneState::Review => 2,
        _ => 3,
    };
    let mut at = l;
    for s in &road[..stops] {
        at = to(store, &at, *s);
    }
    if state == LaneState::Fixing {
        at = to(store, &at, LaneState::Fixing);
    }
    at
}

fn start(store: &Store, main: Uuid, name: &str, lanes: &[Uuid]) -> Result<Train> {
    store.start_train(main, &NewTrain { name: name.into(), base: "origin/main".into(), ..Default::default() }, lanes, Actor::Manager)
}

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn set(store: &Store, train: &Train, update: TrainUpdate) -> Result<Train> {
    store.set_train(train.id, &update, Actor::Manager)
}

fn bodies(store: &Store, l: &Lane) -> Vec<String> {
    store.plan_events(Subject::Lane(l.id), 0).unwrap().into_iter().map(|e| e.body).collect()
}

/// Landing a train lands every open lane on it, with the train's SHA, and says
/// so on each lane's timeline. A lane already landed or dropped is left alone,
/// and a landed lane keeps the SHA it had.
#[test]
fn landing_a_train_lands_its_lanes() {
    let (store, main) = board();
    let (a, b) = (lane_at(&store, main, "a", LaneState::Landing), lane_at(&store, main, "b", LaneState::Landing));
    let train = start(&store, main, "train-83", &[a.id, b.id]).unwrap();
    // A lane the orchestrator put on by hand while still building, and two
    // already closed.
    let stray = lane_at(&store, main, "stray", LaneState::Building);
    store.update_lane(stray.id, &LaneUpdate { train: Some("train-83".into()), ..Default::default() }, Actor::Manager).unwrap();
    let done = lane_at(&store, main, "done", LaneState::Landing);
    store
        .update_lane(done.id, &LaneUpdate { state: Some(LaneState::Landed), train: Some("train-83".into()), landed_sha: Some("0ldsha0".into()), ..Default::default() }, Actor::Manager)
        .unwrap();
    let gone = lane(&store, main, "gone");
    store.update_lane(gone.id, &LaneUpdate { state: Some(LaneState::Dropped), train: Some("train-83".into()), ..Default::default() }, Actor::Manager).unwrap();

    sha(&store, &train, "1a1b3275f00d");
    let landed = set(&store, &train, TrainUpdate { state: Some(TrainState::Landed), ..Default::default() }).unwrap();
    assert_eq!(landed.state, TrainState::Landed);
    for l in [&a, &b, &stray] {
        let now = store.lane(l.id).unwrap();
        assert_eq!((now.state, now.landed_sha.as_deref(), now.train.as_deref()), (LaneState::Landed, Some("1a1b3275f00d"), Some("train-83")), "{}", l.name);
        assert!(bodies(&store, l).contains(&"Landed with Train 83.".to_string()), "{:?}", bodies(&store, l));
    }
    assert_eq!(store.lane(done.id).unwrap().landed_sha.as_deref(), Some("0ldsha0"), "a landed lane keeps its SHA");
    assert_eq!(store.lane(gone.id).unwrap().state, LaneState::Dropped);
    assert!(!bodies(&store, &gone).iter().any(|b| b.starts_with("Landed with")));
}

fn sha(store: &Store, train: &Train, sha: &str) -> Train {
    set(store, train, TrainUpdate { sha: Some(sha.into()), ..Default::default() }).unwrap()
}

/// A train that landed with no SHA recorded still lands its lanes, and leaves
/// each lane's own `landed_sha` as it was.
#[test]
fn a_train_landing_without_a_sha_lands_its_lanes_all_the_same() {
    let (store, main) = board();
    let a = lane_at(&store, main, "a", LaneState::Landing);
    store.update_lane(a.id, &LaneUpdate { landed_sha: Some("abc1234".into()), ..Default::default() }, Actor::Manager).unwrap();
    let train = start(&store, main, "train-85", &[a.id]).unwrap();
    set(&store, &train, TrainUpdate { state: Some(TrainState::Landed), ..Default::default() }).unwrap();
    let now = store.lane(a.id).unwrap();
    assert_eq!((now.state, now.landed_sha.as_deref()), (LaneState::Landed, Some("abc1234")));
}

/// Only a move to landed lands lanes: a train going green, red or dropped does
/// not, and a dropped one lets its lanes go as it always has.
#[test]
fn only_landing_lands() {
    let (store, main) = board();
    let a = lane_at(&store, main, "a", LaneState::Landing);
    let train = start(&store, main, "train-1", &[a.id]).unwrap();
    sha(&store, &train, "1a1b3275");
    set(&store, &train, TrainUpdate { state: Some(TrainState::Green), ..Default::default() }).unwrap();
    assert_eq!(store.lane(a.id).unwrap().state, LaneState::Landing);
    set(&store, &train, TrainUpdate { state: Some(TrainState::Dropped), ..Default::default() }).unwrap();
    let freed = store.lane(a.id).unwrap();
    assert_eq!((freed.state, freed.train), (LaneState::Landing, None));
}

/// Queued, building and fixing lanes are refused, a lane in review is refused
/// with its own word (the CLI's --force moves it first), and a closed lane is
/// refused. The first refusal changes nothing, whichever lane comes first.
#[test]
fn only_ready_lanes_board() {
    let (store, main) = board();
    let ok = lane_at(&store, main, "ok", LaneState::Landing);
    for (name, state, word) in [
        ("q", LaneState::Queued, "lane_not_ready"),
        ("b", LaneState::Building, "lane_not_ready"),
        ("f", LaneState::Fixing, "lane_not_ready"),
        ("r", LaneState::Review, "lane_in_review"),
    ] {
        let l = lane_at(&store, main, name, state);
        assert_eq!(store.lane(l.id).unwrap().state, state, "{name} is where the test thinks");
        assert_eq!(refused(start(&store, main, "train-2", &[ok.id, l.id])), word, "start with a {name} lane");
        assert!(store.train_named(main, "train-2").is_err(), "a refused start leaves no train");
        assert_eq!(store.lane(ok.id).unwrap().train, None, "nor a lane on it");
    }
    let closed = lane(&store, main, "closed");
    to(&store, &closed, LaneState::Dropped);
    assert_eq!(refused(start(&store, main, "train-2", &[closed.id])), "lane_closed");

    let train = start(&store, main, "train-2", &[ok.id]).unwrap();
    let q = lane(&store, main, "later");
    let add = |l: &Lane| set(&store, &train, TrainUpdate { add_lanes: vec![ok.id, l.id], ..Default::default() });
    assert_eq!(refused(add(&q)), "lane_not_ready");
    assert_eq!(store.lane(q.id).unwrap().train, None);
    let r = lane_at(&store, main, "rev", LaneState::Review);
    assert_eq!(refused(add(&r)), "lane_in_review");
    // A lane already on the train can be named again whatever came of it.
    to(&store, &store.lane(ok.id).unwrap(), LaneState::Fixing);
    set(&store, &train, TrainUpdate { add_lanes: vec![ok.id], ..Default::default() }).unwrap();
    let ready = lane_at(&store, main, "ready", LaneState::Landing);
    set(&store, &train, TrainUpdate { add_lanes: vec![ready.id], ..Default::default() }).unwrap();
    assert_eq!(store.lane(ready.id).unwrap().train.as_deref(), Some("train-2"));
}

/// A landed or dropped train takes no more lanes: they would be left open on a
/// train that has already landed.
#[test]
fn a_settled_train_takes_no_lanes() {
    let (store, main) = board();
    let train = start(&store, main, "train-3", &[]).unwrap();
    set(&store, &train, TrainUpdate { state: Some(TrainState::Landed), ..Default::default() }).unwrap();
    let a = lane_at(&store, main, "a", LaneState::Landing);
    assert_eq!(refused(set(&store, &train, TrainUpdate { add_lanes: vec![a.id], ..Default::default() })), "train_settled");
}

/// Today's board as the repair found it (Oct 13): open lanes on landed trains
/// 82, 85 and 87, one with a SHA and the rest without; trains 84 and 89 started
/// with only queued lanes; and the trains the repair must leave alone.
fn the_board_as_it_was() -> (Store, Uuid) {
    let (store, main) = board();
    let on = |name: &str, state: LaneState, train: &str| {
        let l = lane_at(&store, main, name, state);
        store.update_lane(l.id, &LaneUpdate { train: Some(train.into()), ..Default::default() }, Actor::Manager).unwrap();
    };
    let make = |name: &str, state: &str, sha: Option<&str>| {
        let t = start(&store, main, name, &[]).unwrap();
        store
            .conn()
            .execute("UPDATE board_trains SET state = ?2, pushed_sha = ?3 WHERE id = ?1", params![uuid_blob(t.id), state, sha])
            .unwrap();
    };
    make("train-82", "landed", None);
    on("quiet-tests", LaneState::Queued, "train-82");
    on("tailcat-p1", LaneState::Queued, "train-82");
    make("train-85", "landed", Some("85a85a85a"));
    on("lane-start", LaneState::Queued, "train-85");
    make("train-87", "landed", None);
    on("runner-key", LaneState::Building, "TRAIN-87");
    on("closed-already", LaneState::Review, "train-87");
    make("train-84", "integrating", None);
    on("web-site", LaneState::Queued, "train-84");
    make("train-89", "integrating", None);
    on("reps", LaneState::Queued, "train-89");
    on("reps-two", LaneState::Queued, "train-89");
    // Left alone: a mix of queued and review, a landing lane, no lanes at all,
    // and a train that is further along than integrating.
    make("train-86", "integrating", None);
    on("team-dir", LaneState::Review, "train-86");
    on("ios-overwrite", LaneState::Queued, "train-86");
    make("train-94", "integrating", None);
    on("ci-paths", LaneState::Landing, "train-94");
    make("train-95", "integrating", None);
    make("train-90", "red", Some("1e7ea399e"));
    on("relay-401", LaneState::Queued, "train-90");
    (store, main)
}

fn repair(store: &Store) {
    let mut conn = store.conn();
    let tx = conn.transaction().unwrap();
    migration_0047_train_truth(&tx).unwrap();
    tx.commit().unwrap();
}

/// Open lanes on landed trains land (with the train's SHA when it has one), and
/// only a train integrating with every lane queued is dropped, releasing them.
#[test]
fn the_repair_lands_stranded_lanes_and_drops_forgotten_trains() {
    let (store, main) = the_board_as_it_was();
    repair(&store);
    let lane = |n: &str| store.plan(main, 0).unwrap().lanes.into_iter().find(|l| l.lane.name == n).unwrap().lane;
    for (name, sha) in [("quiet-tests", None), ("tailcat-p1", None), ("lane-start", Some("85a85a85a")), ("runner-key", None)] {
        let l = lane(name);
        assert_eq!((l.state, l.landed_sha.as_deref()), (LaneState::Landed, sha), "{name}");
        assert!(bodies(&store, &l).contains(&"Repaired: its train had landed.".to_string()), "{name}: {:?}", bodies(&store, &l));
    }
    assert_eq!(lane("closed-already").state, LaneState::Landed, "any open lane on a landed train, review included");
    let state = |n: &str| store.train_named(main, n).unwrap().state;
    assert_eq!((state("train-84"), state("train-89")), (TrainState::Dropped, TrainState::Dropped));
    for n in ["web-site", "reps", "reps-two"] {
        let l = lane(n);
        assert_eq!((l.state, l.train.clone()), (LaneState::Queued, None), "{n} waits in the queue");
        assert!(bodies(&store, &l).iter().any(|b| b.starts_with("Repaired: its train never started")), "{n}");
    }
    // Everything else is as it was.
    assert_eq!((state("train-86"), state("train-94"), state("train-95"), state("train-90")), (TrainState::Integrating, TrainState::Integrating, TrainState::Integrating, TrainState::Red));
    for (n, s, t) in [("team-dir", LaneState::Review, "train-86"), ("ios-overwrite", LaneState::Queued, "train-86"), ("ci-paths", LaneState::Landing, "train-94"), ("relay-401", LaneState::Queued, "train-90")] {
        let l = lane(n);
        assert_eq!((l.state, l.train.as_deref()), (s, Some(t)), "{n}");
    }
    // Running it again moves nothing.
    let before: Vec<_> = store.plan(main, 0).unwrap().lanes.iter().map(|l| (l.lane.name.clone(), l.lane.state, l.lane.resource_version)).collect();
    repair(&store);
    let after: Vec<_> = store.plan(main, 0).unwrap().lanes.iter().map(|l| (l.lane.name.clone(), l.lane.state, l.lane.resource_version)).collect();
    assert_eq!(before, after);
}

/// The repair writes rows and words only: the schema a build from before it
/// reads is byte for byte the one after it, and the migration says the older
/// build may open the file. This is the pre-migration build's view: its tables
/// and columns, untouched.
#[test]
fn a_build_from_before_the_repair_still_reads_the_database() {
    use crate::compat::{COMPATIBLE_DOWN_TO, Older};
    let schema = |conn: &rusqlite::Connection| -> Vec<(String, String)> {
        let mut stmt = conn.prepare("SELECT name, coalesce(sql, '') FROM sqlite_master ORDER BY name").unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    };
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON; CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn, 37);
    let before = schema(&conn);
    let tx = conn.transaction().unwrap();
    migration_0047_train_truth(&tx).unwrap();
    tx.commit().unwrap();
    assert_eq!(schema(&conn), before, "no table, column, index or trigger moved");

    let last = &crate::migrate::MIGRATIONS[37];
    assert!(std::ptr::fn_addr_eq(last.0, migration_0047_train_truth as fn(&Transaction) -> rusqlite::Result<()>));
    assert_eq!(last.1, Older::Welcome);
    assert!(COMPATIBLE_DOWN_TO <= 37, "the build at schema 37 opens a database at 38");
}
