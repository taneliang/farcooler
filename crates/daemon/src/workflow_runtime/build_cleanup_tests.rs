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
    controller_command(runtime, workspace, Command::Start { start_load: None, item: item.into(), definition: coding_definition(false),
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
    let active = runtime.tree_lock(&tree).try_read_owned().unwrap();
    runtime.exec.release_builds(&["w".into()]).await.unwrap();
    assert!(runtime.store.workflow_cleanups("w").unwrap()[0].result.as_ref().unwrap().retry);
    assert!(tree.join("target/deep/output").exists());
    drop(active);
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
        .filter(|r| r.effect.item == group && r.effect.directory.is_none()).collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].result.as_ref().unwrap().bytes_freed, 12);
    assert!(records[0].result.as_ref().unwrap().skipped.is_empty());
    runtime.pump(100).await.unwrap();
    let again = runtime.store.workflow_cleanups("w").unwrap().into_iter()
        .find(|r| r.effect.item == group && r.effect.directory.is_none()).unwrap();
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
    let record = runtime.store.workflow_cleanups("w").unwrap().into_iter()
        .find(|r| r.effect.item == group && r.effect.directory.is_none()).unwrap();
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
    let active = runtime.tree_lock(&tree).try_read_owned().unwrap();
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.join("target/deep/output").exists());
    assert!(runtime.store.workflow_cleanups("w").unwrap().last().unwrap().result.is_none(), "defer rather than lose cleanup");
    drop(active);
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(!tree.join("target").exists());
    assert_eq!(runtime.store.workflow_cleanups("w").unwrap()[0].result.as_ref().unwrap().bytes_freed, 8);
}

async fn gradle_reports_cleanup(phase: Phase) {
    use super::coding_verbs_tests::{batched_push, lander, phase_of, until};
    let (_dir, mut runtime) = lander(if phase == Phase::Failed { "check-fail" } else { "pass" }).await;
    std::fs::write(runtime.repository().join(".git/info/exclude"), "target/\napps/\nreports/\n").unwrap();
    let definition = batched_push(&runtime, "any", 5000);
    enqueue(&mut runtime, "w", "one", definition);
    until(&mut runtime, |s| s.items["one"].definition.stages[s.items["one"].stage].kind == StageKind::Check).await;
    let tree = runtime.item_tree("w", "one");
    for folder in ["target", "apps/android/build", "apps/android/app/build"] {
        std::fs::create_dir_all(tree.join(folder)).unwrap();
        std::fs::write(tree.join(folder).join("binary"), b"output").unwrap();
    }
    for folder in ["apps/android/build", "apps/android/app/build"] {
        std::fs::create_dir_all(tree.join(folder).join("reports/tests/test")).unwrap();
        std::fs::write(tree.join(folder).join("reports/tests/test/index.html"), b"generated report").unwrap();
    }
    // Durable evidence is outside the selected build folders, even if it has the same name.
    std::fs::create_dir_all(tree.join("reports")).unwrap();
    std::fs::write(tree.join("reports/evidence"), b"keep").unwrap();
    if phase == Phase::Cancelled {
        controller_command(&runtime, "w", Command::Cancel { item: "one".into(), reason: None });
        runtime.pump(100).await.unwrap();
    } else {
        until(&mut runtime, |s| phase_of(s, "one") == Some(phase.clone())).await;
    }
    assert_eq!(phase_of(&runtime.state("w").unwrap().unwrap(), "one"), Some(phase.clone()));
    for folder in ["target", "apps/android/build", "apps/android/app/build"] {
        assert!(!tree.join(folder).exists(), "{folder}, phase={phase:?}");
    }
    assert_eq!(std::fs::read(tree.join("reports/evidence")).unwrap(), b"keep");
    let board = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    assert!(board["build_cleanup"].as_array().unwrap().iter().any(|r|
        r["effect"]["item"] == "one" && r["result"]["bytes_freed"] == 50), "{board}");
}

#[tokio::test]
async fn build_cleanup_waiting_join_releases_gradle_reports_and_target() { gradle_reports_cleanup(Phase::WaitingJoin).await; }

#[tokio::test]
async fn build_cleanup_failed_releases_gradle_reports_and_target() { gradle_reports_cleanup(Phase::Failed).await; }

#[tokio::test]
async fn build_cleanup_cancelled_releases_gradle_reports_and_target() { gradle_reports_cleanup(Phase::Cancelled).await; }

#[tokio::test]
async fn build_cleanup_gradle_reports_still_protects_tracked_files_and_git_metadata() {
    let (_dir, mut runtime, tree) = building().await;
    let tracked = tree.join("apps/android/app/build/reports/evidence");
    std::fs::create_dir_all(tracked.parent().unwrap()).unwrap();
    std::fs::write(&tracked, b"tracked evidence").unwrap();
    adapters::git(&tree, &["add", "--", "apps/android/app/build/reports/evidence"]).await.unwrap();
    let metadata = tree.join("apps/android/build/reports/nested/.git/objects");
    std::fs::create_dir_all(&metadata).unwrap();
    std::fs::write(metadata.join("commit"), b"keep commit").unwrap();
    review(&mut runtime).await;
    assert_eq!(std::fs::read(tracked).unwrap(), b"tracked evidence");
    assert_eq!(std::fs::read(metadata.join("commit")).unwrap(), b"keep commit");
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("tracked")), "{result:?}");
    assert!(result.skipped.iter().any(|s| s.contains("git metadata")), "{result:?}");
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
    // Real Git and lsof calls compete with parallel tests. Keep the companion
    // effect alive beyond both observation deadlines so this still proves that
    // cleanup wakes independently, rather than waiting for that effect to end.
    let script = dir.path().join("adapter.py");
    std::fs::write(&script, std::fs::read_to_string(&script).unwrap().replace("['/bin/sleep', '30']", "['/bin/sleep', '120']")).unwrap();
    let CodingHarness::Command(build) = &mut runtime.coding.as_mut().unwrap().harness else { unreachable!() };
    build.timeout_ms = 120_000;
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
    let exec = runtime.exec.clone();
    let started = dir.path().join("started");
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move { runtime.run(shutdown).await.unwrap(); runtime });
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let retry = store.workflow_cleanups("w").unwrap().iter()
                .any(|r| r.result.as_ref().is_some_and(|r| r.retry));
            if retry && (!with_running_effect || started.exists()) { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert!(tree.join("target/output").exists());
    drop(open);
    let released = tokio::time::timeout(Duration::from_secs(30), async {
        while tree.join("target").exists() { tokio::time::sleep(Duration::from_millis(20)).await; }
    }).await.is_ok();
    if with_running_effect {
        assert!(exec.tree_lock(&exec.item_tree("w", "running")).try_write().is_err(),
            "cleanup must run before the other effect ends");
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
async fn build_cleanup_admits_a_new_start_while_the_open_file_check_is_slow() {
    use super::admission_tests::{factory, mutation, ok, request};
    use farcooler_protocol::{v1::Scope, workflow_commands::Action};
    use farcooler_transport::{Handler, Peer};

    let (_dir, mut runtime, tree) = building().await;
    let output = tree.join("target/deep/output");
    let open = File::open(&output).unwrap();
    review(&mut runtime).await;
    drop(open);
    assert!(output.exists(), "an open file should leave cleanup pending");
    let (_service_dir, service, repo) = crate::test_support::fixture().await;
    let workspace = service.store.ensure_main_workspace(repo).unwrap().id;
    let board = workspace.to_string();
    let publish = runtime.wire_command(&board, "owner", Action::Publish {
        mutation: mutation("publish", 0, 1), expected_pin: None, definition: coding_definition(true),
        adapter: None, reason: None, notifications: None,
    }).unwrap();
    assert!(publish.pointer("/receipt/result/Ok").is_some(), "{publish}");
    assert_eq!(runtime.track_tickets(&[(board.clone(), "fresh".into(), "todo".into())]).unwrap(), 1);
    let state = runtime.state(&board).unwrap().unwrap();
    let fresh = &state.items["fresh"];
    let start = Action::Start {
        mutation: mutation("start-during-cleanup", state.revision, 2), item: "fresh".into(),
        run: fresh.run.clone(), pin: fresh.pin.clone(),
        start_pin: runtime.store.workflow_configuration(&board).unwrap().unwrap().pin,
        models: Default::default(), worktree: None, with: Vec::new(),
    };
    let rpc = factory(&service, &runtime, Peer { client_id: None, scope: Scope::Control }, None);
    let handle = runtime.command_handle(&runtime.controller()).unwrap();
    let check = std::sync::Arc::new((tokio::sync::Notify::new(), tokio::sync::Notify::new()));
    runtime.cleanup_check = Some(check.clone());
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move { runtime.run(shutdown).await.unwrap(); });
    tokio::time::timeout(Duration::from_secs(15), check.0.notified()).await.unwrap();
    let started = tokio::time::timeout(Duration::from_secs(10), async {
        ok(&rpc.handle(request(workspace, &start)).await);
        // Provision actually ran: the loop admitted the effect as well as answering Start.
        while handle.state(&board).await.unwrap().unwrap().items["fresh"].stage == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(output.exists(), "the original cleanup should still be in progress");
    }).await;
    check.1.notify_one();
    stop.send(true).unwrap();
    task.await.unwrap();
    started.expect("the new start waited for cleanup's open-file check");
    assert!(!output.exists(), "cleanup should finish after the check resumes");
}

#[tokio::test]
async fn build_cleanup_holds_effects_on_the_same_tree_until_it_finishes() {
    let (_dir, mut runtime, tree) = building().await;
    let cleanup = runtime.tree_lock(&tree).try_write_owned().unwrap();
    assert_eq!(runtime.pump(3).await.unwrap(), 0, "the build must wait for its tree");
    drop(cleanup);
    review(&mut runtime).await;
}

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
    runtime.store.workflow_cleanup_result(&record.effect, &Default::default()).unwrap();
    let reader = File::open(tree.join("target/binary")).unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert!(tree.exists(), "the recovery sweep leaves a checkout with a live reader");
    drop(reader);
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(!tree.exists(), "recovery releases a known finished group's old tree");
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let released = records.iter().rev().find(|r| r.effect.item == group && r.effect.id != record.effect.id).unwrap();
    assert!(released.result.as_ref().unwrap().bytes_freed > 12);
    runtime.pump(100).await.unwrap();
    assert_eq!(runtime.store.workflow_cleanups("w").unwrap().len(), records.len());
}

#[tokio::test]
async fn recovery_accounts_for_idle_output_left_by_an_older_engine() {
    let (_dir, mut runtime, tree) = building().await;
    review(&mut runtime).await;
    std::fs::create_dir(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/old-output"), b"left behind").unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(100).await.unwrap();
    assert!(!tree.join("target").exists());
    let records = runtime.store.workflow_cleanups("w").unwrap();
    assert_eq!(records.last().unwrap().result.as_ref().unwrap().bytes_freed, 11);
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::WaitingReview);
}

fn seed_nextest_archive(tree: &Path) {
    let target = tree.join("target");
    let file = File::create(target.join("rust-tests.tar.zst")).unwrap();
    let encoder = zstd::stream::write::Encoder::new(file, 1).unwrap();
    let mut archive = tar::Builder::new(encoder);
    for (path, metadata) in [
        ("target/nextest/binaries-metadata.json", serde_json::json!({
            "rust-binaries": {"runner": {"binary-path": target.join("debug/deps/runner")}},
            "rust-build-meta": {"target-directory": target},
        })),
        ("target/nextest/cargo-metadata.json", serde_json::json!({
            "packages": [{"name": "runner"}],
            "workspace_root": tree,
            "target_directory": target,
        })),
    ] {
        let bytes = serde_json::to_vec(&metadata).unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_size(bytes.len() as u64);
        archive.append_data(&mut header, path, bytes.as_slice()).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
}

pub(super) fn incremental_cargo_output(package: &Path, target: &Path) -> Vec<PathBuf> {
    std::fs::create_dir_all(package.join("src")).unwrap();
    std::fs::write(package.join("Cargo.toml"), concat!(
        "[package]\nname = \"cleanup_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        "[lib]\ncrate-type = [\"rlib\", \"staticlib\"]\n",
    )).unwrap();
    std::fs::write(package.join("src/lib.rs"), "pub fn built() -> u64 { 1428 }").unwrap();
    let output = std::process::Command::new(env!("CARGO")).current_dir(package)
        .args(["build", "--offline", "--manifest-path"]).arg(package.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", target).env("CARGO_INCREMENTAL", "1")
        .env_remove("CARGO_BUILD_BUILD_DIR").env_remove("CARGO_BUILD_TARGET")
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let incremental = target.join("debug/incremental");
    let unit = std::fs::read_dir(incremental).unwrap().next().unwrap().unwrap().path();
    let session = std::fs::read_dir(unit).unwrap().map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir()).unwrap();
    ["dep-graph.bin", "query-cache.bin", "work-products.bin"].map(|file| session.join(file)).to_vec()
}

#[tokio::test]
async fn startup_reclaims_unrecorded_private_output_and_keeps_live_group_output() {
    let (dir, runtime, group, active_tree) = building_group(None).await;
    let layouts = [
        ("target/debug/deps/runner", b"\x7fELFtest".as_slice()),
        ("crates/web/target/wasm32-unknown-unknown/debug/deps/site.wasm", b"\0asm\x01\0\0\0"),
        ("apps/macos/.build/out/Intermediates.noindex/AgentKit.o", b"\xcf\xfa\xed\xfetest"),
        ("apps/shared/AgentKit/.build/arm64-apple-macosx/debug/AgentKit.swiftmodule", b"\xe2\x9c\xa8\x0etest"),
    ];
    std::fs::write(runtime.repository().join(".git/info/exclude"),
        "/target/\n/crates/web/target/\n/apps/macos/.build/\n/apps/shared/AgentKit/.build/\n").unwrap();
    let mut orphan_trees = Vec::new();
    let mut orphan_caches = Vec::new();
    for (separator, digit) in [('-', "a"), (':', "b")] {
        let tree = runtime.root.join(format!("tree-wf1{separator}{}", digit.repeat(64)));
        adapters::git(&runtime.repository(), &["worktree", "add", "--detach", tree.to_str().unwrap(), "HEAD"]).await.unwrap();
        for (file, bytes) in &layouts[..2] {
            let path = tree.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        for (file, _) in &layouts[..2] {
            let path = tree.join(file);
            let dep = path.with_extension("d");
            std::fs::write(&dep, format!("{}: src/lib.rs\n\n{}: src/lib.rs\n",
                dep.display(), path.display())).unwrap();
        }
        for (file, bytes) in &layouts[2..] {
            let path = tree.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let output_map = path.with_file_name("output-file-map.json");
            let dependencies = path.with_file_name("primary.swiftdeps");
            std::fs::write(output_map, serde_json::to_vec(&serde_json::json!({
                "": {"swift-dependencies": dependencies},
                "src": {"object": path},
            })).unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
            std::fs::write(dependencies.with_extension("priors"), b"DDEP fixture").unwrap();
        }
        seed_nextest_archive(&tree);
        // Cargo cannot compile into a path containing the PATH separator.
        // The old colon spelling is still covered by the other artifacts.
        if separator == '-' {
            orphan_caches.extend(incremental_cargo_output(&dir.path().join("cargo-fixture"), &tree.join("target")));
        }
        std::fs::write(tree.join("uncommitted.txt"), b"keep this").unwrap();
        orphan_trees.push(tree);
    }
    assert_eq!(runtime.state("w").unwrap().unwrap().items[&group].phase, Phase::Running);
    // Even in the absence of a process lock, the running group's state protects real artifacts.
    let live_output = active_tree.join(layouts[0].0);
    std::fs::create_dir_all(live_output.parent().unwrap()).unwrap();
    std::fs::write(&live_output, layouts[0].1).unwrap();
    let dep = live_output.with_extension("d");
    std::fs::write(&dep, format!("{}: src/lib.rs\n\n{}: src/lib.rs\n",
        dep.display(), live_output.display())).unwrap();
    seed_nextest_archive(&active_tree);
    let active_caches = incremental_cargo_output(&dir.path().join("cargo-fixture"), &active_tree.join("target"));
    runtime.release_orphan_builds(std::slice::from_ref(&active_tree)).await.unwrap();
    assert!(live_output.is_file());
    assert!(active_tree.join("target/rust-tests.tar.zst").is_file());
    assert!(active_caches.iter().all(|path| path.is_file()));

    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    // The legacy and current names map to one lock for a given digest. Use a
    // source reader on the current tree and a held lock on a different digest.
    let reader = File::open(orphan_trees[0].join("uncommitted.txt")).unwrap();
    let lock = runtime.tree_lock(&orphan_trees[1]).read_owned().await;
    runtime.pump(100).await.unwrap();
    let held_deadline = runtime.orphan_cleanup_retries[&orphan_trees[0]];
    let lock_deadline = runtime.orphan_cleanup_retries[&orphan_trees[1]];
    assert!(held_deadline.duration_since(lock_deadline) > Duration::from_secs(60));
    assert!(runtime.next_cleanup.unwrap() <= lock_deadline);
    for tree in &orphan_trees {
        assert!(tree.join(layouts[0].0).is_file());
    }
    assert!(orphan_caches.iter().all(|path| path.is_file()));
    drop(reader);
    drop(lock);
    // Make the engine lock retry due while the checkout check is still deferred.
    runtime.orphan_cleanup_retries.insert(orphan_trees[1].clone(), tokio::time::Instant::now());
    runtime.pump(100).await.unwrap();
    assert!(!orphan_trees[1].join(layouts[0].0).exists());
    assert!(orphan_trees[0].join(layouts[0].0).exists());
    assert_eq!(runtime.orphan_cleanup_retries[&orphan_trees[0]], held_deadline);
    assert!(!runtime.orphan_cleanup_retries.contains_key(&orphan_trees[1]));
    runtime.orphan_cleanup_retries.insert(orphan_trees[0].clone(), tokio::time::Instant::now());
    runtime.pump(100).await.unwrap();
    assert!(orphan_trees.iter().all(|tree| !runtime.orphan_cleanup_retries.contains_key(tree)));
    assert!(orphan_caches.iter().all(|path| !path.exists()));
    for tree in &orphan_trees {
        assert!(!tree.join("target/rust-tests.tar.zst").exists());
        assert!(layouts.iter().all(|(file, _)| !tree.join(file).exists()));
        assert!(tree.join(".git").is_file());
        assert_eq!(std::fs::read(tree.join("uncommitted.txt")).unwrap(), b"keep this");
    }
}

#[tokio::test]
async fn legacy_cleanup_preserves_an_artifact_staged_during_inventory() {
    let (_dir, mut runtime, _, _) = building_group(None).await;
    let tree = runtime.root.join(format!("tree-wf1-{}", "f".repeat(64)));
    adapters::git(&runtime.repository(), &["worktree", "add", "--detach", tree.to_str().unwrap(), "HEAD"]).await.unwrap();
    let relative = "target/debug/deps/runner";
    let artifact = tree.join(relative);
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(&artifact, b"\x7fELFtest").unwrap();
    let dep = artifact.with_extension("d");
    std::fs::write(&dep, format!("{}: src/lib.rs\n\n{}: src/lib.rs\n",
        dep.display(), artifact.display())).unwrap();
    let check = std::sync::Arc::new((tokio::sync::Notify::new(), tokio::sync::Notify::new()));
    runtime.cleanup_check = Some(check.clone());
    let sweep = runtime.release_orphan_builds(std::slice::from_ref(&tree));
    let stage = async {
        check.0.notified().await;
        adapters::git(&tree, &["add", "--force", "--", relative]).await.unwrap();
        check.1.notify_one();
    };
    let (pending, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(sweep, stage)
    }).await.unwrap();
    assert!(pending.unwrap().is_empty());
    assert_eq!(std::fs::read(&artifact).unwrap(), b"\x7fELFtest");
}

#[tokio::test]
async fn build_cleanup_preserves_repository_moved_into_output_during_open_file_check() {
    let (dir, mut runtime, tree) = building().await;
    let new_repo = dir.path().join("new-repository");
    std::fs::create_dir(&new_repo).unwrap();
    adapters::git(&new_repo, &["init", "--bare"]).await.unwrap();
    let nested = tree.join("target/new-repository");
    let check = std::sync::Arc::new((tokio::sync::Notify::new(), tokio::sync::Notify::new()));
    runtime.cleanup_check = Some(check.clone());
    let move_repo = async {
        check.0.notified().await;
        std::fs::rename(&new_repo, &nested).unwrap();
        check.1.notify_one();
    };
    tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(review(&mut runtime), move_repo);
    }).await.unwrap();
    assert!(nested.join("HEAD").is_file());
    let result = runtime.store.workflow_cleanups("w").unwrap().remove(0).result.unwrap();
    assert!(result.skipped.iter().any(|s| s.contains("git metadata")), "{result:?}");
}

#[tokio::test]
async fn workflow_get_samples_private_trees_and_waiting_build_folders() {
    let (_dir, mut runtime, tree) = building_with(Some(vec!["target".into(), "target/deep".into()])).await;
    review(&mut runtime).await;
    let output = tree.join("target/deep/output");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    std::fs::write(&output, vec![0x5a; 8192]).unwrap();
    let waiting = [tree.join("target"), tree.join("target/deep"), output].iter()
        .map(|path| path.symlink_metadata().unwrap().blocks() * 512).sum::<u64>();
    runtime.refresh_disk_usage().unwrap().await.unwrap();
    let reply = runtime.read_get("w").unwrap();
    assert_eq!(reply["disk_usage"]["waiting_build_bytes"], waiting);
    assert!(reply["disk_usage"]["engine_tree_bytes"].as_u64().unwrap() >= waiting);
    assert!(reply["disk_usage"]["sampled_at_ms"].as_i64().is_some());
    assert_eq!(runtime.read_get("other").unwrap()["disk_usage"]["waiting_build_bytes"], 0);
}

#[tokio::test]
async fn build_cleanup_partial_folder_failure_reports_unknown_bytes() {
    let (_dir, mut runtime, group, tree) = building_group(Some(vec!["custom-output".into(), "target".into()])).await;
    super::coding_verbs_tests::until(&mut runtime, |s| s.items[&group].phase == Phase::WaitingReview).await;
    let protected = tree.join("target/protected");
    std::fs::create_dir_all(&protected).unwrap();
    std::fs::write(protected.join("output"), b"remaining output").unwrap();
    std::fs::create_dir_all(tree.join("custom-output")).unwrap();
    std::fs::write(tree.join("custom-output/busy"), b"retry this output").unwrap();
    let open = File::open(tree.join("custom-output/busy")).unwrap();
    std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o555)).unwrap();
    // The group is still live, so only configured build output is released this time.
    let mut effect = runtime.store.workflow_cleanups("w").unwrap().into_iter().rev()
        .find(|r| r.effect.item == group).unwrap().effect;
    effect.id = digest(&("partial-folder-test", &effect.id));
    runtime.store.workflow_cleanup_enqueue(&effect).unwrap();
    runtime.release_builds(&["w".into()]).await.unwrap();
    std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o755)).unwrap();
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let result = records.iter().find(|r| r.effect.id == effect.id).unwrap().result.as_ref().unwrap();
    assert!(result.retry, "the open folder needs a later sweep: {result:?}");
    assert!(result.skipped.iter().any(|s| s.contains("bytes freed unknown")),
        "a failed folder removal may already have deleted output: {result:?}");
    assert_eq!(result.bytes_freed, 0);
    drop(open);
    runtime.release_builds(&["w".into()]).await.unwrap();
    assert!(tree.exists() && !tree.join("target").exists() && !tree.join("custom-output").exists());
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let result = records.iter().find(|r| r.effect.id == effect.id).unwrap().result.as_ref().unwrap();
    assert!(!result.retry && result.bytes_freed == 0, "a retry cannot reconstruct the total: {result:?}");
    assert!(result.skipped.iter().any(|s| s.contains("bytes freed unknown")), "{result:?}");
}
