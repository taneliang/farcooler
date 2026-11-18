//! Finished groups release fold/gate trees; live runs and unknown leftovers remain.
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
/// Git ignores `target/` in every tree of the repository, as a Rust repository's .gitignore does.
fn ignore_target(runtime: &Runtime) {
    let info = runtime.repository().join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(info.join("exclude"), "target/\n").unwrap();
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
    ignore_target(&runtime);
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let first = group_of(&state);
    assert!(!runtime.item_tree("w", &first).exists(), "a landed group frees its fold and gate checkout");
    assert!(branch_exists(&runtime, "w", &first).await, "its commits aren't merged into the local checkout, so its branch stays");
    let listed = adapters::git(&runtime.repository(), &["worktree", "list", "--porcelain"]).await.unwrap();
    assert!(!listed.contains(&folder_name_of(&first)), "the repository unregisters its checkout: {listed}");

    at_join(&mut runtime, "two", &definition).await;
    command(&runtime, Command::Queue { item: "two".into() });
    until(&mut runtime, |s| phase_of(s, "two") == Some(Phase::Done)).await;
    assert_eq!(std::fs::read_to_string(dir.path().join("marker")).unwrap(), "cold\ncold\n",
        "ended groups release output instead of caching it for another run");
    assert!(std::fs::read_dir(&runtime.root).unwrap().all(|e|
        !e.unwrap().file_name().to_string_lossy().starts_with("build-")), "no ended output is parked");
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let cleanup = records.iter().find(|r| r.effect.item == first).expect("group cleanup recorded");
    assert!(cleanup.result.as_ref().unwrap().bytes_freed > 1);
    assert_eq!(trees(&runtime).len(), 2, "only ticket source checkouts remain");
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
async fn a_failed_group_frees_its_source_tree() {
    let (_dir, mut runtime) = lander("gate-fail").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    let group = group_of(&state);
    assert_eq!(phase_of(&state, &group), Some(Phase::Failed), "the failed group stays on the board");
    assert!(!runtime.item_tree("w", &group).exists(), "a failed group frees its fold and gate checkout");
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
async fn a_split_group_frees_its_tree_while_its_halves_rebuild() {
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
    assert!(!runtime.item_tree("w", &split).exists(), "the rejected parent tree is released");
    assert_eq!(trees(&runtime).len(), 3, "the new groups rebuilt and released their trees too");
}

#[tokio::test]
async fn a_cancelled_group_keeps_its_tree_until_the_running_effect_settles() {
    let (_dir, mut runtime, group, tree) = super::build_cleanup_tests::building_group(None).await;
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    std::fs::create_dir_all(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/binary"), b"busy").unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    runtime.active_trees.lock().unwrap().insert(tree.clone());
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("target/binary").exists(), "an in-flight gate still owns the tree");
    runtime.active_trees.lock().unwrap().clear();
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(!tree.exists(), "the cancellation releases the tree after the effect joins");
    let board = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    assert!(board["build_cleanup"].as_array().unwrap().iter().any(|r|
        r["effect"]["item"] == group && r["result"]["bytes_freed"].as_u64().is_some_and(|n| n > 4)));
}


async fn interrupted_group_recovery(point: u8) {
    let (_dir, mut runtime, group, tree) = super::build_cleanup_tests::building_group(None).await;
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    // Keep a detached commit reachable even if deletion removed the .git link first.
    adapters::git(&tree, &["checkout", "--detach"]).await.unwrap();
    adapters::git(&tree, &["-c", "commit.gpgsign=false", "-c", "user.name=Test",
        "-c", "user.email=test@example.com", "commit", "--allow-empty", "-m", "finished group"]).await.unwrap();
    let head = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().rev()
        .find(|r| r.effect.item == group).unwrap();
    if point == 3 {
        let crash = runtime.release_builds_checked(&["w".into()], || Err(Error::Refused("simulated crash"))).await;
        assert!(crash.is_err());
        assert!(!tree.exists());
    } else {
        assert!(runtime.store.workflow_cleanup_begin(&record.effect, None).unwrap());
        if point == 1 { std::fs::remove_file(tree.join(".git")).unwrap(); }
        if point == 2 { std::fs::remove_dir_all(&tree).unwrap(); }
    }
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert!(!tree.exists(), "recovery completes an interrupted finished-group removal");
    let reference = format!("refs/workflow/finished/{}", adapters::folder_name(&digest(&("w", &group))));
    assert_eq!(adapters::git(&runtime.repository(), &["rev-parse", &reference]).await.unwrap(), head);
    let listed = adapters::git(&runtime.repository(), &["worktree", "list", "--porcelain"]).await.unwrap();
    assert!(!listed.contains(tree.to_str().unwrap()), "stale Git registration is reclaimed");
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
async fn recovery_finishes_group_cleanup_interrupted_before_deletion() { interrupted_group_recovery(0).await; }

#[tokio::test]
async fn recovery_finishes_group_cleanup_interrupted_partway_through_deletion() { interrupted_group_recovery(1).await; }

#[tokio::test]
async fn recovery_finishes_group_cleanup_interrupted_after_deletion() { interrupted_group_recovery(2).await; }

#[tokio::test]
async fn recovery_finishes_group_cleanup_interrupted_after_git_prune() { interrupted_group_recovery(3).await; }

#[tokio::test]
async fn recovery_rechecks_safety_and_keeps_unknown_bytes_across_retries() {
    let (_dir, mut runtime, group, tree) = super::build_cleanup_tests::building_group(None).await;
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    std::fs::write(tree.join("busy"), b"open output").unwrap();
    let open = File::open(tree.join("busy")).unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().rev()
        .find(|r| r.effect.item == group).unwrap();
    assert!(runtime.store.workflow_cleanup_begin(&record.effect, None).unwrap());
    std::fs::remove_file(tree.join(".git")).unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert!(tree.exists(), "an open file still blocks a partly removed tree");
    let results = runtime.store.workflow_cleanups("w").unwrap();
    let result = results.iter().find(|r| r.effect.id == record.effect.id).unwrap().result.as_ref().unwrap();
    assert!(result.retry && result.skipped.iter().any(|s| s.contains("open files")), "{result:?}");
    drop(open);
    std::fs::create_dir_all(tree.join("nested/.git")).unwrap();
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("nested/.git").exists(), "recovery still protects nested repositories");
    std::fs::remove_dir_all(tree.join("nested")).unwrap();
    runtime.active_trees.lock().unwrap().insert(tree.clone());
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.exists(), "an active effect still blocks recovery");
    runtime.active_trees.lock().unwrap().clear();
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(!tree.exists());
    let results = runtime.store.workflow_cleanups("w").unwrap();
    let result = results.iter().find(|r| r.effect.id == record.effect.id).unwrap().result.as_ref().unwrap();
    assert!(!result.retry && result.bytes_freed == 0, "{result:?}");
    assert!(result.skipped.iter().any(|s| s.contains("bytes freed unknown")), "{result:?}");
}

#[tokio::test]
async fn recovery_settles_interrupted_cleanup_after_registration_is_already_gone() {
    let (_dir, mut runtime, group, tree) = super::build_cleanup_tests::building_group(None).await;
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().rev()
        .find(|r| r.effect.item == group).unwrap();
    assert!(runtime.store.workflow_cleanup_begin(&record.effect, None).unwrap());
    // A separate Git prune may have removed the registration of the absent tree.
    // Its group branch still retains the commits; recovery has no deletion left to do.
    std::fs::remove_dir_all(&tree).unwrap();
    adapters::git(&runtime.repository(), &["worktree", "prune"]).await.unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let result = records.iter().find(|r| r.effect.id == record.effect.id).unwrap().result.as_ref().unwrap();
    assert!(!result.retry && result.bytes_freed == 0, "{result:?}");
    assert!(result.skipped.iter().any(|s| s.contains("bytes freed unknown")), "{result:?}");
}

#[tokio::test]
async fn recovery_finishes_group_removal_that_returned_a_partial_deletion_error() {
    let (_dir, mut runtime, group, tree) = super::build_cleanup_tests::building_group(None).await;
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    let protected = tree.join("protected");
    std::fs::create_dir(&protected).unwrap();
    std::fs::write(protected.join("output"), b"remaining output").unwrap();
    std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o555)).unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    runtime.release_builds(&["w".into()]).await.unwrap();
    // Restore permissions before assertions so even a failed test can remove its fixture.
    std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o755)).unwrap();
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let record = records.iter().rev().find(|r| r.effect.item == group).unwrap();
    let result = record.result.as_ref().unwrap();
    assert!(result.retry, "a filesystem refusal leaves removal unfinished: {result:?}");
    assert!(result.skipped.iter().any(|s| s.contains("bytes freed unknown")),
        "remove_dir_all may have deleted files before returning an error: {result:?}");
    // Directory enumeration order varies: model loss of the link if it survived the error.
    if tree.join(".git").exists() { std::fs::remove_file(tree.join(".git")).unwrap(); }
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert!(!tree.exists(), "recovery completes the remaining deletion without a .git link");
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let result = records.iter().find(|r| r.effect.id == record.effect.id).unwrap().result.as_ref().unwrap();
    assert!(!result.retry && result.bytes_freed == 0, "{result:?}");
    assert!(result.skipped.iter().any(|s| s.contains("bytes freed unknown")), "{result:?}");
}

async fn interrupted_group_preflight_retry(fault: bool) {
    let (_dir, mut runtime, group, tree) = super::build_cleanup_tests::building_group(None).await;
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().rev()
        .find(|r| r.effect.item == group).unwrap();
    assert!(runtime.store.workflow_cleanup_begin(&record.effect, None).unwrap());
    std::fs::remove_file(tree.join(".git")).unwrap();
    let parked = tree.with_extension("parked");
    if fault {
        std::fs::rename(&tree, &parked).unwrap();
        std::fs::write(&tree, b"temporary invalid filesystem binding").unwrap();
    } else {
        farcooler_store::testing::exec_for_tests(&runtime.store,
            "INSERT INTO engine_cutover_quarantine VALUES('workspace:w','temporary test blocker')");
    }
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert!(tree.symlink_metadata().is_ok(), "preflight still prevents deletion");
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let result = records.iter().find(|r| r.effect.id == record.effect.id).unwrap().result.as_ref().unwrap();
    assert!(result.retry, "preflight must preserve reconciliation: {result:?}");
    if fault {
        std::fs::remove_file(&tree).unwrap();
        std::fs::rename(&parked, &tree).unwrap();
    } else {
        farcooler_store::testing::exec_for_tests(&runtime.store,
            "DELETE FROM engine_cutover_quarantine WHERE resource='workspace:w'");
    }
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(!tree.exists(), "a later sweep finishes the partly removed tree");
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let result = records.iter().find(|r| r.effect.id == record.effect.id).unwrap().result.as_ref().unwrap();
    assert!(!result.retry && result.bytes_freed == 0, "{result:?}");
    assert!(result.skipped.iter().any(|s| s.contains("bytes freed unknown")), "{result:?}");
}

#[tokio::test]
async fn recovery_retries_group_preflight_quarantine_after_losing_git_link() { interrupted_group_preflight_retry(false).await; }

#[tokio::test]
async fn recovery_retries_group_preflight_error_after_losing_git_link() { interrupted_group_preflight_retry(true).await; }
