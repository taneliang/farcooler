//! One polite comment on a pull request nobody has reviewed (ov-580,
//! ov-318 rank 7).
//!
//! **This is the runner's first write to GitHub.** Until now `gh` here only
//! read. The comment goes up as the runner's owner, in public, under their
//! name, so every rule below is a way of making it less likely to be wrong,
//! and none of them is a way of making it more frequent.
//!
//! **Threat model.** Who may be hostile: anyone who can open a pull request
//! against the repository, and a teammate who requests a review. They control
//! a pull request's title, branch, author and reviewer list, so none of those
//! is trusted to decide whom the owner speaks to: a PR is nudged only when
//! **its author is the owner** (the login `gh api user` says the runner posts
//! as, compared with GitHub's own author field; either unknown, no nudge), so a
//! teammate's PR that a card key happens to match is never nudged in the
//! owner's name. Only a **user** GitHub types as one is addressed (a team's
//! slug can be a bare word, and a bot such as Copilot is not a person), every
//! login is checked again here so nothing in the comment can be markdown or
//! a command, and the card's words, the title and the diff are not in it.
//! A fork's PR is never a candidate.
//!
//! **Off unless a workspace opts in** (`Landing::pr_nudge`), per workspace and
//! read fresh each turn, so turning it off stops the next one. Turning it on
//! takes Control scope and a person: an agent's pane is refused (`people::gate`).
//!
//! **When.** A pull request is nudged when all of these hold:
//!
//! - it is open, not a draft, not from a fork, and opened by the owner;
//! - GitHub dates it ready and the workspace's wait (24 hours unless set) has
//!   passed since;
//! - nobody has started on it, by every read the runner has: the timing read
//!   (GraphQL) lists reviews and none by anyone but the author stands *and
//!   was made within two of its intervals*, and the PR list's own fresh
//!   answer shows no review by anyone but the author and no decision of
//!   approved or changes requested. A timing read that stopped answering is
//!   not taken for a PR nobody reviewed;
//! - it asks at least one user (not a team, a bot or the author);
//! - the runner's local clock is outside 22:00 to 08:00, checked again right
//!   before each post. A turn inside those hours does nothing and the next one
//!   outside them sends; the wait is a minimum, not a schedule;
//! - nobody has sent one for it before. The one a pull request ever gets.
//!
//! **At most once, across restarts.** The send is claimed in the store
//! (`Store::claim_pr_nudge`), keyed by the pull request's repository on GitHub
//! and its number (the one the URL itself names, which must be the row's), before `gh` is run, so a second turn, a restart, a second
//! runner on the same data or a repository registered twice cannot send it
//! twice. The claim is given back only when `gh` could not be started at all.
//! Once it has run, whatever it said (a refusal, a timeout, a crash), the claim
//! stays: it may have posted, and a nudge missed is a pity while one doubled is
//! rude.
//!
//! **The words** are fixed and plain: "@alice, a gentle nudge: this is ready
//! for your review whenever you have a moment." Posted to the pull request's
//! own URL, with the body on stdin.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use farcooler_core::reviewers::login_ok;
use farcooler_store::landing::Landing;
use uuid::Uuid;

use crate::pr_timing::Moment;
use crate::service::Service;
use crate::stack::{PrInfo, PrState, ReviewDecision};

/// The first quiet hour, local time: 22:00.
pub const QUIET_FROM: u32 = 22;
/// The first hour after the quiet ones: 08:00.
pub const QUIET_UNTIL: u32 = 8;
/// How long the owner's login, once read, is trusted.
const OWNER_FOR: Duration = Duration::from_secs(3600);

/// Whether `hour` (0 to 23, local) is one when nobody is nudged.
pub fn quiet(hour: u32) -> bool {
    !(QUIET_UNTIL..QUIET_FROM).contains(&hour)
}

/// What a nudge is decided from, besides the pull request.
#[derive(Debug, Clone, Copy)]
pub struct When<'a> {
    /// The login the runner posts as. Empty when unknown, and then nobody is nudged.
    pub owner: &'a str,
    pub now_ms: i64,
    /// The runner's local hour, 0 to 23.
    pub hour: u32,
    pub after_hours: u32,
    /// The oldest a timing read may be, in milliseconds.
    pub timing_fresh_ms: i64,
}

/// Whom to address, or `None` when this pull request is not to be nudged now.
/// Everything the rules above say, with no I/O.
pub fn due(pr: &PrInfo, at: &When) -> Option<Vec<String>> {
    if quiet(at.hour) || pr.is_fork || pr.status.state != PrState::Open {
        return None;
    }
    // Opened by the owner, whom the runner posts as: unknown on either side is no.
    if at.owner.is_empty() || pr.author.is_empty() || !pr.author.eq_ignore_ascii_case(at.owner) {
        return None;
    }
    let Moment::At(ready) = pr.timing.ready else { return None };
    if at.now_ms.saturating_sub(ready) < i64::from(at.after_hours) * 3_600_000 {
        return None;
    }
    // Reviews listed and none standing: unknown is not none, and old is not now.
    if pr.timing.first_review != Moment::NotYet || at.now_ms.saturating_sub(pr.timing.read_at) > at.timing_fresh_ms {
        return None;
    }
    // The PR list is fresher than the timing read: anything it shows is a start.
    if pr.review.others_reviewed || matches!(pr.status.review_decision, ReviewDecision::Approved | ReviewDecision::ChangesRequested) {
        return None;
    }
    let mut people: Vec<String> = Vec::new();
    for asked in &pr.review.requested_users {
        let fine = login_ok(asked) && !asked.contains('/') && !asked.eq_ignore_ascii_case(&pr.author);
        if fine && !people.iter().any(|p| p.eq_ignore_ascii_case(asked)) {
            people.push(asked.clone());
        }
    }
    (!people.is_empty()).then_some(people)
}

fn when(owner: &str, now_ms: i64, after_hours: u32, timing_fresh_ms: i64) -> When<'_> {
    When { owner, now_ms, hour: farcooler_core::local_time::hour(now_ms), after_hours, timing_fresh_ms }
}

/// The comment: "@alice, a gentle nudge: ...", "@alice and @bob, ...",
/// "@alice, @bob and @carol, ...".
pub fn message(people: &[String]) -> String {
    let names: Vec<String> = people.iter().map(|p| format!("@{p}")).collect();
    let who = match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    };
    format!("{who}, a gentle nudge: this is ready for your review whenever you have a moment.")
}

/// A pull request by its repository on GitHub and number, from its URL
/// (`https://github.com/owner/repo/pull/31` is `github.com/owner/repo`): what
/// "the same pull request" means however many times the runner registered
/// the repository. `None` for a URL that isn't one, and then no nudge.
pub fn repo_key(url: &str) -> Option<String> {
    pr_ref(url).map(|(key, _)| key)
}

/// The host of a [`repo_key`] (`github.com` of `github.com/owner/repo`), which
/// is whose account the owner's login is.
pub fn host_of(key: &str) -> &str {
    key.split('/').next().unwrap_or(key)
}

/// The repository key and the pull request number the URL itself names. The
/// claim is keyed on this number, not on the one GitHub's list said, so a URL
/// and a number that disagree (a list row paired with another row's URL)
/// can't claim one pull request and comment on another.
pub fn pr_ref(url: &str) -> Option<(String, u32)> {
    let rest = url.strip_prefix("https://")?;
    let (repo, tail) = rest.split_once("/pull/")?;
    let digits = tail.split(['/', '?', '#']).next()?;
    let parts: Vec<&str> = repo.split('/').collect();
    let ok = parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && !p.contains(char::is_whitespace)) && digits.chars().all(|c| c.is_ascii_digit()) && !digits.is_empty();
    let number = digits.parse::<u32>().ok()?;
    ok.then(|| (repo.to_lowercase(), number))
}

/// How a post ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Posted {
    /// GitHub took it.
    Yes,
    /// `gh` ran and did not succeed, or did not say in time. It may have
    /// posted, so the claim stays.
    Ran,
    /// `gh` could not be started at all: certainly nothing was posted.
    NotRun,
}

/// Where a comment goes, and who it goes as: `gh`, or a test's own.
pub trait Poster {
    /// The login `gh` posts as on GitHub host `host` (`github.com`, or an
    /// enterprise one), or `None` when it can't be said. One login per host:
    /// the same runner can post as different people on different hosts.
    fn owner(&self, repository: Uuid, host: &str) -> impl std::future::Future<Output = Option<String>> + Send;
    /// Comment `body` on the pull request at `url`.
    fn post(&self, repository: Uuid, url: &str, body: &str) -> impl std::future::Future<Output = Posted> + Send;
}

/// `gh`, run in the repository's checkout.
pub struct Gh<'a> {
    pub svc: &'a Service,
}

/// The owner's login on each host, once `gh api user` has said it.
static OWNER: Mutex<Option<HashMap<String, (String, Instant)>>> = Mutex::new(None);

/// The login said for `host` within the last [`OWNER_FOR`]. Never another host's.
fn known_owner(host: &str) -> Option<String> {
    let cache = OWNER.lock().unwrap_or_else(PoisonError::into_inner);
    cache.as_ref()?.get(host).filter(|(_, at)| at.elapsed() < OWNER_FOR).map(|(login, _)| login.clone())
}

fn remember_owner(host: &str, login: &str) {
    OWNER.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(HashMap::new).insert(host.to_string(), (login.to_string(), Instant::now()));
}

impl Poster for Gh<'_> {
    async fn owner(&self, repository: Uuid, host: &str) -> Option<String> {
        // The host is on `gh`'s command line, so it is a host name and nothing else.
        if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':')) || host.starts_with('-') {
            return None;
        }
        if let Some(login) = known_owner(host) {
            return Some(login);
        }
        let repo = self.svc.store.get_repository(repository).ok()?;
        let worktree = self.svc.repository_worktree(&repo);
        let mut gh = crate::stack::gh(&worktree).await.ok()?;
        let _permit = self.svc.gh_permit().await;
        let out = tokio::time::timeout(
            crate::stack::GH_TIMEOUT,
            gh.current_dir(&worktree)
                .args(["api", "user", "--hostname", host, "--jq", ".login"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        let login = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || !login_ok(&login) {
            return None;
        }
        remember_owner(host, &login);
        Some(login)
    }

    async fn post(&self, repository: Uuid, url: &str, body: &str) -> Posted {
        if !url.starts_with("https://") || url.contains(char::is_whitespace) {
            return Posted::NotRun;
        }
        let Ok(repo) = self.svc.store.get_repository(repository) else { return Posted::NotRun };
        let worktree = self.svc.repository_worktree(&repo);
        let Ok(mut gh) = crate::stack::gh(&worktree).await else { return Posted::NotRun };
        let _permit = self.svc.gh_permit().await;
        comment(gh.current_dir(&worktree), url, body, crate::stack::GH_TIMEOUT).await
    }
}

/// Run `gh pr comment <url> --body-file -` as `gh` is set up, the body on
/// stdin, and say how it ended: it could not be started (`NotRun`), it
/// exited 0 (`Yes`), or it ran and failed, crashed, closed its stdin early or
/// did not finish within `limit` (`Ran`, since it may have posted).
async fn comment(gh: &mut tokio::process::Command, url: &str, body: &str, limit: Duration) -> Posted {
    let child = gh
        .args(["pr", "comment"])
        .arg(url)
        .args(["--body-file", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let Ok(mut child) = child else { return Posted::NotRun };
    let run = async {
        if let Some(mut stdin) = child.stdin.take() {
            tokio::io::AsyncWriteExt::write_all(&mut stdin, body.as_bytes()).await.ok()?;
        }
        child.wait().await.ok()
    };
    match tokio::time::timeout(limit, run).await {
        Ok(Some(status)) if status.success() => Posted::Yes,
        _ => Posted::Ran,
    }
}

/// One turn for one repository: nudge each pull request of an opted-in
/// board's live lanes that is due. `clock` is asked for the time at each pull
/// request, so the quiet hours are checked again right before each post.
/// Returns the numbers it nudged.
pub async fn nudge_repository(
    svc: &Service,
    repository: Uuid,
    boards: &std::collections::BTreeSet<Uuid>,
    poster: &impl Poster,
    clock: &(dyn Fn() -> i64 + Sync),
) -> Vec<u32> {
    if quiet(farcooler_core::local_time::hour(clock())) {
        return Vec::new();
    }
    let reads = crate::plan_stage::Reads::of(svc, repository);
    if !reads.known {
        return Vec::new();
    }
    // Asked only once something could be sent, and once per host: it is a call to GitHub.
    let mut owners: HashMap<String, String> = HashMap::new();
    let fresh = i64::try_from(2 * crate::pr_timing_read::every().as_millis()).unwrap_or(i64::MAX);
    let mut sent = Vec::new();
    for board in boards {
        let landing: Landing = svc.store.get_landing(*board).unwrap_or_default();
        if !landing.pr_nudge {
            continue;
        }
        let Ok(plan) = svc.store.plan(*board, i64::MIN) else { continue };
        for lane in plan.lanes.iter().filter(|l| crate::plan_stage::phase_of(l.lane.state).is_active()) {
            for number in crate::plan_stage::prs_of(lane, &plan.cards, &reads) {
                let Some(pr) = reads.prs.iter().find(|p| p.status.number == number) else { continue };
                if sent.contains(&number) {
                    continue;
                }
                let at = |owner| when(owner, clock(), landing.nudge_after_hours(), fresh);
                // Everything but who the owner is, first, so GitHub is not asked for nothing.
                if due(pr, &at(&pr.author)).is_none() {
                    continue;
                }
                // The claim is on the pull request the URL names, which must be this one.
                let Some((key, named)) = pr_ref(&pr.status.url) else { continue };
                if named != number {
                    continue;
                }
                let host = host_of(&key).to_string();
                if !owners.contains_key(&host) {
                    let login = poster.owner(repository, &host).await.unwrap_or_default();
                    owners.insert(host.clone(), login);
                }
                // And once more with the owner, the clock read again: the quiet hours are
                // those of right before the post, not of the start of the turn.
                let Some(people) = due(pr, &at(owners.get(&host).map_or("", String::as_str))) else { continue };
                // The claim comes first: it is what makes this the only one.
                if !svc.store.claim_pr_nudge(&key, number).unwrap_or(false) {
                    continue;
                }
                match poster.post(repository, &pr.status.url, &message(&people)).await {
                    Posted::Yes => sent.push(number),
                    // It may have gone: keep the claim.
                    Posted::Ran => {}
                    Posted::NotRun => {
                        let _ = svc.store.release_pr_nudge(&key, number);
                    }
                }
            }
        }
    }
    sent
}

#[cfg(test)]
#[path = "pr_nudge_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "pr_nudge_settings_tests.rs"]
mod settings_tests;
#[cfg(test)]
#[path = "pr_nudge_post_tests.rs"]
mod post_tests;
