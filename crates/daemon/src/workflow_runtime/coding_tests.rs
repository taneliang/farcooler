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
        landing_ref: "refs/heads/workflow-target".into(), landing: None, pull_request: None, harness: CodingHarness::Command(build),
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

#[tokio::test]
async fn workflow_coding_adapted_person_review_and_optional_archetype() {
    let (_dir,runtime)=configured(true,"pass").await;
    let config=runtime.coding.as_ref().unwrap();let mut definition=config.definition().unwrap();
    definition.source="adapted-local-policy/v1".into();
    for stage in &mut definition.stages {
        stage.id=format!("adapted-{}",stage.id);
        if matches!(stage.kind,StageKind::Review(_)) {stage.kind=StageKind::Review(Reviewer::Person("owner".into()));}
    }
    definition.rework_target=Some("adapted-build".into());
    assert!(config.validate_definition(&definition).is_ok());
    let preflight=definition.stages.remove(2);
    definition.stages.insert(1,preflight);
    assert!(config.validate_definition(&definition).is_err(),"approved pure engine requires a built candidate before Check");
    definition.stages[0].kind=StageKind::Land;
    assert!(config.validate_definition(&definition).is_err(),"landing without candidate prerequisites accepted");
}

/// ov-962 B2: a ticket started through the engine reaches its agent by name, in the prompt and in the
/// environment, because the published prompt is one text for every ticket on the board.
#[tokio::test]
async fn workflow_coding_a_started_ticket_tells_its_agent_which_ticket_it_is() {
    use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::AdapterConfiguration};
    let (dir, mut runtime) = configured(false, "pass").await;
    let settings = dir.path().join("existing-settings.json");
    std::fs::write(&settings, b"{\"hooks\":{}}").unwrap();
    let published = runtime.coding.clone().unwrap();
    let mut config = published.clone();
    config.harness = CodingHarness::Native { preset: "claude:opus".into(),
        settings: Some(LaunchFile { path: settings.canonicalize().unwrap(), digest: digest(&std::fs::read(&settings).unwrap()) }),
        project_hooks: None, plugin_dir: None, prompt: "implement the selected task".into(),
        process: CodingCommand { argv: vec![], environment: Default::default(), timeout_ms: 5000 } };
    let definition = published.definition().unwrap();
    let m = |id: &str, revision, sequence| Mutation { id: id.into(), revision, epoch: 1, sequence };
    let wire = |runtime: &Runtime, principal, action| runtime.wire_command("w", principal, action).unwrap();
    let ok = |value: &serde_json::Value| assert!(value.pointer("/receipt/result/Ok").is_some(), "{value}");
    let adapter = Some(Box::new(AdapterConfiguration::Coding(published.clone())));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    ok(&wire(&runtime, "owner", Action::Publish { notifications: None, adapter, mutation: m("publish", 0, 1), expected_pin: None, definition }));
    // What the daemon's tracking does for a card: the ticket template, no run.
    assert_eq!(runtime.track_tickets(&[("w".into(), "ov-7".into(), "backlog".into())]).unwrap(), 1);
    let state = runtime.state("w").unwrap().unwrap(); let item = &state.items["ov-7"];
    ok(&wire(&runtime, "orchestrator", Action::Start { mutation: m("start", state.revision, 2), item: "ov-7".into(),
        run: item.run.clone(), pin: item.pin.clone(), start_pin: pin, models: Default::default(), worktree: None, with: vec![] }));
    runtime.pump(2).await.unwrap(); // Only provision; never launch a real provider.
    let binding = admitted(&runtime).await;
    // A command harness is told by the environment alone.
    let command = runtime.coding_command(&binding).unwrap();
    assert_eq!(command.environment["FARCOOLER_WORKFLOW_ITEM"], "ov-7");
    // A native one is told in its prompt as well. (A native publication needs live hooks; the launch is
    // built from the same item, so the harness is swapped after Start.)
    runtime.coding = Some(config);
    let command = runtime.coding_command(&binding).unwrap();
    assert!(command.argv[2].contains("ticket ov-7"), "the prompt names the ticket: {}", command.argv[2]);
    assert!(command.argv[2].contains("implement the selected task"), "and keeps the published prompt");
    assert_eq!(command.environment["FARCOOLER_WORKFLOW_ITEM"], "ov-7");
    assert_eq!(command.environment["FARCOOLER_WORKSPACE"], "w");
    // A recovery rebuilds the same command.
    assert_eq!(runtime.coding_command(&binding).unwrap(), command);
}

/// ov-966: the orchestrator starts two tickets on one workflow with different models. Each run records its
/// model, and each build launches with it: a command harness is told in its environment, a native one runs
/// that harness with `--model`, so Claude and Codex runs mix on one board.
#[tokio::test]
async fn workflow_coding_two_tickets_start_with_different_models_and_each_build_launches_its_own() {
    use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::AdapterConfiguration};
    let (dir, mut runtime) = configured(false, "pass").await;
    let published = runtime.coding.clone().unwrap();
    let mut definition = published.definition().unwrap();
    definition.models.insert("build".into(), vec!["claude:opus".into(), "codex:gpt-5.5".into()]);
    let m = |id: &str, revision, sequence| Mutation { id: id.into(), revision, epoch: 1, sequence };
    let wire = |runtime: &Runtime, principal, action| runtime.wire_command("w", principal, action).unwrap();
    let ok = |value: &serde_json::Value| assert!(value.pointer("/receipt/result/Ok").is_some(), "{value}");
    let adapter = Some(Box::new(AdapterConfiguration::Coding(published.clone())));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    ok(&wire(&runtime, "owner", Action::Publish { notifications: None, adapter, mutation: m("publish", 0, 1), expected_pin: None, definition }));
    assert_eq!(runtime.track_tickets(&[("w".into(), "ov-7".into(), "backlog".into()), ("w".into(), "ov-8".into(), "backlog".into())]).unwrap(), 2);
    let start = |runtime: &Runtime, item: &str, models: &[(&str, &str)], sequence| {
        let state = runtime.state("w").unwrap().unwrap(); let held = &state.items[item];
        wire(runtime, "orchestrator", Action::Start { mutation: m(&format!("start-{item}-{sequence}"), state.revision, sequence), item: item.into(),
            run: held.run.clone(), pin: held.pin.clone(), start_pin: pin.clone(),
            models: models.iter().map(|(s, m)| (s.to_string(), m.to_string())).collect(), worktree: None, with: vec![] })
    };
    let refused = start(&runtime, "ov-7", &[("build", "codex:gpt-4")], 2);
    assert_eq!(refused.pointer("/receipt/result/Err").and_then(|e| e.as_str()), Some("Invalid"), "a model the step doesn't list: {refused}");
    ok(&start(&runtime, "ov-7", &[("build", "codex:gpt-5.5")], 3));
    ok(&start(&runtime, "ov-8", &[], 4));
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["ov-7"].models["build"], "codex:gpt-5.5");
    assert_eq!(state.items["ov-8"].models["build"], "claude:opus", "no pick, the step's default");
    runtime.pump(2).await.unwrap(); // Only provision; never launch a real provider.
    let mut builds = std::collections::BTreeMap::new();
    for stored in runtime.store.workflow_outbox("w").unwrap().into_iter().filter(|s| s.active) {
        let state = runtime.state("w").unwrap().unwrap();
        let claim = runtime.store.workflow_claim_effect("w", &stored.effect.id, "test", state.revision, state.epoch, 10, 100).unwrap().unwrap();
        let base = runtime.pin_base("w", &stored.effect).await.unwrap();
        assert!(runtime.store.workflow_admit_effect(&claim, 10).unwrap());
        builds.insert(stored.effect.item.clone(), Binding::new("w", &stored.effect, claim.resource_generation, 0, &base));
    }
    assert_eq!(builds.len(), 2, "both builds are ready");
    assert_eq!(builds["ov-7"].effect.model.as_deref(), Some("codex:gpt-5.5"));
    assert_eq!(runtime.coding_command(&builds["ov-7"]).unwrap().environment["FARCOOLER_WORKFLOW_MODEL"], "codex:gpt-5.5");
    assert_eq!(runtime.coding_command(&builds["ov-8"]).unwrap().environment["FARCOOLER_WORKFLOW_MODEL"], "claude:opus");
    // A native harness with Claude's settings and Codex's project hooks both pinned runs either. (A native
    // publication needs live hooks; the launch is built from the same effect, so the harness is swapped after Start.)
    let pinned = |path: &Path| { std::fs::write(path, b"{\"hooks\":{}}").unwrap();
        LaunchFile { path: path.canonicalize().unwrap(), digest: digest(&std::fs::read(path).unwrap()) } };
    std::fs::create_dir(published.repository.join(".codex")).unwrap();
    let hooks = pinned(&published.repository.join(".codex/hooks.json"));
    std::fs::create_dir(runtime.tree(&builds["ov-7"]).join(".codex")).unwrap();
    std::fs::write(runtime.tree(&builds["ov-7"]).join(".codex/hooks.json"), b"{\"hooks\":{}}").unwrap();
    let mut native = published.clone();
    native.harness = CodingHarness::Native { preset: "claude:sonnet".into(), settings: Some(pinned(&dir.path().join("settings.json"))),
        project_hooks: Some(hooks), plugin_dir: None, prompt: "implement the selected task".into(),
        process: CodingCommand { argv: vec![], environment: Default::default(), timeout_ms: 5000 } };
    runtime.coding = Some(native);
    let codex = runtime.coding_command(&builds["ov-7"]).unwrap().argv[2].clone();
    let claude = runtime.coding_command(&builds["ov-8"]).unwrap().argv[2].clone();
    assert!(codex.contains("codex") && codex.contains("--model gpt-5.5") && !codex.contains("sonnet"), "{codex}");
    assert!(claude.contains("claude") && claude.contains("--model opus") && !claude.contains("sonnet"), "{claude}");
    assert_eq!(runtime.native_agent(&builds["ov-7"]).unwrap(), farcooler_agent_hooks::Agent::Codex, "its turn is read from Codex's hooks");
}

/// ov-966: a workflow can't list a model the runtime can't launch: an unknown harness, or one whose hooks
/// aren't pinned, never reaches a run.
#[tokio::test]
async fn workflow_coding_publish_refuses_a_model_whose_harness_is_not_set_up() {
    let (dir, runtime) = configured(false, "pass").await;
    let mut config = runtime.coding.clone().unwrap();
    let listing = |config: &CodingConfig, models: &[&str]| {
        let mut definition = config.definition().unwrap();
        definition.models.insert("build".into(), models.iter().map(|m| m.to_string()).collect());
        config.validate_definition(&definition)
    };
    assert!(listing(&config, &["claude:opus", "codex:gpt-5.5"]).is_ok(), "a command harness takes the model as a variable");
    assert!(listing(&config, &["gemini:pro"]).is_err(), "a harness Far Cooler can't launch");
    let settings = dir.path().join("settings.json"); std::fs::write(&settings, b"{\"hooks\":{}}").unwrap();
    config.harness = CodingHarness::Native { preset: "claude:sonnet".into(),
        settings: Some(LaunchFile { path: settings.canonicalize().unwrap(), digest: digest(&std::fs::read(&settings).unwrap()) }),
        project_hooks: None, plugin_dir: None, prompt: "p".into(),
        process: CodingCommand { argv: vec![], environment: Default::default(), timeout_ms: 5000 } };
    assert!(listing(&config, &["claude:opus", "claude"]).is_ok());
    assert!(listing(&config, &["claude:opus", "codex:gpt-5.5"]).is_err(), "Codex's hooks aren't pinned");
    assert!(listing(&config, &["cursor"]).is_err(), "nor Cursor's");
}
