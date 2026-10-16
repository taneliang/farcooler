//! The two pull-request kinds of the rollup (ov-315): a review request and a
//! review thread the agent escalated. Its own file to keep `needs_you.rs`
//! inside its size budget.

use farcooler_store::models;
use farcooler_protocol::v1::NeedsYouKind;

use super::{Signal, Subject};
use crate::pr_review::ReviewRequest;

/// The hosts of `repository`'s git remotes, lowercased: what a thread's link
/// may name besides github.com. Empty when git cannot say.
pub(super) async fn remote_hosts(svc: &crate::service::Service, repository: uuid::Uuid) -> Vec<String> {
    let Ok(repo) = svc.store.get_repository(repository) else { return Vec::new() };
    let tree = svc.repository_worktree(&repo);
    let Ok(out) = crate::git::git(&tree, &["remote", "-v"]).await else { return Vec::new() };
    let mut hosts: Vec<String> = out
        .stdout
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(farcooler_core::thread_link::remote_host)
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

/// A row is one line wide on the narrowest client; a title past this is cut
/// with an ellipsis.
const QUESTION_WIDTH: usize = 120;

/// One line of `text`, whitespace collapsed, cut to `width` characters.
fn one_line(text: &str, width: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= width {
        return flat;
    }
    let mut cut: String = flat.chars().take(width.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}

/// `pr-review:` and sixteen hex characters of the URL's hash: stable across
/// reads, and it names no repository, because an item's id reaches a client
/// below Control (`redact_below_control`) and a URL spells the repository.
pub(super) fn item_id(url: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(url.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("pr-review:{hex}")
}

/// An open pull request that asks for this login's review. The title is the
/// question; where it is and who opened it go beneath, in `detail`.
pub(super) fn request_signal(index: usize, request: &ReviewRequest) -> Signal {
    let place = match (request.repository.as_str(), request.author.as_str()) {
        ("", "") => format!("#{}", request.number),
        (repo, "") => format!("{repo} #{}", request.number),
        ("", author) => format!("#{} \u{b7} {author}", request.number),
        (repo, author) => format!("{repo} #{} \u{b7} {author}", request.number),
    };
    Signal {
        kind: NeedsYouKind::PrReview,
        id: item_id(&request.url),
        since_ms: request.created_at,
        subject: Subject::Request(index),
        terminal: None,
        question: Some(one_line(&request.title, QUESTION_WIDTH))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "A pull request wants your review".to_string()),
        detail: Some(place),
        ask_id: None,
        // Review on GitHub is the phones' action (ov-317); the URL rides in `link`.
        actions: Vec::new(),
        link: Some(request.url.clone()),
    }
}

/// A review thread an agent escalated: who it is with and where it lives.
pub(super) struct Thread<'a> {
    pub url: &'a str,
    pub reviewer: Option<&'a str>,
}

/// The thread a QUESTION note escalates, from its `extra.thread`
/// (`farcooler task ask --thread <url> --reviewer <login>`). A note without
/// one is an ordinary decision, and so is one whose link is not GitHub's or
/// one of the board's own remotes' (`remote_hosts`; `thread_link`), so a row
/// never offers a link it should not. A reviewer that is not a login is left
/// out, not quoted: an agent wrote both, and the quote is its claim.
pub(super) fn thread_of<'a>(note: &'a models::TaskNote, remote_hosts: &[String]) -> Option<Thread<'a>> {
    let thread = note.extra.get("thread")?;
    let url = thread.get("url")?.as_str()?;
    if !farcooler_core::thread_link::thread_link_ok(url, remote_hosts) {
        return None;
    }
    let reviewer = thread
        .get("reviewer")
        .and_then(|r| r.as_str())
        .filter(|r| farcooler_core::thread_link::reviewer_ok(r));
    Some(Thread { url, reviewer })
}

/// `owner/name #12` from a thread's URL
/// (`https://github.com/owner/name/pull/12#discussion_r1`), or none when the
/// URL is shaped otherwise.
fn pull_request_of(url: &str, remote_hosts: &[String]) -> Option<String> {
    if !farcooler_core::thread_link::thread_link_ok(url, remote_hosts) {
        return None;
    }
    let path = url.strip_prefix("https://")?.split(['#', '?']).next()?;
    let mut parts = path.split('/').skip(1);
    let (owner, name, pull, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    (pull == "pull" && !owner.is_empty() && !name.is_empty() && number.parse::<u32>().is_ok())
        .then(|| format!("{owner}/{name} #{number}"))
}

/// A decision about a thread, as the thread's own kind: the question quotes
/// the reviewer, and the card's options still answer it.
pub(super) fn thread_signal(
    task: &models::Task,
    note: &models::TaskNote,
    thread: Thread<'_>,
    remote_hosts: &[String],
) -> Signal {
    let quote = one_line(&note.body, QUESTION_WIDTH);
    // The words are the agent's report of a reviewer, not the reviewer's, and
    // the row says so: anyone who can write a note can write any quote.
    let question = match (thread.reviewer, quote.is_empty()) {
        (Some(who), false) => format!("{who} asked, says the agent: \u{201c}{quote}\u{201d}"),
        (Some(who), true) => format!("{who} left a comment, says the agent"),
        (None, false) => format!("A reviewer asked, says the agent: \u{201c}{quote}\u{201d}"),
        (None, true) => "A review thread needs you".to_string(),
    };
    Signal {
        kind: NeedsYouKind::PrThread,
        id: format!("pr-thread:{}", task.id),
        since_ms: note.at,
        subject: Subject::Task(task.id),
        terminal: None,
        question,
        detail: pull_request_of(thread.url, remote_hosts),
        ask_id: None,
        actions: super::decision_options(note).iter().map(|o| super::action(o, o, false, false)).collect(),
        link: Some(thread.url.to_string()),
    }
}

#[cfg(test)]
#[path = "pr_tests.rs"]
mod tests;
