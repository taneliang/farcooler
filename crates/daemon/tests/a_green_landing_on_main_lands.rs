//! A green landing whose SHA is on its base branch lands itself, and its
//! tickets with it (ov-503, ov-872): the CI watch's last step, run against a real git
//! repository and the store behind the in-process daemon.

#[path = "support/in_process.rs"]
mod in_process;

use std::path::{Path, PathBuf};
use std::process::Command;

use farcooler_daemon::green_landing::land_green_landings;
use farcooler_protocol::v1::Scope;
use farcooler_store::board_ci::{CiRead, CiStatus, MAIN_SUBJECT};
use farcooler_store::landing::{LandingMode, LandingPatch};
use farcooler_store::landings::{LandingState, LandingUpdate, NewLanding, Route};
use farcooler_store::models::Actor;
use farcooler_store::ticket_lines::{LineUpdate, NewTicketLine, Stage};
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
    /// A ticket that has passed review: in the landing stage, which puts it on
    /// the collecting landing.
    fn reviewed(&self, title: &str) -> Uuid {
        let store = &self.h.service.store;
        let task = store.create_task(self.workspace, title, Actor::User).unwrap().id;
        store.put_on_line(task, &NewTicketLine::default(), Actor::Manager).unwrap();
        for s in [Stage::Building, Stage::Reviewing, Stage::Landing] {
            store.update_line(task, &LineUpdate { stage: Some(s), ..Default::default() }, Actor::Manager).unwrap();
        }
        task
    }

    /// Main's CI red, so the collecting landing collects and doesn't leave on
    /// its own.
    fn hold(&self) {
        let read = CiRead {
            subject: MAIN_SUBJECT.into(),
            sha: "0000000".into(),
            status: CiStatus::Failed,
            url: String::new(),
            jobs: vec![],
            fetched_at: 0,
            changed_at: 0,
            asked_at: 0,
        };
        self.h.service.store.record_ci(self.workspace, &read).unwrap();
    }

    /// A landing on `sha` carrying `tickets` new reviewed tickets, in `state`.
    fn landing(&self, sha: &str, state: LandingState, tickets: &[&str]) -> (Uuid, Vec<Uuid>) {
        let store = &self.h.service.store;
        self.hold();
        let ids: Vec<Uuid> = tickets.iter().map(|t| self.reviewed(t)).collect();
        let new = NewLanding { route: Route::Push, base: "origin/main".into(), title: String::new(), lander: None, collecting: false };
        let l = store.start_landing(self.workspace, &new, &ids, Actor::Manager).unwrap();
        let up = LandingUpdate { state: Some(state), sha: Some(sha.into()), ..Default::default() };
        store.update_landing(l.id, &up, Actor::Manager).unwrap();
        (l.id, ids)
    }

    fn state(&self, landing: Uuid) -> LandingState {
        self.h.service.store.landing(landing).unwrap().state
    }

    fn stage(&self, task: Uuid) -> Stage {
        self.h.service.store.ticket_line(task).unwrap().stage
    }

    async fn land(&self) -> Vec<Uuid> {
        land_green_landings(&self.h.service, &self.h.watcher).await
    }
}

/// Green, and on main: it lands, and its tickets land with its SHA.
#[tokio::test]
async fn a_green_landing_on_main_lands_with_its_tickets() {
    let w = a_world().await;
    let (landing, tickets) = w.landing(&w.tree.on_main, LandingState::Green, &["a", "b"]);
    assert_eq!(w.land().await, vec![landing]);
    assert_eq!(w.state(landing), LandingState::Landed);
    for t in tickets {
        let line = w.h.service.store.ticket_line(t).unwrap();
        assert_eq!((line.stage, line.landed_sha.as_deref()), (Stage::Landed, Some(w.tree.on_main.as_str())));
    }
}

/// Green but not on main yet (not pushed, or this clone has not seen the push):
/// it waits. Neither does a landing whose CI has not passed land, however much
/// of main it is part of.
#[tokio::test]
async fn a_landing_not_on_main_or_not_green_waits() {
    let w = a_world().await;
    let (ahead, tickets) = w.landing(&w.tree.ahead, LandingState::Green, &["a"]);
    let (red, _) = w.landing(&w.tree.on_main, LandingState::Red, &["b"]);
    let (pushed, _) = w.landing(&w.tree.on_main, LandingState::Pushed, &["c"]);
    assert_eq!(w.land().await, Vec::<Uuid>::new());
    assert_eq!((w.state(ahead), w.state(red), w.state(pushed)), (LandingState::Green, LandingState::Red, LandingState::Pushed));
    assert_eq!(w.stage(tickets[0]), Stage::Landing);
    // Once main has it, the next turn lands it.
    git(&w.tree.path, &["update-ref", "refs/remotes/origin/main", &w.tree.ahead]);
    assert_eq!(w.land().await, vec![ahead]);
}

/// In pull-request mode a landing is a rehearsal: it stays however green.
#[tokio::test]
async fn a_landing_in_pull_request_mode_is_left_alone() {
    let w = a_world().await;
    let store = &w.h.service.store;
    let version = store.get_workspace(w.workspace).unwrap().resource_version;
    store.set_landing(w.workspace, version, &LandingPatch { mode: Some(LandingMode::PullRequests), ..Default::default() }).unwrap();
    let (landing, tickets) = w.landing(&w.tree.on_main, LandingState::Green, &["a"]);
    assert_eq!(w.land().await, Vec::<Uuid>::new());
    assert_eq!(w.state(landing), LandingState::Green);
    assert_eq!(w.stage(tickets[0]), Stage::Landing);
}

/// The CI watch's own turn does it, with no read of GitHub needed to say so: a
/// landing already green when the turn begins lands in that turn.
#[tokio::test]
async fn the_watch_s_turn_lands_it() {
    let w = a_world().await;
    let (landing, tickets) = w.landing(&w.tree.on_main, LandingState::Green, &["a"]);
    farcooler_daemon::ci_watch::read_all(&w.h.service, &w.h.watcher, &mut farcooler_daemon::ci_watch::Memo::default()).await;
    assert_eq!(w.state(landing), LandingState::Landed);
    assert_eq!(w.stage(tickets[0]), Stage::Landed);
}

/// A repository path that is gone lands nothing, and says so once, not on every
/// turn; it says so again after the path works and breaks again.
#[tokio::test]
async fn a_missing_repository_lands_nothing_and_is_logged_once() {
    use farcooler_daemon::green_landing::note_check;
    let w = a_world().await;
    let (landing, tickets) = w.landing(&w.tree.on_main, LandingState::Green, &["a"]);
    std::fs::remove_dir_all(&w.tree.path).unwrap();
    assert_eq!(w.land().await, Vec::<Uuid>::new());
    assert_eq!(w.land().await, Vec::<Uuid>::new());
    assert_eq!(w.state(landing), LandingState::Green);
    assert_eq!(w.stage(tickets[0]), Stage::Landing);
    // The first turn already noted the failure, so the next is not news.
    assert!(!note_check(landing, false), "already logged");
    assert!(!note_check(landing, true));
    assert!(note_check(landing, false), "news again after it worked");
}

/// The landing ahead reaching main sends the collecting landing off for its
/// checks (ov-520), and a new collecting landing is waiting behind it.
#[tokio::test]
async fn a_landing_reaching_main_sends_the_collecting_landing_off_for_its_checks() {
    let w = a_world().await;
    let store = &w.h.service.store;
    // Main isn't red, so the first reviewed ticket's landing leaves at once.
    let a = w.reviewed("a");
    let ahead = store.landing_of(a).unwrap().expect("a joined the collecting landing");
    assert_eq!(ahead.state, LandingState::Gating, "nothing was ahead of it");
    let up = LandingUpdate { state: Some(LandingState::Green), sha: Some(w.tree.on_main.clone()), ..Default::default() };
    store.update_landing(ahead.id, &up, Actor::Manager).unwrap();
    // A ticket that passes review while the landing ahead is out joins the
    // collecting landing behind it.
    let b = w.reviewed("b");
    let open = store.landing_of(b).unwrap().expect("b joined the collecting landing");
    assert!(open.collecting && open.state == LandingState::Open, "{open:?}");
    assert_eq!(w.land().await, vec![ahead.id]);
    assert_eq!(w.state(open.id), LandingState::Gating, "it started its checks when the landing ahead landed");
    let plan = store.plan(w.workspace, 0).unwrap();
    let waiting: Vec<_> = plan.landings.iter().filter(|l| l.landing.state == LandingState::Open).collect();
    assert_eq!(waiting.len(), 0, "the one behind it is empty, so the plan read leaves it out");
    // A ticket that passes review now joins it.
    let c = w.reviewed("c");
    let behind = store.landing_of(c).unwrap().expect("c joined the collecting landing");
    assert_ne!(behind.id, open.id);
    assert_eq!(behind.state, LandingState::Open);
}

/// The watch settles only the tickets carried by the green SHA. A ready
/// ticket behind it cannot join while CI is finishing.
#[tokio::test]
async fn green_landing_membership_cannot_absorb_unrouted_tickets() {
    let w = a_world().await;
    let (landing, tickets) = w.landing(&w.tree.on_main, LandingState::Green, &["routed"]);
    let behind = w.reviewed("unrouted site work");
    let store = &w.h.service.store;
    let collecting = store.landing_of(behind).unwrap().unwrap();
    assert!(collecting.collecting && collecting.state == LandingState::Open);
    let add = LandingUpdate { add_tickets: vec![behind], ..Default::default() };
    let result = store.update_landing(landing, &add, Actor::Manager);
    // Exercise the actual watch settlement even on the broken store.
    assert_eq!(w.land().await, vec![landing]);
    assert_eq!(w.stage(tickets[0]), Stage::Landed);
    assert_eq!(w.stage(behind), Stage::Landing, "the watch certified work absent from the pushed SHA");
    assert!(matches!(result, Err(farcooler_core::DomainError::InvalidArgument { what: "landing_moving" })), "{result:?}");
    assert_eq!(store.landing_of(behind).unwrap().unwrap().id, collecting.id);
}

/// After release landing is the engine's: a green landing on main is left alone
/// (ov-947). The landing fences are lifted so only the gate can keep it out.
#[tokio::test]
async fn a_released_board_lands_nothing_by_hand() {
    let w = a_world().await;
    let (landing, tickets) = w.landing(&w.tree.on_main, LandingState::Green, &["a"]);
    farcooler_store::testing::exec_for_tests(&w.h.service.store, "UPDATE engine_cutover_authority SET phase='released'");
    for table in ["landings", "landing_tickets"] {
        for suffix in ["i", "u", "d"] {
            farcooler_store::testing::exec_for_tests(&w.h.service.store, &format!("DROP TRIGGER engine_fence_{table}_{suffix}"));
        }
    }
    assert_eq!(w.land().await, Vec::<Uuid>::new());
    assert_eq!(w.state(landing), LandingState::Green);
    assert_eq!(w.stage(tickets[0]), Stage::Landing);
}
