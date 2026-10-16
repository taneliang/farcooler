//! The nudge (ov-580), end to end through a real service: off unless the
//! workspace opts in, once per pull request however often the watch turns or
//! the runner restarts, never at night, and never the owner's own PR to the
//! owner. `gh` is a recorder of the test's own; the store is the real one.

#[path = "support/in_process.rs"]
mod in_process;

use std::collections::BTreeSet;
use std::sync::Mutex;

use farcooler_daemon::pr_nudge::{Poster, Posted, nudge_repository};
use farcooler_daemon::pr_timing::{Moment, PrTiming};
use farcooler_daemon::stack::{CheckState, PrInfo, PrReview, PrState, PrStatus, ReviewDecision};
use farcooler_protocol::v1::Scope;
use farcooler_store::landing::LandingPatch;
use farcooler_store::models::Actor;
use farcooler_store::plan::{LaneCard, LaneState, LaneUpdate, NewLane};
use in_process::*;
use uuid::Uuid;

const HOUR: i64 = 3_600_000;
const FROM: i64 = 1_790_000_000_000;

/// The recorder: who `gh` says it posts as, what was posted, and how the next
/// post ends.
struct Recorder {
    owner: Option<&'static str>,
    ends: Posted,
    sent: Mutex<Vec<(String, String)>>,
}

impl Recorder {
    fn new(ends: Posted) -> Recorder {
        Recorder { owner: Some("owner"), ends, sent: Mutex::new(vec![]) }
    }
    fn sent(&self) -> Vec<(String, String)> {
        self.sent.lock().unwrap().clone()
    }
}

impl Poster for Recorder {
    async fn owner(&self, _repository: Uuid) -> Option<String> {
        self.owner.map(str::to_string)
    }
    async fn post(&self, _repository: Uuid, url: &str, body: &str) -> Posted {
        self.sent.lock().unwrap().push((url.to_string(), body.to_string()));
        self.ends
    }
}

/// A moment at `hour` o'clock local time, found from `from` on.
fn first_at(hour: u32, from: i64) -> i64 {
    (from..from + 48 * HOUR).step_by(HOUR as usize).find(|t| farcooler_core::local_time::hour(*t) == hour).unwrap()
}

/// When the pull request became ready: noon, local time, so that `ready() + n
/// hours` is at 12:00 + n hours on the clock, which a test can pick to be day
/// or night.
fn ready() -> i64 {
    first_at(12, FROM)
}

/// A daytime moment a day or more after ready, at local `hour`.
fn at(hour: u32) -> i64 {
    first_at(hour, ready() + 24 * HOUR)
}

const URL: &str = "https://github.example/o/r/pull/31";
const KEY: &str = "github.example/o/r";

struct World {
    h: Harness,
    repo: Repo,
    boards: BTreeSet<Uuid>,
}

/// A runner with a lane in review whose pull request is ready at `ready()`,
/// asks `asks` (users), was opened by `author`, and no one has reviewed; its
/// timing read counts as fresh at whatever moment a test asks. Not opted in.
async fn world(asks: &[&str], author: &str) -> World {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let task = store.create_task(repo.workspace, "Waits for a review", Actor::User).unwrap();
    let key = store.get_task(task.id).unwrap().key;
    let new = NewLane { name: "review".into(), reason: "Because".into(), branch: "lane/review".into(), ..Default::default() };
    let lane = store.create_lane(repo.workspace, &new, &[LaneCard { task_id: task.id, slice: String::new() }], None, Actor::Manager).unwrap();
    for state in [LaneState::Building, LaneState::Review] {
        store.update_lane(lane.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager).unwrap();
    }
    let pr = PrInfo {
        head_ref: format!("{key}/work"),
        base_ref: "main".into(),
        title: format!("{key}: work"),
        is_fork: false,
        author: author.into(),
        review: PrReview {
            requested: asks.iter().map(|a| a.to_string()).collect(),
            requested_users: asks.iter().map(|a| a.to_string()).collect(),
            ..Default::default()
        },
        timing: PrTiming { ready: Moment::At(ready()), first_review: Moment::NotYet, read_at: i64::MAX / 2, ..Default::default() },
        status: PrStatus {
            number: 31,
            url: URL.into(),
            state: PrState::Open,
            checks: CheckState::Unknown,
            review_decision: ReviewDecision::Unknown,
            head_oid: "abc".into(),
            merged_at: None,
            fetched_at: 0,
        },
    };
    h.service.pr_cache_put(repo.id, Some(vec![pr]));
    let boards = BTreeSet::from([repo.workspace]);
    World { h, repo, boards }
}

impl World {
    fn opt_in(&self, patch: LandingPatch) {
        let store = &self.h.service.store;
        let version = store.get_workspace(self.repo.workspace).unwrap().resource_version;
        store.set_landing(self.repo.workspace, version, &patch).unwrap();
    }

    async fn turn(&self, poster: &Recorder, now: i64) -> Vec<u32> {
        nudge_repository(&self.h.service, self.repo.id, &self.boards, poster, &|| now).await
    }

    fn nudged(&self) -> bool {
        self.h.service.store.pr_nudged(KEY, 31).unwrap()
    }

    /// Change the cached PR.
    fn edit(&self, f: impl FnOnce(&mut PrInfo)) {
        let mut prs = self.h.service.pr_cache_get(self.repo.id).unwrap();
        f(&mut prs[0]);
        self.h.service.pr_cache_put(self.repo.id, Some(prs));
    }
}

fn on() -> LandingPatch {
    LandingPatch { pr_nudge: Some(true), ..Default::default() }
}

const WORDS: &str = "@alice, a gentle nudge: this is ready for your review whenever you have a moment.";

#[tokio::test]
async fn nothing_is_posted_unless_the_workspace_opted_in() {
    let w = world(&["alice"], "owner").await;
    let rec = Recorder::new(Posted::Yes);
    assert!(w.turn(&rec, ready() + 30 * HOUR).await.is_empty());
    assert!(rec.sent().is_empty(), "off by default: it posts as the owner");
    assert!(!w.nudged());
    // Turned on, it goes; turned off again, the next one does not.
    w.opt_in(on());
    assert_eq!(w.turn(&rec, ready() + 30 * HOUR).await, [31]);
    let w2 = world(&["alice"], "owner").await;
    w2.opt_in(on());
    w2.opt_in(LandingPatch { pr_nudge: Some(false), ..Default::default() });
    assert!(w2.turn(&Recorder::new(Posted::Yes), ready() + 30 * HOUR).await.is_empty());
}

#[tokio::test]
async fn it_is_sent_once_to_the_pull_request_s_own_url_and_never_again_however_often_the_watch_turns() {
    let w = world(&["alice"], "owner").await;
    w.opt_in(on());
    let rec = Recorder::new(Posted::Yes);
    assert_eq!(w.turn(&rec, ready() + 30 * HOUR).await, [31]);
    assert_eq!(rec.sent(), [(URL.to_string(), WORDS.to_string())]);
    for later in [32, 56, 480] {
        assert!(w.turn(&rec, ready() + later * HOUR).await.is_empty(), "{later} hours on");
    }
    assert_eq!(rec.sent().len(), 1);
    assert!(w.nudged(), "the record is in the store, which a restart keeps");
}

#[tokio::test]
async fn the_delay_is_the_workspace_s_and_the_default_is_a_day() {
    let w = world(&["alice"], "owner").await;
    w.opt_in(on());
    let rec = Recorder::new(Posted::Yes);
    assert!(w.turn(&rec, ready() + 20 * HOUR).await.is_empty(), "twenty hours (08:00) is under a day");
    assert!(w.turn(&rec, ready() + 24 * HOUR - 1).await.is_empty());
    assert_eq!(w.turn(&rec, ready() + 24 * HOUR).await, [31]);
    let slow = world(&["alice"], "owner").await;
    slow.opt_in(LandingPatch { pr_nudge: Some(true), pr_nudge_hours: Some(72), ..Default::default() });
    let rec = Recorder::new(Posted::Yes);
    assert!(slow.turn(&rec, ready() + 30 * HOUR).await.is_empty());
    assert!(slow.turn(&rec, ready() + 71 * HOUR).await.is_empty());
    assert_eq!(slow.turn(&rec, ready() + 72 * HOUR).await, [31]);
}

#[tokio::test]
async fn nothing_goes_between_ten_at_night_and_eight_in_the_morning_and_the_next_turn_after_does() {
    let w = world(&["alice"], "owner").await;
    w.opt_in(on());
    let rec = Recorder::new(Posted::Yes);
    for hour in [22, 23, 3, 7] {
        assert!(w.turn(&rec, at(hour)).await.is_empty(), "{hour}:00");
    }
    assert!(rec.sent().is_empty());
    assert!(!w.nudged(), "a quiet turn claims nothing");
    assert_eq!(w.turn(&rec, at(8)).await, [31], "08:00 is the first hour");
}

#[tokio::test]
async fn the_quiet_hours_are_checked_again_right_before_the_post() {
    let w = world(&["alice"], "owner").await;
    w.opt_in(on());
    // The turn starts at 21:00; asking GitHub who the owner is takes until 22:00.
    let evening = at(21);
    let rec = Recorder::new(Posted::Yes);
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let clock_flag = flag.clone();
    struct Slow(Recorder, std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Poster for Slow {
        async fn owner(&self, r: Uuid) -> Option<String> {
            self.1.store(true, std::sync::atomic::Ordering::SeqCst);
            self.0.owner(r).await
        }
        async fn post(&self, r: Uuid, url: &str, body: &str) -> Posted {
            self.0.post(r, url, body).await
        }
    }
    let slow = Slow(rec, flag);
    let clock = move || if clock_flag.load(std::sync::atomic::Ordering::SeqCst) { evening + HOUR } else { evening };
    let sent = nudge_repository(&w.h.service, w.repo.id, &w.boards, &slow, &clock).await;
    assert!(sent.is_empty() && slow.0.sent().is_empty(), "22:00 came before the post");
    assert!(!w.nudged());
}

#[tokio::test]
async fn the_owner_who_is_the_author_is_not_nudged_and_a_teammate_beside_them_is() {
    let alone = world(&["owner"], "owner").await;
    alone.opt_in(on());
    let rec = Recorder::new(Posted::Yes);
    assert!(alone.turn(&rec, ready() + 30 * HOUR).await.is_empty());
    assert!(rec.sent().is_empty());
    assert!(!alone.nudged(), "no claim for nobody");

    let both = world(&["Owner", "alice"], "owner").await;
    both.opt_in(on());
    let rec = Recorder::new(Posted::Yes);
    assert_eq!(both.turn(&rec, ready() + 30 * HOUR).await, [31]);
    assert!(rec.sent()[0].1.starts_with("@alice, a gentle nudge"), "{:?}", rec.sent());
    assert!(!rec.sent()[0].1.to_lowercase().contains("@owner"));
}

#[tokio::test]
async fn a_teammate_s_pull_request_that_a_card_key_matched_is_never_nudged_in_the_owner_s_name() {
    // The card key names the PR's branch, so the lane finds it; its author is dave.
    let w = world(&["alice"], "dave").await;
    w.opt_in(on());
    let rec = Recorder::new(Posted::Yes);
    assert!(w.turn(&rec, ready() + 30 * HOUR).await.is_empty());
    assert!(rec.sent().is_empty() && !w.nudged());
    // And the same when gh can't say who it posts as, or GitHub didn't name the author.
    let mut unknown = Recorder::new(Posted::Yes);
    unknown.owner = None;
    let mine = world(&["alice"], "owner").await;
    mine.opt_in(on());
    assert!(mine.turn(&unknown, ready() + 30 * HOUR).await.is_empty() && unknown.sent().is_empty());
    mine.edit(|p| p.author = String::new());
    assert!(mine.turn(&Recorder::new(Posted::Yes), ready() + 30 * HOUR).await.is_empty());
}

#[tokio::test]
async fn once_gh_has_run_the_claim_stays_whatever_it_said_and_only_a_gh_that_never_started_gives_it_back() {
    // Ran and failed (a non-zero exit, a timeout): it may have posted, so never again.
    let ran = world(&["alice"], "owner").await;
    ran.opt_in(on());
    let rec = Recorder::new(Posted::Ran);
    assert!(ran.turn(&rec, ready() + 30 * HOUR).await.is_empty());
    assert!(ran.nudged(), "the claim stays");
    assert!(ran.turn(&rec, ready() + 32 * HOUR).await.is_empty());
    assert_eq!(rec.sent().len(), 1, "and it is not tried again, not after an hour either");

    // Never started: certainly nothing went, so the next turn may try again.
    let never = world(&["alice"], "owner").await;
    never.opt_in(on());
    let rec = Recorder::new(Posted::NotRun);
    assert!(never.turn(&rec, ready() + 30 * HOUR).await.is_empty());
    assert!(!never.nudged(), "given back");
    let working = Recorder::new(Posted::Yes);
    assert_eq!(never.turn(&working, ready() + 32 * HOUR).await, [31]);
}

#[tokio::test]
async fn a_pull_request_someone_has_reviewed_is_not_nudged_even_when_the_timing_read_is_old() {
    let w = world(&["alice"], "owner").await;
    w.opt_in(on());
    // The timing read, long ago, said nobody had. The list, just now, shows a review.
    w.edit(|p| {
        p.timing.read_at = ready() + HOUR;
        p.review.others_reviewed = true;
    });
    let rec = Recorder::new(Posted::Yes);
    assert!(w.turn(&rec, ready() + 30 * HOUR).await.is_empty());
    // An old timing read alone, with nothing else to show, is not trusted either.
    let old = world(&["alice"], "owner").await;
    old.opt_in(on());
    old.edit(|p| p.timing.read_at = ready() + HOUR);
    assert!(old.turn(&rec, ready() + 30 * HOUR).await.is_empty());
    // A review the timing has already seen.
    let seen = world(&["alice"], "owner").await;
    seen.opt_in(on());
    seen.edit(|p| p.timing.first_review = Moment::At(ready() + HOUR));
    assert!(seen.turn(&rec, ready() + 30 * HOUR).await.is_empty());
    assert!(rec.sent().is_empty());
}

/// `workspace.set_settings` over a real socket, as a peer of `scope`.
async fn set_nudge(scope: Scope) -> (Result<(), farcooler_transport::ClientError>, bool) {
    let h = start(scope).await;
    let repo = a_repository(&h);
    let mut link = connect(&h).await;
    let mut req = farcooler_transport::request("workspace.set_settings");
    req.target_resource_id = Some(repo.workspace.as_bytes().to_vec().into());
    req.payload = Some(farcooler_protocol::v1::request::Payload::WorkspaceSetSettings(farcooler_protocol::v1::WorkspaceSetSettings {
        pr_nudge: Some(true),
        ..Default::default()
    }));
    req.required_capabilities = vec![farcooler_protocol::capability::PR_NUDGE.into()];
    let answer = link.call(req).await.map(drop);
    let landing = h.service.store.get_landing(repo.workspace).unwrap();
    (answer, landing.pr_nudge)
}

#[tokio::test]
async fn only_a_control_peer_can_turn_nudging_on() {
    let (answer, on) = set_nudge(Scope::Read).await;
    let refused = answer.expect_err("a Read peer is refused");
    assert!(
        matches!(&refused, farcooler_transport::ClientError::Daemon { code, .. } if farcooler_core::error::word_for(*code) == "scope-denied"),
        "{refused:?}"
    );
    assert!(!on, "and nothing was turned on");
    let (answer, on) = set_nudge(Scope::Control).await;
    answer.expect("a Control peer may");
    assert!(on);
}
