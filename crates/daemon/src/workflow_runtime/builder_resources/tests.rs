use super::*;
use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish},
    coding_verbs_tests::{lander, batched_push, at_join, command, until, phase_of}, concurrency_tests::enqueue_item};

fn adapter(dir: &Path, transform: impl FnOnce(String) -> String) {
    let path = dir.join("adapter.py");
    std::fs::write(&path, transform(std::fs::read_to_string(&path).unwrap())).unwrap();
}
fn ignore_target(runtime: &Runtime) {
    std::fs::write(runtime.repository().join(".git/info/exclude"), "target/\n").unwrap();
}
fn local_output(dir: &Path) {
    adapter(dir, |s| s.replace("if stage == 'build':\n", "if stage == 'build':\n    pathlib.Path('target').mkdir(exist_ok=True)\n    pathlib.Path('target/built').write_text('local output')\n"));
}

#[tokio::test]
async fn builder_and_its_children_yield_while_a_landing_gate_runs() {
    let (dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "first", &definition).await;
    let shared = dir.path().to_string_lossy().into_owned();
    adapter(dir.path(), |s| {
        let setup = format!("import time, pathlib\nshared = pathlib.Path({shared:?})\ndef wait(name):\n    deadline = time.monotonic() + 4\n    while not (shared / name).exists():\n        assert time.monotonic() < deadline, name\n        time.sleep(0.02)\n");
        let build = "if stage == 'build':\n    wait('gate-nice')\n    nice = os.getpriority(os.PRIO_PROCESS, 0)\n    child = subprocess.check_output(['/usr/bin/python3', '-c', 'import os; print(os.getpriority(os.PRIO_PROCESS, 0))'], text=True).strip()\n    (shared / 'builder-nice').write_text(str(nice) + ' ' + child)\n";
        let gate = "elif stage == 'gate':\n    (shared / 'gate-nice').write_text(str(os.getpriority(os.PRIO_PROCESS, 0)))\n    wait('builder-nice')\n";
        setup + &s.replace("if stage == 'build':\n", build).replace("elif stage == 'gate':\n", gate)
    });
    command(&runtime, Command::Queue { item: "first".into() });
    enqueue_item(&mut runtime, "w", "second", definition);
    runtime.continuous = true;
    runtime.pump(20).await.unwrap();
    let gate: i32 = std::fs::read_to_string(dir.path().join("gate-nice")).unwrap().parse().unwrap();
    let builders = std::fs::read_to_string(dir.path().join("builder-nice")).unwrap();
    for nice in builders.split_whitespace().map(|n| n.parse::<i32>().unwrap()) {
        assert!(nice > gate, "builder and compiler must yield: builder={nice}, gate={gate}");
    }
}

#[tokio::test]
async fn no_build_starts_below_25_gb_and_the_refusal_survives_recovery() {
    let (dir, mut runtime) = configured(false, "pass").await;
    runtime.build_free_bytes = Some(MIN_FREE - 1);
    adapter(dir.path(), |s| s.replace("if stage == 'build':\n", "if stage == 'build':\n    pathlib.Path(os.environ['STARTED']).write_text('launched')\n"));
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap(); // provision
    let binding = admitted(&runtime).await;
    let (_send, cancel) = tokio::sync::watch::channel(false);
    assert_eq!(runtime.execute_coding("w", &binding, cancel).await.unwrap(), Outcome::SafeFailureBecause(LOW_DISK.into()));
    assert!(!dir.path().join("started").exists());
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(200).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].phase, Phase::Failed);
    assert_eq!(state.items["one"].failure.as_deref(), Some(LOW_DISK));
    assert!(!dir.path().join("started").exists(), "recovery must never launch a refused build");
    runtime.build_free_bytes = Some(MIN_FREE);
    assert!(runtime.build_has_space(dir.path()).unwrap(), "exactly 25 GB is enough");
}

#[tokio::test]
async fn waiting_join_releases_local_and_managed_targets_and_keeps_source() {
    let (dir, mut runtime) = lander("pass").await;
    ignore_target(&runtime);
    local_output(dir.path());
    adapter(dir.path(), |s| s.replace("if stage == 'build':\n", "if stage == 'build':\n    cache = pathlib.Path(os.environ['CARGO_TARGET_DIR'])\n    cache.joinpath('dependency').write_text('reusable')\n"));
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    let tree = runtime.item_tree("w", "one");
    assert!(!tree.join("target").exists());
    assert!(tree.join(".git").exists());
    assert!(!runtime.builder_target(&tree).exists(), "the run's managed target is released too");
    let first = runtime.builder_target(&tree);
    at_join(&mut runtime, "two", &definition).await;
    let second = runtime.builder_target(&runtime.item_tree("w", "two"));
    assert_eq!(first.parent(), second.parent(), "one engine-owned cache");
    assert_ne!(first, second, "Cargo must not reuse another checkout's workspace fingerprints");
}

#[tokio::test]
async fn failed_run_releases_target_but_keeps_uncommitted_work() {
    let (dir, mut runtime) = configured(false, "prose").await;
    ignore_target(&runtime);
    local_output(dir.path());
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Failed);
    let tree = runtime.item_tree("w", "one");
    assert!(!tree.join("target").exists());
    assert!(tree.join("result.txt").exists());
}

#[tokio::test]
async fn cancelled_run_releases_target_only_after_its_writer_ends() {
    let (dir, mut runtime) = configured(false, "cancel").await;
    ignore_target(&runtime);
    local_output(dir.path());
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let tree = runtime.item_tree("w", "one");
    let controller = runtime.controller();
    let handle = runtime.command_handle(&controller).unwrap();
    let started = dir.path().join("started");
    let during = tree.clone();
    let stop = tokio::spawn(async move {
        for _ in 0..200 {
            if started.exists() { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(started.exists());
        assert!(during.join("target/built").exists(), "live output stays");
        let state = handle.state("w").await.unwrap().unwrap();
        handle.submit(&controller, Request { id: "cancel".into(), workspace: "w".into(),
            expected_revision: state.revision, sequence: state.source_sequences["controller"] + 1,
            command: Command::Cancel { item: "one".into(), reason: None } }, 20).await.unwrap()
    });
    runtime.pump(3).await.unwrap();
    assert!(stop.await.unwrap().result.is_ok());
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Cancelled)).await;
    assert!(!tree.join("target").exists());
    assert!(tree.join("result.txt").exists());
    // Once cleanup finished, a person can build again, even after this ticket is archived.
    std::fs::create_dir(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/left-by-crash"), "output").unwrap();
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "another", definition);
    assert!(runtime.state("w").unwrap().unwrap().archive.contains_key("one"));
    runtime.prune_builder_targets(&["w".into()]).await.unwrap();
    assert!(tree.join("target/left-by-crash").exists(), "archiving must not reclaim later user output");
}

#[test]
fn cache_eviction_is_lru_and_never_removes_a_live_or_foreign_target() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let names = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];
    for name in &names {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("output"), vec![1; 8192]).unwrap();
        lease(root, name).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    let live = lease(root, &names[0]).unwrap();
    flock(&live, FlockOperation::LockShared).unwrap();
    let bucket_size = disk_bytes(&root.join(&names[2])).unwrap();
    trim(root, bucket_size * 2, &BTreeSet::new()).unwrap();
    assert!(root.join(&names[0]).exists(), "leased output cannot be evicted");
    assert!(!root.join(&names[1]).exists(), "oldest idle output goes first");
    assert!(root.join(&names[2]).exists());
    drop(live);
    let retained_live = BTreeSet::from([root.join(&names[0])]);
    trim(root, 0, &retained_live).unwrap();
    assert!(root.join(&names[0]).exists(), "retained live runs stay safe after a daemon loses its lease");
    trim(root, 0, &BTreeSet::new()).unwrap();
    assert!(!root.join(&names[0]).exists());
    assert!(!root.join(&names[2]).exists());
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(foreign.path().join("source"), "keep").unwrap();
    std::os::unix::fs::symlink(foreign.path(), root.join("d".repeat(64))).unwrap();
    trim(root, 0, &BTreeSet::new()).unwrap();
    assert!(foreign.path().join("source").exists());
}

#[tokio::test]
async fn cleanup_preserves_tracked_output_and_never_follows_target_symlinks() {
    let (_dir, mut runtime) = lander("pass").await;
    ignore_target(&runtime);
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    let tree = runtime.item_tree("w", "one");
    std::fs::create_dir(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/source"), "tracked source").unwrap();
    adapters::git(&tree, &["add", "-f", "--", "target/source"]).await.unwrap();
    runtime.prune_builder_targets(&["w".into()]).await.unwrap();
    assert!(tree.join("target/source").exists(), "ignored directories can still contain tracked source");

    at_join(&mut runtime, "two", &definition).await;
    let tree = runtime.item_tree("w", "two");
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(foreign.path().join("keep"), "not build output").unwrap();
    std::os::unix::fs::symlink(foreign.path(), tree.join("target")).unwrap();
    runtime.prune_builder_targets(&["w".into()]).await.unwrap();
    assert!(foreign.path().join("keep").exists(), "cleanup must never traverse a symlink");
}

#[tokio::test]
async fn pre_policy_process_receipts_still_reconcile_after_upgrade() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let (_send, cancel) = tokio::sync::watch::channel(false);
    runtime.execute_coding("w", &binding, cancel).await.unwrap();
    let path = runtime.root.join("receipts").join(format!("{}.process.json", digest(&binding.effect)));
    let mut fact: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    fact.as_object_mut().unwrap().remove("managed_build");
    fact["argv"].as_array_mut().unwrap().drain(..3);
    fact["environment"].as_object_mut().unwrap().remove("CARGO_TARGET_DIR");
    std::fs::write(&path, serde_json::to_vec(&fact).unwrap()).unwrap();
    assert!(matches!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::Passed(_)));
}

#[test]
fn disk_space_probe_reads_available_bytes() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_bytes(dir.path()).unwrap() > 0);
    assert!(free_bytes(&dir.path().join("missing")).is_err());
}

#[tokio::test]
async fn managed_cargo_output_remains_available_through_target_for_existing_scripts() {
    let (dir, mut runtime) = configured(false, "pass").await;
    ignore_target(&runtime); // A directory-only rule must also ignore the managed link.
    adapter(dir.path(), |s| s.replace("if stage == 'build':\n", "if stage == 'build':\n    cache = pathlib.Path(os.environ['CARGO_TARGET_DIR'])\n    cache.joinpath('debug').mkdir(exist_ok=True)\n    cache.joinpath('debug/library.a').write_text('fresh library')\n    assert pathlib.Path('target/debug/library.a').read_text() == 'fresh library'\n    assert pathlib.Path('target').resolve() == cache\n    assert not pathlib.Path('target/debug/stale').exists()\n"));
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let tree = runtime.item_tree("w", "one");
    runtime.pump(3).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert!(state.items["one"].candidate.is_some(), "a build script must see Cargo's fresh managed output");
    assert_eq!(std::fs::read_link(tree.join("target")).unwrap(), runtime.builder_target(&tree));
    assert!(adapters::git(&tree, &["status", "--porcelain"]).await.unwrap().is_empty(), "the output link stays ignored");
}

#[tokio::test]
async fn a_step_entering_an_approval_wait_trims_its_idle_cache_and_the_next_gate_can_rebuild() {
    let (dir, mut runtime) = configured(false, "pass").await;
    ignore_target(&runtime);
    runtime.build_cache_cap = 0;
    local_output(dir.path());
    let mut definition = runtime.configured_definition().unwrap();
    definition.stages[2].progression = Progression::AwaitApproval { principal: "owner".into() };
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    runtime.pump(3).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    let item = &state.items["one"];
    assert_eq!(item.phase, Phase::WaitingApproval);
    let tree = runtime.item_tree("w", "one");
    assert!(!runtime.builder_target(&tree).exists(), "the last step's output must respect the cap once idle");
    let session = runtime.decision_session(&runtime.controller(), "owner").unwrap();
    let request = Request { id: "approve".into(), workspace: "w".into(), expected_revision: state.revision,
        sequence: 1, command: Command::Approve { item: "one".into(), run: item.run.clone(), pin: item.pin.clone(), candidate: item.candidate.clone() } };
    assert!(runtime.decide(&session, request, 100).unwrap().result.is_ok());
    runtime.pump(101).await.unwrap();
    assert!(tree.join("target").is_dir(), "the next gate recreates an evicted bucket before it runs");
    assert_ne!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
}

/// Use a real adopted checkout, rather than an engine-private tree, for ownership regressions.
async fn adopted() -> (tempfile::TempDir, Runtime, PathBuf, String) {
    let (dir, mut runtime) = configured(false, "prose").await;
    let host = Uuid::now_v7();
    let repo = runtime.repository();
    let root = runtime.store.create_repository_root(host, repo.to_str().unwrap(), 0).unwrap();
    let repository = runtime.store.create_repository(host, root.id, "adopted", repo.join(".git").to_str().unwrap(), "").unwrap();
    let workspace = runtime.store.create_workspace(repository.id, "Adopted", "ad").unwrap().id.to_string();
    runtime.host = Some(WorktreeHost { dir: dir.path().join("trees"), install_id: "test".into(), locks: Default::default() });
    ignore_target(&runtime);
    let tree = dir.path().join("adopted");
    adapters::git(&runtime.repository(), &["worktree", "add", "-b", "adopted", tree.to_str().unwrap(), "HEAD"]).await.unwrap();
    let receipt = runtime.apply(Request { id: "track".into(), workspace: workspace.clone(), expected_revision: 0, sequence: 1,
        command: Command::Enqueue { item: "one".into(), definition: Definition::ticket(), policy_pin: "p".into(),
            resource: Exec::item_resource(&workspace, "one"), dependencies: BTreeSet::new(), initial_candidate: Some("one".into()) } },
        "controller", Grant::Controller, 1).unwrap();
    assert!(receipt.result.is_ok(), "{receipt:?}");
    runtime.store.create_worktree(repository.id, "adopted", tree.to_str().unwrap(), false).unwrap();
    let named = RunTree { name: "adopted".into(), branch: "adopted".into(), path: tree.to_str().unwrap().into(),
        base: runtime.base.clone(), adopted: true };
    let state = runtime.state(&workspace).unwrap().unwrap();
    let receipt = runtime.apply(Request { id: "start".into(), workspace: workspace.clone(), expected_revision: state.revision, sequence: 2,
        command: Command::Start { item: "one".into(), definition: runtime.configured_definition().unwrap(),
        policy_pin: "p".into(), resource: Exec::item_resource(&workspace, "one"), dependencies: BTreeSet::new(),
        initial_candidate: None, models: Default::default(), tree: Some(Box::new(named)), with: vec![] } }, "controller", Grant::Controller, 1).unwrap();
    assert!(receipt.result.is_ok(), "{receipt:?}");
    (dir, runtime, tree, workspace)
}

#[tokio::test]
async fn adopted_preexisting_output_is_preserved_through_build_and_cleanup() {
    for symlink in [false, true] {
        let (dir, mut runtime, tree, workspace) = adopted().await;
        let output = if symlink { dir.path().join("personal-output") } else { tree.join("target") };
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("keep"), "personal output").unwrap();
        if symlink { std::os::unix::fs::symlink(&output, tree.join("target")).unwrap(); }
        runtime.pump(2).await.unwrap();
        runtime.pump(3).await.unwrap();
        assert_eq!(std::fs::read_to_string(tree.join("target/keep")).unwrap(), "personal output");
        assert_eq!(finish(&mut runtime, &workspace).await.items["one"].phase, Phase::Failed);
        assert_eq!(std::fs::read_to_string(tree.join("target/keep")).unwrap(), "personal output");
        assert!(!runtime.builder_target(&tree).exists());
    }
}

#[tokio::test]
async fn low_disk_refusal_never_touches_adopted_output_or_prepares_a_bucket() {
    let (_dir, mut runtime, tree, workspace) = adopted().await;
    std::fs::create_dir(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/keep"), "personal output").unwrap();
    runtime.build_free_bytes = Some(MIN_FREE - 1);
    assert_eq!(finish(&mut runtime, &workspace).await.items["one"].failure.as_deref(), Some(LOW_DISK));
    assert_eq!(std::fs::read_to_string(tree.join("target/keep")).unwrap(), "personal output");
    assert!(!runtime.builder_target(&tree).exists());
}

#[tokio::test]
async fn adopted_user_rebuild_after_cleanup_survives_archive_and_restart() {
    let (_dir, mut runtime, tree, workspace) = adopted().await;
    assert_eq!(finish(&mut runtime, &workspace).await.items["one"].phase, Phase::Failed);
    assert!(tree.join("target").symlink_metadata().is_err());
    // Recreate both local output and the managed bucket to prove cleanup is durably complete.
    std::fs::create_dir(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/user"), "new output").unwrap();
    let cached = runtime.builder_target(&tree);
    std::fs::create_dir(&cached).unwrap();
    std::fs::write(cached.join("user"), "new cached output").unwrap();
    runtime.prune_builder_targets(std::slice::from_ref(&workspace)).await.unwrap();
    assert!(tree.join("target/user").exists());
    assert!(runtime.internal(&workspace, Command::Cancel { item: "one".into(), reason: None }, Grant::Controller, 50).unwrap().result.is_ok());
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, &workspace, "another", definition);
    assert!(runtime.state(&workspace).unwrap().unwrap().archive.contains_key("one"));
    let root = runtime.root.clone();
    drop(runtime);
    let runtime = Runtime::open(&root).unwrap();
    for _ in 0..2 { runtime.prune_builder_targets(std::slice::from_ref(&workspace)).await.unwrap(); }
    assert!(tree.join("target/user").exists());
    assert!(cached.join("user").exists());
}

#[tokio::test]
async fn cleanup_recovery_preserves_a_replacement_target() {
    let (_dir, mut runtime, tree, workspace) = adopted().await;
    runtime.pump(2).await.unwrap();
    let effect = runtime.store.workflow_outbox(&workspace).unwrap().into_iter()
        .find(|s| s.active && matches!(s.effect.kind, EffectKind::Execute(StageKind::Build))).unwrap().effect;
    drop(runtime.prepare_builder_target(&tree, &effect).await.unwrap());
    // Replace the engine link before terminal cleanup. Ownership must identify the original link.
    std::fs::remove_file(tree.join("target")).unwrap();
    std::fs::create_dir(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/user"), "new output").unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    finish(&mut runtime, &workspace).await;
    assert!(tree.join("target/user").exists());
    assert!(!runtime.builder_target(&tree).exists());
}

#[tokio::test]
async fn cleanup_preserves_an_engine_link_that_was_explicitly_tracked() {
    let (_dir, mut runtime, tree, workspace) = adopted().await;
    runtime.pump(2).await.unwrap();
    let effect = runtime.store.workflow_outbox(&workspace).unwrap().into_iter()
        .find(|s| s.active && matches!(s.effect.kind, EffectKind::Execute(StageKind::Build))).unwrap().effect;
    drop(runtime.prepare_builder_target(&tree, &effect).await.unwrap());
    adapters::git(&tree, &["add", "-f", "--", "target"]).await.unwrap();
    finish(&mut runtime, &workspace).await;
    assert!(tree.join("target").symlink_metadata().unwrap().file_type().is_symlink());
}

#[tokio::test]
async fn adopted_existing_target_refuses_before_compiling_or_reading_stale_artifacts() {
    for preexisting in [false, true] {
        let (dir, mut runtime, tree, workspace) = adopted().await;
        // Exercise a real compiler and a target-based consumer when allowed to start. The same
        // command must be refused before launch if target/ contains somebody else's artifacts.
        adapter(dir.path(), |s| s.replace("if stage == 'build':\n", "if stage == 'build':\n    pathlib.Path(os.environ['STARTED']).write_text('launched')\n    cache = pathlib.Path(os.environ['CARGO_TARGET_DIR'])\n    (cache / 'debug').mkdir(exist_ok=True)\n    subprocess.run(['/usr/bin/cc', '-x', 'c', '-', '-o', str(cache / 'debug/fresh')], input='int main(void) { return 0; }', text=True, check=True)\n    subprocess.run(['target/debug/fresh'], check=True)\n    pathlib.Path(os.environ['STARTED'] + '.consumed').write_text('yes')\n"));
        if preexisting {
            std::fs::create_dir_all(tree.join("target/debug")).unwrap();
            std::fs::write(tree.join("target/debug/fresh"), "personal artifact").unwrap();
        }
        runtime.pump(2).await.unwrap();
        runtime.pump(3).await.unwrap();
        if preexisting {
            assert!(!dir.path().join("started").exists(), "refuse before any build command starts");
            assert!(!runtime.builder_target(&tree).exists());
            assert_eq!(std::fs::read_to_string(tree.join("target/debug/fresh")).unwrap(), "personal artifact");
            let root = runtime.root.clone();
            drop(runtime);
            let mut runtime = Runtime::open(&root).unwrap();
            assert_eq!(finish(&mut runtime, &workspace).await.items["one"].failure.as_deref(), Some(EXISTING_TARGET));
            assert_eq!(std::fs::read_to_string(tree.join("target/debug/fresh")).unwrap(), "personal artifact");
        } else {
            assert!(dir.path().join("started.consumed").exists(), "compiler and target/ consumer agree");
        }
    }
}

#[tokio::test]
async fn compatibility_link_publication_recovers_at_each_crash_boundary() {
    for boundary in 0..3 {
        let (_dir, mut runtime, tree, workspace) = adopted().await;
        runtime.pump(2).await.unwrap();
        let effect = runtime.store.workflow_outbox(&workspace).unwrap().into_iter()
            .find(|s| s.active && matches!(s.effect.kind, EffectKind::Execute(StageKind::Build))).unwrap().effect;
        let cached = runtime.builder_target(&tree);
        let staged = runtime.staged_builder_link(&tree, &effect).unwrap();
        if boundary == 0 {
            // Interrupted between staging the symlink and persisting ownership: target/ is absent.
            std::os::unix::fs::symlink(&cached, &staged).unwrap();
        } else {
            runtime.link_builder_target(&tree, &cached, &effect).await.unwrap();
            assert!(tree.join("target").symlink_metadata().is_err(), "publish only after durable ownership");
            assert!(runtime.target_receipt(&effect, "owner").is_file());
            if boundary == 2 {
                // Interrupted after publication but before removing the staged name.
                std::fs::hard_link(&staged, tree.join("target")).unwrap();
            }
        }
        let root = runtime.root.clone();
        drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        drop(runtime.prepare_builder_target(&tree, &effect).await.unwrap());
        assert_eq!(std::fs::read_link(tree.join("target")).unwrap(), cached);
        assert!(staged.symlink_metadata().is_err());
        finish(&mut runtime, &workspace).await;
        assert!(tree.join("target").symlink_metadata().is_err(), "no dangling compatibility link after cleanup");
        assert!(!cached.exists());
    }
}
