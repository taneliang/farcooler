//! Every fixture stops before build, then crosses the real build -> review transition.
use super::*;
use super::tests::enqueue;

async fn building() -> (tempfile::TempDir, Runtime, PathBuf) {
    building_with(None).await
}
async fn building_with(folders: Option<Vec<String>>) -> (tempfile::TempDir, Runtime, PathBuf) {
    building_with_review_at(folders, 2).await
}
async fn building_with_review_at(folders: Option<Vec<String>>, review_at: usize) -> (tempfile::TempDir, Runtime, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let mut definition = coding_definition(false);
    definition.build_folders = folders;
    let mut review = definition.stages[1].clone();
    review.id = "person-review".into();
    review.kind = StageKind::Review(Reviewer::Person("owner".into()));
    review.operation = OperationSpec { adapter_version: "fixture-review/v1".into(),
        arguments: vec!["ready".into()], environment: Default::default() };
    definition.stages.insert(review_at, review);
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].definition.stages[state.items["one"].stage].kind, StageKind::Build);
    let tree = runtime.item_tree("w", "one");
    std::fs::create_dir_all(tree.join("target/deep")).unwrap();
    std::fs::write(tree.join("target/deep/output"), b"compiled").unwrap();
    (dir, runtime, tree)
}
async fn review(runtime: &mut Runtime) {
    runtime.pump(3).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::WaitingReview);
}

#[tokio::test]
async fn build_cleanup_build_to_review_releases_target_and_records_bytes_once() {
    let (_dir, mut runtime, tree) = building().await;
    review(&mut runtime).await;
    assert!(!tree.join("target").exists());
    let records = runtime.store.workflow_cleanups("w").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].result.as_ref().unwrap().bytes_freed, 8);
    let board = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    assert_eq!(board["build_cleanup"][0]["result"]["bytes_freed"], 8);
    runtime.pump(4).await.unwrap();
    assert_eq!(runtime.store.workflow_cleanups("w").unwrap()[0].result, records[0].result);
}
#[tokio::test]
async fn build_cleanup_still_building_keeps_output_until_it_leaves_build() {
    let (_dir, mut runtime, tree) = building().await;
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("target/deep/output").exists());
    assert!(runtime.store.workflow_cleanups("w").unwrap().is_empty());
    review(&mut runtime).await;
    assert!(!tree.join("target").exists());
}
#[tokio::test]
async fn build_cleanup_open_file_skips_and_records_why() {
    let (_dir, mut runtime, tree) = building().await;
    let handle = File::open(tree.join("target/deep/output")).unwrap();
    review(&mut runtime).await;
    assert!(tree.join("target/deep/output").exists());
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("open files")), "{result:?}");
    drop(handle);
}
#[tokio::test]
async fn build_cleanup_preserves_commits_untracked_files_and_unlisted_folders() {
    let (_dir, mut runtime, tree) = building().await;
    std::fs::write(tree.join("personal-notes"), b"keep").unwrap();
    std::fs::create_dir(tree.join("unlisted")).unwrap();
    std::fs::write(tree.join("unlisted/output"), b"keep too").unwrap();
    let head = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
    review(&mut runtime).await;
    assert_eq!(std::fs::read(tree.join("personal-notes")).unwrap(), b"keep");
    assert_eq!(std::fs::read(tree.join("unlisted/output")).unwrap(), b"keep too");
    assert!(adapters::git(&tree, &["cat-file", "-e", &head]).await.is_ok());
    assert!(!tree.join("target").exists());
}

#[tokio::test]
async fn build_cleanup_uses_the_published_list_and_expands_defaults_without_following_links() {
    let (dir, mut runtime, tree) = building_with(Some(vec!["custom-output".into()])).await;
    std::fs::create_dir(tree.join("custom-output")).unwrap();
    std::fs::write(tree.join("custom-output/binary"), b"test").unwrap();
    review(&mut runtime).await;
    assert!(!tree.join("custom-output").exists());
    assert!(tree.join("target").exists(), "an explicit list replaces the default");
    drop(dir);
    let (dir, mut runtime, tree) = building().await;
    for folder in ["apps/android/app/build", "apps/macos/build", ".build"] {
        std::fs::create_dir_all(tree.join(folder)).unwrap();
        std::fs::write(tree.join(folder).join("binary"), b"test").unwrap();
    }
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("important"), b"safe").unwrap();
    std::os::unix::fs::symlink(&outside, tree.join("target/link")).unwrap();
    review(&mut runtime).await;
    for folder in ["target", "apps/android/app/build", "apps/macos/build", ".build"] {
        assert!(!tree.join(folder).exists(), "{folder}");
    }
    assert_eq!(std::fs::read(outside.join("important")).unwrap(), b"safe");
}
#[tokio::test]
async fn build_cleanup_symlink_ancestors_and_tracked_source_are_skipped() {
    let (dir, mut runtime, tree) = building().await;
    std::fs::remove_dir_all(tree.join("target")).unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("important"), b"safe").unwrap();
    std::os::unix::fs::symlink(&outside, tree.join("target")).unwrap();
    std::fs::create_dir_all(tree.join("apps/macos/build")).unwrap();
    std::fs::write(tree.join("apps/macos/build/source"), b"tracked").unwrap();
    adapters::git(&tree, &["add", "--", "apps/macos/build/source"]).await.unwrap();
    review(&mut runtime).await;
    assert_eq!(std::fs::read(outside.join("important")).unwrap(), b"safe");
    assert!(tree.join("apps/macos/build/source").exists());
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("symlink")), "{result:?}");
    assert!(result.skipped.iter().any(|s| s.contains("tracked")), "{result:?}");
}
#[tokio::test]
async fn build_cleanup_uncertain_run_records_skip_and_retains_output() {
    let (_dir, runtime, tree) = building().await;
    let state = runtime.state("w").unwrap().unwrap();
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect("w", &effect.id, "test", state.revision, state.epoch, 3, 100).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 3).unwrap());
    assert!(runtime.observe("w", &effect, Outcome::Uncertain, false, 3).unwrap());
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("target/deep/output").exists());
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("Uncertain")));
}
#[tokio::test]
async fn build_cleanup_cancellation_releases_output_and_recovery_keeps_the_result() {
    let (_dir, runtime, tree) = building().await;
    std::fs::create_dir_all(tree.join("apps/android/app/build")).unwrap();
    std::fs::write(tree.join("apps/android/app/build/binary"), b"gradle").unwrap();
    // This build was queued but never admitted; cancellation needs no stop effect.
    runtime.internal("w", Command::Cancel { item: "one".into(), reason: None }, Grant::Controller, 3).unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(4).await.unwrap();
    assert!(!tree.join("target").exists());
    assert!(!tree.join("apps/android/app/build").exists());
    let records = runtime.store.workflow_cleanups("w").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].result.as_ref().unwrap().bytes_freed, 14);
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(5).await.unwrap();
    assert_eq!(runtime.store.workflow_cleanups("w").unwrap()[0].result, records[0].result);
}

fn controller_command(runtime: &Runtime, workspace: &str, command: Command) {
    let state = runtime.state(workspace).unwrap();
    let sequence = state.as_ref().and_then(|s| s.source_sequences.get("controller").copied()).unwrap_or(0) + 1;
    let receipt = runtime.apply(Request { id: format!("command-{sequence}"),
        workspace: workspace.into(), expected_revision: state.map_or(0, |s| s.revision), sequence, command }, "controller", Grant::Controller, 3).unwrap();
    assert!(receipt.result.is_ok(), "{receipt:?}");
}
fn named_start(runtime: &Runtime, workspace: &str, item: &str, path: &Path) {
    controller_command(runtime, workspace, Command::Enqueue { item: item.into(), definition: Definition::ticket(),
        policy_pin: "policy/v1".into(), resource: Exec::item_resource(workspace, item), dependencies: BTreeSet::new(),
        initial_candidate: Some("ticket".into()) });
    controller_command(runtime, workspace, Command::Start { item: item.into(), definition: coding_definition(false),
        policy_pin: "policy/v1".into(), resource: Exec::item_resource(workspace, item), dependencies: BTreeSet::new(),
        initial_candidate: None, models: Default::default(), with: vec![], tree: Some(Box::new(RunTree {
            name: item.into(), branch: item.into(), path: path.to_str().unwrap().into(), base: runtime.base.clone(), adopted: false })) });
}
#[tokio::test]
async fn build_cleanup_shared_named_tree_on_another_board_is_skipped() {
    let (_dir, runtime, tree) = building().await;
    named_start(&runtime, "other-board", "other", &tree);
    // Even if the caller's workspace inventory predates the other board, the deletion checks all live runs.
    // Resolve the first build without launching the second run's Provision into the shared tree.
    let state = runtime.state("w").unwrap().unwrap();
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect("w", &effect.id, "test", state.revision, state.epoch, 3, 100).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 3).unwrap());
    let evidence = Evidence { candidate: runtime.base.clone(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: effect.policy_pin.clone() };
    assert!(runtime.observe("w", &effect, Outcome::Passed(evidence), false, 3).unwrap());
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("target/deep/output").exists());
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("shared")), "{result:?}");
}
#[tokio::test]
async fn build_cleanup_an_exclusive_named_tree_is_released_when_cancelled() {
    let (_dir, mut runtime, _) = building().await;
    let tree = runtime.root.join("named");
    named_start(&runtime, "named-board", "named", &tree);
    runtime.pump(4).await.unwrap();
    std::fs::create_dir_all(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/binary"), b"test").unwrap();
    controller_command(&runtime, "named-board", Command::Cancel { item: "named".into(), reason: None });
    runtime.pump(5).await.unwrap();
    assert!(tree.exists());
    assert!(!tree.join("target").exists());
    assert_eq!(runtime.store.workflow_cleanups("named-board").unwrap().remove(0).result.unwrap().bytes_freed, 4);
}
#[tokio::test]
async fn build_cleanup_delayed_intent_cannot_delete_a_later_builds_output() {
    let (_dir, runtime, tree) = building().await;
    // Queue cleanup without sweeping it, then send the run straight back to build.
    let state = runtime.state("w").unwrap().unwrap();
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect("w", &effect.id, "test", state.revision, state.epoch, 3, 100).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 3).unwrap());
    let evidence = Evidence { candidate: runtime.base.clone(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: effect.policy_pin.clone() };
    assert!(runtime.observe("w", &effect, Outcome::Passed(evidence), false, 3).unwrap());
    let handle = File::open(tree.join("target/deep/output")).unwrap();
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert!(runtime.store.workflow_cleanups("w").unwrap()[0].result.as_ref().unwrap().retry);
    drop(handle);
    runtime.active_trees.lock().unwrap().insert(tree.clone());
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert!(runtime.store.workflow_cleanups("w").unwrap()[0].result.as_ref().unwrap().retry);
    assert!(tree.join("target/deep/output").exists());
    runtime.active_trees.lock().unwrap().clear();
    controller_command(&runtime, "w", Command::SendBack { item: "one".into(), reason: "try again".into() });
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("target/deep/output").exists());
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("later build")), "{result:?}");
}

#[tokio::test]
async fn build_cleanup_output_rebuilt_by_checks_is_released_before_review_waits() {
    let (_dir, mut runtime, tree) = building_with_review_at(None, 3).await;
    // A real build directory is ignored; the fixture check refuses any other untracked change.
    std::fs::write(runtime.repository().join(".git/info/exclude"), "target/\n").unwrap();
    runtime.pump(3).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].definition.stages[state.items["one"].stage].kind, StageKind::Check);
    std::fs::create_dir_all(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/rebuilt"), b"check output").unwrap();
    runtime.pump(4).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::WaitingReview);
    assert!(!tree.join("target").exists());
    assert!(runtime.store.workflow_cleanups("w").unwrap().iter().any(|r| r.result.as_ref().is_some_and(|s| s.bytes_freed == 20)));
}

#[tokio::test]
async fn build_cleanup_preserves_nested_git_metadata_with_mixed_case_names() {
    let (_dir, mut runtime, tree) = building().await;
    // Git metadata must survive even on case-insensitive filesystems, including
    // an untracked nested repository inside an otherwise disposable output.
    let metadata = tree.join("target/nested/.GiT/objects");
    std::fs::create_dir_all(&metadata).unwrap();
    std::fs::write(metadata.join("commit"), b"keep commit").unwrap();
    review(&mut runtime).await;
    assert_eq!(std::fs::read(metadata.join("commit")).unwrap(), b"keep commit");
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("git metadata")), "{result:?}");
}

#[tokio::test]
async fn build_cleanup_preserves_commits_in_a_nested_bare_repository() {
    let (_dir, mut runtime, tree) = building().await;
    let bare = tree.join("target/cached-repository");
    let cloned = tokio::process::Command::new("git").env_clear().env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(["clone", "--bare", "--local"]).arg(runtime.repository()).arg(&bare).output().await.unwrap();
    assert!(cloned.status.success(), "{}", String::from_utf8_lossy(&cloned.stderr));
    let commit = adapters::git(&bare, &["rev-parse", "HEAD"]).await.unwrap();
    review(&mut runtime).await;
    assert!(bare.join("HEAD").exists(), "cleanup deleted a bare repository's commits");
    assert!(adapters::git(&bare, &["cat-file", "-e", &commit]).await.is_ok());
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("git metadata")), "{result:?}");
}


// A named cleanup with a real host uses the very mutex provision_named and Service claims use.
async fn named_cleanup() -> (tempfile::TempDir, Runtime, PathBuf, Uuid, farcooler_workflow::cleanup::CleanupEffect) {
    let (dir, mut runtime, tree) = building().await;
    controller_command(&runtime, "w", Command::Cancel { item: "one".into(), reason: None });
    runtime.pump(4).await.unwrap();
    let host = Uuid::now_v7();
    let repo = runtime.repository();
    let root = runtime.store.create_repository_root(host, repo.to_str().unwrap(), 0).unwrap();
    let repository = runtime.store.create_repository(host, root.id, "cleanup", repo.join(".git").to_str().unwrap(), "").unwrap();
    let workspace = runtime.store.create_workspace(repository.id, "Cleanup", "cl").unwrap().id.to_string();
    runtime.exec.host = Some(WorktreeHost { dir: dir.path().join("trees"), install_id: "test".into(), locks: Default::default() });
    named_start(&runtime, &workspace, "named", &tree);
    let state = runtime.state(&workspace).unwrap().unwrap();
    let provision = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect(&workspace, &provision.id, "fixture", state.revision, state.epoch, 3, 100).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 3).unwrap());
    let evidence = Evidence { candidate: runtime.base.clone(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: provision.policy_pin.clone() };
    runtime.observe(&workspace, &provision, Outcome::Passed(evidence), false, 3).unwrap();
    let state = runtime.state(&workspace).unwrap().unwrap();
    let build = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect(&workspace, &build.id, "fixture", state.revision, state.epoch, 3, 100).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 3).unwrap());
    let evidence = Evidence { candidate: runtime.base.clone(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: build.policy_pin.clone() };
    assert!(runtime.observe(&workspace, &build, Outcome::Passed(evidence), false, 3).unwrap());
    let state = runtime.state(&workspace).unwrap().unwrap();
    let check = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect(&workspace, &check.id, "fixture", state.revision, state.epoch, 3, 100).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 3).unwrap());
    let evidence = Evidence { candidate: runtime.base.clone(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: check.policy_pin.clone() };
    assert!(runtime.observe(&workspace, &check, Outcome::Passed(evidence), false, 3).unwrap());
    std::fs::create_dir_all(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/output"), b"old output").unwrap();
    let records = runtime.store.workflow_cleanups(&workspace).unwrap();
    assert!(!records.is_empty(), "{:?}", runtime.state(&workspace).unwrap());
    let effect = records[0].effect.clone();
    (dir, runtime, tree, repository.id, effect)
}

#[tokio::test]
async fn build_cleanup_second_run_cannot_claim_between_ownership_check_and_delete() {
    let (_dir, runtime, tree, repository, effect) = named_cleanup().await;
    let lock = runtime.exec.host.as_ref().unwrap().lock(repository);
    let claimed = std::cell::Cell::new(false);
    runtime.exec.release_build_checked(&effect, || {
        // Deterministically schedule the competing claim in the old check/delete gap.
        if let Ok(_held) = lock.try_lock() {
            named_start(&runtime, "second-board", "second", &tree);
            std::fs::write(tree.join("target/second-output"), b"new build").unwrap();
            claimed.set(true);
        }
    }).await.unwrap();
    assert!(!claimed.get(), "second run claimed the tree during cleanup and lost its build output");
    assert!(lock.try_lock().is_ok(), "cleanup must release the claim fence afterwards");
}


#[tokio::test]
async fn build_cleanup_busy_claim_fence_retries_ended_run() {
    let (_dir, runtime, tree, repository, effect) = named_cleanup().await;
    let lock = runtime.exec.host.as_ref().unwrap().lock(repository);
    controller_command(&runtime, &effect.workspace, Command::Cancel { item: effect.item.clone(), reason: None });
    let held = lock.lock().await;
    runtime.exec.release_builds(std::slice::from_ref(&effect.workspace)).await.unwrap();
    assert!(tree.join("target/output").exists());
    let result = runtime.store.workflow_cleanups(&effect.workspace).unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("fence is busy")), "{result:?}");
    drop(held);
    runtime.exec.release_builds(std::slice::from_ref(&effect.workspace)).await.unwrap();
    assert!(!tree.join("target").exists(), "ended runs must retry after the fence clears");
    let records = runtime.store.workflow_cleanups(&effect.workspace).unwrap();
    assert_eq!(records.iter().filter_map(|r| r.result.as_ref()).map(|r| r.bytes_freed).sum::<u64>(), 10);
}

#[tokio::test]
async fn build_cleanup_crash_after_deletion_does_not_replay_against_rebuilt_output() {
    let (_dir, runtime, tree) = building().await;
    let state = runtime.state("w").unwrap().unwrap();
    let build = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect("w", &build.id, "fixture", state.revision, state.epoch, 3, 100).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 3).unwrap());
    let evidence = Evidence { candidate: runtime.base.clone(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: build.policy_pin.clone() };
    assert!(runtime.observe("w", &build, Outcome::Passed(evidence), false, 3).unwrap());
    let effect = runtime.store.workflow_cleanups("w").unwrap().remove(0).effect;
    // Interrupt the real sweep after deletion, before workflow_cleanup_result is written.
    let crash = runtime.exec.release_builds_checked(&["w".into()], || Err(Error::Refused("simulated crash"))).await;
    assert!(crash.is_err());
    assert!(!tree.join("target").exists());
    assert!(!runtime.store.workflow_cleanup_begin(&effect, None).unwrap(), "only one executor may begin");
    let root = runtime.root.clone();
    drop(runtime);
    // A later owner repopulates the same path before recovery sees the old effect.
    std::fs::create_dir_all(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/new-output"), b"keep new build").unwrap();
    let runtime = Runtime::open(&root).unwrap();
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert_eq!(std::fs::read(tree.join("target/new-output")).unwrap(), b"keep new build");
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert_eq!(result.bytes_freed, 0, "interrupted deletion must not invent a byte count");
    assert!(result.skipped.iter().any(|s| s.contains("interrupted")), "{result:?}");
}


// A published group may provision and build its own candidate before a person's review.
pub(super) async fn building_group(folders: Option<Vec<String>>) -> (tempfile::TempDir, Runtime, String, PathBuf) {
    use super::coding_verbs_tests::{at_join, batched_push, command, lander, until};
    let (dir, mut runtime) = lander("pass").await;
    std::fs::write(runtime.repository().join(".git/info/exclude"), "target/\ncustom-output/\n").unwrap();
    let mut definition = batched_push(&runtime, "any", 5000);
    definition.build_folders = folders;
    let config = runtime.coding.as_ref().unwrap();
    let review = config.stage("review", StageKind::Review(Reviewer::Person("owner".into())),
        "coding-review/v1", "person:owner");
    definition.groups.get_mut("landing").unwrap().steps = vec![
        definition.stages[0].clone(), definition.stages[1].clone(), review];
    at_join(&mut runtime, "one", &definition).await;
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    let state = until(&mut runtime, |s| s.items.values().any(|i| i.group_of.is_some()
        && i.definition.stages[i.stage].kind == StageKind::Build)).await;
    let group = state.items.iter().find(|(_, i)| i.group_of.is_some()).unwrap().0.clone();
    let tree = runtime.item_tree("w", &group);
    std::fs::create_dir_all(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/output"), b"group output").unwrap();
    (dir, runtime, group, tree)
}

#[tokio::test]
async fn build_cleanup_group_build_to_review_releases_output_and_records_result_once() {
    use super::coding_verbs_tests::until;
    let (_dir, mut runtime, group, tree) = building_group(None).await;
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("target/output").exists(), "a group still building keeps its output");
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    assert!(!tree.join("target").exists(), "a group waiting for review releases its output");
    let records: Vec<_> = runtime.store.workflow_cleanups("w").unwrap().into_iter()
        .filter(|r| r.effect.item == group).collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].result.as_ref().unwrap().bytes_freed, 12);
    assert!(records[0].result.as_ref().unwrap().skipped.is_empty());
    runtime.pump(100).await.unwrap();
    let again = runtime.store.workflow_cleanups("w").unwrap().into_iter().find(|r| r.effect.item == group).unwrap();
    assert_eq!(again.result, records[0].result);
}

#[tokio::test]
async fn build_cleanup_group_inherits_published_folder_list() {
    use super::coding_verbs_tests::until;
    let (_dir, mut runtime, group, tree) = building_group(Some(vec!["custom-output".into()])).await;
    std::fs::create_dir(tree.join("custom-output")).unwrap();
    std::fs::write(tree.join("custom-output/binary"), b"custom").unwrap();
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    assert!(tree.join("target/output").exists());
    assert!(!tree.join("custom-output").exists());
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().find(|r| r.effect.item == group).unwrap();
    assert_eq!(record.effect.folders, ["custom-output"]);
    assert_eq!(record.result.unwrap().bytes_freed, 6);
}

#[tokio::test]
async fn build_cleanup_cancelled_group_retains_open_output_and_unlisted_files() {
    use super::coding_verbs_tests::command;
    let (_dir, mut runtime, group, tree) = building_group(None).await;
    let handle = File::open(tree.join("target/output")).unwrap();
    std::fs::create_dir_all(tree.join("apps/android/app/build")).unwrap();
    std::fs::write(tree.join("apps/android/app/build/binary"), b"gradle").unwrap();
    std::fs::write(tree.join("personal-notes"), b"keep").unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    runtime.pump(100).await.unwrap();
    assert_eq!(std::fs::read(tree.join("target/output")).unwrap(), b"group output");
    assert_eq!(std::fs::read(tree.join("personal-notes")).unwrap(), b"keep");
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().find(|r| r.effect.item == group).unwrap();
    let result = record.result.unwrap();
    assert!(result.retry);
    assert_eq!(result.bytes_freed, 6, "other folders are freed during contention");
    assert!(result.skipped.iter().any(|s| s.contains("open files")));
    drop(handle);
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert!(!tree.join("target").exists(), "ended groups retry after recovery and file closure");
    assert!(!tree.exists(), "the disposable group checkout is released after file closure");
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().find(|r| r.effect.item == group).unwrap();
    assert!(record.result.as_ref().unwrap().bytes_freed > 18);
    assert!(!record.result.as_ref().unwrap().retry);
    runtime.pump(100).await.unwrap();
    let again = runtime.store.workflow_cleanups("w").unwrap().into_iter().find(|r| r.effect.item == group).unwrap();
    assert_eq!(again.result, record.result, "settled bytes are counted once");
}


#[tokio::test]
async fn build_cleanup_cancelled_group_releases_the_whole_disposable_tree() {
    use super::coding_verbs_tests::command;
    let (_dir, mut runtime, group, tree) = building_group(None).await;
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(foreign.path().join("live-output"), b"keep").unwrap();
    std::os::unix::fs::symlink(foreign.path(), tree.join("foreign-tree")).unwrap();
    let head = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
    std::fs::write(tree.join("personal-notes"), b"keep").unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    runtime.pump(100).await.unwrap();
    assert!(!tree.join("target").exists());
    assert!(!tree.exists());
    assert_eq!(std::fs::read(foreign.path().join("live-output")).unwrap(), b"keep");
    assert!(adapters::git(&runtime.repository(), &["cat-file", "-e", &head]).await.is_ok());
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().find(|r| r.effect.item == group).unwrap();
    assert!(record.result.unwrap().bytes_freed > 12);
}

#[tokio::test]
async fn build_cleanup_ended_group_with_empty_output_policy_still_releases_its_tree() {
    use super::coding_verbs_tests::command;
    let (_dir, mut runtime, group, tree) = building_group(Some(vec![])).await;
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    runtime.pump(100).await.unwrap();
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter()
        .find(|r| r.effect.item == group).unwrap();
    assert!(record.effect.folders.is_empty(), "the published policy disables output deletion");
    assert!(record.result.as_ref().unwrap().bytes_freed > 12);
    assert!(record.result.as_ref().unwrap().skipped.is_empty());
    assert!(!tree.exists(), "folder policy does not retain finished group checkouts");
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert!(!tree.exists());
    let again = runtime.store.workflow_cleanups("w").unwrap().into_iter()
        .find(|r| r.effect.item == group).unwrap();
    assert_eq!(again.result, record.result, "recovery preserves the settled cleanup result");
}

#[tokio::test]
async fn build_cleanup_cancelled_group_preserves_committed_work_after_pruning() {
    use super::coding_verbs_tests::command;
    let (_dir, mut runtime, group, tree) = building_group(None).await;
    std::fs::write(tree.join("group-work.txt"), b"keep committed work").unwrap();
    adapters::git(&tree, &["add", "group-work.txt"]).await.unwrap();
    adapters::git(&tree, &["-c", "commit.gpgsign=false", "-c", "user.name=Test",
        "-c", "user.email=test@example.com", "commit", "-m", "group work"]).await.unwrap();
    let head = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
    let branch = adapters::git(&tree, &["symbolic-ref", "HEAD"]).await.unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    runtime.pump(100).await.unwrap();
    assert!(!tree.join("target").exists(), "commits must not prevent output release");
    assert!(!tree.exists(), "cleanup releases the source checkout while retaining its branch");
    let retained = adapters::git(&runtime.repository(), &["rev-parse", "--verify", &branch]).await.unwrap();
    assert_eq!(retained, head, "pruning must retain the ref protecting a group's commits from GC");
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().find(|r| r.effect.item == group).unwrap();
    assert!(record.result.unwrap().bytes_freed > 12);
}

#[tokio::test]
async fn build_cleanup_cancelled_group_anchors_detached_commits_before_removing_the_tree() {
    use super::coding_verbs_tests::command;
    let (_dir, mut runtime, group, tree) = building_group(None).await;
    adapters::git(&tree, &["checkout", "--detach"]).await.unwrap();
    adapters::git(&tree, &["-c", "commit.gpgsign=false", "-c", "user.name=Test",
        "-c", "user.email=test@example.com", "commit", "--allow-empty", "-m", "detached group work"]).await.unwrap();
    let head = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    runtime.pump(100).await.unwrap();
    assert!(!tree.join("target").exists(), "unanchored commits must not prevent output release");
    assert!(!tree.exists());
    let reference = format!("refs/workflow/finished/{}", adapters::folder_name(&digest(&("w", &group))));
    assert_eq!(adapters::git(&runtime.repository(), &["rev-parse", &reference]).await.unwrap(), head);
}


#[tokio::test]
async fn build_cleanup_group_uncertain_records_skip_and_retains_output() {
    let (_dir, runtime, group, tree) = building_group(None).await;
    let state = runtime.state("w").unwrap().unwrap();
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .find(|s| s.active && s.effect.item == group).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect("w", &effect.id, "test", state.revision, state.epoch, 3, 100).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 3).unwrap());
    assert!(runtime.observe("w", &effect, Outcome::Uncertain, false, 3).unwrap());
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("target/output").exists());
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().find(|r| r.effect.item == group).unwrap();
    assert_eq!(record.result.unwrap().skipped, ["run is Uncertain"]);
}

#[tokio::test]
async fn build_cleanup_defers_an_executing_effect_then_releases_after_it_settles() {
    let (_dir, mut runtime, tree) = building().await;
    review(&mut runtime).await;
    std::fs::create_dir_all(tree.join("target/deep")).unwrap();
    std::fs::write(tree.join("target/deep/output"), b"compiled").unwrap();
    controller_command(&runtime, "w", Command::Cancel { item: "one".into(), reason: None });
    runtime.active_trees.lock().unwrap().insert(tree.clone());
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("target/deep/output").exists());
    assert!(runtime.store.workflow_cleanups("w").unwrap().last().unwrap().result.is_none(), "defer rather than lose cleanup");
    runtime.active_trees.lock().unwrap().clear();
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(!tree.join("target").exists());
    assert_eq!(runtime.store.workflow_cleanups("w").unwrap()[0].result.as_ref().unwrap().bytes_freed, 8);
}

#[tokio::test]
async fn build_cleanup_waiting_join_and_failed_runs_release_gradle_and_target() {
    use super::coding_verbs_tests::{batched_push, lander, phase_of, until};
    for failed in [false, true] {
        let (_dir, mut runtime) = lander(if failed { "check-fail" } else { "pass" }).await;
        std::fs::write(runtime.repository().join(".git/info/exclude"), "target/\napps/\n").unwrap();
        let definition = batched_push(&runtime, "any", 5000);
        enqueue(&mut runtime, "w", "one", definition);
        until(&mut runtime, |s| s.items["one"].definition.stages[s.items["one"].stage].kind == StageKind::Check).await;
        let tree = runtime.item_tree("w", "one");
        for folder in ["target", "apps/android/build", "apps/android/app/build"] {
            std::fs::create_dir_all(tree.join(folder)).unwrap();
            std::fs::write(tree.join(folder).join("binary"), b"output").unwrap();
        }
        until(&mut runtime, |s| phase_of(s, "one") == Some(if failed { Phase::Failed } else { Phase::WaitingJoin })).await;
        for folder in ["target", "apps/android/build", "apps/android/app/build"] {
            assert!(!tree.join(folder).exists(), "{folder}, failed={failed}");
        }
        assert!(runtime.store.workflow_cleanups("w").unwrap().iter().any(|r|
            r.result.as_ref().is_some_and(|r| r.bytes_freed == 18)));
    }
}

#[tokio::test]
async fn build_cleanup_preserves_reports_even_inside_output() {
    let (_dir, mut runtime, tree) = building().await;
    std::fs::create_dir_all(tree.join("target/reports")).unwrap();
    std::fs::write(tree.join("target/reports/evidence"), b"keep").unwrap();
    review(&mut runtime).await;
    assert_eq!(std::fs::read(tree.join("target/reports/evidence")).unwrap(), b"keep");
    let record = runtime.store.workflow_cleanups("w").unwrap().remove(0);
    assert!(record.result.unwrap().skipped.iter().any(|s| s.contains("reports")));
}

#[tokio::test]
async fn build_cleanup_resumed_run_rebuilds_from_a_cold_target() {
    let (_dir, mut runtime, tree) = building().await;
    review(&mut runtime).await;
    let first = runtime.state("w").unwrap().unwrap().items["one"].candidate.clone();
    assert!(!tree.join("target").exists());
    controller_command(&runtime, "w", Command::SendBack { item: "one".into(), reason: "rebuild".into() });
    // The fixture build commits result.txt again on a resumed iteration.
    review(&mut runtime).await;
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].sent_back.len(), 1);
    assert_ne!(state.items["one"].candidate, first, "the build produced a new commit after cleanup");
    assert_eq!(std::fs::read_to_string(tree.join("result.txt")).unwrap(), "draft\n");
}

// Read the store/files directly: an inbox read would wake the dispatch loop and hide this bug.
async fn scheduled_cleanup(with_running_effect: bool) {
    use super::coding_tests::{admitted, configured};
    let (dir, mut runtime) = configured(false, "cancel").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "ended", definition.clone());
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    runtime.observe("w", &binding.effect, Outcome::SafeFailure, false, 3).unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["ended"].phase, Phase::Failed);
    let tree = runtime.item_tree("w", "ended");
    std::fs::create_dir_all(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/output"), b"ended output").unwrap();
    let open = File::open(tree.join("target/output")).unwrap();
    if with_running_effect { enqueue(&mut runtime, "w", "running", definition); }
    let store = runtime.store.clone();
    let active = runtime.active_trees.clone();
    let started = dir.path().join("started");
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move { runtime.run(shutdown).await.unwrap(); runtime });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let retry = store.workflow_cleanups("w").unwrap().iter()
                .any(|r| r.result.as_ref().is_some_and(|r| r.retry));
            if retry && (!with_running_effect || started.exists()) { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert!(tree.join("target/output").exists());
    drop(open);
    let released = tokio::time::timeout(Duration::from_secs(3), async {
        while tree.join("target").exists() { tokio::time::sleep(Duration::from_millis(20)).await; }
    }).await.is_ok();
    if with_running_effect {
        assert!(!active.lock().unwrap().is_empty(), "cleanup must run before the other effect ends");
    }
    // Once cleanup settles, idle runtimes must go back to sleeping without periodic sweeps.
    if released && !with_running_effect { tokio::time::sleep(Duration::from_millis(1200)).await; }
    stop.send(true).unwrap();
    let runtime = task.await.unwrap();
    assert!(released, "cleanup needs its own wakeup after the open file closes");
    let record = store.workflow_cleanups("w").unwrap().into_iter().find(|r| r.effect.item == "ended").unwrap();
    assert_eq!(record.result.as_ref().unwrap().bytes_freed, 12);
    assert!(!record.result.unwrap().retry);
    assert!(runtime.sweeps <= 4, "retry wakeups must be bounded and stop after cleanup settles");
}

#[tokio::test]
async fn build_cleanup_scheduled_retry_when_runtime_is_idle() { scheduled_cleanup(false).await; }

#[tokio::test]
async fn build_cleanup_scheduled_retry_while_another_effect_runs() { scheduled_cleanup(true).await; }

#[tokio::test]
async fn build_cleanup_recovery_releases_a_group_tree_retained_by_an_older_engine() {
    use super::coding_verbs_tests::{command, until};
    let (_dir, mut runtime, group, tree) = building_group(None).await;
    until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    std::fs::create_dir_all(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/binary"), b"old output retained").unwrap();
    command(&runtime, Command::Cancel { item: group.clone(), reason: None });
    // An older engine settled folder cleanup but deliberately left this checkout behind.
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter().rev().find(|r| r.effect.item == group).unwrap();
    assert!(runtime.store.workflow_cleanup_begin(&record.effect, None).unwrap());
    runtime.store.workflow_cleanup_result(&record.effect, &farcooler_workflow::cleanup::CleanupResult {
        bytes_freed: 0, skipped: Vec::new(), retry: false,
    }).unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert!(!tree.exists(), "recovery releases a known finished group's old tree");
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let released = records.iter().rev().find(|r| r.effect.item == group && r.effect.id != record.effect.id).unwrap();
    assert!(released.result.as_ref().unwrap().bytes_freed > 12);
    runtime.pump(100).await.unwrap();
    assert_eq!(runtime.store.workflow_cleanups("w").unwrap().len(), records.len());
}
