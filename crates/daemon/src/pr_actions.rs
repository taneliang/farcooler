//! A phone acting on a pull request as the runner's `gh` login (ov-317, team
//! mode T6, ov-305 section 5.3): `pr.ready` marks a lane's draft pull request
//! ready for review, and `pr.thread_reply` replies to a review thread an agent
//! escalated. Both are Control scope, and both run `gh` on the runner with the
//! runner owner's own account, so each says who it acts as before it acts.
//!
//! **Threat model.** Who may be hostile, and what each controls:
//! - *The agent* writes the thread's link and the reviewer's name on its card
//!   (`task ask --thread`), and the card's quote. So the runner never posts to
//!   a link as written: it parses it into host, owner, repository, number and
//!   comment id, each checked against what GitHub allows, and rebuilds every
//!   URL and API path from those parts. A link it cannot read to exactly one
//!   comment or one pull request is refused, never posted to a neighbor.
//! - *A peer below Control* is refused before anything runs (`rpc::scope_of`).
//! - *A phone at Control* is a shell on this runner already (`pr_review`'s
//!   note), so what it types is the person's own words. They are length-capped
//!   and travel on `gh`'s stdin, never in an argument.
//! - *A phone's choice of pull request*: `pr.ready` names a lane, not a URL or
//!   a repository, and the number must be one the lane's own cards work.
//!
//! - *The board's own repository*: a reply is posted only to a repository that
//!   is a `git remote` of the card's repository. An agent that escalates a link
//!   into another repository its owner's login can reach gets a refusal.
//! - *A thread that changes between the preview and the send*: the send carries
//!   the question's note id and a digest of where the link resolves, from the
//!   preview, and is refused when either differs.
//!
//! A post also answers the card: an ANSWER note ("Replied on GitHub as <login>:
//! <text>") is written by the same verb, so the Needs You row clears by the
//! rule every answer clears by (`needs_you::waiting_question`), and a second send
//! is refused. The post and the note run **detached** (`tokio::spawn`), so a
//! phone that drops the connection mid-send cannot cancel `gh` half way
//! (`kill_on_drop`) and send again on a retry, and a send for a question already
//! in flight is refused. The send takes that hold and then reads the card again, so
//! a send that waited on `gh` while another posted and finished is refused too.
//! The hold ends on every path (`Flight`), except when the post went through and
//! the note could not be written (after one retry): then it is kept, so retries
//! are refused until the daemon restarts. A daemon that dies between the post and
//! the note is the one window left; it would show the row again and the retry would
//! double-post.
//!
//! The reply is two steps in one verb (`PrThreadReply.send`): a preview that
//! posts nothing and answers where the reply would go and as whom, for the
//! confirm sheet, and the send. Neither trusts the other: the send resolves the
//! thread again.

use std::path::Path;
use std::sync::Arc;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, request, result};
use farcooler_store::models::{NoteKind, TaskStatus};

use crate::service::Service;
use crate::watch::Watcher;
use crate::stack::PrState;

/// The longest reply, in characters. A thread reply is a few sentences; past
/// this a person writes it in GitHub.
pub const REPLY_MAX: usize = 2000;

/// Where a thread's link points, once read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub number: u32,
    /// The review comment the link names (`#discussion_r<id>`), or none when
    /// it names the pull request or one conversation comment on it.
    pub comment: Option<u64>,
}

impl Target {
    /// A digest of where the link resolves, which the preview hands the person's
    /// phone and the send must hand back unchanged.
    pub fn digest(&self) -> String {
        use sha2::Digest;
        let text = format!("{}\n{}\n{}\n{}\n{}", self.host, self.owner, self.repo, self.number, self.comment.unwrap_or(0));
        sha2::Sha256::digest(text.as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect()
    }

    /// Whether `host/owner/repo` is one of the card's repository's remotes.
    pub fn is_remote(&self, remotes: &[(String, String, String)]) -> bool {
        remotes.iter().any(|(h, o, r)| *h == self.host && *o == self.owner.to_ascii_lowercase() && *r == self.repo.to_ascii_lowercase())
    }

    /// `owner/name`.
    pub fn repository(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// The pull request's page, rebuilt from the checked parts.
    pub fn pull_url(&self) -> String {
        format!("https://{}/{}/{}/pull/{}", self.host, self.owner, self.repo, self.number)
    }
}

/// A GitHub owner or repository name: letters, digits and `-`, `_`, `.`, but
/// never `.` or `..`, and at most 100 characters.
fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A thread's link, read: `https://<host>/<owner>/<repo>/pull/<n>` with
/// nothing after the number, and a fragment that is `#discussion_r<id>`
/// (replied to in its thread), `#issuecomment-<id>` or `#pullrequestreview-<id>`
/// (left on the pull request), or none. Anything else is `None`, which is a
/// refusal: a link this cannot read to one place is not posted to.
pub(crate) fn resolve(url: &str, remote_hosts: &[String]) -> Option<Target> {
    if !farcooler_core::thread_link::thread_link_ok(url, remote_hosts) {
        return None;
    }
    let (before, fragment) = match url.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (url, None),
    };
    // A query changes nothing GitHub shows of a pull request, and nothing is
    // taken from it.
    let before = before.split('?').next()?;
    let rest = before.strip_prefix("https://")?;
    let mut parts = rest.split('/');
    let (host, owner, repo, pull, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if parts.any(|extra| !extra.is_empty()) || pull != "pull" || !name_ok(owner) || !name_ok(repo) {
        return None;
    }
    let number: u32 = number.parse().ok().filter(|n| *n > 0)?;
    let comment = match fragment {
        None | Some("") => None,
        Some(f) => {
            let id = |prefix: &str| f.strip_prefix(prefix).filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
            if let Some(d) = id("discussion_r") {
                Some(d.parse::<u64>().ok()?)
            } else if id("issuecomment-").is_some() || id("pullrequestreview-").is_some() {
                None
            } else {
                return None;
            }
        }
    };
    Some(Target { host: host.to_ascii_lowercase(), owner: owner.to_string(), repo: repo.to_string(), number, comment })
}

/// The reply's text, checked: not empty once trimmed, at most `REPLY_MAX`
/// characters, and no NUL.
pub(crate) fn body_ok(body: &str) -> Result<&str> {
    let body = body.trim();
    if body.is_empty() || body.chars().count() > REPLY_MAX || body.contains('\0') {
        return Err(DomainError::InvalidArgument { what: "reply_body" });
    }
    Ok(body)
}

/// Run `gh` in `tree` with `args`, `stdin` on its input, and answer its
/// standard output. Whatever goes wrong (no `gh`, logged out, offline, a
/// refusal from GitHub) is one answer, `PrStateUnavailable`: the apps say it
/// could not be done, and nothing here quotes `gh`'s words, which carry the
/// text sent.
async fn gh(svc: &Service, tree: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    use tokio::io::AsyncWriteExt;
    let mut cmd = crate::stack::gh(tree).await.map_err(|_| DomainError::PrStateUnavailable)?;
    let _permit = svc.gh_permit().await;
    cmd.current_dir(tree)
        .args(args)
        .stdin(if stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|_| DomainError::PrStateUnavailable)?;
    if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
        // A write that fails is `gh` having stopped reading: its exit says why.
        let _ = input.write_all(text.as_bytes()).await;
    }
    let out = tokio::time::timeout(crate::stack::GH_TIMEOUT, child.wait_with_output()).await;
    match out {
        Ok(Ok(o)) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        _ => Err(DomainError::PrStateUnavailable),
    }
}

/// The (host, owner, repository) of each `git remote` of `tree`, lowercased.
async fn remotes(tree: &Path) -> Vec<(String, String, String)> {
    let Ok(out) = crate::git::git(tree, &["remote", "-v"]).await else { return Vec::new() };
    let mut found: Vec<_> =
        out.stdout.lines().filter_map(|line| line.split_whitespace().nth(1)).filter_map(farcooler_core::thread_link::remote_repo).collect();
    found.sort();
    found.dedup();
    found
}

/// A send in flight for a question, held until it ends. Dropped on every path
/// (an early return, a panic) it lets the question go, unless the reply was posted
/// and its note was not written: then it is kept, so a retry is refused until the
/// daemon restarts rather than posting twice.
struct Flight {
    question: uuid::Uuid,
    posted_unrecorded: bool,
}

impl Flight {
    /// Take the question, or `None` when a send holds it already.
    fn take(question: uuid::Uuid) -> Option<Flight> {
        let fresh = in_flight().lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(question);
        fresh.then_some(Flight { question, posted_unrecorded: false })
    }
}

impl Drop for Flight {
    fn drop(&mut self) {
        if !self.posted_unrecorded {
            in_flight().lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&self.question);
        }
    }
}

/// The questions a send is in flight for: a second send for one is refused.
fn in_flight() -> &'static std::sync::Mutex<std::collections::HashSet<uuid::Uuid>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<uuid::Uuid>>> = std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
}

/// `pr.ready` and `pr.thread_reply`, as `Rpc::dispatch` hands them over.
pub(crate) async fn dispatch(svc: &Arc<Service>, watcher: &Arc<Watcher>, req: Request) -> Result<result::Value> {
    match req.payload {
        Some(request::Payload::PrReady(p)) => Ok(result::Value::PrReadied(ready(svc, &p).await?)),
        Some(request::Payload::PrThreadReply(p)) => Ok(result::Value::PrThreadReplied(thread_reply(svc, watcher, &p).await?)),
        _ => Err(DomainError::InvalidArgument { what: "payload" }),
    }
}

/// Mark one of a lane's draft pull requests ready for review.
pub(crate) async fn ready(svc: &Service, p: &pb::PrReady) -> Result<pb::PrReadied> {
    let lane = svc.store.lane(crate::task_ops::required_id(&p.lane_id)?)?;
    let workspace = svc.store.get_workspace(lane.workspace_id)?;
    let repository = svc.store.get_repository(workspace.repository_id)?;
    let tree = svc.repository_worktree(&repository);

    // The lane's own pull requests, from what the runner last read: a number
    // outside them is not this lane's, whoever asks.
    let plan = svc.store.plan(lane.workspace_id, i64::MIN)?;
    let view = plan.lanes.iter().find(|v| v.lane.id == lane.id).ok_or(DomainError::NotFound)?;
    let reads = crate::plan_stage::Reads::of(svc, repository.id);
    if !reads.known {
        return Err(DomainError::PrStateUnavailable);
    }
    let theirs = crate::plan_stage::prs_of(view, &plan.cards, &reads);
    let number = match p.pr_number {
        0 => {
            let drafts: Vec<u32> = theirs
                .iter()
                .copied()
                .filter(|n| reads.prs.iter().any(|pr| pr.status.number == *n && pr.status.state == PrState::Draft))
                .collect();
            match drafts.as_slice() {
                [only] => *only,
                _ => return Err(DomainError::InvalidArgument { what: "lane_pr" }),
            }
        }
        n if theirs.contains(&n) => n,
        _ => return Err(DomainError::InvalidArgument { what: "lane_pr" }),
    };

    // Ask GitHub now rather than trust the read: a pull request someone else
    // already readied, or closed, is not this verb's to touch.
    let number_arg = number.to_string();
    let seen = gh(svc, &tree, &["pr", "view", &number_arg, "--json", "isDraft,state,url,headRefName,author,isCrossRepository"], None).await?;
    let seen: serde_json::Value = serde_json::from_str(&seen).map_err(|e| {
        tracing::warn!(class = ?e.classify(), "could not read gh's pull request");
        DomainError::PrStateUnavailable
    })?;
    if seen["state"] != "OPEN" {
        return Err(DomainError::InvalidArgument { what: "lane_pr" });
    }
    // It is the lane's own: on the lane's branch, or opened by this login. The cards' keys
    // name it in the list the runner read, which anyone who can open a PR can arrange.
    // A branch of a fork named like the lane's is not the lane's.
    let on_branch = !lane.branch.is_empty()
        && seen["headRefName"].as_str() == Some(lane.branch.as_str())
        && seen["isCrossRepository"] == false;
    if !on_branch {
        let host = seen["url"].as_str().and_then(farcooler_core::thread_link::https_host).unwrap_or_else(|| farcooler_core::thread_link::GITHUB_HOST.into());
        let login = gh(svc, &tree, &["api", "--hostname", &host, "user", "--jq", ".login"], None).await?;
        let author = seen["author"]["login"].as_str().unwrap_or_default();
        if author.is_empty() || author != login.trim() {
            return Err(DomainError::InvalidArgument { what: "lane_pr" });
        }
    }
    let url = seen["url"].as_str().filter(|u| u.starts_with("https://")).unwrap_or_default().to_string();
    let already_ready = seen["isDraft"] == false;
    if !already_ready {
        gh(svc, &tree, &["pr", "ready", &number_arg], None).await?;
        tracing::info!(number, "marked a lane's pull request ready for review");
        // Its stage moves from Agent Review to Waiting on a reviewer: read it now.
        crate::pr_watch::kick_lane(lane.id, repository.id);
    }
    Ok(pb::PrReadied { number, url, already_ready })
}

/// Preview, or post, a reply to the review thread a card escalated.
pub(crate) async fn thread_reply(svc: &Arc<Service>, watcher: &Arc<Watcher>, p: &pb::PrThreadReply) -> Result<pb::PrThreadReplied> {
    let task = svc.store.get_task(crate::task_ops::required_id(&p.task_id)?)?;
    let refuse = |what: &'static str| DomainError::InvalidArgument { what };
    // The thread is the one the card is waiting on now, read the way the Needs
    // You row reads it, so the sheet and the row cannot name different ones.
    if task.status != TaskStatus::NeedsDecision {
        return Err(refuse("thread_link"));
    }
    let notes: Vec<_> =
        svc.store.notes_for(task.id, None)?.into_iter().filter(|n| matches!(n.kind, NoteKind::Question | NoteKind::Answer)).collect();
    // `Some(None)` is a card waiting on no question; `None` is one already answered.
    let Some(waiting) = crate::needs_you::waiting_question(&task, &notes) else { return Err(refuse("already_answered")) };
    let note = waiting.ok_or(refuse("thread_link"))?;
    let repository = svc.store.get_repository(task.repository_id)?;
    let tree = svc.repository_worktree(&repository);
    let remotes = remotes(&tree).await;
    let hosts: Vec<String> = remotes.iter().map(|r| r.0.clone()).collect();
    let thread = crate::needs_you::pr::thread_of(note, &hosts).ok_or(refuse("thread_link"))?;
    let target = resolve(thread.url, &hosts).filter(|t| t.is_remote(&remotes)).ok_or(refuse("thread_link"))?;

    let note_id = bytes::Bytes::copy_from_slice(note.id.as_bytes());
    let digest = target.digest();
    if p.send && (p.note_id.as_ref() != note_id.as_ref() || p.digest != digest) {
        return Err(refuse("thread_link"));
    }
    let body = if p.send { Some(body_ok(&p.body)?.to_string()) } else { None };

    let login = gh(svc, &tree, &["api", "--hostname", &target.host, "user", "--jq", ".login"], None).await?;
    let login = login.trim().to_string();
    if !farcooler_core::thread_link::reviewer_ok(&login) {
        return Err(DomainError::PrStateUnavailable);
    }
    let mut out = pb::PrThreadReplied {
        repository: target.repository(),
        number: target.number,
        login: login.clone(),
        sent: false,
        in_thread: target.comment.is_some(),
        note_id: note_id.clone(),
        digest,
    };
    let Some(body) = body else { return Ok(out) };

    // Detached: dropping this request (a phone's connection) must not kill `gh` half way
    // and invite a retry that posts twice. One send per question at a time.
    let mut flight = Flight::take(note.id).ok_or(refuse("already_answered"))?;
    // Read the card again now that the question is held: a send that waited on `gh` above
    // while another posted and finished passed the same check, and must not post again.
    let notes: Vec<_> =
        svc.store.notes_for(task.id, None)?.into_iter().filter(|n| matches!(n.kind, NoteKind::Question | NoteKind::Answer)).collect();
    let task_now = svc.store.get_task(task.id)?;
    if crate::needs_you::waiting_question(&task_now, &notes).flatten().map(|n| n.id) != Some(note.id) {
        return Err(refuse("already_answered"));
    }
    let (svc, watcher, tree, thread_url) = (svc.clone(), watcher.clone(), tree, thread.url.to_string());
    let task_id = task.id;
    let done = tokio::spawn(async move {
        let sent = send(&svc, &tree, &target, &body).await;
        if sent.is_ok() {
            tracing::info!(repository = %target.repository(), number = target.number, chars = body.chars().count(), "replied to a review thread");
            let answer = pb::TaskNoteAppend {
                task_id: bytes::Bytes::copy_from_slice(task_id.as_bytes()),
                kind: pb::TaskNoteKind::Answer as i32,
                body: format!("Replied on GitHub as {login}: {body}"),
                extra_json: serde_json::json!({ "thread_reply": { "url": thread_url, "login": login, "comment": target.comment } }).to_string(),
                supersedes: None,
                actor: "user".into(),
            };
            // Once more if the first write fails; else the post stands unrecorded and is kept held.
            if crate::task_ops::note(&svc, &watcher, &answer).is_err() && crate::task_ops::note(&svc, &watcher, &answer).is_err() {
                tracing::warn!("a reply was posted and its answer note was not written");
                flight.posted_unrecorded = true;
            }
        }
        drop(flight);
        sent
    });
    done.await.map_err(|_| DomainError::OperationFailed)??;
    out.sent = true;
    Ok(out)
}

/// The post itself: the comment's thread, or the pull request.
async fn send(svc: &Service, tree: &Path, target: &Target, body: &str) -> Result<()> {
    match target.comment {
        Some(id) => {
            let path = format!("repos/{}/{}/pulls/{}/comments/{}/replies", target.owner, target.repo, target.number, id);
            gh(svc, tree, &["api", "--hostname", &target.host, "-X", "POST", &path, "-F", "body=@-"], Some(body)).await?;
        }
        None => {
            let url = target.pull_url();
            gh(svc, tree, &["pr", "comment", &url, "--body-file", "-"], Some(body)).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "pr_actions_tests.rs"]
mod tests;
