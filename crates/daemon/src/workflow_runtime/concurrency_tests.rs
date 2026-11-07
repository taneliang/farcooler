//! Two items in one repository run at once, under a cap (ov-952).
use super::*;
use super::coding_tests::configured;

pub(super) fn enqueue_item(runtime: &mut Runtime, workspace: &str, item: &str, definition: Definition) {
    let controller = runtime.controller();
    let state = runtime.state(workspace).unwrap();
    let revision = state.as_ref().map_or(0, |s| s.revision);
    let sequence = state.and_then(|s| s.source_sequences.get("controller").copied()).unwrap_or(0) + 1;
    let request = Request { id: format!("enqueue-{item}"), workspace: workspace.into(), expected_revision: revision,
        sequence, command: Command::Enqueue { item: item.into(), definition, policy_pin: "policy/v1".into(),
            resource: Exec::item_resource(workspace, item), dependencies: BTreeSet::new(), initial_candidate: None } };
    assert!(runtime.submit(&controller, request, 1).unwrap().result.is_ok());
}

/// The build stage of each item waits (up to four seconds) until both builds have started,
/// so it can only pass when the two builds really overlap.
async fn barrier(review: bool) -> (tempfile::TempDir, Runtime) {
    let (dir, runtime) = configured(review, "barrier").await;
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    let wait = "if stage == 'build':\n    if mode == 'barrier':\n        import time\n        d = pathlib.Path(os.environ['STARTED']).parent / 'barrier'\n        d.mkdir(exist_ok=True)\n        (d / os.environ['FARCOOLER_WORKFLOW_RUN']).write_text('x')\n        t = time.time()\n        while len(list(d.iterdir())) < 2:\n            if time.time() - t > 4: sys.exit(9)\n            time.sleep(0.05)\n";
    assert!(text.contains("if stage == 'build':\n"));
    std::fs::write(&script, text.replacen("if stage == 'build':\n", wait, 1)).unwrap();
    (dir, runtime)
}

#[tokio::test]
async fn workflow_two_items_in_one_repository_build_at_once() {
    let (_dir, mut runtime) = barrier(false).await;
    let definition = runtime.configured_definition().unwrap();
    enqueue_item(&mut runtime, "w", "one", definition.clone());
    enqueue_item(&mut runtime, "w", "two", definition);
    let state = runtime.state("w").unwrap().unwrap();
    assert!(state.items.values().all(|i| i.phase == Phase::Running), "both start at once: {state:?}");
    assert_ne!(state.items["one"].resource, state.items["two"].resource, "each item holds its own resource");
    for time in 2..4 { runtime.pump(time).await.unwrap(); }
    let state = runtime.state("w").unwrap().unwrap();
    for id in ["one", "two"] {
        assert_eq!(state.items[id].stage, 2, "{id} passed its build only if both builds overlapped: {state:?}");
        assert!(state.items[id].candidate.is_some());
    }
    assert_ne!(state.items["one"].candidate, state.items["two"].candidate);
}

#[tokio::test]
async fn workflow_effect_cap_holds_builds_back() {
    let (_dir, mut runtime) = barrier(false).await;
    runtime.set_effect_cap(1);
    let definition = runtime.configured_definition().unwrap();
    enqueue_item(&mut runtime, "w", "one", definition.clone());
    enqueue_item(&mut runtime, "w", "two", definition);
    runtime.pump(2).await.unwrap();
    // With a cap of one the first build can never see the second, so it fails its wait.
    runtime.pump(3).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert!(state.items.values().any(|i| i.stage < 2), "a cap of one must serialize the builds: {state:?}");
}

#[tokio::test]
async fn workflow_work_enqueued_during_a_sweep_starts_without_waiting() {
    let (_dir, mut runtime) = barrier(false).await;
    runtime.continuous = true;
    let definition = runtime.configured_definition().unwrap();
    enqueue_item(&mut runtime, "w", "one", definition.clone());
    // Item two arrives while item one's build is running. Its build only passes if it starts
    // while one is still running, rather than after one's build ends.
    let send = runtime.send.clone();
    let controller = runtime.controller();
    let late = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (reply, state) = oneshot::channel();
        send.send(Inbox::State("w".into(), reply)).await.ok().unwrap();
        let state = state.await.unwrap().unwrap().unwrap();
        let sequence = state.source_sequences.get("controller").copied().unwrap_or(0) + 1;
        let request = Request { id: "enqueue-two".into(), workspace: "w".into(), expected_revision: state.revision, sequence,
            command: Command::Enqueue { item: "two".into(), definition, policy_pin: "policy/v1".into(),
                resource: Exec::item_resource("w", "two"), dependencies: BTreeSet::new(), initial_candidate: None } };
        let (reply, receipt) = oneshot::channel();
        send.send(Inbox::Apply(Authority::Controller(controller.instance), Box::new(request), 1, reply)).await.ok().unwrap();
        receipt.await.unwrap().unwrap()
    });
    runtime.pump(2).await.unwrap();
    assert!(late.await.unwrap().result.is_ok());
    let state = runtime.state("w").unwrap().unwrap();
    for id in ["one", "two"] {
        assert!(state.items[id].candidate.is_some(), "{id} must overlap the other build: {state:?}");
    }
}

#[tokio::test]
async fn workflow_a_failed_write_ends_siblings_before_the_sweep_returns() {
    let (_dir, mut runtime) = barrier(false).await;
    let definition = runtime.configured_definition().unwrap();
    enqueue_item(&mut runtime, "w", "one", definition.clone());
    enqueue_item(&mut runtime, "w", "two", definition);
    runtime.fail_observe.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(runtime.pump(2).await.is_err());
    assert_eq!(runtime.drained, 1, "the sibling effect must be stopped and joined, not dropped");
}

#[tokio::test]
async fn workflow_a_later_landing_on_top_does_not_strand_an_earlier_item_in_ci() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue_item(&mut runtime, "w", "one", definition);
    // Step until the item has landed and CI is its next stage.
    let mut time = 2;
    while runtime.state("w").unwrap().unwrap().items["one"].stage < 5 {
        runtime.pump(time).await.unwrap(); time += 1;
        assert!(time < 30, "item never reached CI");
    }
    // Another item lands on top of it, a clean fast-forward.
    let repo = runtime.repository();
    let candidate = adapters::git(&repo, &["rev-parse", "refs/heads/workflow-target"]).await.unwrap();
    let on_top = adapters::git(&repo, &["-c", "user.name=Test", "-c", "user.email=test@localhost", "commit-tree",
        &format!("{candidate}^{{tree}}"), "-p", &candidate, "-m", "later"]).await.unwrap();
    adapters::git(&repo, &["update-ref", "refs/heads/workflow-target", &on_top]).await.unwrap();
    for _ in 0..8 { runtime.pump(time).await.unwrap(); time += 1; }
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].phase, Phase::Done, "{state:?}");
}
