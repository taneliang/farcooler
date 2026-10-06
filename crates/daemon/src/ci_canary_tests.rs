//! Canary's runs, attributed by name (ov-301).

use super::*;

const TRAIN: &str = "1111111111111111111111111111111111111111";
const BOT: &str = "2222222222222222222222222222222222222222";

fn run(id: u64, name: &str, sha: &str, event: &str, title: &str, status: &str, conclusion: Option<&str>) -> GhRun {
    GhRun {
        id,
        name: name.into(),
        head_sha: sha.into(),
        status: status.into(),
        conclusion: conclusion.map(Into::into),
        html_url: format!("https://github.com/o/r/actions/runs/{id}"),
        created_at: "2026-10-05T10:00:00Z".into(),
        display_title: title.into(),
        event: event.into(),
        updated_at: "2026-10-05T10:20:00Z".into(),
    }
}

fn ci(sha: &str) -> GhRun {
    run(1, "CI", sha, "push", "a commit", "completed", Some("success"))
}

/// 10:25 on the day CI passed at 10:20.
const SOON: i64 = 1_791_195_900;

#[test]
fn the_stamp_reads_as_epoch_seconds() {
    assert_eq!(epoch_secs("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(epoch_secs("2026-10-05T10:20:00Z"), Some(SOON - 300));
    assert_eq!(epoch_secs("2024-02-29T23:59:59Z"), Some(1_709_251_199));
    assert_eq!(epoch_secs("yesterday"), None);
    assert_eq!(epoch_secs("2026-10-05 10:20:00Z"), None);
}

/// A Canary run for the train carries the baseline bot's SHA, so it is not in
/// the list for the train's SHA; it is found by its name. Another commit's run
/// that carries the train's SHA is not the train's.
#[test]
fn a_canary_run_is_found_by_its_name_not_its_sha() {
    let theirs = run(5, "Canary", TRAIN, "workflow_run", &title(BOT), "completed", Some("failure"));
    let ours = run(6, "Canary", BOT, "workflow_run", &title(TRAIN), "completed", Some("success"));
    let runs = attribute(TRAIN, vec![ci(TRAIN), theirs], vec![ours.clone()]);
    let names: Vec<(u64, &str)> = runs.iter().map(|r| (r.id, r.name.as_str())).collect();
    assert_eq!(names, [(1, "CI"), (6, "Canary")], "the failure belongs to the bot's commit");
    assert_eq!(runs[1], ours);
}

/// The wire-baseline workflow follows Canary (`workflow_run`) and carries the
/// SHA of main's head when it started, which can be the train's; it is not
/// the train's CI (ov-341).
#[test]
fn a_workflow_run_of_another_workflow_is_not_the_trains() {
    let wire = run(7, "Canary wire baseline", TRAIN, "workflow_run", "Canary wire baseline", "completed", Some("failure"));
    let runs = attribute(TRAIN, vec![ci(TRAIN), wire.clone()], vec![]);
    let names: Vec<&str> = runs.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["CI"]);
    let settled = settle(TRAIN, vec![ci(TRAIN), wire], Some(vec![]), SOON).unwrap();
    assert!(settled.iter().all(|r| r.name != "Canary wire baseline"));
}

/// A Canary run from before the change was triggered by the push itself and
/// carries the commit; it stays.
#[test]
fn an_old_push_triggered_canary_run_stays() {
    let old = run(4, "Canary", TRAIN, "push", "the commit message", "completed", Some("success"));
    let runs = attribute(TRAIN, vec![ci(TRAIN), old], vec![]);
    assert_eq!(runs.len(), 2);
}

/// Canary's list holds every commit's runs; only the one naming this commit
/// is this commit's.
#[test]
fn another_commit_s_canary_run_is_not_added() {
    let other = run(7, "Canary", BOT, "workflow_run", &title(BOT), "completed", Some("failure"));
    let runs = attribute(TRAIN, vec![ci(TRAIN)], vec![other]);
    assert_eq!(runs.len(), 1);
}

#[test]
fn a_run_is_not_added_twice() {
    let ours = run(6, "Canary", TRAIN, "workflow_run", &title(TRAIN), "completed", Some("success"));
    assert_eq!(attribute(TRAIN, vec![ci(TRAIN), ours.clone()], vec![ours]).len(), 2);
}

/// CI has passed and Canary's run is not there yet: the commit reads queued,
/// so a train does not go green in the gap.
#[test]
fn a_commit_is_not_green_before_its_canary_run_exists() {
    let runs = vec![ci(TRAIN)];
    let placeholder = pending(&runs, TRAIN, SOON).expect("expected a placeholder");
    assert_eq!((placeholder.name.as_str(), placeholder.status.as_str(), placeholder.id), ("Canary", "queued", 0));
    let mut all = runs.clone();
    all.push(placeholder);
    let read = crate::ci_watch::summarize("sha:1111", &all.into_iter().map(|r| (r, None)).collect::<Vec<_>>());
    assert_eq!(read.status, farcooler_store::board_ci::CiStatus::Queued);
}

#[test]
fn no_placeholder_once_canary_has_a_run_or_ci_is_not_green_or_the_window_passed() {
    let canary = run(6, "Canary", BOT, "workflow_run", &title(TRAIN), "in_progress", None);
    assert!(pending(&[ci(TRAIN), canary], TRAIN, SOON).is_none(), "a Canary run exists");
    let mut red = ci(TRAIN);
    red.conclusion = Some("failure".into());
    assert!(pending(&[red], TRAIN, SOON).is_none(), "CI did not pass");
    let mut going = ci(TRAIN);
    going.status = "in_progress".into();
    going.conclusion = None;
    assert!(pending(&[going], TRAIN, SOON).is_none(), "CI is still going");
    let mut pr = ci(TRAIN);
    pr.event = "pull_request".into();
    assert!(pending(&[pr], TRAIN, SOON).is_none(), "a pull request's CI starts no Canary");
    assert!(pending(&[ci(TRAIN)], TRAIN, SOON + PENDING_FOR_SECS).is_none(), "a Canary that never starts cannot hold a train");
    assert!(pending(&[ci(TRAIN)], TRAIN, SOON - 300 + PENDING_FOR_SECS).is_some(), "the window's last second");
}

#[test]
fn canary_is_only_looked_for_after_ci_passes_on_a_push() {
    assert!(worth_looking(&[ci(TRAIN)]));
    let mut going = ci(TRAIN);
    going.status = "in_progress".into();
    going.conclusion = None;
    assert!(!worth_looking(&[going]));
}

/// A failed read of Canary's list is unknown, not "no Canary run": a train
/// whose Canary failed must not read green because gh went offline.
#[test]
fn an_unreadable_canary_list_is_unknown() {
    assert_eq!(settle(TRAIN, vec![ci(TRAIN)], None, SOON), None);
    assert_eq!(settle(TRAIN, vec![ci(TRAIN)], None, SOON + 10 * PENDING_FOR_SECS), None);
}

/// CI passed long ago and no Canary run names the commit (it may have aged out
/// of the newest 50): keep the last read rather than read CI alone.
#[test]
fn a_commit_whose_canary_run_is_not_found_after_the_window_keeps_its_last_read() {
    let late = SOON + 10 * PENDING_FOR_SECS;
    assert_eq!(settle(TRAIN, vec![ci(TRAIN)], Some(vec![]), late), None);
    // Within the window it is still the placeholder.
    let soon = settle(TRAIN, vec![ci(TRAIN)], Some(vec![]), SOON).expect("a read");
    assert_eq!(soon.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["CI", "Canary"]);
    // Found: its own read, however late.
    let ours = run(6, "Canary", BOT, "workflow_run", &title(TRAIN), "completed", Some("failure"));
    let found = settle(TRAIN, vec![ci(TRAIN)], Some(vec![ours]), late).expect("a read");
    assert_eq!(found.len(), 2);
    // Where CI never passed on a push there is nothing to wait for.
    let mut red = ci(TRAIN);
    red.conclusion = Some("failure".into());
    assert_eq!(settle(TRAIN, vec![red], Some(vec![]), late).map(|r| r.len()), Some(1));
}
