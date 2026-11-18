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
async fn a_running_builder_is_paused_while_a_landing_gate_runs_and_resumes_after() {
    let (dir, mut runtime) = lander("pass").await;
    // The gate and the builder start together, and the builder is stopped for as long as the gate runs: on a loaded host the
    // default 5 s limit can expire before the gate has seen it (the limit runs while the builder is stopped, too).
    let coding = runtime.coding.as_mut().unwrap();
    for check in &mut coding.checks { check.timeout_ms = 120_000; }
    if let CodingHarness::Command(build) = &mut coding.harness { build.timeout_ms = 120_000; }
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "first", &definition).await;
    let shared = dir.path().to_string_lossy().into_owned();
    adapter(dir.path(), |s| {
        let setup = format!("import time, pathlib, subprocess\nshared = pathlib.Path({shared:?})\ndef wait(name):\n    deadline = time.monotonic() + 60\n    while not (shared / name).exists():\n        assert time.monotonic() < deadline, name\n        time.sleep(0.02)\n");
        // The builder only waits. It cannot report that it was stopped, and under load it may be stopped before it has run a line.
        let build = "if stage == 'build':\n    wait('gate-saw-it-stopped')\n    (shared / 'resumed').write_text(str(os.getpriority(os.PRIO_PROCESS, 0)))\n";
        // The gate finds this test's own builder by its command line (the full adapter path, so no other test's builder can match), and polls its real process state until the OS reports it stopped
        // (SIGSTOP shows as 'T'). The signal is asynchronous and the builder may start after the gate, so a single look can miss it.
        let gate = "elif stage == 'gate':\n    (shared / 'gate-nice').write_text(str(os.getpriority(os.PRIO_PROCESS, 0)))\n    deadline = time.monotonic() + 60\n    while True:\n        rows = subprocess.run(['ps', '-axww', '-o', 'stat=,command='], capture_output=True, text=True).stdout.splitlines()\n        if any(r.split(None, 1)[0].startswith('T') and r.rstrip().endswith(str(shared / 'adapter.py') + ' build') for r in rows if r.strip()): break\n        assert time.monotonic() < deadline, 'builder was never paused'\n        time.sleep(0.05)\n    (shared / 'gate-saw-it-stopped').write_text('yes')\n    time.sleep(0.3)\n";
        setup + &s.replace("if stage == 'build':\n", build).replace("elif stage == 'gate':\n", gate)
    });
    command(&runtime, Command::Queue { item: "first".into() });
    enqueue_item(&mut runtime, "w", "second", definition);
    runtime.continuous = true;
    runtime.pump(20).await.unwrap();
    assert!(dir.path().join("gate-saw-it-stopped").exists(), "the gate saw the builder paused");
    // It was paused, not killed: it ran on after the gate, and it still yields by niceness.
    let resumed: i32 = std::fs::read_to_string(dir.path().join("resumed")).expect("the builder resumed after the gate").parse().unwrap();
    let gate: i32 = std::fs::read_to_string(dir.path().join("gate-nice")).unwrap().parse().unwrap();
    assert!(resumed > gate, "builder={resumed}, gate={gate}");
    assert!(!runtime.exec.yielding.paused(), "no gate runs now");
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
async fn waiting_join_releases_the_runs_target_and_keeps_source() {
    let (dir, mut runtime) = lander("pass").await;
    ignore_target(&runtime);
    local_output(dir.path());
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    let tree = runtime.item_tree("w", "one");
    assert!(!tree.join("target").exists());
    assert!(tree.join(".git").exists());
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
        assert!(super::super::wait_until(tokio::time::Instant::now() + Duration::from_secs(60),
            Duration::from_millis(10), || started.exists()).await, "builder never started");
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
    runtime.release_run_targets().await.unwrap();
    assert!(tree.join("target/source").exists(), "ignored directories can still contain tracked source");

    at_join(&mut runtime, "two", &definition).await;
    let tree = runtime.item_tree("w", "two");
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(foreign.path().join("keep"), "not build output").unwrap();
    std::os::unix::fs::symlink(foreign.path(), tree.join("target")).unwrap();
    runtime.release_run_targets().await.unwrap();
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
    std::fs::write(&path, serde_json::to_vec(&fact).unwrap()).unwrap();
    assert!(matches!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::Passed(_)));
}

#[test]
fn disk_space_probe_reads_available_bytes() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_bytes(dir.path()).unwrap() > 0);
    assert!(free_bytes(&dir.path().join("missing")).is_err());
}

async fn adopted() -> (tempfile::TempDir, Runtime, PathBuf, String) {
    let (dir, mut runtime) = configured(false, "prose").await;
    let host = Uuid::now_v7();
    let repo = runtime.repository();
    let root = runtime.store.create_repository_root(host, repo.to_str().unwrap(), 0).unwrap();
    let repository = runtime.store.create_repository(host, root.id, "adopted", repo.join(".git").to_str().unwrap(), "").unwrap();
    let workspace = runtime.store.create_workspace(repository.id, "Adopted", "ad").unwrap().id.to_string();
    runtime.host = Some(WorktreeHost { dir: dir.path().join("trees"), install_id: "test".into(), locks: Default::default() });
    ignore_target(&runtime);
    // Canonical, as a person's worktree path is: on macOS the temp dir sits behind the /var -> /private/var link, and
    // the build cleanup skips a non-canonical tree, so without this the test passed there and failed on Linux (ov-1263).
    let tree = dir.path().canonicalize().unwrap().join("adopted");
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
        command: Command::Start { start_load: None, item: "one".into(), definition: runtime.configured_definition().unwrap(),
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
    }
}

#[tokio::test]
async fn low_disk_refusal_never_touches_adopted_output() {
    let (_dir, mut runtime, tree, workspace) = adopted().await;
    std::fs::create_dir(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/keep"), "personal output").unwrap();
    runtime.build_free_bytes = Some(MIN_FREE - 1);
    assert_eq!(finish(&mut runtime, &workspace).await.items["one"].failure.as_deref(), Some(LOW_DISK));
    assert_eq!(std::fs::read_to_string(tree.join("target/keep")).unwrap(), "personal output");
}
