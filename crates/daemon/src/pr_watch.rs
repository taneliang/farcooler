//! The PR watch (ov-312, ov-305 section 5.1, Phase 1): keeps what the runner
//! knows of GitHub's pull requests fresh for the lanes that are waiting on them.
//!
//! While any lane is in review, fixing or landing, the watch re-reads that
//! lane's repository every minute (`ACTIVE`): one `gh pr list` for the
//! repository, and one GraphQL call per pull request a live lane works, for
//! its unresolved review threads and its place in the merge queue (which
//! `gh pr list` has no field for). A stage that moved announces `plan_changed`,
//! so the apps re-read the plan and show it without anyone asking.
//!
//! While no lane is, it reads nothing from GitHub, which is what the runner did
//! before: a PR cache is filled when somebody looks (`review_ops`), and never
//! polled. It still wakes every `IDLE` to see whether a lane has entered
//! review, and a lane entering review kicks it at once (`kick_lane`), so the first read of a
//! lane that just opened its PR is not five minutes away. The wake is a
//! store read; no `gh` runs.
//!
//! **What it costs.** Per repository, a turn is two `gh pr list` calls (the open
//! PRs, and the recent closed ones), each a GraphQL query of a few points for its
//! checks, reviews and requests, plus one point-sized GraphQL call per open PR a
//! live lane works. A repository with five live PRs is about seven calls a
//! minute and a few hundred of GitHub's 5,000 points an hour, on whichever `gh`
//! login the runner holds; several runners on one login add up. A `gh` that is
//! logged out or rate limited is not retried every minute: the repository backs
//! off from five minutes, doubling to thirty (`backoff_after`), and a lane
//! entering review tries it once at once.
//!
//! A `gh` that could not answer leaves the cache as it was. A runner that read
//! a PR a minute ago and is offline now still knows what it knew, with its
//! `read_at` to say how long ago; replacing that with "unknown" on a blip would
//! make every stage flicker. (`stack::fetch_prs`'s `Ok(None)` stays what it
//! was for a first read: nothing known.)
//!
//! Part of the plan layer and as removable: only the real daemon runs it
//! (`main.rs`), never a `--stdio` session.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use farcooler_store::models::Actor;
use uuid::Uuid;

use crate::plan_stage::{self, LaneStages, Reads};
use crate::service::Service;
use crate::stack::{self, PrInfo, PrState};
use crate::watch::Watcher;

/// How often the pull requests of live lanes are read while any lane is in
/// review or landing. Inside `gh`'s rate limits for a handful of PRs.
pub const ACTIVE: Duration = Duration::from_secs(60);
/// How often the watch looks for a lane entering review when none is. No `gh`.
pub const IDLE: Duration = Duration::from_secs(5 * 60);

/// How long to wait before the next turn, given whether any lane is waiting on
/// a pull request.
pub fn cadence(any_active: bool) -> Duration {
    if any_active { ACTIVE } else { IDLE }
}

fn kicked() -> &'static tokio::sync::Notify {
    static KICK: OnceLock<tokio::sync::Notify> = OnceLock::new();
    KICK.get_or_init(tokio::sync::Notify::new)
}

/// The repositories a lane has kicked since the last turn, which that turn
/// reads past their backoff. Only theirs: another repository's backoff stands.
static FORCED: Mutex<Option<HashSet<Uuid>>> = Mutex::new(None);

/// The repositories kicked since the last call, and forget them.
pub fn take_forced() -> HashSet<Uuid> {
    FORCED.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_default()
}

/// The least time between two kicks for one lane.
pub const KICK_FLOOR: Duration = Duration::from_secs(15);

/// The first wait after a repository's `gh` failed, and the longest.
pub const BACKOFF_FIRST: Duration = Duration::from_secs(5 * 60);
pub const BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);

/// How long to leave a repository alone after `misses` reads in a row that
/// `gh` could not answer: five minutes, doubling, at most thirty.
pub fn backoff_after(misses: u32) -> Duration {
    BACKOFF_FIRST.saturating_mul(1u32 << misses.saturating_sub(1).min(16)).min(BACKOFF_MAX)
}

/// The variable a test harness sets, in a debug `farcoolerd`'s environment,
/// to a floor of its own in milliseconds: a test that must see the floor run
/// out then waits a second, not fifteen. A release build never reads it, so
/// no one's environment can take the floor off `gh` (`kick_floor`).
pub const TEST_KICK_FLOOR_MS: &str = "FARCOOLER_TEST_KICK_FLOOR_MS";

/// The least floor `TEST_KICK_FLOOR_MS` may set: it shortens the floor, and
/// never removes it.
pub const TEST_KICK_FLOOR_LEAST: Duration = Duration::from_millis(500);

/// The floor a `TEST_KICK_FLOOR_MS` value asks for: `KICK_FLOOR` for none,
/// for one that isn't a number, and for one under `TEST_KICK_FLOOR_LEAST`.
pub fn floor_from(ms: Option<&str>) -> Duration {
    ms.and_then(|ms| ms.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .filter(|floor| *floor >= TEST_KICK_FLOOR_LEAST)
        .unwrap_or(KICK_FLOOR)
}

/// `KICK_FLOOR`, unless this is a debug build started with
/// `TEST_KICK_FLOOR_MS` (`floor_from`). Read once.
pub fn kick_floor() -> Duration {
    #[cfg(debug_assertions)]
    {
        static FLOOR: OnceLock<Duration> = OnceLock::new();
        *FLOOR.get_or_init(|| floor_from(std::env::var(TEST_KICK_FLOOR_MS).ok().as_deref()))
    }
    #[cfg(not(debug_assertions))]
    KICK_FLOOR
}

/// When each lane last kicked the watch, and the least time between two.
pub struct KickFloor(HashMap<Uuid, Instant>, Duration);

impl Default for KickFloor {
    /// `KICK_FLOOR` apart, whatever the environment says: what the unit test
    /// of the floor itself pins. `kick_lane` takes `kick_floor()`.
    fn default() -> Self {
        KickFloor::new(KICK_FLOOR)
    }
}

impl KickFloor {
    /// Kicks at least `floor` apart.
    pub fn new(floor: Duration) -> Self {
        KickFloor(HashMap::new(), floor)
    }

    /// Whether `lane` may kick at `now`, recording it when it may. A lane's
    /// first kick always may; its next waits out the floor.
    pub fn allows(&mut self, lane: Uuid, now: Instant) -> bool {
        if self.0.get(&lane).is_some_and(|last| now.duration_since(*last) < self.1) {
            return false;
        }
        self.0.insert(lane, now);
        true
    }
}

/// A lane moved into review, fixing or landing: read its repository now, past
/// that repository's backoff, unless this lane kicked within `KICK_FLOOR`.
pub fn kick_lane(lane: Uuid, repository: Uuid) {
    static FLOOR: Mutex<Option<KickFloor>> = Mutex::new(None);
    let allowed = FLOOR
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(|| KickFloor::new(kick_floor()))
        .allows(lane, Instant::now());
    if allowed {
        FORCED.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(HashSet::new).insert(repository);
        kicked().notify_one();
    }
}

/// What the watch remembers between turns: how many reads in a row `gh` could
/// not answer for each repository, and when it may be asked again.
#[derive(Default)]
pub struct Memo {
    misses: HashMap<Uuid, (u32, Instant)>,
    /// The same for the timing read (`pr_timing_read`), which runs apart on its
    /// own slower cadence and backoff: how many reads in a row failed, and
    /// when the next is due.
    timing: HashMap<Uuid, (u32, Instant)>,
}

impl Memo {
    /// Whether `repository` may be read at `now`: not while it backs off,
    /// unless a lane of its own kicked (`forced`).
    pub fn due(&self, repository: Uuid, now: Instant, forced: &HashSet<Uuid>) -> bool {
        forced.contains(&repository) || self.misses.get(&repository).is_none_or(|(_, until)| now >= *until)
    }

    /// Whether `repository`'s timing is due at `now`.
    pub fn timing_due(&self, repository: Uuid, now: Instant) -> bool {
        self.timing.get(&repository).is_none_or(|(_, until)| now >= *until)
    }

    /// A timing read that answered, or one that did not: the next is `EVERY`
    /// away, or the longer of that and the backoff after the misses so far.
    pub fn timing_read(&mut self, repository: Uuid, now: Instant, answered: bool) {
        let n = if answered { 0 } else { self.timing.get(&repository).map_or(0, |(n, _)| *n) + 1 };
        let every = crate::pr_timing_read::every();
        let wait = if answered { every } else { backoff_after(n).max(every) };
        self.timing.insert(repository, (n, now + wait));
    }

    /// A read that `gh` could not answer.
    pub fn missed(&mut self, repository: Uuid, now: Instant) {
        let n = self.misses.get(&repository).map_or(0, |(n, _)| *n) + 1;
        self.misses.insert(repository, (n, now + backoff_after(n)));
    }

    /// A read that `gh` answered.
    pub fn answered(&mut self, repository: Uuid) {
        self.misses.remove(&repository);
    }
}

/// The watch, for the life of the daemon.
pub async fn run(svc: Arc<Service>, watcher: Arc<Watcher>) {
    let mut memo = Memo::default();
    loop {
        let wait = tick(&svc, &watcher, &mut memo, &take_forced()).await;
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = kicked().notified() => {}
        }
    }
}

/// What one pull request says that `gh pr list` cannot: its unresolved review
/// threads, and its place in the merge queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extras {
    pub unresolved_threads: u32,
    /// From 1 as GitHub counts it; `None` when it is not in the queue.
    pub queue_position: Option<u32>,
}

/// The query for one pull request: its threads (the first hundred, which is
/// more than a review is read in, and a thread past that is not counted) and
/// its queue entry. One call, run in the
/// repository's checkout, in place of two.
const QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){\
    pullRequest(number:$number){reviewThreads(first:100){nodes{isResolved}} mergeQueueEntry{position}}}}";
/// What of the answer is kept.
const JQ: &str = "{threads: ([.data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved | not)] | length), \
    position: .data.repository.pullRequest.mergeQueueEntry.position}";

/// One pull request's extras, or `None` when `gh` could not say.
pub async fn extras(worktree: &Path, number: u32) -> Option<Extras> {
    let mut gh = stack::gh(worktree).await.ok()?;
    let out = tokio::time::timeout(
        stack::GH_TIMEOUT,
        gh.current_dir(worktree)
            .args(["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-F"])
            .arg(format!("number={number}"))
            .args(["-f"])
            .arg(format!("query={QUERY}"))
            .args(["--jq", JQ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_extras(&out.stdout)
}

/// `{"threads":2,"position":null}`, as `JQ` prints it.
fn parse_extras(stdout: &[u8]) -> Option<Extras> {
    let v: serde_json::Value = serde_json::from_slice(stdout).ok()?;
    Some(Extras {
        unresolved_threads: v.get("threads")?.as_u64()? as u32,
        queue_position: v.get("position").and_then(|p| p.as_u64()).map(|p| p as u32),
    })
}

/// The live lanes of every board, by repository: what each is waiting on.
struct Live {
    /// Per repository: its boards' ids and the pull requests its live lanes work.
    boards: BTreeMap<Uuid, BTreeSet<Uuid>>,
}

fn live_lanes(svc: &Service) -> Live {
    let mut boards: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
    for ws in svc.store.list_workspaces(None).unwrap_or_default() {
        let Ok(plan) = svc.store.plan(ws.id, i64::MAX) else { continue };
        if plan.lanes.iter().any(|l| plan_stage::phase_of(l.lane.state).is_active()) {
            boards.entry(ws.repository_id).or_default().insert(ws.id);
        }
    }
    Live { boards }
}

/// The stages of every lane of `workspace` against `reads`, ignoring when they
/// were read: two reads that say the same thing are not a change.
fn stages_of(svc: &Service, workspace: Uuid, reads: &Reads) -> HashMap<Uuid, LaneStages> {
    let Ok(plan) = svc.store.plan(workspace, i64::MAX) else { return HashMap::new() };
    plan.lanes
        .iter()
        .map(|l| {
            let mut s = plan_stage::lane_stages(l, &plan.cards, reads);
            for stage in s.lane.iter_mut().chain(s.cards.values_mut()) {
                stage.read_at = 0;
            }
            (l.lane.id, s)
        })
        .collect()
}

/// One turn: re-read each repository that has a lane waiting on its pull
/// requests, and say so when a stage moved. Returns how long to wait.
pub async fn tick(svc: &Service, watcher: &Watcher, memo: &mut Memo, forced: &HashSet<Uuid>) -> Duration {
    let live = live_lanes(svc);
    for (repository, boards) in &live.boards {
        if !memo.due(*repository, Instant::now(), forced) {
            continue;
        }
        let Ok(repo) = svc.store.get_repository(*repository) else { continue };
        let worktree = svc.repository_worktree(&repo);
        let before: Vec<_> = boards.iter().map(|b| stages_of(svc, *b, &Reads::of(svc, *repository))).collect();

        let fetched = {
            let _permit = svc.gh_permit().await;
            stack::fetch_prs(&worktree).await
        };
        let Ok(Some(mut prs)) = fetched else {
            memo.missed(*repository, Instant::now());
            continue;
        };
        memo.answered(*repository);
        count_threads(svc, boards, &worktree, &mut prs).await;
        svc.pr_cache_put(*repository, Some(prs));
        read_timing(svc, memo, *repository, boards, &worktree).await;

        let reads = Reads::of(svc, *repository);
        for (board, was) in boards.iter().zip(before) {
            if stages_of(svc, *board, &reads) != was {
                watcher.announce_plan_changed(*board, Actor::Runner);
            }
        }
    }
    cadence(!live.boards.is_empty())
}

/// The timing of the lanes' PRs of `repository` (`pr_timing_read`), when it is
/// due. After the stage's own read is in the cache, and touching nothing but
/// the timing: a read that fails leaves the last one.
async fn read_timing(svc: &Service, memo: &mut Memo, repository: Uuid, boards: &BTreeSet<Uuid>, worktree: &Path) {
    if !memo.timing_due(repository, Instant::now()) {
        return;
    }
    let reads = Reads::of(svc, repository);
    let mut wanted: BTreeSet<u32> = BTreeSet::new();
    for board in boards {
        let Ok(plan) = svc.store.plan(*board, i64::MIN) else { continue };
        for lane in &plan.lanes {
            wanted.extend(plan_stage::prs_of(lane, &plan.cards, &reads));
        }
    }
    // Open, and the recently merged: a closed PR nobody merged says nothing of a wait.
    let mut numbers: Vec<u32> = reads
        .prs
        .iter()
        .filter(|p| wanted.contains(&p.status.number) && matches!(p.status.state, PrState::Open | PrState::Draft | PrState::Merged))
        .map(|p| p.status.number)
        .collect();
    numbers.sort_unstable_by(|a, b| b.cmp(a));
    numbers.truncate(crate::pr_timing_read::BATCH);
    let read = {
        let _permit = svc.gh_permit().await;
        crate::pr_timing_read::fetch(worktree, &numbers).await
    };
    memo.timing_read(repository, Instant::now(), read.is_some());
    if let Some(timing) = read {
        svc.pr_timing_put(repository, &timing);
    }
}

/// Count unresolved threads, and find the queue position, of each open pull request a live lane works, and
/// no others: a merged PR's threads are history, and a repository's other PRs
/// are not this runner's business.
async fn count_threads(svc: &Service, boards: &BTreeSet<Uuid>, worktree: &Path, prs: &mut [PrInfo]) {
    let reads = Reads { known: true, prs: prs.to_vec() };
    let mut wanted: BTreeSet<u32> = BTreeSet::new();
    for board in boards {
        let Ok(plan) = svc.store.plan(*board, i64::MAX) else { continue };
        for lane in plan.lanes.iter().filter(|l| plan_stage::phase_of(l.lane.state).is_active()) {
            wanted.extend(plan_stage::prs_of(lane, &plan.cards, &reads));
        }
    }
    for pr in prs.iter_mut() {
        if !wanted.contains(&pr.status.number) || !matches!(pr.status.state, PrState::Open | PrState::Draft) {
            continue;
        }
        let read = {
            let _permit = svc.gh_permit().await;
            extras(worktree, pr.status.number).await
        };
        // A read `gh` could not make leaves the last one, which the cache
        // keeps for a PR not counted this turn (`stack::merge_read`).
        if let Some(e) = read {
            pr.review.unresolved_threads = Some(e.unresolved_threads);
            pr.review.queue_position = e.queue_position;
        }
    }
}

#[cfg(test)]
#[path = "pr_watch_tests.rs"]
mod tests;
