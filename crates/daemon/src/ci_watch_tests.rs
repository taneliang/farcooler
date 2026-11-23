//! The CI watch's parsers, fed what a real `gh` printed for this repository
//! with the watch's own filters (`test/fixtures/ci/`, recorded Oct 5 with
//! `gh api -X GET ...`).

use super::*;

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/ci").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The failed commit: four workflows, CI failed on one job and had another
/// canceled. The subject reads failed, links the failing run, and names every
/// job under its workflow.
#[test]
fn a_failed_commit_reads_failed_with_its_jobs() {
    let runs = parse_runs(&fixture("runs-sha-failed.json")).expect("gh's runs parse");
    assert_eq!(runs.len(), 4);
    let jobs = parse_jobs(&fixture("jobs-failed.json")).expect("gh's jobs parse");
    assert_eq!(jobs.len(), 12);
    let with: Vec<(GhRun, Option<Vec<GhJob>>)> = latest_per_workflow(&runs)
        .into_iter()
        .map(|r| {
            let its = (r.name == "CI").then(|| jobs.clone());
            (r, its)
        })
        .collect();
    let read = summarize("sha:c85bf83d", &with);
    assert_eq!(read.status, CiStatus::Failed);
    assert_eq!(read.sha, "c85bf83dce46a6b71d7312afc623899ae7914658");
    assert_eq!(read.url, "https://github.com/taneliang/farcooler/actions/runs/37275435256", "the failing run");
    let state = |name: &str| read.jobs.iter().find(|j| j.name == name).map(|j| j.state.as_str());
    assert_eq!(state("CI / Swift (shared + macOS)"), Some("failed"));
    assert_eq!(state("CI / iOS UI (phone)"), Some("canceled"));
    assert_eq!(state("CI / Android"), Some("passed"));
    assert_eq!(state("Canary"), Some("passed"), "a run whose jobs weren't read stands as one line");
    assert_eq!(read.jobs.len(), 12 + 3);
    assert!(read.jobs.iter().find(|j| j.name == "CI / Android").unwrap().url.starts_with("https://github.com/"));
}

/// One commit's runs (here 898ae57d's, from a list of main's), one per
/// workflow, all passed.
#[test]
fn a_commit_s_runs_count_each_workflow_once() {
    let runs = parse_runs(&fixture("runs-main.json")).expect("gh's runs parse");
    assert_eq!(runs.len(), 30);
    let head: Vec<GhRun> = runs.into_iter().filter(|r| r.head_sha.starts_with("898ae57d")).collect();
    assert_eq!(head.len(), 5);
    let latest = latest_per_workflow(&head);
    let mut names: Vec<&str> = latest.iter().map(|r| r.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["CI", "Canary", "Canary wire baseline", "Doc comments"], "a re-triggered workflow counts once");
    let wire = latest.iter().find(|r| r.name == "Canary wire baseline").unwrap();
    assert_eq!(wire.id, 37286531417, "the newer of its two runs");
    let read = summarize("main", &latest.into_iter().map(|r| (r, None)).collect::<Vec<_>>());
    assert_eq!((read.status, read.jobs.len()), (CiStatus::Passed, 4));
}

/// One run by id.
#[test]
fn one_run_reads_alone() {
    let run = parse_run(&fixture("run-one.json")).expect("gh's run parses");
    assert_eq!((run.id, run.name.as_str(), run.conclusion.as_deref()), (37275435256, "CI", Some("failure")));
    let read = summarize("run:37275435256", &[(run, None)]);
    assert_eq!(read.status, CiStatus::Failed);
}

#[tokio::test]
async fn the_retry_controller_keeps_only_its_own_attempt_pending() {
    use std::cell::Cell;
    let run = parse_run(br#"{
        "id":45191956221,"name":"CI","head_sha":"6a02832c7","event":"push",
        "html_url":"","created_at":"2026-11-23T14:13:54Z","updated_at":"2026-11-23T14:54:35Z",
        "status":"completed","conclusion":"cancelled"
    }"#).unwrap();
    let finished = crate::ci_canary::epoch_secs(&run.updated_at).unwrap();
    for (title, status, age, next_attempt, pending) in [
        ("unrelated", "queued", 10, 2, true), // Completion event is still being delivered.
        ("unrelated", "queued", RETRY_EVENT_WAIT_SECS, 2, false),
        ("unrelated", "missing", 10, 2, false),
        ("Retry CI 45191956221 attempt 2", "queued", 3600, 2, true),
        ("Retry CI 45191956221 attempt 2", "completed", 10, 3, true),
        ("Retry CI 45191956221 attempt 2", "completed", 10, 2, false),
    ] {
        let source_reads = Cell::new(0);
        let observed = retry_pending(&run, Some(2), finished + age, |path| {
            let value = if path.starts_with("actions/workflows/") {
                if status == "missing" { return std::future::ready(RetryRead::error(b"gh: Not Found (HTTP 404)")); }
                serde_json::json!({"workflow_runs":[{"id":45_199_285_951_u64,"display_title":title,
                    "status":status,"conclusion":"success"}]})
            } else {
                assert_eq!(path, "actions/runs/45191956221");
                let reads = source_reads.get();
                source_reads.set(reads + 1);
                serde_json::json!({"id":run.id,"head_sha":run.head_sha,"status":"completed",
                    "head_branch":"main","conclusion":"cancelled","run_attempt":if reads == 0 { 2 } else { next_attempt }})
            };
            std::future::ready(RetryRead::Data(serde_json::to_vec(&value).unwrap()))
        }).await;
        assert_eq!(observed, pending, "{title} {status} age {age}, next attempt {next_attempt}");
    }
}

#[tokio::test]
async fn failed_submission_waits_for_recovery_or_exhaustion() {
    let run = parse_run(br#"{
        "id":45191956221,"name":"CI","head_sha":"6a02832c7","event":"push",
        "html_url":"","created_at":"2026-11-23T14:13:54Z","updated_at":"2026-11-23T14:54:35Z",
        "status":"completed","conclusion":"cancelled"
    }"#).unwrap();
    let controller_finished = "2026-11-23T15:54:35Z";
    let finished = crate::ci_canary::epoch_secs(controller_finished).unwrap();
    for (controller, recovery, age, rerun, pending) in [
        ("failure", None, 10, false, true),
        ("cancelled", None, 10, false, true),
        ("failure", None, RETRY_EVENT_WAIT_SECS, false, false),
        ("failure", Some("queued"), 3600, false, true),
        ("failure", Some("completed"), 10, false, false),
        ("failure", Some("completed"), 10, true, true),
    ] {
        let recovery_read = std::cell::Cell::new(false);
        let observed = retry_pending(&run, Some(2), finished + age, |path| {
            let value = if path.contains("ci-runner-retry-recover.yml") {
                recovery_read.set(true);
                let runs = recovery.into_iter().map(|status| serde_json::json!({
                    "id": 25, "display_title":"Recover Retry CI 45191956221 attempt 2",
                    "status":status, "conclusion":"failure"
                })).collect::<Vec<_>>();
                serde_json::json!({"workflow_runs":runs})
            } else if path.contains("ci-runner-retry.yml") {
                serde_json::json!({"workflow_runs":[{
                    "id":24, "display_title":"Retry CI 45191956221 attempt 2",
                    "status":"completed", "conclusion":controller, "updated_at":controller_finished
                }]})
            } else {
                assert_eq!(path, "actions/runs/45191956221");
                serde_json::json!({"id":run.id,"head_sha":run.head_sha,"status":"completed",
                    "head_branch":"main","conclusion":"cancelled",
                    "run_attempt":if rerun && recovery_read.get() { 3 } else { 2 }})
            };
            std::future::ready(RetryRead::Data(serde_json::to_vec(&value).unwrap()))
        }).await;
        assert_eq!(observed, pending, "{controller} {recovery:?} age {age}, rerun {rerun}");
    }
}

/// A commit with no runs yet reads none, and a run still going reads running
/// whatever has passed beside it.
#[test]
fn nothing_yet_and_still_going() {
    assert_eq!(summarize("sha:1a1b3275", &[]).status, CiStatus::None);
    let mut runs = parse_runs(&fixture("runs-main.json")).unwrap();
    runs.truncate(2);
    runs[0].status = "in_progress".into();
    runs[0].conclusion = None;
    runs[1].status = "queued".into();
    runs[1].conclusion = None;
    let read = summarize("main", &runs.iter().cloned().map(|r| (r, None)).collect::<Vec<_>>());
    assert_eq!(read.status, CiStatus::Running);
    let queued = summarize("main", &[(runs[1].clone(), None)]);
    assert_eq!(queued.status, CiStatus::Queued);
}

/// What's not gh's JSON doesn't parse into an empty list, which would read as
/// "no runs".
#[test]
fn what_gh_says_on_failure_is_not_an_empty_list() {
    assert_eq!(parse_runs(b"gh: To get started with GitHub CLI, please run: gh auth login"), None);
    assert_eq!(parse_jobs(b""), None);
}

/// Every call is a GET, with the filter the fixtures were recorded with.
#[test]
fn every_call_is_a_read() {
    let args = api_args("repos/{owner}/{repo}/actions/runs", &["head_sha=abc".into()], RUNS_JQ);
    assert_eq!(&args[..4], ["api", "-X", "GET", "repos/{owner}/{repo}/actions/runs"]);
    assert_eq!(&args[4..], ["-f", "head_sha=abc", "--jq", RUNS_JQ]);
}

/// The failed commit's CI run, canceled instead of failed, as a newer push
/// does: the subject is superseded, not failed, and links the first run
/// (review train-1005c H1).
#[test]
fn a_canceled_run_reads_superseded() {
    let mut runs = parse_runs(&fixture("runs-sha-failed.json")).unwrap();
    for run in &mut runs {
        if run.name == "CI" {
            run.conclusion = Some("cancelled".into());
        }
    }
    let read = summarize("sha:c85bf83d", &runs.into_iter().map(|r| (r, None)).collect::<Vec<_>>());
    assert_eq!(read.status, CiStatus::Superseded);
    assert_eq!(read.jobs.iter().find(|j| j.name == "CI").map(|j| j.state.as_str()), Some("canceled"));
}

/// A subject GitHub can't answer for, or with no runs, never holds the watch
/// at a minute: it backs off from a minute, doubling, to ten (review
/// train-1005c M2). Only a run that's going or waiting keeps it busy.
#[test]
fn a_read_that_fails_backs_off() {
    assert!(keeps_busy(CiStatus::Running) && keeps_busy(CiStatus::Queued));
    for quiet in [CiStatus::Unknown, CiStatus::None, CiStatus::Passed, CiStatus::Failed, CiStatus::Superseded] {
        assert!(!keeps_busy(quiet), "{quiet:?}");
    }
    let secs: Vec<u64> = (1..=7).map(|n| backoff_after(n).as_secs()).collect();
    assert_eq!(secs, [60, 120, 240, 480, 600, 600, 600]);
    assert_eq!(backoff_after(u32::MAX), QUIET);
    assert_eq!(next_wait(true, Some(0), 0), BUSY);
    assert_eq!(next_wait(false, None, 0), QUIET);
    assert_eq!(next_wait(false, Some(240_000), 0), Duration::from_secs(240));
    assert_eq!(next_wait(false, Some(1_000), 0), BUSY, "never sooner than a minute");
}

/// A run's jobs are read again while it runs, and reused only once it has
/// finished (review train-1005c L5).
#[test]
fn jobs_are_reread_until_their_run_finishes() {
    assert!(!memo_holds("in_progress"));
    assert!(!memo_holds("queued"));
    assert!(memo_holds("completed"));
}

/// The nightly iOS UI run (ov-534) names main's head, so a train pushed at that
/// commit would read it as one of its own workflows. It must not: a red shard
/// at 4 am is not the train's CI, and a running one is not "still going".
#[test]
fn the_nightly_run_is_not_a_trains_ci() {
    let run = |id: u64, name: &str, conclusion: Option<&str>| GhRun {
        id,
        name: name.into(),
        head_sha: "a".repeat(40),
        status: if conclusion.is_some() { "completed" } else { "in_progress" }.into(),
        conclusion: conclusion.map(Into::into),
        html_url: format!("https://github.com/o/r/actions/runs/{id}"),
        created_at: "2026-10-15T11:23:00Z".into(),
        display_title: String::new(),
        event: "schedule".into(),
        updated_at: String::new(),
    };
    let listed = vec![run(3, "Nightly", Some("failure")), run(2, "CI", Some("success"))];
    let latest = latest_per_workflow(&listed);
    assert_eq!(latest.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["CI"]);
    let read = summarize("sha:a", &latest.into_iter().map(|r| (r, None)).collect::<Vec<_>>());
    assert_eq!(read.status, CiStatus::Passed, "a red night does not redden the train");
    let running = latest_per_workflow(&[run(4, "Nightly", None), run(2, "CI", Some("success"))]);
    assert_eq!(summarize("sha:a", &running.into_iter().map(|r| (r, None)).collect::<Vec<_>>()).status, CiStatus::Passed);
}

#[test]
fn completed_landing_is_not_polled_and_running_landing_is_rate_limited() {
    let mut finished = unknown("sha:feed");
    finished.status = CiStatus::Passed;
    // A historical run is older than even the quiet refresh window.
    let minute = BUSY.as_millis() as i64;
    let now = 20 * minute;
    let mut calls = 0;
    for tick in now..now + minute {
        calls += usize::from(poll_due(Some(&finished), None, tick));
    }
    assert_eq!(calls, 0, "an idle board's finished subjects must not reach gh");
    let mut running = finished;
    running.status = CiStatus::Running;
    assert!(poll_due(Some(&running), None, now));
    assert!(!poll_due(Some(&running), Some(now), now + minute - 1));
    assert!(poll_due(Some(&running), Some(now), now + minute));
}
