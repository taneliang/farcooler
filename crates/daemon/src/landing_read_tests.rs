//! Reading a repository's way of landing (ov-313): the settings a board keeps,
//! what a read says beside them, the daily schedule, and the two looks at a
//! branch's own tree, on real git.

use super::*;
use farcooler_protocol::v1::Scope;

use crate::landing::{Rules, decide};
use crate::test_support::fixture;
use crate::workspace_ops::{list, set_settings};

fn settings(landing: Option<pb::LandingMode>) -> pb::WorkspaceSetSettings {
    pb::WorkspaceSetSettings { landing: landing.map(|m| m as i32), ..Default::default() }
}

/// A read whose rules say `pull_request`, filed for `workspace`.
fn remember_a_base_that_requires_pull_requests(workspace: Uuid) -> Reading {
    let facts = Facts {
        base: "main".into(),
        rules: Some(Rules { pull_request: true, approvals: 1, ..Default::default() }),
        protected: Some(true),
        ..Default::default()
    };
    let reading = Reading { asked: None, decision: decide(&facts), facts, read_at: now_ms() };
    remember(workspace, &reading);
    reading
}

#[tokio::test]
async fn a_board_has_chosen_nothing_until_it_is_told_and_cost_is_off() {
    let (_dir, svc, repo) = fixture().await;
    let main = list(&svc, Some(repo), Scope::Control).unwrap().items.remove(0);
    assert_eq!(main.landing, None, "not chosen is not direct");
    assert_eq!((main.base, main.pr_max_lines), (None, None));
    assert_eq!(main.pr_cost_line, Some(false), "PR cost is off by default");
}

#[tokio::test]
async fn each_landing_setting_is_set_and_read_back_and_the_wake_switch_is_left_alone() {
    let (_dir, svc, repo) = fixture().await;
    let watcher = Watcher::new(svc.clone());
    let main = list(&svc, Some(repo), Scope::Control).unwrap().items.remove(0);
    let id = Uuid::from_slice(&main.id).unwrap();
    let req = pb::WorkspaceSetSettings {
        landing: Some(pb::LandingMode::PullRequests as i32),
        base: Some("trunk".into()),
        pr_max_lines: Some(400),
        pr_cost_line: Some(true),
        ..Default::default()
    };
    let set = set_settings(&svc, &watcher, id, &req, Scope::Control).unwrap();
    assert_eq!(set.landing, Some(pb::LandingMode::PullRequests as i32));
    assert_eq!((set.base.as_deref(), set.pr_max_lines, set.pr_cost_line), (Some("trunk"), Some(400), Some(true)));
    assert_eq!(set.wake_on_answer, Some(true));
    let read = list(&svc, Some(repo), Scope::Read).unwrap().items.remove(0);
    assert_eq!(read.landing, set.landing);
    assert_eq!(read.base.as_deref(), Some("trunk"));
}

#[tokio::test]
async fn a_write_that_cannot_land_leaves_the_wake_switch_as_it_was() {
    let (_dir, svc, repo) = fixture().await;
    let watcher = Watcher::new(svc.clone());
    let main = list(&svc, Some(repo), Scope::Control).unwrap().items.remove(0);
    let id = Uuid::from_slice(&main.id).unwrap();
    let both = pb::WorkspaceSetSettings {
        wake_on_answer: Some(false),
        base: Some("has a space".into()),
        ..Default::default()
    };
    assert!(matches!(
        set_settings(&svc, &watcher, id, &both, Scope::Control),
        Err(DomainError::InvalidArgument { what: "base" })
    ));
    assert_eq!(list(&svc, Some(repo), Scope::Control).unwrap().items[0].wake_on_answer, Some(true));
    let unspecified = pb::WorkspaceSetSettings { landing: Some(0), ..Default::default() };
    assert!(matches!(
        set_settings(&svc, &watcher, id, &unspecified, Scope::Control),
        Err(DomainError::InvalidArgument { what: "landing" })
    ));
}

/// Detection suggests and never switches: a read that found direct impossible
/// shows why beside a board that hasn't chosen, or chose direct, and leaves
/// the choice as it is; choosing pull requests ends the sentence.
#[tokio::test]
async fn a_base_that_refuses_pushes_is_said_and_the_choice_is_never_changed() {
    let (_dir, svc, repo) = fixture().await;
    let watcher = Watcher::new(svc.clone());
    let main = list(&svc, Some(repo), Scope::Control).unwrap().items.remove(0);
    let id = Uuid::from_slice(&main.id).unwrap();
    assert_eq!(main.direct_refused, None, "nothing has been read");

    remember_a_base_that_requires_pull_requests(id);
    let unchosen = list(&svc, Some(repo), Scope::Control).unwrap().items.remove(0);
    assert_eq!(
        unchosen.direct_refused.as_deref(),
        Some("Landing straight on main won't work. main requires pull requests, with one approving review.")
    );
    assert_eq!(unchosen.landing, None, "it was not switched for the owner");

    let chose_direct = set_settings(&svc, &watcher, id, &settings(Some(pb::LandingMode::Direct)), Scope::Control).unwrap();
    assert_eq!(chose_direct.landing, Some(pb::LandingMode::Direct as i32));
    assert!(chose_direct.direct_refused.is_some(), "a choice that can't work is still said");

    let chose_prs = set_settings(&svc, &watcher, id, &settings(Some(pb::LandingMode::PullRequests)), Scope::Control).unwrap();
    assert_eq!(chose_prs.direct_refused, None);
}

#[test]
fn a_read_that_found_direct_possible_says_nothing() {
    let id = Uuid::now_v7();
    let facts = Facts { base: "main".into(), rules: Some(Rules::default()), protected: Some(false), ..Default::default() };
    remember(id, &Reading { asked: None, decision: decide(&facts), facts, read_at: 1 });
    assert_eq!(refusal(id, None, None), None);
    assert_eq!(refusal(Uuid::now_v7(), None, None), None, "a board never read says nothing");
}

#[test]
fn the_daily_read_is_due_after_a_day_and_an_empty_one_after_six_hours() {
    let hour = 60 * 60 * 1000;
    let learned = Reading {
        asked: None,
        facts: Facts { rules: Some(Rules::default()), ..Default::default() },
        decision: Decision::default(),
        read_at: 0,
    };
    assert!(due(None, 0), "a board never read is due");
    assert!(!due(Some(&learned), 23 * hour));
    assert!(due(Some(&learned), 24 * hour));
    let empty = Reading { asked: None, facts: Facts::default(), decision: Decision::default(), read_at: 0 };
    assert!(!due(Some(&empty), 5 * hour));
    assert!(due(Some(&empty), 6 * hour), "gh that couldn't answer is retried sooner than a day");
}

#[test]
fn a_branch_name_is_one_path_segment() {
    assert_eq!(path_segment("main"), "main");
    assert_eq!(path_segment("release/1.2"), "release%2F1.2");
    assert_eq!(path_segment("a b#"), "a%20b%23");
}

fn commit_all(dir: &std::path::Path, message: &str) {
    for args in [vec!["add", "-A"], vec!["commit", "-q", "-m", message]] {
        assert!(std::process::Command::new("git").args(&args).current_dir(dir).status().unwrap().success());
    }
}

fn a_repo_on_main() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        assert!(std::process::Command::new("git").args(&args).current_dir(dir.path()).status().unwrap().success());
    }
    dir
}

fn write(dir: &std::path::Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

#[tokio::test]
async fn a_planted_workflow_without_merge_group_is_seen_and_one_with_it_is_too() {
    let dir = a_repo_on_main();
    write(dir.path(), ".github/workflows/ci.yml", "name: CI\non:\n  push:\n  pull_request:\njobs: {}\n");
    commit_all(dir.path(), "ci");
    assert_eq!(read_tree(dir.path(), "main").await, (Some(false), Some(false)));

    write(dir.path(), ".github/workflows/queue.yaml", "on:\n  merge_group:\njobs: {}\n");
    commit_all(dir.path(), "queue");
    assert_eq!(read_tree(dir.path(), "main").await.1, Some(true));
}

#[tokio::test]
async fn the_base_branch_s_tree_is_read_not_the_working_directory() {
    let dir = a_repo_on_main();
    write(dir.path(), "README", "x");
    commit_all(dir.path(), "base");
    // On another branch with a workflow and a CODEOWNERS that main hasn't.
    assert!(std::process::Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap().success());
    write(dir.path(), ".github/workflows/q.yml", "on: merge_group\n");
    write(dir.path(), ".github/CODEOWNERS", "* @alice\n");
    commit_all(dir.path(), "feature");
    assert_eq!(read_tree(dir.path(), "main").await, (Some(false), Some(false)), "main has neither");
    assert_eq!(read_tree(dir.path(), "feature").await, (Some(true), Some(true)));
    assert_eq!(base_ref(dir.path(), "main").await.as_deref(), Some("main"));
    assert_eq!(base_ref(dir.path(), "nope").await, None, "a branch that isn't there is unread, not empty");
    assert_eq!(base_ref(dir.path(), "-x").await, None);
}

#[tokio::test]
async fn codeowners_is_found_in_each_of_the_three_places() {
    for place in ["CODEOWNERS", ".github/CODEOWNERS", "docs/CODEOWNERS"] {
        let dir = a_repo_on_main();
        write(dir.path(), place, "* @alice\n");
        commit_all(dir.path(), "owners");
        assert_eq!(read_tree(dir.path(), "main").await.0, Some(true), "{place}");
    }
}

/// A workflow listed in the tree whose text can't be read might have been the
/// one that runs on `merge_group`, so the answer is unread, not "no": a
/// submodule entry named like a workflow is listed and can't be shown.
#[tokio::test]
async fn a_workflow_that_cannot_be_read_leaves_the_answer_unread() {
    let dir = a_repo_on_main();
    write(dir.path(), ".github/workflows/ci.yml", "on:\n  push:\n");
    commit_all(dir.path(), "ci");
    let sha = "1111111111111111111111111111111111111111";
    for args in [
        vec!["update-index", "--add", "--cacheinfo", &format!("160000,{sha},.github/workflows/vendored.yml")],
        vec!["commit", "-q", "-m", "a gitlink named like a workflow"],
    ] {
        assert!(std::process::Command::new("git").args(&args).current_dir(dir.path()).status().unwrap().success());
    }
    assert_eq!(read_tree(dir.path(), "main").await.1, None);

    // But a workflow that does run on merge_group is found whatever else failed.
    write(dir.path(), ".github/workflows/queue.yml", "on: merge_group\n");
    commit_all(dir.path(), "queue");
    assert_eq!(read_tree(dir.path(), "main").await.1, Some(true));
}

/// A read of one base says nothing about another: changing `--base` drops the
/// old base's refusal at once, and a refusal read for a different base than the
/// board's current one is never shown.
#[tokio::test]
async fn a_changed_base_does_not_keep_the_old_bases_refusal() {
    let (_dir, svc, repo) = fixture().await;
    let watcher = Watcher::new(svc.clone());
    let main = list(&svc, Some(repo), Scope::Control).unwrap().items.remove(0);
    let id = Uuid::from_slice(&main.id).unwrap();
    remember_a_base_that_requires_pull_requests(id);
    assert!(list(&svc, Some(repo), Scope::Control).unwrap().items[0].direct_refused.is_some());

    let to_release = pb::WorkspaceSetSettings { base: Some("release".into()), ..Default::default() };
    let set = set_settings(&svc, &watcher, id, &to_release, Scope::Control).unwrap();
    assert_eq!(set.direct_refused, None, "main's refusal isn't release's");

    // Back to the default branch: the old read must not come back to life.
    let back = pb::WorkspaceSetSettings { base: Some(String::new()), ..Default::default() };
    assert_eq!(set_settings(&svc, &watcher, id, &back, Scope::Control).unwrap().direct_refused, None);

    // And a read that is about another base than the current one is ignored.
    remember_a_base_that_requires_pull_requests(id);
    assert_eq!(refusal(id, None, Some("release")), None);
    assert!(refusal(id, None, None).is_some());
}

#[tokio::test]
async fn more_workflows_than_the_cap_leave_the_answer_unread() {
    let dir = a_repo_on_main();
    for n in 0..=MOST_WORKFLOWS {
        write(dir.path(), &format!(".github/workflows/w{n:03}.yml"), "on: push\n");
    }
    commit_all(dir.path(), "many");
    assert_eq!(read_tree(dir.path(), "main").await.1, None, "51 files, 50 looked at: not \"none\"");
    std::fs::remove_file(dir.path().join(".github/workflows/w050.yml")).unwrap();
    commit_all(dir.path(), "fewer");
    assert_eq!(read_tree(dir.path(), "main").await.1, Some(false), "exactly the cap is all of them");
}

#[tokio::test]
async fn a_workflow_in_a_subdirectory_does_not_run_so_it_does_not_count() {
    let dir = a_repo_on_main();
    write(dir.path(), ".github/workflows/ci.yml", "on: push\n");
    write(dir.path(), ".github/workflows/templates/queue.yml", "on: merge_group\n");
    commit_all(dir.path(), "template");
    assert_eq!(read_tree(dir.path(), "main").await.1, Some(false));
}
