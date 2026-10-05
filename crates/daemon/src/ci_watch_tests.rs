//! The CI watch's parsers, fed what a real `gh` printed for this repository
//! with the watch's own filters (`test/fixtures/ci/`, recorded Oct 5 with
//! `gh api -X GET ... --jq <RUNS_JQ | RUN_JQ | JOBS_JQ>`).

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

/// Main: the runs of the branch's newest commit, one per workflow, all passed.
#[test]
fn main_reads_its_newest_commit() {
    let runs = parse_runs(&fixture("runs-main.json")).expect("gh's runs parse");
    assert_eq!(runs.len(), 30);
    let head = newest_commit(&runs);
    assert!(head.iter().all(|r| r.head_sha.starts_with("898ae57d")), "{head:?}");
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
