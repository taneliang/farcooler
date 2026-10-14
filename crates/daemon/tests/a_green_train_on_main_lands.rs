//! A green train whose SHA is on its base branch lands itself, and its lanes
//! with it (ov-503): the CI watch's last step, run against a real git
//! repository and the store behind the in-process daemon.

#[path = "support/in_process.rs"]
mod in_process;

use std::path::{Path, PathBuf};
use std::process::Command;

use farcooler_daemon::train_landing::land_green_trains;
use farcooler_protocol::v1::Scope;
use farcooler_store::landing::{LandingMode, LandingPatch};
use farcooler_store::models::Actor;
use farcooler_store::plan::{LaneState, LaneUpdate, NewLane};
use farcooler_store::trains::{NewTrain, TrainState, TrainUpdate};
use in_process::*;
use uuid::Uuid;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repository whose `origin/main` is at the second of three commits: `on_main`
/// is an ancestor of it, `ahead` is not.
struct Tree {
    _dir: tempfile::TempDir,
    path: PathBuf,
    on_main: String,
    ahead: String,
}

fn a_tree() -> Tree {
    std::fs::create_dir_all("/tmp/fc-t").unwrap();
    let dir = tempfile::Builder::new().prefix("land").tempdir_in("/tmp/fc-t").unwrap();
    let path = dir.path().join("demo");
    std::fs::create_dir_all(&path).unwrap();
    git(&path, &["init", "-q", "-b", "main", "."]);
    for (k, v) in [("user.email", "t@example.com"), ("user.name", "t"), ("commit.gpgsign", "false")] {
        git(&path, &["config", k, v]);
    }
    git(&path, &["commit", "-q", "--allow-empty", "-m", "one"]);
    git(&path, &["commit", "-q", "--allow-empty", "-m", "two"]);
    let on_main = git(&path, &["rev-parse", "HEAD"]);
    git(&path, &["update-ref", "refs/remotes/origin/main", &on_main]);
    git(&path, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    git(&path, &["commit", "-q", "--allow-empty", "-m", "three"]);
    let ahead = git(&path, &["rev-parse", "HEAD"]);
    Tree { _dir: dir, path, on_main, ahead }
}

struct World {
    h: Harness,
    tree: Tree,
    workspace: Uuid,
}

async fn a_world() -> World {
    let h = start(Scope::Control).await;
    let tree = a_tree();
    let store = &h.service.store;
    let host = Uuid::now_v7();
    let root = store.create_repository_root(host, tree.path.parent().unwrap().to_str().unwrap(), 1_000).unwrap();
    let git_dir = tree.path.join(".git");
    let repo = store.create_repository(host, root.id, "demo", git_dir.to_str().unwrap(), "").unwrap();
    let workspace = store.ensure_main_workspace(repo.id).unwrap().id;
    World { h, tree, workspace }
}

impl World {
    /// A train on `sha` with `n` ready lanes, in `state`.
    fn train(&self, name: &str, sha: &str, state: TrainState, lanes: &[&str]) -> (Uuid, Vec<Uuid>) {
        let store = &self.h.service.store;
        let ids: Vec<Uuid> = lanes
            .iter()
            .map(|n| {
                let l = store.create_lane(self.workspace, &NewLane { name: (*n).into(), ..Default::default() }, &[], None, Actor::Manager).unwrap();
                for s in [LaneState::Building, LaneState::Review, LaneState::Landing] {
                    store.update_lane(l.id, &LaneUpdate { state: Some(s), ..Default::default() }, Actor::Manager).unwrap();
                }
                l.id
            })
            .collect();
        let new = NewTrain { name: name.into(), base: "origin/main".into(), ..Default::default() };
        let t = store.start_train(self.workspace, &new, &ids, Actor::Manager).unwrap();
        let up = TrainUpdate { state: Some(state), sha: Some(sha.into()), ..Default::default() };
        store.set_train(t.id, &up, Actor::Manager).unwrap();
        (t.id, ids)
    }

    fn state(&self, train: Uuid) -> TrainState {
        self.h.service.store.train(train).unwrap().state
    }

    async fn land(&self) -> Vec<Uuid> {
        land_green_trains(&self.h.service, &self.h.watcher).await
    }
}

/// Green, and on main: it lands, and its lanes land with its SHA.
#[tokio::test]
async fn a_green_train_on_main_lands_with_its_lanes() {
    let w = a_world().await;
    let (train, lanes) = w.train("train-91", &w.tree.on_main, TrainState::Green, &["a", "b"]);
    assert_eq!(w.land().await, vec![train]);
    assert_eq!(w.state(train), TrainState::Landed);
    for l in lanes {
        let lane = w.h.service.store.lane(l).unwrap();
        assert_eq!((lane.state, lane.landed_sha.as_deref()), (LaneState::Landed, Some(w.tree.on_main.as_str())));
    }
}

/// Green but not on main yet (not pushed, or this clone has not seen the push):
/// it waits. Neither does a train whose CI has not passed land, however much
/// of main it is part of.
#[tokio::test]
async fn a_train_not_on_main_or_not_green_waits() {
    let w = a_world().await;
    let (ahead, lanes) = w.train("train-92", &w.tree.ahead, TrainState::Green, &["a"]);
    let (red, _) = w.train("train-93", &w.tree.on_main, TrainState::Red, &["b"]);
    let (pushed, _) = w.train("train-94", &w.tree.on_main, TrainState::Pushed, &["c"]);
    assert_eq!(w.land().await, Vec::<Uuid>::new());
    assert_eq!((w.state(ahead), w.state(red), w.state(pushed)), (TrainState::Green, TrainState::Red, TrainState::Pushed));
    assert_eq!(w.h.service.store.lane(lanes[0]).unwrap().state, LaneState::Landing);
    // Once main has it, the next turn lands it.
    git(&w.tree.path, &["update-ref", "refs/remotes/origin/main", &w.tree.ahead]);
    assert_eq!(w.land().await, vec![ahead]);
}

/// In pull-request mode a train is a rehearsal: it stays however green.
#[tokio::test]
async fn a_pull_request_train_is_left_alone() {
    let w = a_world().await;
    let store = &w.h.service.store;
    let version = store.get_workspace(w.workspace).unwrap().resource_version;
    store.set_landing(w.workspace, version, &LandingPatch { mode: Some(LandingMode::PullRequests), ..Default::default() }).unwrap();
    let (train, lanes) = w.train("train-95", &w.tree.on_main, TrainState::Green, &["a"]);
    assert_eq!(w.land().await, Vec::<Uuid>::new());
    assert_eq!(w.state(train), TrainState::Green);
    assert_eq!(store.lane(lanes[0]).unwrap().state, LaneState::Landing);
}

/// The CI watch's own turn does it, with no read of GitHub needed to say so: a
/// train already green when the turn begins lands in that turn.
#[tokio::test]
async fn the_watch_s_turn_lands_it() {
    let w = a_world().await;
    let (train, lanes) = w.train("train-96", &w.tree.on_main, TrainState::Green, &["a"]);
    farcooler_daemon::ci_watch::read_all(&w.h.service, &w.h.watcher, &mut farcooler_daemon::ci_watch::Memo::default()).await;
    assert_eq!(w.state(train), TrainState::Landed);
    assert_eq!(w.h.service.store.lane(lanes[0]).unwrap().state, LaneState::Landed);
}

/// A repository path that is gone lands nothing, and says so once, not on every
/// turn; it says so again after the path works and breaks again.
#[tokio::test]
async fn a_missing_repository_lands_nothing_and_is_logged_once() {
    use farcooler_daemon::train_landing::note_check;
    let w = a_world().await;
    let (train, lanes) = w.train("train-97", &w.tree.on_main, TrainState::Green, &["a"]);
    std::fs::remove_dir_all(&w.tree.path).unwrap();
    assert_eq!(w.land().await, Vec::<Uuid>::new());
    assert_eq!(w.land().await, Vec::<Uuid>::new());
    assert_eq!(w.state(train), TrainState::Green);
    assert_eq!(w.h.service.store.lane(lanes[0]).unwrap().state, LaneState::Landing);
    // The first turn already noted the failure, so the next is not news.
    assert!(!note_check(train, false), "already logged");
    assert!(!note_check(train, true));
    assert!(note_check(train, false), "news again after it worked");
}

/// The train ahead reaching main sends the open train off for its checks
/// (ov-520), and a new open train is waiting behind it.
#[tokio::test]
async fn a_landing_train_sends_the_open_train_off_for_its_checks() {
    let w = a_world().await;
    let store = &w.h.service.store;
    let (ahead, _) = w.train("train-96", &w.tree.on_main, TrainState::Green, &["a"]);
    // A lane that passes review while the train ahead is out joins the open train behind it.
    let l = store.create_lane(w.workspace, &NewLane { name: "b".into(), ..Default::default() }, &[], None, Actor::Manager).unwrap();
    for s in [LaneState::Building, LaneState::Review, LaneState::Landing] {
        store.update_lane(l.id, &LaneUpdate { state: Some(s), ..Default::default() }, Actor::Manager).unwrap();
    }
    let open = store.train_named(w.workspace, store.lane(l.id).unwrap().train.as_deref().unwrap()).unwrap();
    assert_eq!(open.state, TrainState::Integrating);
    assert_eq!(w.land().await, vec![ahead]);
    assert_eq!(w.state(open.id), TrainState::Gating, "it started its checks when the train ahead landed");
    let plan = store.plan(w.workspace, 0).unwrap();
    let waiting: Vec<_> = plan.trains.iter().filter(|t| t.train.state == TrainState::Integrating).collect();
    assert_eq!(waiting.len(), 0, "the one behind it is empty, so the plan read leaves it out");
    // A lane that passes review now rides it.
    let c = store.create_lane(w.workspace, &NewLane { name: "c".into(), ..Default::default() }, &[], None, Actor::Manager).unwrap();
    for s in [LaneState::Building, LaneState::Review, LaneState::Landing] {
        store.update_lane(c.id, &LaneUpdate { state: Some(s), ..Default::default() }, Actor::Manager).unwrap();
    }
    let behind = store.train_named(w.workspace, store.lane(c.id).unwrap().train.as_deref().unwrap()).unwrap();
    assert_ne!(behind.id, open.id);
    assert_eq!(behind.state, TrainState::Integrating);
}
