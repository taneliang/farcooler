//! The timing read (ov-579): one GraphQL call per repository for the lane PRs,
//! apart from the PR list, so a slow or failing read costs the stage nothing
//! (`pr_timing` says what is kept and why).
//!
//! **What it asks.** Per PR, by alias: `createdAt`, `isDraft`, the diff size,
//! the author, the first fifty reviews (login, state, time) and the latest
//! `READY_FOR_REVIEW_EVENT`. No review bodies, no files, no comments.
//!
//! **What it costs.** GitHub prices a GraphQL call as the requests its
//! connections need divided by a hundred, at least one point. Each PR adds two
//! connections (reviews, the timeline) of at most fifty and one nodes, so a
//! repository's batch of `BATCH` PRs is about `1 + 2 * BATCH` requests: one
//! point for up to forty-nine PRs. A repository with live lanes is read every
//! `EVERY`, about twelve calls and twelve points an hour of GitHub's 5,000, and
//! backs off as the PR watch does (`pr_watch::backoff_after`) when `gh` cannot
//! answer.
//!
//! A read that fails changes nothing: the cache keeps the last timing of every
//! PR (`Service::pr_timing_put`, `stack::merge_read`).

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::pr_timing::{PrTiming, Read, ReviewFact};
use crate::stack::parse_iso8601_millis;

/// How often a repository with live lanes has its timing read.
pub const EVERY: Duration = Duration::from_secs(5 * 60);
/// The variable a debug `farcoolerd` reads for an `EVERY` of its own, in
/// milliseconds, so a test sees a second read within the second. A release
/// build never reads it (`every`).
pub const TEST_EVERY_MS: &str = "FARCOOLER_TEST_TIMING_EVERY_MS";

/// `EVERY`, unless this is a debug build started with `TEST_EVERY_MS` set to at
/// least 100.
pub fn every() -> Duration {
    #[cfg(debug_assertions)]
    {
        let ms = std::env::var(TEST_EVERY_MS).ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|ms| *ms >= 100);
        if let Some(ms) = ms {
            return Duration::from_millis(ms);
        }
    }
    EVERY
}

/// The most PRs one call asks about: the newest lane PRs.
pub const BATCH: usize = 40;

#[derive(Deserialize)]
struct Login {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct Review {
    author: Option<Login>,
    #[serde(default)]
    state: String,
    #[serde(rename = "submittedAt", default)]
    submitted_at: Option<String>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    #[serde(default = "Vec::new")]
    nodes: Vec<T>,
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "createdAt", default)]
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct Pr {
    #[serde(rename = "createdAt", default)]
    created_at: Option<String>,
    #[serde(rename = "isDraft", default)]
    is_draft: bool,
    author: Option<Login>,
    additions: Option<u32>,
    deletions: Option<u32>,
    reviews: Option<Nodes<Review>>,
    #[serde(rename = "timelineItems")]
    timeline_items: Option<Nodes<Event>>,
}

/// The query for `numbers`, each PR under the alias `p<number>`.
pub fn query(numbers: &[u32]) -> String {
    let each = "createdAt isDraft additions deletions author{login} \
        reviews(first:50){nodes{author{login} state submittedAt}} \
        timelineItems(itemTypes:[READY_FOR_REVIEW_EVENT], last:1){nodes{... on ReadyForReviewEvent{createdAt}}}";
    let aliased: String = numbers.iter().map(|n| format!("p{n}:pullRequest(number:{n}){{{each}}} ")).collect();
    format!("query($owner:String!,$name:String!){{repository(owner:$owner,name:$name){{{aliased}}}}}")
}

/// What `gh api graphql` printed for `query`, as each PR's timing. `None` for
/// anything that is not that answer, which must never read as "no PRs". A PR
/// GitHub returned `null` for is left out, and so keeps what was known.
pub fn parse(stdout: &[u8]) -> Option<HashMap<u32, PrTiming>> {
    let v: serde_json::Value = match serde_json::from_slice(stdout) {
        Ok(v) => v,
        Err(e) => {
            // The kind of failure, never its text: a decoder quotes its input.
            tracing::warn!(class = ?e.classify(), "could not read gh's timing answer");
            return None;
        }
    };
    let repository = v.get("data")?.get("repository")?.as_object()?;
    let mut out = HashMap::new();
    for (alias, value) in repository {
        let Some(number) = alias.strip_prefix('p').and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(pr) = serde_json::from_value::<Pr>(value.clone()) else { continue };
        let at = |t: &Option<String>| t.as_deref().and_then(parse_iso8601_millis);
        let reviews = pr.reviews.as_ref().map(|r| {
            r.nodes
                .iter()
                .map(|n| ReviewFact {
                    reviewer: n.author.as_ref().map_or("", |a| a.login.as_str()),
                    state: n.state.as_str(),
                    at: at(&n.submitted_at),
                })
                .collect()
        });
        out.insert(
            number,
            PrTiming::of(Read {
                opened: at(&pr.created_at),
                draft: pr.is_draft,
                ready_event: pr.timeline_items.as_ref().and_then(|t| t.nodes.last()).and_then(|e| at(&e.created_at)),
                author: pr.author.as_ref().map_or("", |a| a.login.as_str()),
                reviews,
                additions: pr.additions,
                deletions: pr.deletions,
            }),
        );
    }
    Some(out)
}

/// The timing of `numbers`, or `None` when `gh` could not say.
pub async fn fetch(worktree: &Path, numbers: &[u32]) -> Option<HashMap<u32, PrTiming>> {
    if numbers.is_empty() {
        return Some(HashMap::new());
    }
    let mut gh = crate::stack::gh(worktree).await.ok()?;
    let out = tokio::time::timeout(
        crate::stack::GH_TIMEOUT,
        gh.current_dir(worktree)
            .args(["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-f"])
            .arg(format!("query={}", query(numbers)))
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
    parse(&out.stdout)
}

#[cfg(test)]
#[path = "pr_timing_read_tests.rs"]
mod tests;
