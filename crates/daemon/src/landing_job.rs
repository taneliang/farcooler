//! The runner's landing job (ov-872, slice 5): the push route, run by the
//! daemon, step by step, with nothing left to judgment.
//!
//! It is `train.sh`'s mechanical half, generalized, with the charter's
//! `## Landing` section (`farcooler_core::charter_landing`) in place of what the script
//! hard-coded:
//!
//! 1. **Route.** Each ticket on the landing takes the route its changed files
//!    choose. Tickets the charter asks about move to a landing of their own
//!    that waits on you (`Store::hold_for_you`), and so do pull-request ones,
//!    which the job doesn't run yet (slice 6). The push tickets stay.
//! 2. **Wait for the base.** Fetch, and wait for CI on the base branch's head
//!    to pass. A red base stops the job.
//! 3. **Fold.** A scratch worktree at the base's head, beside the others this
//!    job makes, takes each ticket's commits (`base..branch`) by `cherry-pick
//!    -x`, in order. An empty pick (already on the base) is skipped. A
//!    conflict stops the job with the files and commit.
//! 4. **Gate.** Each of the charter's gate commands, by `sh -c`, at the top
//!    of the worktree, its output in a log beside it. When one fails on a
//!    batch, the batch splits in two and each half goes back to step 2 on a
//!    landing of its own, so a good ticket never waits on a bad one. A single
//!    ticket that fails stops.
//! 5. **Push.** Check that the gated base has not moved, push to the base
//!    branch, and record the SHA on the landing (pushed). A moved base stops
//!    for a fresh attempt, so an ungated rebase is never pushed.
//! 6. **Watch CI** on that SHA. A run canceled is run again once. Green lands
//!    the landing, and its tickets with it, at that SHA; red marks it red and
//!    stops.
//!
//! Every step goes on the landing's timeline (`Store::note_landing`), which
//! `farcooler landing show` prints. A stop leaves the landing open, or red,
//! with its worktree kept, for whoever takes it from there (slice 7's lander).
//!
//! **Git here is not the daemon's sandboxed git** (`git_launch`). The job
//! exists to fetch, push and run the charter's gate, which is a shell command
//! by design, so it runs the user's own `git` and `sh` as the runner's user,
//! as an agent pane would. Who may start it is `landing.start`'s gate.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use farcooler_core::DomainError;
use farcooler_store::Store;
use farcooler_store::board_ci::CiStatus;
use farcooler_store::landings::{Landing, LandingState, LandingUpdate, NewLanding, Route, landing_title};
use farcooler_store::models::Actor;
use uuid::Uuid;

use farcooler_core::charter_landing::{self as charter, LandingRules};

/// Reads CI for a commit, and runs a canceled run again.
pub trait Ci: Send + Sync {
    /// Where CI stands on `sha`, a full SHA.
    fn status(&self, sha: &str) -> impl Future<Output = CiStatus> + Send;
    /// Run `sha`'s canceled runs again. True when one was.
    fn rerun(&self, sha: &str) -> impl Future<Output = bool> + Send;
}

/// What the job works with.
pub struct Setup<'a> {
    pub store: &'a Store,
    /// The repository's main checkout, where the tickets' branches are.
    pub repo: PathBuf,
    /// Where the job makes its worktrees, one per landing.
    pub scratch: PathBuf,
    pub rules: LandingRules,
    /// How long between CI reads.
    pub poll: Duration,
    /// The most a CI wait lasts before the job stops.
    pub ci_patience: Duration,
    /// The most one gate command runs before it's stopped.
    pub gate_timeout: Duration,
    /// Called after each write, with the workspace, to tell clients.
    pub announce: &'a (dyn Fn(Uuid) + Send + Sync),
}

/// How one landing the job ran came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Landed { landing: u32, tickets: Vec<String>, sha: String },
    /// It waits on you: the charter asks about it.
    Asked { landing: u32, tickets: Vec<String> },
    /// It lands by pull request, which this job doesn't run yet.
    PullRequest { landing: u32, tickets: Vec<String> },
    Stopped { landing: u32, tickets: Vec<String>, why: String },
}

/// What a step that stops says.
#[derive(Debug)]
struct Stop(String);

type Step<T> = std::result::Result<T, Stop>;

/// The landing the job runs on for `tickets`, started as `landing.start`
/// starts one, after three things a job meets that a hand-started landing
/// doesn't:
///
/// - a ticket on a landing the charter asked about waits for you: with
///   `approved` (you said land it) that landing takes the push route, and
///   without it the ticket is refused `waits_for_you`;
/// - a ticket on the board's collecting landing that left for its gate on its
///   own, with nothing pushed, is taken up rather than refused as moving: its
///   landing opens again;
/// - a landing those moves leave empty is dropped, so none is left behind.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    store: &Store,
    workspace: Uuid,
    tickets: &[Uuid],
    base: &str,
    title: &str,
    lander: Option<Uuid>,
    approved: bool,
    actor: Actor,
) -> farcooler_core::Result<Landing> {
    let mut left = Vec::new();
    for task in tickets {
        let Some(on) = store.landing_of(*task)? else { continue };
        if on.route == Route::Ask && on.state == LandingState::Open {
            if !approved {
                return Err(DomainError::InvalidArgument { what: "waits_for_you" });
            }
            store.land_asked(on.id, actor)?;
        }
        if on.state == LandingState::Gating && on.pushed_sha.is_none() {
            let open = LandingUpdate { state: Some(LandingState::Open), ..Default::default() };
            store.update_landing(on.id, &open, Actor::Runner)?;
        }
        if !left.contains(&on.id) {
            left.push(on.id);
        }
    }
    let new = NewLanding { route: Route::Push, base: base.to_string(), title: title.to_string(), lander, collecting: false };
    let landing = store.start_landing(workspace, &new, tickets, actor)?;
    for id in left.into_iter().filter(|id| *id != landing.id) {
        let Ok(old) = store.landing(id) else { continue };
        if old.tickets.is_empty() && old.state == LandingState::Open && !old.collecting {
            let dropped = LandingUpdate { state: Some(LandingState::Dropped), ..Default::default() };
            store.update_landing(id, &dropped, Actor::Runner)?;
        }
    }
    Ok(landing)
}

/// The exact work whose paths were routed. Never resolve a mutable branch
/// again to choose commits after a CI wait.
#[derive(Debug)]
struct RoutedTicket {
    branch: String,
    tip: String,
}

type Routed = HashMap<Uuid, RoutedTicket>;

/// Run the job on `landing`. `approved`: you said to land it, so the charter's
/// ask route reads as push.
pub async fn run<C: Ci>(s: &Setup<'_>, ci: &C, landing: Uuid, approved: bool) -> Vec<Outcome> {
    let Ok(l) = s.store.landing(landing) else { return Vec::new() };
    let mut outcomes = Vec::new();
    let push = match route_all(s, &l, approved, &mut outcomes).await {
        Ok(push) => push,
        Err(Stop(why)) => {
            outcomes.push(stop(s, &l, &why));
            return outcomes;
        }
    };
    if !push.is_empty() {
        land_batch(s, ci, landing, &push, &mut outcomes).await;
    }
    outcomes
}

/// Step 1: split off every ticket whose route isn't push. The push tickets,
/// left on `l`.
async fn route_all(s: &Setup<'_>, l: &Landing, approved: bool, out: &mut Vec<Outcome>) -> Step<Routed> {
    note(s, l, "routing", &format!("Fetching {}.", s.rules.remote), serde_json::json!({}));
    git_ok(&s.repo, &["fetch", "--quiet", &s.rules.remote]).await.map_err(|e| Stop(format!("Couldn't fetch {}: {e}", s.rules.remote)))?;
    let base = rev(&s.repo, &base_ref(s)).await?;
    let mut routed = Routed::new();
    let mut groups: Vec<(Route, Option<String>, Vec<Uuid>)> = Vec::new();
    for task in &l.tickets {
        let line = s.store.ticket_line(*task).map_err(|e| Stop(format!("Couldn't read a ticket: {}", e.what())))?;
        if line.stage.is_closed() {
            continue;
        }
        if line.branch.is_empty() {
            return Err(Stop(format!("{} has no branch to land.", keys(s, &[*task]).join(", "))));
        }
        let tip = rev(&s.repo, &line.branch).await?;
        let paths = changed_paths(&s.repo, &base, &tip).await?;
        let (said, rule) = s.rules.route_of(&paths);
        let mut route = match said {
            charter::Route::Push => Route::Push,
            charter::Route::PullRequest => Route::PullRequest,
            charter::Route::Ask => Route::Ask,
        };
        if approved && route == Route::Ask {
            route = Route::Push;
        }
        if route == Route::Push {
            routed.insert(*task, RoutedTicket { branch: line.branch, tip });
        }
        match groups.iter_mut().find(|(r, _, _)| *r == route) {
            Some(group) => group.2.push(*task),
            None => groups.push((route, rule.map(str::to_string), vec![*task])),
        }
    }
    // The push group keeps this landing; with none, the first group does.
    groups.sort_by_key(|(r, _, _)| *r != Route::Push);
    for (i, (route, rule, tickets)) in groups.into_iter().enumerate() {
        let on = if i == 0 { l.clone() } else { split_off(s, l, &tickets, route)? };
        let keys = keys(s, &tickets);
        match route {
            Route::Push => {}
            Route::Ask => {
                let why = match rule {
                    Some(glob) => format!("The charter asks before landing `{glob}`. Say land it to push it."),
                    None => "The charter asks before landing. Say land it to push it.".to_string(),
                };
                let held = s.store.hold_for_you(on.id, &why, Actor::Runner).map_err(|e| Stop(format!("Couldn't hold it for you: {}", e.what())))?;
                (s.announce)(held.workspace_id);
                out.push(Outcome::Asked { landing: held.number, tickets: keys });
            }
            Route::PullRequest => {
                let body = "Lands by pull request, which the runner doesn't run yet. Open its pull request.";
                note(s, &on, "pull_request", body, serde_json::json!({ "tickets": keys }));
                out.push(Outcome::PullRequest { landing: on.number, tickets: keys });
            }
        }
    }
    Ok(routed)
}

/// Move `tickets` off `l` onto a landing of their own on `route`.
fn split_off(s: &Setup<'_>, l: &Landing, tickets: &[Uuid], route: Route) -> Step<Landing> {
    let back = |e: DomainError| Stop(format!("Couldn't split the landing: {}", e.what()));
    let current = s.store.landing(l.id).map_err(back)?;
    if current.state != LandingState::Open {
        let open = LandingUpdate { state: Some(LandingState::Open), ..Default::default() };
        s.store.update_landing(l.id, &open, Actor::Runner).map_err(back)?;
    }
    let off = LandingUpdate { remove_tickets: tickets.to_vec(), ..Default::default() };
    s.store.update_landing(l.id, &off, Actor::Runner).map_err(back)?;
    let new = NewLanding { route, base: l.base.clone(), title: String::new(), lander: l.lander_id, collecting: false };
    let made = s.store.start_landing(l.workspace_id, &new, tickets, Actor::Runner).map_err(back)?;
    let body = format!("{} went to {}.", keys(s, tickets).join(", "), landing_title(&made));
    note(s, l, "split", &body, serde_json::json!({ "to": made.number }));
    Ok(made)
}

/// Steps 2 to 6 for the tickets on `landing`, splitting on a red gate.
async fn land_batch<C: Ci>(s: &Setup<'_>, ci: &C, landing: Uuid, routed: &Routed, out: &mut Vec<Outcome>) {
    // A batch splits into landings that each go round again; a stack, not
    // recursion, so the future stays a plain one.
    let mut todo = vec![landing];
    while let Some(id) = todo.pop() {
        let Ok(l) = s.store.landing(id) else { continue };
        match land_one(s, ci, &l, routed).await {
            Ok(Done::Landed(sha)) => {
                let tickets = keys(s, &l.tickets);
                out.push(Outcome::Landed { landing: l.number, tickets, sha });
            }
            Ok(Done::Split(first, second)) => {
                todo.push(second);
                todo.push(first);
            }
            Err(Stop(why)) => out.push(stop(s, &l, &why)),
        }
    }
}

enum Done {
    Landed(String),
    /// The gate failed on a batch: two landings to try, in order.
    Split(Uuid, Uuid),
}

async fn land_one<C: Ci>(s: &Setup<'_>, ci: &C, l: &Landing, routed: &Routed) -> Step<Done> {
    let base = base_ref(s);
    git_ok(&s.repo, &["fetch", "--quiet", &s.rules.remote]).await.map_err(|e| Stop(format!("Couldn't fetch {}: {e}", s.rules.remote)))?;
    let base_sha = rev(&s.repo, &base).await?;
    if s.rules.ci {
        note(s, l, "base", &format!("Waiting for CI on {base} at {}.", short(&base_sha)), serde_json::json!({ "sha": base_sha }));
        wait_green(s, ci, l, &base_sha).await.map_err(|why| Stop(format!("CI on {base} at {} {why}.", short(&base_sha))))?;
    }
    unchanged(s, l, routed).await?;
    let tree = worktree_for(s, l, &base_sha).await?;
    fold(s, l, &tree, &base_sha, routed).await?;
    set_state(s, l, LandingState::Gating)?;
    if let Err(Stop(why)) = gate(s, l, &tree).await {
        set_state(s, l, LandingState::Open)?;
        if l.tickets.len() < 2 {
            return Err(Stop(why));
        }
        let second: Vec<Uuid> = l.tickets[l.tickets.len() / 2..].to_vec();
        note(s, l, "splitting", &format!("{why} Trying it in two halves."), serde_json::json!({}));
        let made = split_off(s, l, &second, Route::Push)?;
        return Ok(Done::Split(l.id, made.id));
    }
    unchanged(s, l, routed).await?;
    let sha = push(s, l, &tree, &base, &base_sha).await?;
    if s.rules.ci {
        if let Err(why) = wait_green(s, ci, l, &sha).await {
            if why == "failed" {
                set_state(s, l, LandingState::Red)?;
            }
            return Err(Stop(format!("CI on {} {why}.", short(&sha))));
        }
    }
    let land = LandingUpdate { state: Some(LandingState::Landed), ..Default::default() };
    match s.store.update_landing(l.id, &land, Actor::Runner) {
        // The CI watch saw it green and landed it first.
        Ok(_) | Err(DomainError::InvalidArgument { what: "landing_settled" }) => {}
        Err(e) => return Err(Stop(format!("Pushed {}, but couldn't mark it landed: {}", short(&sha), e.what()))),
    }
    (s.announce)(l.workspace_id);
    note(s, l, "landed", &format!("Landed at {}.", short(&sha)), serde_json::json!({ "sha": sha }));
    let _ = git(&s.repo, &["worktree", "remove", "--force", &tree.to_string_lossy()]).await;
    Ok(Done::Landed(sha))
}

/// Wait until CI on `sha` passes: `Err` says how it didn't.
async fn wait_green<C: Ci>(s: &Setup<'_>, ci: &C, l: &Landing, sha: &str) -> std::result::Result<(), &'static str> {
    let started = std::time::Instant::now();
    let mut reran = false;
    loop {
        match ci.status(sha).await {
            CiStatus::Passed => return Ok(()),
            CiStatus::Failed => return Err("failed"),
            CiStatus::Superseded if reran => return Err("was canceled twice"),
            CiStatus::Superseded => {
                reran = true;
                let again = ci.rerun(sha).await;
                let body = if again { "CI was canceled; running it again." } else { "CI was canceled, and couldn't be run again." };
                note(s, l, "rerun", body, serde_json::json!({ "sha": sha }));
                if !again {
                    return Err("was canceled");
                }
            }
            CiStatus::Running | CiStatus::Queued | CiStatus::None | CiStatus::Unknown => {}
        }
        if started.elapsed() >= s.ci_patience {
            return Err("had no result in time");
        }
        tokio::time::sleep(s.poll).await;
    }
}

/// A fresh worktree for `l` at `base_sha`, preserving any earlier attempt.
async fn worktree_for(s: &Setup<'_>, l: &Landing, base_sha: &str) -> Step<PathBuf> {
    let mut tree = s.scratch.join(format!("landing-{}", l.number));
    // A failed attempt can hold a lander's fixes and reports. Keep it;
    // every retry gets a fresh scratch worktree instead of deleting it.
    if tree.symlink_metadata().is_ok() {
        tree = s.scratch.join(format!("landing-{}-{}", l.number, Uuid::now_v7()));
    }
    let path = tree.to_string_lossy().into_owned();
    std::fs::create_dir_all(&s.scratch).map_err(|e| Stop(format!("Couldn't make {}: {e}", s.scratch.display())))?;
    git_ok(&s.repo, &["worktree", "add", "--quiet", "--detach", &path, base_sha])
        .await
        .map_err(|e| Stop(format!("Couldn't make the landing's worktree: {e}")))?;
    Ok(tree)
}

/// Refuse stale routing if a ticket, branch tip or landing membership changed.
/// The fold still uses pinned SHAs: these checks never select new work.
async fn unchanged(s: &Setup<'_>, l: &Landing, routed: &Routed) -> Step<()> {
    let current = s.store.landing(l.id).map_err(|e| Stop(e.what().into()))?;
    if current.tickets != l.tickets {
        return Err(Stop("The landing's tickets changed after routing; start a fresh attempt.".into()));
    }
    for task in &l.tickets {
        let pin = routed.get(task).ok_or_else(|| Stop("A ticket wasn't routed for push; start a fresh attempt.".into()))?;
        let line = s.store.ticket_line(*task).map_err(|e| Stop(e.what().into()))?;
        if line.branch != pin.branch || line.stage.is_closed() || rev(&s.repo, &pin.branch).await? != pin.tip {
            return Err(Stop(format!("{} changed after routing; start a fresh attempt.", keys(s, &[*task]).join(", "))));
        }
    }
    Ok(())
}

/// Step 3: every routed ticket's pinned commits, in order, onto the worktree.
async fn fold(s: &Setup<'_>, l: &Landing, tree: &Path, base_sha: &str, routed: &Routed) -> Step<()> {
    for task in &l.tickets {
        let key = keys(s, &[*task]).remove(0);
        let tip = &routed.get(task).ok_or_else(|| Stop(format!("{key} wasn't routed for push.")))?.tip;
        let range = format!("{base_sha}..{tip}");
        let listed = git_ok(tree, &["rev-list", "--reverse", &range]).await.map_err(|e| Stop(format!("Couldn't list {key}'s commits: {e}")))?;
        for commit in listed.lines().filter(|c| !c.is_empty()) {
            let picked = git(tree, &["cherry-pick", "-x", commit]).await;
            if picked.ok {
                continue;
            }
            let unmerged = git(tree, &["diff", "--name-only", "--diff-filter=U"]).await.out;
            let staged = git(tree, &["diff", "--cached", "--quiet"]).await.ok && git(tree, &["diff", "--quiet"]).await.ok;
            if unmerged.trim().is_empty() && staged {
                // Already on the base: an empty pick.
                if git(tree, &["cherry-pick", "--skip"]).await.ok {
                    continue;
                }
            }
            let _ = git(tree, &["cherry-pick", "--abort"]).await;
            let files: Vec<&str> = unmerged.lines().filter(|f| !f.is_empty()).collect();
            return Err(Stop(if files.is_empty() {
                format!("{key}'s commit {} didn't apply: {}", short(commit), last_line(&picked.err))
            } else {
                format!("{key} conflicts at {} in {}.", short(commit), files.join(", "))
            }));
        }
        note(s, l, "folded", &format!("Folded {key}."), serde_json::json!({ "ticket": key }));
    }
    Ok(())
}

/// Step 4: the charter's gate, every command in order.
async fn gate(s: &Setup<'_>, l: &Landing, tree: &Path) -> Step<()> {
    let log = s.scratch.join(format!("landing-{}-gate.log", l.number));
    if s.rules.gate.is_empty() {
        note(s, l, "gated", "No gate: the charter says none.", serde_json::json!({}));
        return Ok(());
    }
    let _ = std::fs::write(&log, "");
    for command in &s.rules.gate {
        note(s, l, "gating", &format!("Running `{command}`."), serde_json::json!({ "log": log.to_string_lossy() }));
        let (passed, tail) = run_gate(tree, command, &log, s.gate_timeout).await;
        if !passed {
            let why = format!("The gate failed: `{command}`: {tail}");
            note(s, l, "gate_failed", &why, serde_json::json!({ "log": log.to_string_lossy() }));
            return Err(Stop(why));
        }
    }
    note(s, l, "gated", "The gate passed.", serde_json::json!({}));
    Ok(())
}

/// One gate command, its output appended to `log`: whether it passed, and
/// its last line of output.
async fn run_gate(tree: &Path, command: &str, log: &Path, limit: Duration) -> (bool, String) {
    let Ok(file) = std::fs::OpenOptions::new().append(true).create(true).open(log) else {
        return (false, format!("couldn't write {}", log.display()));
    };
    let Ok(err) = file.try_clone() else { return (false, "couldn't write its log".into()) };
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command).current_dir(tree).stdin(Stdio::null()).stdout(file).stderr(err).kill_on_drop(true);
    cmd.process_group(0);
    let Ok(mut child) = cmd.spawn() else { return (false, "couldn't start sh".into()) };
    let pid = child.id();
    let passed = match tokio::time::timeout(limit, child.wait()).await {
        Ok(Ok(status)) => status.success(),
        Ok(Err(_)) => false,
        Err(_) => {
            if let Some(pid) = pid {
                // SAFETY: killpg on the group this child leads; it was made
                // for this command alone.
                unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
            }
            let _ = child.wait().await;
            return (false, format!("still running after {} minutes", limit.as_secs() / 60));
        }
    };
    let text = std::fs::read_to_string(log).unwrap_or_default();
    (passed, last_line(&text).to_string())
}

/// Step 5: push only while the gated base remains current. The pushed SHA.
async fn push(s: &Setup<'_>, l: &Landing, tree: &Path, base: &str, gated_base: &str) -> Step<String> {
    git_ok(tree, &["fetch", "--quiet", &s.rules.remote]).await.map_err(|e| Stop(format!("Couldn't fetch {}: {e}", s.rules.remote)))?;
    if rev(tree, base).await? != gated_base {
        // Rebase changes the tree that passed the gate. Never push that
        // untested result; a fresh attempt must fold and gate the new base.
        return Err(Stop(format!("{base} moved after the gate; start a fresh landing attempt to gate it again.")));
    }
    let target = format!("HEAD:refs/heads/{}", s.rules.base);
    git_ok(tree, &["push", "--quiet", &s.rules.remote, &target]).await.map_err(|e| Stop(format!("The push was refused: {e}")))?;
    let sha = rev(tree, "HEAD").await?;
    s.store.record_landing_push(l.id, &sha, &l.tickets).map_err(|e| Stop(match e {
        DomainError::InvalidArgument { what: "landing_changed" } =>
            format!("Pushed {}, but the landing's tickets changed. Start a fresh landing attempt.", short(&sha)),
        _ => format!("Pushed {}, but couldn't record it: {}", short(&sha), e.what()),
    }))?;
    (s.announce)(l.workspace_id);
    crate::ci_watch::kick();
    let body = format!("Pushed {} to {}.", short(&sha), s.rules.base);
    note(s, l, "pushed", &body, serde_json::json!({ "sha": sha }));
    Ok(sha)
}

/// The job stopped on `l`: say so on its timeline.
fn stop(s: &Setup<'_>, l: &Landing, why: &str) -> Outcome {
    note(s, l, "stopped", &format!("Stopped. {why}"), serde_json::json!({}));
    tracing::warn!(landing = %landing_title(l), why = %why, "a landing job stopped");
    let tickets = s.store.landing(l.id).map(|now| keys(s, &now.tickets)).unwrap_or_default();
    Outcome::Stopped { landing: l.number, tickets, why: why.to_string() }
}

fn set_state(s: &Setup<'_>, l: &Landing, to: LandingState) -> Step<()> {
    let update = LandingUpdate { state: Some(to), ..Default::default() };
    s.store.update_landing(l.id, &update, Actor::Runner).map_err(|e| Stop(format!("Couldn't move it to {}: {}", to.as_str(), e.what())))?;
    (s.announce)(l.workspace_id);
    Ok(())
}

fn note(s: &Setup<'_>, l: &Landing, step: &str, body: &str, extra: serde_json::Value) {
    if let Err(e) = s.store.note_landing(l.id, step, body, extra) {
        tracing::warn!(landing = %landing_title(l), error = %e.what(), "couldn't write a landing job step");
    }
    (s.announce)(l.workspace_id);
}

fn keys(s: &Setup<'_>, tickets: &[Uuid]) -> Vec<String> {
    tickets.iter().map(|t| s.store.get_task(*t).map(|t| t.key).unwrap_or_else(|_| "a ticket".into())).collect()
}

fn base_ref(s: &Setup<'_>) -> String {
    format!("{}/{}", s.rules.remote, s.rules.base)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(9)]
}

fn last_line(text: &str) -> &str {
    text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("it printed nothing")
}

/// The files `branch` changes since it left `base`.
async fn changed_paths(repo: &Path, base: &str, branch: &str) -> Step<Vec<String>> {
    let range = format!("{base}...{branch}");
    let out = git_ok(repo, &["diff", "--name-only", "--no-renames", "-z", &range, "--"]).await
        .map_err(|why| Stop(format!("Couldn't read {branch}'s changed files: {why}")))?;
    Ok(out.split('\0').filter(|l| !l.is_empty()).map(str::to_string).collect())
}

async fn rev(dir: &Path, name: &str) -> Step<String> {
    let spec = format!("{name}^{{commit}}");
    git_ok(dir, &["rev-parse", "--verify", "--quiet", &spec]).await.map(|o| o.trim().to_string()).map_err(|_| Stop(format!("{name} isn't there.")))
}

/// What one git said.
struct GitRun {
    ok: bool,
    out: String,
    err: String,
}

/// The most one git of the job may take: a fetch or push over a slow line.
const GIT_LIMIT: Duration = Duration::from_secs(10 * 60);

#[cfg(test)]
thread_local! {
    // A test pauses a real fetch without changing process-wide PATH.
    static GIT_PROGRAM: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

async fn git(dir: &Path, args: &[&str]) -> GitRun {
    #[cfg(test)]
    let program = GIT_PROGRAM.with(|p| p.borrow().clone()).unwrap_or_else(|| "git".into());
    #[cfg(not(test))]
    let program = "git";
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null()).kill_on_drop(true);
    match tokio::time::timeout(GIT_LIMIT, cmd.output()).await {
        Ok(Ok(o)) => GitRun {
            ok: o.status.success(),
            out: String::from_utf8_lossy(&o.stdout).into_owned(),
            err: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Ok(Err(e)) => GitRun { ok: false, out: String::new(), err: e.to_string() },
        Err(_) => GitRun { ok: false, out: String::new(), err: "git took too long".into() },
    }
}

async fn git_ok(dir: &Path, args: &[&str]) -> std::result::Result<String, String> {
    let run = git(dir, args).await;
    if run.ok { Ok(run.out) } else { Err(last_line(&run.err).to_string()) }
}

#[cfg(test)]
#[path = "landing_job_tests.rs"]
mod tests;
