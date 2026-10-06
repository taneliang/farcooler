//! Which Canary run belongs to a commit, now that Canary runs after CI
//! (ov-301).
//!
//! Canary is a `workflow_run` workflow: GitHub gives its run the `head_sha` of
//! main's head at the moment it started, not of the commit CI passed. Listing
//! runs by a train's SHA therefore misses the Canary run for that commit, or
//! worse, finds another commit's: when the baseline bot's commit lands on top
//! of the train's, Canary's run for the train carries the bot's SHA, and a
//! Canary failure would show on the bot's commit instead. The commit a Canary
//! run builds is in its run name instead: `Canary <sha>` (canary.yml's
//! `run-name`, which GitHub reports as the run's `display_title`).
//!
//! And a train's CI going green no longer means Canary has begun: between CI
//! completing and the Canary run appearing the commit reads passed. While CI
//! has passed within `PENDING_FOR_SECS` and no Canary run names the commit, a
//! placeholder queued run stands for it, so the train does not move to green
//! before Canary's run exists. After the window it is dropped, so a Canary that
//! never starts (the workflow disabled) cannot hold a train forever.

use crate::ci_watch::GhRun;

/// The workflow's name, as the runs API reports it.
pub const WORKFLOW: &str = "Canary";
/// Canary's own runs, newest first.
pub const WORKFLOW_RUNS: &str = "repos/{owner}/{repo}/actions/workflows/canary.yml/runs";
/// How long after CI passes a missing Canary run is still expected.
pub const PENDING_FOR_SECS: i64 = 15 * 60;

/// The run name Canary gives a run that builds `sha`.
pub fn title(sha: &str) -> String {
    format!("{WORKFLOW} {sha}")
}

fn ci_passed(run: &GhRun) -> bool {
    run.name == "CI" && run.status == "completed" && run.conclusion.as_deref() == Some("success") && run.event == "push"
}

/// Whether Canary's own runs are worth listing for these: CI has passed on a
/// push, which is what starts one.
pub fn worth_looking(runs: &[GhRun]) -> bool {
    runs.iter().any(ci_passed)
}

/// `listed` (the runs whose `head_sha` is `sha`) with Canary's runs attributed
/// by name: a workflow_run Canary run that names another commit is dropped, and
/// Canary's runs that name this one are added.
///
/// Every other workflow_run run is dropped too (ov-341). "Canary wire baseline"
/// is one: it follows Canary and carries the SHA of main's head when it
/// started, which can be a train's, and its failure turned that train red. A
/// workflow that follows another says nothing of this commit.
pub fn attribute(sha: &str, listed: Vec<GhRun>, canary: Vec<GhRun>) -> Vec<GhRun> {
    let want = title(sha);
    let mut runs: Vec<GhRun> = listed
        .into_iter()
        .filter(|r| r.event != "workflow_run" || (r.name == WORKFLOW && r.display_title == want))
        .collect();
    for run in canary {
        if run.name == WORKFLOW && run.display_title == want && !runs.iter().any(|r| r.id == run.id) {
            runs.push(run);
        }
    }
    runs
}

/// Seconds since the epoch of an `YYYY-MM-DDTHH:MM:SSZ` stamp, as GitHub prints.
pub fn epoch_secs(stamp: &str) -> Option<i64> {
    let b = stamp.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let n = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, s) = (n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3_600 + mi * 60 + s)
}

/// A queued placeholder for the Canary run that has not appeared yet: CI
/// passed within the window and nothing in `runs` is a Canary run.
pub fn pending(runs: &[GhRun], sha: &str, now_secs: i64) -> Option<GhRun> {
    if runs.iter().any(|r| r.name == WORKFLOW) {
        return None;
    }
    let ci = runs.iter().find(|r| ci_passed(r))?;
    let finished = epoch_secs(&ci.updated_at)?;
    if now_secs - finished > PENDING_FOR_SECS {
        return None;
    }
    Some(GhRun {
        id: 0,
        name: WORKFLOW.to_string(),
        head_sha: sha.to_string(),
        status: "queued".into(),
        conclusion: None,
        html_url: ci.html_url.clone(),
        created_at: ci.updated_at.clone(),
        display_title: title(sha),
        event: "workflow_run".into(),
        updated_at: ci.updated_at.clone(),
    })
}

/// A commit's runs, once Canary's are attributed: the newest of each workflow,
/// plus a placeholder while Canary's run is yet to start.
///
/// `None`, which the watch reads as unknown and never lets replace a known
/// read, when the answer would be a guess:
///   - Canary's list could not be read (`canary` is `None`). Taking that for
///     "no Canary run" let a red train read green until the next good read.
///   - CI passed on a push more than `PENDING_FOR_SECS` ago and no Canary run
///     names the commit. Its run may have aged out of the list's newest 50,
///     and reading CI alone would flip a train whose Canary failed to green.
pub fn settle(sha: &str, listed: Vec<GhRun>, canary: Option<Vec<GhRun>>, now_secs: i64) -> Option<Vec<GhRun>> {
    let canary = canary?;
    let mut runs = crate::ci_watch::latest_per_workflow(&attribute(sha, listed, canary));
    match pending(&runs, sha, now_secs) {
        Some(placeholder) => runs.push(placeholder),
        None if runs.iter().any(ci_passed) && !runs.iter().any(|r| r.name == WORKFLOW) => return None,
        None => {}
    }
    Some(runs)
}

#[cfg(test)]
#[path = "ci_canary_tests.rs"]
mod tests;
