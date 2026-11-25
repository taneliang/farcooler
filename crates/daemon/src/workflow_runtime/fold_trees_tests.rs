//! Finished groups release verified build output and retain their linked checkouts.
use super::*;
use super::coding_verbs_tests::{at_join, batched_push, command, lander, phase_of, until};

/// The group a ticket ran in, live or archived.
fn group_of(state: &State) -> String {
    state.items.iter().find(|(_, i)| i.group_of.is_some()).map(|(id, _)| id.clone())
        .or_else(|| state.archive.iter().find(|(_, a)| a.group.is_some()).map(|(id, _)| id.clone()))
        .expect("a group ran")
}
async fn branch_exists(runtime: &Runtime, workspace: &str, item: &str) -> bool {
    let branch = format!("refs/heads/wf-{}", &digest(&(workspace, item))[4..]);
    crate::git::git(&runtime.repository(), &["rev-parse", "--verify", "--quiet", &branch]).await.unwrap().ok
}
fn ignore_build_output(runtime: &Runtime) {
    let info = runtime.repository().join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(info.join("exclude"), "target/\n/apps/android/app/build/\n").unwrap();
}
fn trees(runtime: &Runtime) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(&runtime.root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("tree-")).collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_landed_group_releases_build_output_instead_of_parking_it() {
    let (dir, mut runtime) = lander("build").await;
    ignore_build_output(&runtime);
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let first = group_of(&state);
    assert_eq!(phase_of(&state, &first), Some(Phase::Done), "the landed group stays on the board");
    let tree = runtime.item_tree("w", &first);
    assert!(tree.exists());
    assert!(!tree.join("apps/android/app/build/outputs/apk/debug/app-debug.apk").exists());
    assert!(branch_exists(&runtime, "w", &first).await, "its commits aren't merged into the local checkout, so its branch stays");
    let listed = adapters::git(&runtime.repository(), &["worktree", "list", "--porcelain"]).await.unwrap();
    assert!(listed.contains(tree.to_str().unwrap()), "the checkout stays registered: {listed}");

    at_join(&mut runtime, "two", &definition).await;
    command(&runtime, Command::Queue { item: "two".into() });
    until(&mut runtime, |s| phase_of(s, "two") == Some(Phase::Done)).await;
    assert_eq!(std::fs::read_to_string(dir.path().join("marker")).unwrap(), "cold\ncold\n",
        "ended groups release output instead of caching it for another run");
    assert!(std::fs::read_dir(&runtime.root).unwrap().all(|e|
        !e.unwrap().file_name().to_string_lossy().starts_with("build-")), "no ended output is parked");
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let cleanup = records.iter().find(|r| r.effect.item == first && r.effect.directory.is_none()).expect("group cleanup recorded");
    assert!(cleanup.result.as_ref().unwrap().bytes_freed > 1);
    assert_eq!(trees(&runtime).len(), 4, "ticket and group source checkouts remain");
}

fn folder_name_of(item: &str) -> String { adapters::folder_name(&digest(&("w", item))) }

#[tokio::test]
async fn recovery_preserves_an_unowned_legacy_trees_files_and_detached_commits() {
    let (_dir, runtime) = lander("pass").await;
    let repo = runtime.repository();
    let tree = runtime.root.join(format!("tree-{}", digest(&("w", "long-gone"))));
    adapters::git(&repo, &["worktree", "add", "--detach", tree.to_str().unwrap(), "HEAD"]).await.unwrap();
    adapters::git(&tree, &["-c", "commit.gpgsign=false", "-c", "user.name=Test",
        "-c", "user.email=test@example.com", "commit", "--allow-empty", "-m", "legacy work"]).await.unwrap();
    let head = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
    std::fs::write(tree.join("personal-notes"), b"keep notes").unwrap();
    std::fs::create_dir(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/output"), b"unknown policy").unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(10).await.unwrap();
    assert!(tree.exists(), "a legacy name alone is not permission to discard a tree");
    assert_eq!(std::fs::read(tree.join("personal-notes")).unwrap(), b"keep notes");
    assert_eq!(std::fs::read(tree.join("target/output")).unwrap(), b"unknown policy");
    assert_eq!(adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap(), head);
    let listed = adapters::git(&repo, &["worktree", "list", "--porcelain"]).await.unwrap();
    assert!(listed.contains(tree.to_str().unwrap()), "the detached commit keeps its worktree reference");
}

#[tokio::test]
async fn a_failed_group_keeps_its_source_tree() {
    let (_dir, mut runtime) = lander("gate-fail").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    let group = group_of(&state);
    assert_eq!(phase_of(&state, &group), Some(Phase::Failed), "the failed group stays on the board");
    assert!(runtime.item_tree("w", &group).join(".git").is_file());
}

/// The fold tree of a group that is still working stays, and so does a run's tree under the old name while the run
/// is live. An unowned old-name tree stays too: recovery cannot guess its policy or ownership.
/// Unknown leftovers from earlier deletion implementations also remain.
#[tokio::test]
async fn recovery_keeps_unowned_live_and_unknown_discarded_trees() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    let repo = runtime.repository();
    let base = adapters::git(&repo, &["rev-parse", "HEAD"]).await.unwrap();
    let orphan = runtime.root.join(format!("tree-{}", digest(&("w", "long-gone"))));
    let live = runtime.root.join(format!("tree-{}", digest(&("w", "one"))));
    for tree in [&orphan, &live] {
        adapters::git(&repo, &["worktree", "add", "--detach", tree.to_str().unwrap(), &base]).await.unwrap();
    }
    let unknown = runtime.root.join(format!("tree-{}", folder_name_of("not-on-this-board")));
    adapters::git(&repo, &["worktree", "add", "--detach", unknown.to_str().unwrap(), &base]).await.unwrap();
    let trash = runtime.root.join("trash-left-by-a-crash");
    std::fs::create_dir_all(trash.join("deep")).unwrap();

    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(10).await.unwrap();
    assert!(orphan.exists(), "without an owner and cleanup record even a clean old-name tree stays");
    assert!(live.exists(), "a live run's old-name tree stays");
    assert!(unknown.exists(), "a current-name tree no state knows of is left alone");
    let listed = adapters::git(&repo, &["worktree", "list", "--porcelain"]).await.unwrap();
    assert!(listed.contains(orphan.to_str().unwrap()), "{listed}");
    assert!(trash.exists(), "unknown leftovers are not permission to delete source");
}

#[tokio::test]
async fn a_split_group_keeps_its_tree_while_its_halves_rebuild() {
    let (_dir, mut runtime) = lander("batch-fail").await;
    let definition = batched_push(&runtime, "any", 5000);
    for item in ["x", "a", "b"] { at_join(&mut runtime, item, &definition).await; }
    for item in ["x", "a", "b"] { command(&runtime, Command::Queue { item: item.into() }); }
    let state = runtime.state("w").unwrap().unwrap();
    let split = state.items["a"].grouped_in.clone().unwrap();
    assert_eq!(state.items[&split].members.len(), 2);
    for item in ["x", "a", "b"] {
        until(&mut runtime, |s| phase_of(s, item) == Some(Phase::Done)).await;
    }
    assert!(runtime.item_tree("w", &split).join(".git").is_file(), "the rejected parent checkout is retained");
}

#[tokio::test]
async fn a_cancelled_group_keeps_its_output_until_the_running_effect_settles() {
    let (_dir, mut runtime, group, tree) = super::build_cleanup_tests::building_group(None).await;
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    let archive = super::build_cleanup_tests::seed_nextest_archive(&tree);
    let bytes = archive.metadata().unwrap().len();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    let active = runtime.tree_lock(&tree).try_read_owned().unwrap();
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(archive.exists(), "an in-flight gate still owns the tree");
    drop(active);
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.exists() && !archive.exists());
    let board = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    assert!(board["build_cleanup"].as_array().unwrap().iter().any(|r|
        r["effect"]["item"] == group && r["result"]["bytes_freed"] == bytes));
}

async fn interrupted_group_recovery(missing_git_link: bool, missing_result: bool) {
    let (_dir, mut runtime, group, tree) = super::build_cleanup_tests::building_group(None).await;
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    let archive = super::build_cleanup_tests::seed_nextest_archive(&tree);
    std::fs::write(tree.join("notes"), b"keep").unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().rev()
        .find(|r| r.effect.item == group).unwrap();
    if missing_result {
        let crash = runtime.release_builds_checked(&["w".into()], || Err(Error::Refused("simulated crash"))).await;
        assert!(crash.is_err());
        assert!(!archive.exists());
    } else {
        assert!(runtime.store.workflow_cleanup_begin(&record.effect, None).unwrap());
        if missing_git_link { std::fs::remove_file(tree.join(".git")).unwrap(); }
    }
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert_eq!(std::fs::read(tree.join("notes")).unwrap(), b"keep");
    assert_eq!(archive.exists(), missing_git_link, "only a linked checkout can verify its index");
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let result = records.iter().find(|r| r.effect.id == record.effect.id).unwrap().result.as_ref().unwrap();
    assert!(!result.retry, "{result:?}");
    assert_eq!(result.bytes_freed, 0, "remaining bytes cannot reconstruct the interrupted total");
    assert!(result.skipped.iter().any(|s| s.contains("bytes freed unknown")), "{result:?}");
    let board = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    assert!(board["build_cleanup"].as_array().unwrap().iter().any(|r|
        r["effect"]["id"] == record.effect.id && r["result"]["skipped"].to_string().contains("bytes freed unknown")));
    runtime.pump(100).await.unwrap();
    assert_eq!(runtime.store.workflow_cleanups("w").unwrap().len(), records.len());
}

#[tokio::test]
async fn recovery_retries_interrupted_group_output_cleanup() { interrupted_group_recovery(false, false).await; }

#[tokio::test]
async fn recovery_preserves_partly_deleted_group_without_git_link() { interrupted_group_recovery(true, false).await; }

#[tokio::test]
async fn recovery_keeps_unknown_bytes_after_group_output_was_removed() { interrupted_group_recovery(false, true).await; }
