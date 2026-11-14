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
    // A crash can fall between the durable cancellation and cleanup; another command can then archive it.
    std::fs::create_dir(tree.join("target")).unwrap();
    std::fs::write(tree.join("target/left-by-crash"), "output").unwrap();
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "another", definition);
    assert!(runtime.state("w").unwrap().unwrap().archive.contains_key("one"));
    runtime.prune_builder_targets(&["w".into()]).await.unwrap();
    assert!(!tree.join("target").exists(), "archiving cannot strand output after a crash");
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
    std::fs::create_dir_all(tree.join("target/debug")).unwrap();
    std::fs::write(tree.join("target/debug/stale"), "legacy output").unwrap();
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
