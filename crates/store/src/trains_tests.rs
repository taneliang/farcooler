use super::*;

use crate::board_ci::{CiJob, CiRead, CiStatus, sha_subject};
use crate::models::Task;
use crate::plan::{Lane, NewLane};

fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn lane(store: &Store, main: Uuid, name: &str) -> Lane {
    store.create_lane(main, &NewLane { name: name.into(), ..Default::default() }, &[], None, Actor::Manager).unwrap()
}

fn start(store: &Store, main: Uuid, name: &str, lanes: &[Uuid]) -> Result<Train> {
    store.start_train(main, &NewTrain { name: name.into(), base: "origin/main".into() }, lanes, Actor::Manager)
}

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn sha(store: &Store, train: &Train, sha: &str) -> Result<Train> {
    store.set_train(train.id, &TrainUpdate { sha: Some(sha.into()), ..Default::default() }, Actor::Manager)
}

fn state(store: &Store, train: &Train, to: TrainState) -> Result<Train> {
    store.set_train(train.id, &TrainUpdate { state: Some(to), ..Default::default() }, Actor::Manager)
}

fn read(subject: &str, status: CiStatus) -> CiRead {
    CiRead {
        subject: subject.into(),
        sha: "1a1b3275f00d".into(),
        status,
        url: "https://github.com/o/r/actions/runs/7".into(),
        jobs: vec![CiJob { name: "CI / rust".into(), state: "running".into(), url: String::new() }],
        fetched_at: 0,
        changed_at: 0,
        asked_at: 0,
    }
}

/// The migration is the 27th and `Welcome`, so the build before it can still
/// open the file.
#[test]
fn the_migration_is_welcome() {
    use crate::compat::Older;
    let last = &crate::migrate::MIGRATIONS[26];
    assert!(std::ptr::fn_addr_eq(last.0, migration_0027_trains as fn(&Transaction) -> rusqlite::Result<()>));
    assert_eq!(last.1, Older::Welcome);
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 29);
}

/// A train starts integrating, its lanes say they're on it, and its name is
/// used once per board.
#[test]
fn a_train_starts_integrating_with_its_lanes() {
    let (store, main, _) = board(0);
    let (a, b) = (lane(&store, main, "mac-ux"), lane(&store, main, "phones"));
    let train = start(&store, main, " integ-14 ", &[a.id, b.id]).unwrap();
    assert_eq!((train.name.as_str(), train.base.as_str(), train.state), ("integ-14", "origin/main", TrainState::Integrating));
    assert_eq!((train.pushed_sha.as_deref(), train.landed_at, train.actor.as_str()), (None, None, "manager"));
    assert_eq!(store.lane(a.id).unwrap().train.as_deref(), Some("integ-14"));
    assert_eq!(store.lane(b.id).unwrap().train.as_deref(), Some("integ-14"));

    assert_eq!(refused(start(&store, main, "INTEG-14", &[])), "name_taken");
    assert_eq!(refused(start(&store, main, "integ 15", &[])), "name");
    assert_eq!(refused(start(&store, main, "", &[])), "name");
    assert_eq!(store.train_named(main, "Integ-14").unwrap().id, train.id);
}

/// A lane from another board is refused, as a card from one is.
#[test]
fn a_lane_from_another_board_is_refused() {
    let (store, main, _) = board(0);
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    let theirs = lane(&store, other.id, "theirs");
    assert_eq!(refused(start(&store, main, "integ-1", &[theirs.id])), "other_board");
    let train = start(&store, main, "integ-1", &[]).unwrap();
    let add = TrainUpdate { add_lanes: vec![theirs.id], ..Default::default() };
    assert_eq!(refused(store.set_train(train.id, &add, Actor::Manager)), "other_board");
    assert_eq!(store.lane(theirs.id).unwrap().train, None);
}

/// A pushed SHA moves an integrating or gating train to pushed; a train can't
/// be pushed, green or red without one; a bad SHA is refused.
#[test]
fn a_sha_moves_a_train_to_pushed() {
    let (store, main, _) = board(0);
    let train = start(&store, main, "integ-2", &[]).unwrap();
    assert_eq!(refused(state(&store, &train, TrainState::Pushed)), "sha");
    assert_eq!(refused(state(&store, &train, TrainState::Green)), "sha");
    let gating = state(&store, &train, TrainState::Gating).unwrap();
    assert_eq!(gating.state, TrainState::Gating);
    assert_eq!(refused(sha(&store, &train, "xyz1234")), "sha");
    assert_eq!(refused(sha(&store, &train, "1a1b3")), "sha");
    let pushed = sha(&store, &train, " 1A1B3275 ").unwrap();
    assert_eq!((pushed.state, pushed.pushed_sha.as_deref()), (TrainState::Pushed, Some("1a1b3275")));
    assert_eq!(pushed.ci_subject().as_deref(), Some("sha:1a1b3275"));
}

/// A train that landed or was dropped takes no more moves and no new SHA;
/// landing stamps when.
#[test]
fn a_settled_train_stays_settled() {
    let (store, main, _) = board(0);
    let train = start(&store, main, "integ-3", &[]).unwrap();
    sha(&store, &train, "1a1b3275").unwrap();
    let landed = state(&store, &train, TrainState::Landed).unwrap();
    assert!(landed.landed_at.is_some());
    assert_eq!(refused(state(&store, &train, TrainState::Red)), "train_settled");
    assert_eq!(refused(sha(&store, &train, "2b2c4386")), "train_settled");
    // Rebasing the record, or the same state again, is still allowed.
    let same = store
        .set_train(train.id, &TrainUpdate { base: Some("abc".into()), state: Some(TrainState::Landed), ..Default::default() }, Actor::Manager)
        .unwrap();
    assert_eq!((same.base.as_str(), same.landed_at), ("abc", landed.landed_at));
}

/// Lanes go on and come off; taking off a lane that's on another train leaves
/// it there.
#[test]
fn lanes_go_on_and_come_off() {
    let (store, main, _) = board(0);
    let (a, b) = (lane(&store, main, "a"), lane(&store, main, "b"));
    let one = start(&store, main, "integ-4", &[a.id]).unwrap();
    let two = start(&store, main, "integ-5", &[b.id]).unwrap();
    let swap = TrainUpdate { add_lanes: vec![b.id], remove_lanes: vec![a.id], ..Default::default() };
    store.set_train(one.id, &swap, Actor::Manager).unwrap();
    assert_eq!(store.lane(a.id).unwrap().train, None);
    assert_eq!(store.lane(b.id).unwrap().train.as_deref(), Some("integ-4"));
    let off = TrainUpdate { remove_lanes: vec![b.id], ..Default::default() };
    store.set_train(two.id, &off, Actor::Manager).unwrap();
    assert_eq!(store.lane(b.id).unwrap().train.as_deref(), Some("integ-4"), "only integ-4 can take b off");
}

/// CI moves a pushed train to green or red, and back to pushed when a run
/// starts again; a train not yet pushed doesn't follow it.
#[test]
fn ci_moves_a_pushed_train() {
    let (store, main, _) = board(0);
    let train = start(&store, main, "integ-6", &[]).unwrap();
    let other = start(&store, main, "integ-7", &[]).unwrap();
    sha(&store, &train, "1a1b3275").unwrap();
    let subject = sha_subject("1a1b3275");

    let w = store.record_ci(main, &read(&subject, CiStatus::Running)).unwrap();
    assert_eq!((w.changed, w.trains_moved.len()), (true, 0), "already pushed");
    let w = store.record_ci(main, &read(&subject, CiStatus::Failed)).unwrap();
    assert_eq!(w.trains_moved, vec![train.id]);
    assert_eq!(store.train(train.id).unwrap().state, TrainState::Red);
    store.record_ci(main, &read(&subject, CiStatus::Running)).unwrap();
    assert_eq!(store.train(train.id).unwrap().state, TrainState::Pushed);
    store.record_ci(main, &read(&subject, CiStatus::Passed)).unwrap();
    assert_eq!(store.train(train.id).unwrap().state, TrainState::Green);
    assert_eq!(store.train(other.id).unwrap().state, TrainState::Integrating);

    // Landed, it stops following.
    state(&store, &train, TrainState::Landed).unwrap();
    store.record_ci(main, &read(&subject, CiStatus::Failed)).unwrap();
    assert_eq!(store.train(train.id).unwrap().state, TrainState::Landed);
}

/// A read that couldn't reach GitHub never replaces one that did, and the
/// same read twice isn't a change.
#[test]
fn an_unknown_read_keeps_the_last_known_one() {
    let (store, main, _) = board(0);
    let w = store.record_ci(main, &read("main", CiStatus::Passed)).unwrap();
    assert!(w.changed);
    let first = store.ci_read(main, "main").unwrap().unwrap();
    assert!(!store.record_ci(main, &read("main", CiStatus::Passed)).unwrap().changed);
    assert!(!store.record_ci(main, &read("main", CiStatus::Unknown)).unwrap().changed);
    let kept = store.ci_read(main, "main").unwrap().unwrap();
    assert_eq!((kept.status, kept.changed_at, kept.jobs.len()), (CiStatus::Passed, first.changed_at, 1));
    // With nothing known yet, unknown is what's known.
    store.record_ci(main, &read("run:9", CiStatus::Unknown)).unwrap();
    assert_eq!(store.ci_read(main, "run:9").unwrap().unwrap().status, CiStatus::Unknown);
}

/// Reads of subjects the board no longer names are forgotten.
#[test]
fn reads_nothing_names_are_forgotten() {
    let (store, main, _) = board(0);
    store.record_ci(main, &read("main", CiStatus::Passed)).unwrap();
    store.record_ci(main, &read("run:9", CiStatus::Running)).unwrap();
    assert_eq!(store.keep_ci(main, &["main".to_string()]).unwrap(), 1);
    let subjects: Vec<String> = store.plan(main, 0).unwrap().ci.into_iter().map(|r| r.subject).collect();
    assert_eq!(subjects, vec!["main"]);
}

/// The plan read carries trains with their lanes, landed ones included, and a
/// settled train ages out of it like a finished lane.
#[test]
fn the_plan_reads_trains_and_their_lanes() {
    let (store, main, _) = board(0);
    let (a, b) = (lane(&store, main, "a"), lane(&store, main, "b"));
    let old = start(&store, main, "integ-8", &[a.id]).unwrap();
    let new = start(&store, main, "integ-9", &[b.id]).unwrap();
    store.update_lane(a.id, &crate::plan::LaneUpdate { state: Some(crate::plan::LaneState::Dropped), ..Default::default() }, Actor::Manager).unwrap();
    store.record_ci(main, &read("sha:1a1b3275", CiStatus::Passed)).unwrap();

    let plan = store.plan(main, 0).unwrap();
    let names: Vec<(&str, Vec<Uuid>)> = plan.trains.iter().map(|t| (t.train.name.as_str(), t.lanes.clone())).collect();
    assert_eq!(names, vec![("integ-8", vec![a.id]), ("integ-9", vec![b.id])]);
    assert_eq!(plan.ci.len(), 1);

    state(&store, &old, TrainState::Dropped).unwrap();
    let later = store.plan(main, i64::MAX).unwrap();
    assert_eq!(later.trains.iter().map(|t| t.train.id).collect::<Vec<_>>(), vec![new.id]);
    let all = store.plan(main, 0).unwrap();
    assert_eq!(all.trains.iter().map(|t| t.train.id).collect::<Vec<_>>(), vec![new.id, old.id], "live first, then settled");
}

/// The runner's CI watch walks only trains that follow CI.
#[test]
fn only_pushed_trains_are_followed() {
    let (store, main, _) = board(0);
    let a = start(&store, main, "integ-10", &[]).unwrap();
    let b = start(&store, main, "integ-11", &[]).unwrap();
    let c = start(&store, main, "integ-12", &[]).unwrap();
    sha(&store, &b, "1a1b3275").unwrap();
    sha(&store, &c, "2b2c4386").unwrap();
    state(&store, &c, TrainState::Landed).unwrap();
    let followed: Vec<Uuid> = store.trains_following_ci().unwrap().into_iter().map(|t| t.id).collect();
    assert_eq!(followed, vec![b.id]);
    assert!(!followed.contains(&a.id));
}

// ---- live page data (ov-306) ----

/// One run of `agent`'s, `tokens` in and nothing else.
fn turn(store: &Store, agent: &str, tokens: u64) {
    use crate::usage::{NewTurn, Surface, TurnKind, TurnModel};
    use farcooler_core::usage::TokenCounts;
    store
        .record_turn(&NewTurn {
            key: format!("claude-log:agent:{agent}"),
            terminal_id: None,
            worktree_id: None,
            repository_id: None,
            workspace_id: None,
            task_id: None,
            harness: "claude".into(),
            surface: Surface::Terminal,
            started_at: None,
            ended_at: 1000,
            active_ms: None,
            usage: "reported",
            models: vec![TurnModel::priced(
                Some("claude-opus-5".into()),
                TokenCounts { input: tokens, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0 },
                Some(2_000),
            )],
            kind: TurnKind::Subagent,
        })
        .unwrap();
}

/// A theme's spend is its lanes' spend shared out over their cards: a lane on
/// one of the theme's cards and one outside gives it half, and a finished lane
/// still counts. The board's counts cover every card.
#[test]
fn a_theme_spends_its_share_of_its_lanes() {
    use crate::plan::{AgentRecord, AgentRole, LaneCard, NewTheme};
    let (store, main, t) = board(3);
    let theme = store.create_theme(main, &NewTheme { name: "Visual".into(), outcome: String::new() }, &[t[0].id, t[1].id], Actor::Manager).unwrap();
    let agent = |id: &str| AgentRecord { harness: "claude".into(), agent_id: id.into(), role: AgentRole::Build, model: None, ended: false };
    let card = |task: &Task| LaneCard { task_id: task.id, slice: String::new() };
    let half = store
        .create_lane(main, &NewLane { name: "half".into(), ..Default::default() }, &[card(&t[0]), card(&t[2])], Some(&agent("a1")), Actor::Manager)
        .unwrap();
    let whole = store
        .create_lane(main, &NewLane { name: "whole".into(), ..Default::default() }, &[card(&t[1])], Some(&agent("a2")), Actor::Manager)
        .unwrap();
    turn(&store, "a1", 1000);
    turn(&store, "a2", 300);
    store.update_lane(whole.id, &crate::plan::LaneUpdate { state: Some(crate::plan::LaneState::Dropped), ..Default::default() }, Actor::Manager).unwrap();

    let plan = store.plan(main, i64::MAX).unwrap();
    assert!(plan.lanes.iter().all(|l| l.lane.id != whole.id), "the dropped lane is out of the window");
    let view = plan.themes.iter().find(|v| v.theme.id == theme.id).unwrap();
    let total = |s: &crate::plan_read::LaneSpend| s.input_tokens + s.output_tokens + s.cache_read_tokens + s.cache_write_tokens;
    let half_spend = total(&plan.lanes.iter().find(|l| l.lane.id == half.id).unwrap().spend);
    assert_eq!(half_spend, 1000);
    assert_eq!(total(&view.spend), 500 + 300);
    assert_eq!(view.spend.runs, 2);
    assert_eq!((plan.board_counts.backlog, plan.board_counts.total()), (3, 3));
}

// ---- a superseded run (review train-1005c H1) ----

/// CI cancels a run a newer push supersedes. A commit whose only run that
/// didn't pass was canceled isn't failed, and its train doesn't go red: it
/// waits, pushed.
#[test]
fn a_canceled_run_is_superseded_not_red() {
    let (store, main, _) = board(0);
    let train = start(&store, main, "integ-20", &[]).unwrap();
    sha(&store, &train, "1a1b3275").unwrap();
    let status = crate::board_ci::status_of(&["passed", "canceled", "passed"]);
    assert_ne!(status, CiStatus::Failed, "canceled is superseded, not failed");
    store.record_ci(main, &read(&sha_subject("1a1b3275"), status)).unwrap();
    assert_eq!(store.train(train.id).unwrap().state, TrainState::Pushed);
    // A real failure beside a cancel is still red.
    assert_eq!(crate::board_ci::status_of(&["failed", "canceled"]), CiStatus::Failed);
}

// ---- a read that goes stale (review train-1005c M1) ----

/// When GitHub can't be asked, the last read stands but says when it was
/// made: `fetched_at` stays the last read that worked, and `asked_at` moves.
#[test]
fn an_unknown_read_leaves_when_it_was_last_read() {
    let (store, main, _) = board(0);
    store.record_ci(main, &read("main", CiStatus::Passed)).unwrap();
    let first = store.ci_read(main, "main").unwrap().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    store.record_ci(main, &read("main", CiStatus::Unknown)).unwrap();
    let after = store.ci_read(main, "main").unwrap().unwrap();
    assert_eq!(after.fetched_at, first.fetched_at, "the last read that worked");
    assert!(after.asked_at > first.fetched_at, "the last time it was asked");
    // Never read at all: nothing was fetched.
    store.record_ci(main, &read("run:9", CiStatus::Unknown)).unwrap();
    assert_eq!(store.ci_read(main, "run:9").unwrap().unwrap().fetched_at, 0);
}

/// A dropped train lets its lanes go: they no longer say they're in it
/// (review train-1005c L1). A landed train keeps them, as its record.
#[test]
fn a_dropped_train_lets_its_lanes_go() {
    let (store, main, _) = board(0);
    let (a, b) = (lane(&store, main, "a"), lane(&store, main, "b"));
    let dropped = start(&store, main, "integ-30", &[a.id]).unwrap();
    let landed = start(&store, main, "integ-31", &[b.id]).unwrap();
    state(&store, &dropped, TrainState::Dropped).unwrap();
    sha(&store, &landed, "1a1b3275").unwrap();
    state(&store, &landed, TrainState::Landed).unwrap();
    assert_eq!(store.lane(a.id).unwrap().train, None);
    assert_eq!(store.lane(b.id).unwrap().train.as_deref(), Some("integ-31"));
}

/// A lane with two of a theme's cards gives it half its tokens twice but its
/// runs once (review train-1005c L3).
#[test]
fn a_theme_counts_a_lane_s_runs_once() {
    use crate::plan::{AgentRecord, AgentRole, LaneCard, NewTheme};
    let (store, main, t) = board(3);
    let theme = store
        .create_theme(main, &NewTheme { name: "Two".into(), outcome: String::new() }, &[t[0].id, t[1].id], Actor::Manager)
        .unwrap();
    let agent = AgentRecord { harness: "claude".into(), agent_id: "a9".into(), role: AgentRole::Build, model: None, ended: false };
    let card = |task: &Task| LaneCard { task_id: task.id, slice: String::new() };
    store
        .create_lane(main, &NewLane { name: "both".into(), ..Default::default() }, &[card(&t[0]), card(&t[1])], Some(&agent), Actor::Manager)
        .unwrap();
    turn(&store, "a9", 1000);
    let plan = store.plan(main, 0).unwrap();
    let view = plan.themes.iter().find(|v| v.theme.id == theme.id).unwrap();
    assert_eq!((view.spend.input_tokens, view.spend.runs), (1000, 1));
}
