//! The landing job against real git: a bare repository in a temporary folder
//! stands in for the remote, so nothing here reaches a real one.

use std::collections::VecDeque;
use std::process::Command;
use std::sync::Mutex;

use farcooler_store::board_ci::{CiRead, MAIN_SUBJECT};
use farcooler_store::ticket_lines::{LineUpdate, NewTicketLine, Stage};

use super::*;
use farcooler_core::charter_landing::parse;

/// CI that says what it's told, in order, then the last thing forever.
struct FakeCi {
    said: Mutex<VecDeque<CiStatus>>,
    reruns: Mutex<u32>,
}

impl FakeCi {
    fn saying(said: &[CiStatus]) -> FakeCi {
        FakeCi { said: Mutex::new(said.iter().copied().collect()), reruns: Mutex::new(0) }
    }
}

impl Ci for FakeCi {
    async fn status(&self, _sha: &str) -> CiStatus {
        let mut said = self.said.lock().unwrap();
        if said.len() > 1 { said.pop_front().unwrap() } else { said.front().copied().unwrap_or(CiStatus::Passed) }
    }
    async fn rerun(&self, _sha: &str) -> bool {
        *self.reruns.lock().unwrap() += 1;
        true
    }
}

/// A repository with a bare remote, a board on it, and tickets on branches.
struct Rig {
    _dir: tempfile::TempDir,
    remote: PathBuf,
    repo: PathBuf,
    scratch: PathBuf,
    store: Store,
    board: Uuid,
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, path: &str, text: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let (remote, repo, scratch) = (dir.path().join("remote.git"), dir.path().join("repo"), dir.path().join("scratch"));
        sh(dir.path(), &["init", "--quiet", "--bare", "--initial-branch=main", &remote.to_string_lossy()]);
        sh(dir.path(), &["init", "--quiet", "--initial-branch=main", &repo.to_string_lossy()]);
        for (k, v) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false")] {
            sh(&repo, &["config", k, v]);
        }
        sh(&repo, &["remote", "add", "origin", &remote.to_string_lossy()]);
        write(&repo, "shared.txt", "one\n");
        write(&repo, "services/site/index.html", "<p>hi</p>\n");
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "--quiet", "-m", "start"]);
        sh(&repo, &["push", "--quiet", "origin", "main"]);
        let store = Store::open_in_memory().unwrap();
        let root = store.create_repository_root(Uuid::now_v7(), &repo.to_string_lossy(), 0).unwrap();
        let r = store.create_repository(root.host_id, root.id, "repo", &format!("{}/.git", repo.display()), "").unwrap();
        let board = store.ensure_main_workspace(r.id).unwrap().id;
        // Main's CI read red holds the board's collecting landing back, so the
        // runner doesn't send it off on its own while a test sets up.
        let red = CiRead {
            subject: MAIN_SUBJECT.into(),
            sha: "0000000".into(),
            status: CiStatus::Failed,
            url: String::new(),
            jobs: vec![],
            fetched_at: 0,
            changed_at: 0,
            asked_at: 0,
        };
        store.record_ci(board, &red).unwrap();
        Rig { _dir: dir, remote, repo, scratch, store, board }
    }

    /// A ticket ready to land, its branch off main with one commit writing
    /// `files`.
    fn ticket(&self, branch: &str, files: &[(&str, &str)]) -> Uuid {
        sh(&self.repo, &["branch", branch, "main"]);
        sh(&self.repo, &["switch", "--quiet", branch]);
        for (path, text) in files {
            write(&self.repo, path, text);
        }
        sh(&self.repo, &["add", "-A"]);
        sh(&self.repo, &["commit", "--quiet", "-m", branch]);
        sh(&self.repo, &["switch", "--quiet", "main"]);
        let task = self.store.create_task(self.board, branch, Actor::Manager).unwrap().id;
        let new = NewTicketLine { branch: branch.into(), ..Default::default() };
        self.store.put_on_line(task, &new, Actor::Manager).unwrap();
        for stage in [Stage::Building, Stage::Reviewing, Stage::Landing] {
            self.store.update_line(task, &LineUpdate { stage: Some(stage), ..Default::default() }, Actor::Manager).unwrap();
        }
        task
    }

    fn landing(&self, tickets: &[Uuid]) -> Landing {
        prepare(&self.store, self.board, tickets, "origin/main", "", None, false, Actor::Manager).unwrap()
    }

    async fn run(&self, charter: &str, ci: &FakeCi, landing: &Landing) -> Vec<Outcome> {
        let announce = |_: Uuid| {};
        let setup = Setup {
            store: &self.store,
            repo: self.repo.clone(),
            scratch: self.scratch.clone(),
            rules: parse(charter).unwrap().unwrap(),
            poll: Duration::from_millis(1),
            ci_patience: Duration::from_secs(5),
            gate_timeout: Duration::from_secs(30),
            announce: &announce,
        };
        run(&setup, ci, landing.id, false).await
    }

    /// The remote's main, as a list of subjects, newest first.
    fn remote_log(&self) -> Vec<String> {
        sh(&self.remote, &["log", "--format=%s", "main"]).lines().map(str::to_string).collect()
    }

    fn remote_head(&self) -> String {
        sh(&self.remote, &["rev-parse", "main"])
    }

    fn stage(&self, task: Uuid) -> Stage {
        self.store.ticket_line(task).unwrap().stage
    }
}

const PUSH: &str = "## Landing\n- Route: push\n- Route `services/site/**`: ask\n- Gate: test ! -e bad.txt\n- CI: github\n";

/// The normal case: one ticket, folded, gated, pushed to the bare remote,
/// CI green, and landed at the pushed SHA, ticket and landing both.
#[tokio::test]
async fn one_ticket_is_pushed_to_the_remote_and_lands() {
    let rig = Rig::new();
    let t = rig.ticket("t1", &[("a.txt", "a\n")]);
    let l = rig.landing(&[t]);
    let ci = FakeCi::saying(&[CiStatus::Running, CiStatus::Passed]);
    let out = rig.run(PUSH, &ci, &l).await;
    let sha = rig.remote_head();
    assert_eq!(out, vec![Outcome::Landed { landing: l.number, tickets: vec!["re-1".into()], sha: sha.clone() }]);
    assert_eq!(rig.remote_log(), ["t1", "start"]);
    let landed = rig.store.landing(l.id).unwrap();
    assert_eq!((landed.state, landed.pushed_sha.as_deref()), (LandingState::Landed, Some(sha.as_str())));
    let line = rig.store.ticket_line(t).unwrap();
    assert_eq!((line.stage, line.landed_sha.as_deref()), (Stage::Landed, Some(sha.as_str())));
    assert!(!rig.scratch.join(format!("landing-{}", l.number)).exists(), "its worktree goes once it lands");
}

/// A ticket that touches the marketing site waits for you; one that doesn't
/// is pushed without it.
#[tokio::test]
async fn a_site_ticket_waits_for_you_and_the_rest_is_pushed() {
    let rig = Rig::new();
    let code = rig.ticket("code", &[("a.txt", "a\n")]);
    let site = rig.ticket("site", &[("services/site/index.html", "<p>new</p>\n")]);
    let l = rig.landing(&[code, site]);
    let out = rig.run(PUSH, &FakeCi::saying(&[CiStatus::Passed]), &l).await;
    let asked = rig.store.landing_of(site).unwrap().expect("the site ticket is on a landing");
    assert_eq!(out[0], Outcome::Asked { landing: asked.number, tickets: vec!["re-2".into()] });
    assert!(matches!(&out[1], Outcome::Landed { tickets, .. } if tickets == &["re-1".to_string()]), "{out:?}");
    assert_eq!(asked.route, Route::Ask);
    assert_eq!(rig.stage(site), Stage::NeedsYou);
    assert!(rig.store.ticket_line(site).unwrap().reason.contains("services/site/**"));
    assert_eq!(rig.remote_log(), ["code", "start"], "the site change wasn't pushed");

    // You say land it: the same job pushes it.
    let told = prepare(&rig.store, rig.board, &[site], "origin/main", "", None, true, Actor::User).unwrap();
    let setup_out = {
        let announce = |_: Uuid| {};
        let setup = Setup {
            store: &rig.store,
            repo: rig.repo.clone(),
            scratch: rig.scratch.clone(),
            rules: parse(PUSH).unwrap().unwrap(),
            poll: Duration::from_millis(1),
            ci_patience: Duration::from_secs(5),
            gate_timeout: Duration::from_secs(30),
            announce: &announce,
        };
        run(&setup, &FakeCi::saying(&[CiStatus::Passed]), told.id, true).await
    };
    assert!(matches!(&setup_out[..], [Outcome::Landed { .. }]), "{setup_out:?}");
    assert_eq!(rig.stage(site), Stage::Landed);
    assert_eq!(rig.remote_log(), ["site", "code", "start"]);
}

/// Without your word, a ticket the charter asked about can't be started again.
#[tokio::test]
async fn an_asked_ticket_needs_your_word() {
    let rig = Rig::new();
    let site = rig.ticket("site", &[("services/site/index.html", "<p>new</p>\n")]);
    let l = rig.landing(&[site]);
    rig.run(PUSH, &FakeCi::saying(&[CiStatus::Passed]), &l).await;
    let again = prepare(&rig.store, rig.board, &[site], "origin/main", "", None, false, Actor::Manager);
    assert!(matches!(again, Err(DomainError::InvalidArgument { what: "waits_for_you" })), "{again:?}");
}

/// A batch whose gate fails splits, and the good half lands while the bad
/// ticket stops on its own landing.
#[tokio::test]
async fn a_failed_batch_splits_and_lands_the_good_half() {
    let rig = Rig::new();
    let good1 = rig.ticket("good1", &[("a.txt", "a\n")]);
    let good2 = rig.ticket("good2", &[("b.txt", "b\n")]);
    let bad = rig.ticket("bad", &[("bad.txt", "bad\n")]);
    let l = rig.landing(&[good1, good2, bad]);
    let out = rig.run(PUSH, &FakeCi::saying(&[CiStatus::Passed]), &l).await;
    let landed: Vec<&Vec<String>> = out.iter().filter_map(|o| if let Outcome::Landed { tickets, .. } = o { Some(tickets) } else { None }).collect();
    let stopped: Vec<&Vec<String>> = out.iter().filter_map(|o| if let Outcome::Stopped { tickets, .. } = o { Some(tickets) } else { None }).collect();
    assert_eq!(landed.into_iter().flatten().cloned().collect::<Vec<_>>(), ["re-1", "re-2"], "{out:?}");
    assert_eq!(stopped, [&vec!["re-3".to_string()]], "{out:?}");
    assert_eq!((rig.stage(good1), rig.stage(good2), rig.stage(bad)), (Stage::Landed, Stage::Landed, Stage::Landing));
    let log = rig.remote_log();
    assert!(log.contains(&"good1".to_string()) && log.contains(&"good2".to_string()) && !log.contains(&"bad".to_string()), "{log:?}");
    let left = rig.store.landing_of(bad).unwrap().expect("the bad ticket keeps a landing");
    assert_eq!(left.state, LandingState::Open, "stopped, for whoever takes it from here");
}

/// A conflict stops the job before anything is pushed, and leaves nothing
/// half-picked.
#[tokio::test]
async fn a_conflict_stops_cleanly() {
    let rig = Rig::new();
    let first = rig.ticket("first", &[("shared.txt", "first\n")]);
    let second = rig.ticket("second", &[("shared.txt", "second\n")]);
    let before = rig.remote_head();
    let l = rig.landing(&[first, second]);
    let out = rig.run(PUSH, &FakeCi::saying(&[CiStatus::Passed]), &l).await;
    let [Outcome::Stopped { why, .. }] = &out[..] else { panic!("{out:?}") };
    assert!(why.contains("re-2 conflicts at") && why.contains("shared.txt"), "{why}");
    assert_eq!(rig.remote_head(), before, "nothing was pushed");
    assert_eq!(rig.store.landing(l.id).unwrap().state, LandingState::Open);
    assert_eq!((rig.stage(first), rig.stage(second)), (Stage::Landing, Stage::Landing));
    let tree = rig.scratch.join(format!("landing-{}", l.number));
    assert_eq!(sh(&tree, &["status", "--porcelain"]), "", "the pick was aborted");
    let events = rig.store.line_events(farcooler_store::ticket_lines::LineSubject::Landing(l.id), 0).unwrap();
    assert!(events.iter().any(|e| e.body.starts_with("Stopped. re-2 conflicts")), "the timeline says why");
}

/// Red CI on the push marks the landing red and stops; a canceled run is run
/// again once, and a second cancel stops.
#[tokio::test]
async fn red_ci_stops_and_a_canceled_run_is_run_again_once() {
    let rig = Rig::new();
    let t = rig.ticket("t1", &[("a.txt", "a\n")]);
    let l = rig.landing(&[t]);
    // The base passes, then the push is canceled, run again, and fails.
    let ci = FakeCi::saying(&[CiStatus::Passed, CiStatus::Superseded, CiStatus::Running, CiStatus::Failed]);
    let out = rig.run(PUSH, &ci, &l).await;
    assert!(matches!(&out[..], [Outcome::Stopped { why, .. }] if why.contains("failed")), "{out:?}");
    assert_eq!(*ci.reruns.lock().unwrap(), 1);
    assert_eq!(rig.store.landing(l.id).unwrap().state, LandingState::Red);

    let rig = Rig::new();
    let t = rig.ticket("t1", &[("a.txt", "a\n")]);
    let l = rig.landing(&[t]);
    let ci = FakeCi::saying(&[CiStatus::Passed, CiStatus::Superseded, CiStatus::Superseded]);
    let out = rig.run(PUSH, &ci, &l).await;
    assert!(matches!(&out[..], [Outcome::Stopped { why, .. }] if why.contains("canceled twice")), "{out:?}");
}

/// A red base stops the job before it folds anything.
#[tokio::test]
async fn a_red_base_stops_the_job() {
    let rig = Rig::new();
    let t = rig.ticket("t1", &[("a.txt", "a\n")]);
    let before = rig.remote_head();
    let l = rig.landing(&[t]);
    let out = rig.run(PUSH, &FakeCi::saying(&[CiStatus::Failed]), &l).await;
    assert!(matches!(&out[..], [Outcome::Stopped { why, .. }] if why.starts_with("CI on origin/main")), "{out:?}");
    assert_eq!(rig.remote_head(), before);
}

/// A board's collecting landing that left for its gate on its own is taken
/// up by the job, not refused as moving.
#[tokio::test]
async fn a_departed_landing_is_taken_up() {
    let rig = Rig::new();
    let t = rig.ticket("t1", &[("a.txt", "a\n")]);
    // Main's CI turns green: the collecting landing departs with the ticket.
    let green = CiRead {
        subject: MAIN_SUBJECT.into(),
        sha: "0000001".into(),
        status: CiStatus::Passed,
        url: String::new(),
        jobs: vec![],
        fetched_at: 1,
        changed_at: 1,
        asked_at: 1,
    };
    rig.store.record_ci(rig.board, &green).unwrap();
    rig.store.collect_ready_tickets(rig.board, Actor::Runner).unwrap();
    let departed = rig.store.landing_of(t).unwrap().expect("collected");
    assert_eq!(departed.state, LandingState::Gating);
    let l = rig.landing(&[t]);
    let out = rig.run(PUSH, &FakeCi::saying(&[CiStatus::Passed]), &l).await;
    assert!(matches!(&out[..], [Outcome::Landed { .. }]), "{out:?}");
    let old = rig.store.landing(departed.id).unwrap();
    assert!(old.id == l.id || old.state == LandingState::Dropped, "no empty landing is left behind: {old:?}");
}

#[tokio::test]
async fn an_unreadable_branch_never_defaults_to_push() {
    let rig = Rig::new();
    assert!(changed_paths(&rig.repo, "origin/main", "missing-branch").await.is_err());
}

#[tokio::test]
async fn changed_paths_preserves_git_path_bytes() {
    let rig = Rig::new();
    rig.ticket("odd-path", &[("services/site/line\nbreak.html", "new\n")]);
    let paths = changed_paths(&rig.repo, "origin/main", "odd-path").await.unwrap();
    assert_eq!(paths, vec!["services/site/line\nbreak.html"]);
}

#[tokio::test]
async fn retrying_keeps_the_previous_scratch_worktree() {
    let rig = Rig::new();
    let t = rig.ticket("retry", &[("a.txt", "a\n")]);
    let l = rig.landing(&[t]);
    let announce = |_: Uuid| {};
    let setup = Setup { store: &rig.store, repo: rig.repo.clone(), scratch: rig.scratch.clone(),
        rules: parse(PUSH).unwrap().unwrap(), poll: Duration::from_millis(1),
        ci_patience: Duration::from_secs(5), gate_timeout: Duration::from_secs(30), announce: &announce };
    let sha = rig.remote_head();
    let first = worktree_for(&setup, &l, &sha).await.ok().unwrap();
    write(&first, "previous-report.txt", "keep me\n");
    let second = worktree_for(&setup, &l, &sha).await.ok().unwrap();
    assert_ne!(first, second);
    assert_eq!(std::fs::read_to_string(first.join("previous-report.txt")).unwrap(), "keep me\n");
}

#[tokio::test]
async fn a_moved_base_is_never_pushed_without_a_new_gate() {
    let rig = Rig::new();
    let t = rig.ticket("gated", &[("a.txt", "a\n")]);
    let l = rig.landing(&[t]);
    let announce = |_: Uuid| {};
    let setup = Setup { store: &rig.store, repo: rig.repo.clone(), scratch: rig.scratch.clone(),
        rules: parse(PUSH).unwrap().unwrap(), poll: Duration::from_millis(1),
        ci_patience: Duration::from_secs(5), gate_timeout: Duration::from_secs(30), announce: &announce };
    let sha = rig.remote_head();
    let tree = worktree_for(&setup, &l, &sha).await.unwrap();
    fold(&setup, &l, &tree, &sha).await.unwrap();
    rig.ticket("advance", &[("b.txt", "b\n")]);
    sh(&rig.repo, &["push", "--quiet", "origin", "advance:main"]);
    assert!(push(&setup, &l, &tree, "origin/main", &sha).await.is_err());
    assert_eq!(rig.remote_log(), ["advance", "start"]);
}
