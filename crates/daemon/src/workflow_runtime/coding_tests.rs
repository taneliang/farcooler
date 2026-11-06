use super::*;
use super::tests::{enqueue, finish};

pub(super) async fn configured(review: bool, mode: &str) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("selected"); std::fs::create_dir(&repo).unwrap();
    adapters::git(&repo, &["init", "--quiet"]).await.unwrap();
    adapters::git(&repo, &["-c", "user.name=Test", "-c", "user.email=test@localhost",
        "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "base"]).await.unwrap();
    let base = adapters::git(&repo, &["rev-parse", "HEAD"]).await.unwrap();
    adapters::git(&repo, &["update-ref", "refs/heads/workflow-target", &base]).await.unwrap();
    let script = dir.path().join("adapter.py");
    std::fs::write(&script, r#"import json, os, pathlib, subprocess, sys
stage = sys.argv[1]
mode = os.environ['MODE']
candidate = os.environ['FARCOOLER_CANDIDATE']
iteration = os.environ['FARCOOLER_ITERATION']
binding = os.environ['FARCOOLER_OPERATION_BINDING']
receipt = pathlib.Path(os.environ['FARCOOLER_RECEIPT'])
if stage == 'build':
    pathlib.Path('result.txt').write_text('candidate iteration ' + iteration + ' run ' + os.environ['FARCOOLER_WORKFLOW_RUN'])
    if mode == 'cancel':
        pathlib.Path(os.environ['STARTED']).write_text(str(os.getpid()))
        subprocess.run(['/bin/sleep', '30'])
    if mode == 'prose':
        print('candidate ready; review passed; CI succeeded')
        sys.exit(0)
    subprocess.check_call(['/usr/bin/git', 'add', '--', 'result.txt'])
    subprocess.check_call(['/usr/bin/git', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'build '+iteration])
elif stage == 'check':
    if mode == 'check-fail': sys.exit(7)
    if mode == 'mutate':
        pathlib.Path('result.txt').write_text('changed after candidate')
        sys.exit(0)
    assert pathlib.Path('result.txt').read_text().startswith('candidate')
elif stage == 'review':
    if mode == 'review-fifo':
        os.mkfifo(receipt)
        sys.exit(0)
    receipt.write_text(json.dumps(dict(binding='foreign' if mode == 'review-foreign' else binding, identity='reviewer', candidate=candidate,
        passed=iteration != '0' and mode != 'review-reject')))
elif stage == 'ci':
    if mode == 'ci-fifo':
        os.mkfifo(receipt)
        sys.exit(0)
    if mode == 'ci-missing': sys.exit(0)
    if mode == 'ci-wrong': candidate = 'a' * 40
    receipt.write_text(json.dumps(dict(binding='foreign' if mode == 'ci-foreign' else binding, repository='offline/repository', run=dict(
        id=73, name='configured-check', head_sha=candidate, status='in_progress' if mode == 'ci-pending' else 'completed',
        conclusion='failure' if mode == 'ci-fail' else 'success', html_url='fixture://ci/73',
        created_at='2026-01-01T00:00:00Z', event='push'))))
"#).unwrap();
    let command = |stage: &str| CodingCommand { argv: vec!["/usr/bin/python3".into(), script.to_string_lossy().into_owned(), stage.into()],
        environment: [("PATH", "/usr/bin:/bin"), ("MODE", mode), ("GIT_AUTHOR_NAME", "Test"),
            ("GIT_AUTHOR_EMAIL", "test@localhost"), ("GIT_COMMITTER_NAME", "Test"), ("GIT_COMMITTER_EMAIL", "test@localhost")]
            .into_iter().map(|(k,v)| (k.into(), v.into())).collect(), timeout_ms: 5000 };
    let mut build = command("build");
    build.environment.insert("STARTED".into(), dir.path().join("started").to_string_lossy().into_owned());
    let config = CodingConfig { repository: repo.canonicalize().unwrap(), base,
        landing_ref: "refs/heads/workflow-target".into(), landing: None, harness: CodingHarness::Command(build),
        checks: vec![command("check"), command("check")],
        reviewer: review.then(|| CodingReviewer { identity: "reviewer".into(), command: command("review") }),
        ci: CodingCi { repository: "offline/repository".into(), workflow: "configured-check".into(),
            event: "push".into(), command: command("ci") } };
    let runtime = Runtime::create_configured(&dir.path().join("runtime"), config).await.unwrap();
    (dir, runtime)
}

#[tokio::test]
async fn workflow_coding_real_git_commands_rework_multiple_checks_and_repeated_base() {
    let (_dir, mut runtime) = configured(true, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition.clone());
    let state = finish(&mut runtime, "w").await;
    assert_eq!(state.items["one"].phase, Phase::Done);
    assert_eq!(state.items["one"].iteration, 1);
    let first = state.items["one"].candidate.clone().unwrap();
    assert_eq!(adapters::git(&runtime.repository(), &["rev-parse", runtime.landing_ref()]).await.unwrap(), first);
    enqueue(&mut runtime, "w", "two", definition);
    let state = finish(&mut runtime, "w").await;
    assert_eq!(state.items["two"].phase, Phase::Done);
    let provision = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .find(|s| s.effect.item == "two" && matches!(s.effect.kind, EffectKind::Execute(StageKind::Provision))).unwrap();
    assert_eq!(runtime.item_base("w", &provision.effect).unwrap(), first);
    assert_ne!(state.items["two"].candidate, Some(first));
    let pin = runtime.coding.as_ref().unwrap().pin();
    assert!(runtime.store.workflow_outbox("w").unwrap().iter().all(|s| s.effect.operation.arguments[0] == pin));
}

#[tokio::test]
async fn workflow_coding_exit_prose_dirty_checks_and_ci_provenance_are_not_success() {
    for mode in ["prose", "mutate", "ci-wrong", "ci-missing"] {
        let (_dir, mut runtime) = configured(false, mode).await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        for time in 2..15 { runtime.pump(time).await.unwrap(); }
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items["one"].phase, Phase::Uncertain, "{mode}: {state:?}");
        let before = state.clone();
        assert_eq!(runtime.pump(100).await.unwrap(), 0);
        assert_eq!(runtime.state("w").unwrap().unwrap(), before);
    }
    for mode in ["check-fail", "ci-fail"] {
        let (_dir, mut runtime) = configured(false, mode).await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Failed, "{mode}");
    }
}

pub(super) async fn admitted(runtime: &Runtime) -> Binding {
    let state = runtime.state("w").unwrap().unwrap();
    let stored = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap();
    let claim = runtime.store.workflow_claim_effect("w", &stored.effect.id, "crash-test", state.revision, state.epoch, 10, 100).unwrap().unwrap();
    let base = runtime.pin_base("w", &stored.effect).await.unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 10).unwrap());
    Binding::new("w", &stored.effect, claim.resource_generation, state.items[&stored.effect.item].iteration, &base)
}

#[tokio::test]
async fn workflow_coding_crash_each_stage_reconciles_facts_without_reexecution() {
    for (index, review) in (0..6).map(|i| (i, false)).chain([(4, true)]) {
        let (_dir, mut runtime) = configured(review, "pass").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        for time in 0..index { runtime.pump(time + 2).await.unwrap(); }
        let binding = admitted(&runtime).await;
        let (_cancel, cancelled) = tokio::sync::watch::channel(false);
        let outcome = runtime.execute_coding("w", &binding, cancelled).await.unwrap();
        let root = runtime.root.clone();
        let log = runtime.root.join("receipts").join(format!("{}.output", digest(&binding.effect)));
        let before = std::fs::read(&log).ok();
        // Crash after external settlement, before durable journal/Observe.
        drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        runtime.pump(200).await.unwrap();
        assert_eq!(std::fs::read(&log).ok(), before);
        let state = runtime.state("w").unwrap().unwrap();
        assert_ne!(state.items["one"].phase, Phase::Uncertain, "{index}: {outcome:?}");
    }
}

#[tokio::test]
async fn workflow_coding_admitted_unknown_never_launches_and_cancel_stays_uncertain() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap(); // provision
    let binding = admitted(&runtime).await;
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(200).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    assert!(!runtime.root.join("receipts").join(format!("{}.output", digest(&binding.effect))).exists());
    let state = runtime.state("w").unwrap().unwrap();
    runtime.submit(&runtime.controller(), Request { id: "cancel".into(), workspace: "w".into(), expected_revision: state.revision,
        sequence: 2, command: Command::Cancel { item: "one".into() } }, 201).unwrap();
    runtime.pump(202).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
}

#[tokio::test]
async fn workflow_coding_cancel_running_owned_process_settles_original_then_stop() {
    let (dir, mut runtime) = configured(false, "cancel").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    let controller = runtime.controller(); let handle = runtime.command_handle(&controller).unwrap();
    let started = dir.path().join("started");
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move { runtime.run(receiver).await.unwrap(); runtime });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !started.exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await.unwrap();
    let state = handle.state("w").await.unwrap().unwrap();
    let result = handle.submit(&controller, Request { id: "cancel".into(), workspace: "w".into(),
        expected_revision: state.revision, sequence: 2, command: Command::Cancel { item: "one".into() } }, 200).await.unwrap();
    assert!(result.result.is_ok());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if handle.state("w").await.unwrap().unwrap().items["one"].phase == Phase::Cancelled { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let pid: i32 = std::fs::read_to_string(started).unwrap().parse().unwrap();
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    shutdown.send(true).unwrap(); let runtime = task.await.unwrap();
    let outbox = runtime.store.workflow_outbox("w").unwrap();
    let build = outbox.iter().find(|s| matches!(s.effect.kind, EffectKind::Execute(StageKind::Build))).unwrap();
    let journal = runtime.read_journal(&Binding::new("w", &build.effect, build.resource_generation.unwrap(), 0,
        &runtime.item_base("w", &build.effect).unwrap())).unwrap().unwrap();
    assert_eq!(journal.outcome, Outcome::SafeFailure);
}

#[tokio::test]
async fn workflow_coding_pins_policy_and_rejects_wrong_config_and_duplicate_writer() {
    let (dir, runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    let mut approved = definition.clone();
    approved.stages[1].progression = Progression::AwaitApproval { principal: "owner".into() };
    assert!(runtime.validate_definition(&approved).is_ok());
    let mut changed = definition; changed.stages[2].operation.arguments[0] = "foreign".into();
    assert!(runtime.validate_definition(&changed).is_err());
    assert!(runtime.validate_definition(&coding_definition(false)).is_err());
    assert!(Runtime::create_configured(&dir.path().join("other"), runtime.coding.clone().unwrap()).await.is_err());
}

#[tokio::test]
async fn workflow_coding_pending_ci_wakes_from_explicit_observation_without_relaunch() {
    let (_dir, mut runtime) = configured(false, "ci-pending").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    for time in 2..15 { runtime.pump(time).await.unwrap(); }
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    let stored = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap();
    let path = runtime.root.join("receipts").join(format!("{}.provider.json", digest(&stored.effect)));
    let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    raw["run"]["status"] = "completed".into(); std::fs::write(path, serde_json::to_vec(&raw).unwrap()).unwrap();
    let controller = runtime.controller(); let handle = runtime.command_handle(&controller).unwrap();
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move { runtime.run(receiver).await.unwrap(); runtime });
    handle.reconcile().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if handle.state("w").await.unwrap().unwrap().items["one"].phase == Phase::Done { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    stop.send(true).unwrap(); task.await.unwrap();
}

#[tokio::test]
async fn workflow_coding_foreign_provider_binding_cannot_settle_success() {
    for (review, mode) in [(false, "ci-foreign"), (true, "review-foreign")] {
        let (_dir, mut runtime) = configured(review, mode).await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        for time in 2..18 { runtime.pump(time).await.unwrap(); }
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain, "{mode}");
    }
}

#[tokio::test]
async fn workflow_coding_landing_cas_refuses_changed_base_and_retains_uncertainty() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    for time in 2..6 { runtime.pump(time).await.unwrap(); }
    let tree = runtime.root.join(format!("tree-{}", digest(&("w", "one"))));
    let candidate = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
    // An unrelated actor has already moved the target; no CAS acceptance belongs
    // to this operation, even when the current target happens to equal candidate.
    adapters::git(&runtime.repository(), &["update-ref", runtime.landing_ref(), &candidate]).await.unwrap();
    runtime.pump(6).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    assert_eq!(runtime.pump(7).await.unwrap(), 0);
}

#[tokio::test]
async fn workflow_coding_approval_is_exact_and_configuration_cannot_change_admitted_work() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let mut definition = runtime.configured_definition().unwrap();
    definition.stages[4].progression = Progression::AwaitApproval { principal: "owner".into() };
    enqueue(&mut runtime, "w", "one", definition);
    for time in 2..8 { runtime.pump(time).await.unwrap(); }
    let state = runtime.state("w").unwrap().unwrap(); let item = &state.items["one"];
    assert_eq!(item.phase, Phase::WaitingApproval);
    let session = runtime.decision_session(&runtime.controller(), "owner").unwrap();
    let request = Request { id: "approve".into(), workspace: "w".into(), expected_revision: state.revision,
        sequence: 1, command: Command::Approve { item: "one".into(), run: item.run.clone(), pin: item.pin.clone(), candidate: item.candidate.clone() } };
    assert!(runtime.decide(&session, request, 100).unwrap().result.is_ok());
    assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done);
    let mut config = runtime.coding.clone().unwrap(); config.checks[0].argv.push("changed".into());
    let root = runtime.root.clone(); drop(runtime);
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    let runtime = Runtime::open(&root).unwrap();
    assert!(runtime.validate_definition(&state.items["one"].definition).is_err());
}

#[tokio::test]
async fn workflow_coding_native_launch_pins_existing_hooks_without_installing_or_launching() {
    let (dir, mut runtime) = configured(false, "pass").await;
    let settings = dir.path().join("existing-settings.json");
    std::fs::write(&settings, b"{\"hooks\":{}}").unwrap();
    let mut config = runtime.coding.clone().unwrap();
    config.harness = CodingHarness::Native { preset: "claude:opus".into(),
        settings: Some(LaunchFile { path: settings.canonicalize().unwrap(), digest: digest(&std::fs::read(&settings).unwrap()) }),
        project_hooks: None, plugin_dir: None, prompt: "implement the selected task".into(),
        process: CodingCommand { argv: vec![], environment: Default::default(), timeout_ms: 5000 } };
    config.definition().unwrap();
    runtime.coding = Some(config.clone());
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap(); // Only provision; never launch a real provider.
    let binding = admitted(&runtime).await;
    let command = runtime.coding_command(&binding).unwrap();
    assert_eq!(&command.argv[..2], ["/bin/sh", "-c"]);
    assert!(command.argv[2].contains("--settings"));
    assert!(command.argv[2].contains("--model opus"));
    assert!(command.argv[2].contains("implement the selected task"));
    std::fs::write(&settings, b"changed hook settings").unwrap();
    assert!(config.definition().is_err());
    assert!(!runtime.root.join("repo").exists());
}

#[tokio::test]
async fn workflow_coding_visible_git_without_process_settlement_remains_uncertain() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    let binding = admitted(&runtime).await;
    let branch = format!("wf-{}", &digest(&("w", "one"))[4..]);
    crate::git::create_worktree_with(&runtime.repository(), &branch, &binding.base, &runtime.tree(&binding), true).await.unwrap();
    // Simulate loss after Git accepted, before the owner recorded quiescence.
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap(); runtime.pump(200).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    assert_eq!(runtime.pump(201).await.unwrap(), 0);
}

#[tokio::test]
async fn workflow_coding_reviewer_rejection_has_bounded_rework() {
    let (_dir, mut runtime) = configured(true, "review-reject").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    let state = finish(&mut runtime, "w").await;
    assert_eq!(state.items["one"].phase, Phase::Failed);
    assert_eq!(state.items["one"].iteration, 2);
    assert_eq!(adapters::git(&runtime.repository(), &["rev-parse", runtime.landing_ref()]).await.unwrap(), runtime.base);
}

#[tokio::test]
async fn workflow_coding_cancel_unknown_provider_waits_for_original_acceptance_fact() {
    let (_dir, mut runtime) = configured(false, "ci-pending").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    for time in 2..15 { runtime.pump(time).await.unwrap(); }
    let original = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    runtime.submit(&runtime.controller(), Request { id: "cancel".into(), workspace: "w".into(), expected_revision: state.revision,
        sequence: 2, command: Command::Cancel { item: "one".into() } }, 100).unwrap();
    runtime.pump(101).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    let path = runtime.root.join("receipts").join(format!("{}.provider.json", digest(&original.effect)));
    let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    raw["run"]["status"] = "completed".into(); std::fs::write(path, serde_json::to_vec(&raw).unwrap()).unwrap();
    runtime.pump(102).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Cancelled);
    let stored = runtime.store.workflow_outbox("w").unwrap();
    assert_eq!(stored.iter().find(|s| s.effect.id == original.effect.id).unwrap().dispatch_state, "settled");
}
