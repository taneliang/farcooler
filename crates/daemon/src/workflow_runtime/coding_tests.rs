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

/// A mode whose stage exits 0 without a real result: Uncertain, and it stays so.
async fn not_success_is_uncertain(mode: &str) {
    let (_dir, mut runtime) = configured(false, mode).await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    for time in 2..15 { runtime.pump(time).await.unwrap(); }
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].phase, Phase::Uncertain, "{mode}: {state:?}");
    let before = state.clone();
    assert_eq!(runtime.pump(100).await.unwrap(), 0);
    assert_eq!(runtime.state("w").unwrap().unwrap(), before);
}
/// A mode whose build, check or CI says it failed: Failed.
async fn a_failure_fails(mode: &str) {
    let (_dir, mut runtime) = configured(false, mode).await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Failed, "{mode}");
}
/// A test per mode (ov-993): six runtimes in one test took 12 s.
mod workflow_coding_exit_prose_dirty_checks_and_ci_provenance_are_not_success {
    // A build that says it's done and commits nothing has answered: no candidate (ov-1090).
    #[tokio::test] async fn prose() { super::a_failure_fails("prose").await; }
    #[tokio::test] async fn mutate() { super::not_success_is_uncertain("mutate").await; }
    #[tokio::test] async fn ci_wrong() { super::not_success_is_uncertain("ci-wrong").await; }
    #[tokio::test] async fn ci_missing() { super::not_success_is_uncertain("ci-missing").await; }
    #[tokio::test] async fn check_fail() { super::a_failure_fails("check-fail").await; }
    #[tokio::test] async fn ci_fail() { super::a_failure_fails("ci-fail").await; }
}

pub(super) async fn admitted(runtime: &Runtime) -> Binding {
    let state = runtime.state("w").unwrap().unwrap();
    let stored = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap();
    let claim = runtime.store.workflow_claim_effect("w", &stored.effect.id, "crash-test", state.revision, state.epoch, 10, 100).unwrap().unwrap();
    let base = runtime.pin_base("w", &stored.effect).await.unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 10).unwrap());
    Binding::new("w", &stored.effect, claim.resource_generation, state.items[&stored.effect.item].iteration, &base)
}

/// A crash after stage `index` settled outside, before its journal: the
/// restart reconciles the fact and never runs the stage again.
async fn a_crash_after_stage(index: i64, review: bool) {
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
/// A test per stage (ov-993): seven runtimes in one test took 10 s.
mod workflow_coding_crash_each_stage_reconciles_facts_without_reexecution {
    #[tokio::test] async fn stage_0() { super::a_crash_after_stage(0, false).await; }
    #[tokio::test] async fn stage_1() { super::a_crash_after_stage(1, false).await; }
    #[tokio::test] async fn stage_2() { super::a_crash_after_stage(2, false).await; }
    #[tokio::test] async fn stage_3() { super::a_crash_after_stage(3, false).await; }
    #[tokio::test] async fn stage_4() { super::a_crash_after_stage(4, false).await; }
    #[tokio::test] async fn stage_5() { super::a_crash_after_stage(5, false).await; }
    #[tokio::test] async fn stage_4_with_review() { super::a_crash_after_stage(4, true).await; }
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
        sequence: 2, command: Command::Cancel { item: "one".into() , reason: None } }, 201).unwrap();
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
        expected_revision: state.revision, sequence: 2, command: Command::Cancel { item: "one".into() , reason: None } }, 200).await.unwrap();
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
    let tree = runtime.item_tree("w", "one");
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
        project_hooks: None, codex_hooks: None, plugin_dir: None, prompt: "implement the selected task".into(),
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
async fn workflow_coding_cancel_unknown_provider_completes_when_its_process_has_settled() {
    let (_dir, mut runtime) = configured(false, "ci-pending").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    for time in 2..15 { runtime.pump(time).await.unwrap(); }
    let original = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    runtime.submit(&runtime.controller(), Request { id: "cancel".into(), workspace: "w".into(), expected_revision: state.revision,
        sequence: 2, command: Command::Cancel { item: "one".into() , reason: None } }, 100).unwrap();
    runtime.pump(101).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Cancelled);
    let path = runtime.root.join("receipts").join(format!("{}.provider.json", digest(&original.effect)));
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(raw["run"]["status"], "in_progress", "cancellation does not claim the provider succeeded");
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
        project_hooks: None, codex_hooks: None, plugin_dir: None, prompt: "implement the selected task".into(),
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
        let claim = runtime.store.workflow_claim_effect("w", &stored.effect.id, "test", state.revision, state.epoch, 10, 100).unwrap().unwrap_or_else(|| panic!(
            "the build effect for {} could not be claimed. It is {:?}; the run is {:?}; the outbox is {:?}",
            stored.effect.item, stored.effect.kind, state.items[&stored.effect.item].phase, runtime.store.workflow_outbox("w").unwrap()
                .iter().map(|o| (o.effect.item.clone(), o.dispatch_state.clone(), o.active)).collect::<Vec<_>>()));
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
        project_hooks: Some(hooks), codex_hooks: None, plugin_dir: None, prompt: "implement the selected task".into(),
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
        project_hooks: None, codex_hooks: None, plugin_dir: None, prompt: "p".into(),
        process: CodingCommand { argv: vec![], environment: Default::default(), timeout_ms: 5000 } };
    assert!(listing(&config, &["claude:opus", "claude"]).is_ok());
    assert!(listing(&config, &["claude:opus", "codex:gpt-5.5"]).is_err(), "Codex's hooks aren't pinned");
    assert!(listing(&config, &["cursor"]).is_err(), "nor Cursor's");
}

/// ov-1088: Codex's hooks can be the runner's own file, outside the repository, as Claude's settings are. A
/// definition listing Codex models publishes on it with no `.codex/hooks.json` in the repository; the engine copies it
/// into the build's worktree before Codex starts, and puts it back if it changed. It never writes over a hooks file the
/// repository commits, and refuses a copy kept inside the repository, one beside a committed one, or one that reports
/// anywhere but this host.
#[tokio::test]
async fn workflow_coding_codex_runs_on_the_runners_hooks_file_with_none_in_the_repository() {
    let (dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap(); // Only provision; never launch a real provider.
    let binding = admitted(&runtime).await;
    let tree = runtime.tree(&binding);
    let pinned = |path: &Path, text: &str| { std::fs::create_dir_all(path.parent().unwrap()).unwrap(); std::fs::write(path, text).unwrap();
        LaunchFile { path: path.canonicalize().unwrap(), digest: digest(&text.as_bytes().to_vec()) } };
    let text = crate::hook_install::merge_codex("", &runtime.root.join("h.sock"));
    let hooks = pinned(&dir.path().join("engine-codex-hooks.json"), &text);
    let mut config = runtime.coding.clone().unwrap();
    let native = |project_hooks: Option<LaunchFile>, codex_hooks: Option<LaunchFile>| CodingHarness::Native { preset: "codex".into(),
        settings: None, project_hooks, codex_hooks, plugin_dir: None, prompt: "p".into(),
        process: CodingCommand { argv: vec![], environment: Default::default(), timeout_ms: 5000 } };
    config.harness = native(None, Some(hooks.clone()));
    let mut definition = config.definition().unwrap();
    definition.models.insert("build".into(), vec!["codex:gpt-6.1-sol".into(), "codex".into()]);
    config.validate_definition(&definition).expect("Codex models publish on the runner's hooks");
    runtime.validate_native_configuration(&config).expect("which report to this host");
    assert!(!config.repository.join(".codex").exists() && !tree.join(".codex").exists(), "no hooks file in the repository");

    // Before Codex starts, the engine writes the copy; the launch checks it and runs Codex.
    runtime.coding = Some(config.clone());
    assert!(runtime.coding_command(&binding).is_err(), "no copy yet, no launch");
    runtime.install_codex_hooks(&binding, &tree).await.unwrap();
    assert_eq!(std::fs::read_to_string(tree.join(".codex/hooks.json")).unwrap(), text);
    let command = runtime.coding_command(&binding).unwrap();
    assert!(command.argv[2].contains("codex") && !command.argv[2].contains("--settings"), "{}", command.argv[2]);
    assert!(runtime.tree_changes(&binding, &tree).await.unwrap().is_empty(), "the copy isn't the candidate's change");
    std::fs::write(tree.join(".codex/hooks.json"), "{}").unwrap();
    assert!(runtime.coding_command(&binding).is_err(), "a changed copy doesn't launch");
    runtime.install_codex_hooks(&binding, &tree).await.unwrap();
    assert_eq!(std::fs::read_to_string(tree.join(".codex/hooks.json")).unwrap(), text, "and is put back");

    // Kept in the repository, or beside the repository's own, it is refused; so is one reporting to another host.
    let committed = pinned(&config.repository.join(".codex/hooks.json"), &text);
    let refused = |harness| { let mut c = config.clone(); c.harness = harness; c.validate().unwrap_err() };
    assert!(matches!(refused(native(None, Some(committed.clone()))), Error::Refused("codex hooks inside the repository")));
    assert!(matches!(refused(native(Some(committed), Some(hooks.clone()))), Error::Refused("two codex hooks files")));
    let elsewhere = pinned(&dir.path().join("elsewhere.json"), &crate::hook_install::merge_codex("", Path::new("/tmp/other.sock")));
    let mut foreign = config.clone(); foreign.harness = native(None, Some(elsewhere));
    assert!(runtime.validate_native_configuration(&foreign).is_err(), "hooks that report elsewhere");

    // A worktree whose branch commits its own `.codex/hooks.json` is the repository's choice, and isn't written over.
    std::fs::write(tree.join(".codex/hooks.json"), "{\"hooks\":{}}").unwrap();
    adapters::git(&tree, &["add", "-f", "--", ".codex/hooks.json"]).await.unwrap();
    adapters::git(&tree, &["-c", "user.name=Test", "-c", "user.email=test@localhost", "-c", "commit.gpgsign=false",
        "commit", "--quiet", "-m", "its own hooks"]).await.unwrap();
    assert!(matches!(runtime.install_codex_hooks(&binding, &tree).await, Err(Error::Refused("the repository commits its own codex hooks"))));
    assert_eq!(std::fs::read_to_string(tree.join(".codex/hooks.json")).unwrap(), "{\"hooks\":{}}");
}

/// ov-1074 review L1: a tree made by Provision that someone changes before the step is read back again fails with the
/// sentence. The tree was made, so what happened is known, and a retry stops at the branch that exists.
#[tokio::test]
async fn workflow_coding_a_provisioned_tree_changed_before_its_reconcile_fails_the_step_with_the_sentence() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    let mut found = None;
    for time in 2..12 {
        runtime.pump(time).await.unwrap();
        let provision = runtime.store.workflow_outbox("w").unwrap().into_iter()
            .find(|s| s.effect.kind == EffectKind::Execute(StageKind::Provision));
        if let Some(stored) = provision {
            let binding = Binding::new("w", &stored.effect, stored.resource_generation.unwrap(), 0, &runtime.item_base("w", &stored.effect).unwrap());
            if runtime.git_settlement(&binding).exists() { found = Some(binding); break; }
        }
    }
    let binding = found.expect("Provision made its tree");
    std::fs::write(runtime.tree(&binding).join("mine.txt"), "someone's work").unwrap();
    let Outcome::SafeFailureBecause(said) = runtime.reconcile_coding(&binding).await.unwrap() else { panic!("not a sentence") };
    assert!(said.contains("mine.txt") && said.contains("Commit them"), "{said}");
}

/// ov-1087, on ov-1088's adapter shape: checks the Codex-models bundle the owner publishes, read from the file named by
/// `FARCOOLER_OV1087_BUNDLE` (`.claude/agent/reports/ov-1087/workflow-codex.json`). Run it by hand:
/// `FARCOOLER_OV1087_BUNDLE=<workflow-codex.json> cargo test -p farcooler-daemon --lib workflow_codex_bundle -- --ignored`.
/// First the file itself: it reads as `workspace set-workflow --file` reads it, every operation pins its configuration,
/// Claude's pinned hooks are the file on disk, and Codex's are the runner's own file beside them, with exactly the bytes
/// a daemon with ov-1088 writes there (checked against the file itself once it exists). No project hooks are pinned,
/// and nothing in the repository. Then the same models in a scratch runtime, whose host differs from the live one (so
/// its hooks report to its own socket): with no Codex hooks, as in the live adapter today, the definition is refused;
/// with the runner's Codex hooks file it publishes, a model the step doesn't list is refused, `build=codex:gpt-6.1-sol`
/// starts, and its build launches Codex with that model once the engine has copied the hooks into its worktree.
#[tokio::test]
#[ignore = "checks a draft file the owner has yet to publish; set FARCOOLER_OV1087_BUNDLE"]
async fn workflow_codex_bundle_publishes_and_starts_a_codex_build() {
    use crate::workflow_runtime::coding::FileValidation;
    use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::{AdapterConfiguration, CodingHarness, LaunchFile}};
    let (value, bundle) = ov1087_bundle();
    let models = bundle.definition.models.clone();
    assert_eq!(models["build"], ["claude:sonnet", "claude:opus", "claude:haiku", "codex:gpt-6.1-sol", "codex:gpt-6.1-terra", "codex:gpt-6.1-luna"]);
    assert_eq!(models["gate-0"], ["claude:opus", "claude:sonnet", "codex:gpt-6.1-sol", "codex:gpt-6.1-terra", "codex:gpt-6.1-luna"]);
    let AdapterConfiguration::Coding(live) = bundle.adapter.unwrap();
    let CodingHarness::Native { settings: Some(settings), project_hooks: None, codex_hooks: Some(codex), .. } = &live.harness
    else { panic!("native, with Claude's settings and Codex's hooks pinned, and no project hooks: {value}") };
    // Every operation, in stages and groups, pins this configuration, and Claude's hooks are the file on disk.
    let operations = bundle.definition.stages.iter().map(|s| &s.operation)
        .chain(bundle.definition.groups.values().flat_map(|g| g.steps.iter().map(|s| &s.operation)));
    assert!(operations.clone().count() > 5 && operations.into_iter().all(|o| o.arguments[0] == live.pin()), "operations pin the configuration");
    settings.verify().unwrap();
    // Codex's hooks are the runner's file beside Claude's, outside the repository, as the daemon writes it.
    assert_eq!(codex.path, settings.path.with_file_name("workflow-codex-hooks.json"));
    assert!(!codex.path.starts_with(&live.repository));
    assert_eq!(codex.digest, digest(&live_codex_hooks(&settings.path).into_bytes()), "the bytes a daemon with ov-1088 writes");
    if codex.path.exists() { codex.verify().unwrap(); }

    let (dir, mut runtime) = configured(false, "pass").await;
    let host = runtime.root.join("h.sock");
    let pinned = |path: std::path::PathBuf, text: String| { std::fs::create_dir_all(path.parent().unwrap()).unwrap(); std::fs::write(&path, &text).unwrap();
        LaunchFile { path: path.canonicalize().unwrap(), digest: farcooler_workflow::digest(&text.into_bytes()) } };
    let mut config = runtime.coding.clone().unwrap();
    config.harness = CodingHarness::Native { preset: "claude".into(), plugin_dir: None, prompt: "p".into(), process: live_process(&live),
        settings: Some(pinned(dir.path().join("native-settings.json"), serde_json::to_string(&crate::hook_install::claude_settings(&host)).unwrap())),
        project_hooks: None, codex_hooks: Some(pinned(dir.path().join("workflow-codex-hooks.json"), crate::hook_install::merge_codex("", &host))) };
    runtime.coding = Some(config.clone());
    let mut definition = runtime.configured_definition().unwrap();
    definition.models = models.into_iter().filter(|(step, _)| step == "build").collect(); // The scratch definition has no gate-0.
    let m = |id: &str, revision, sequence| Mutation { id: id.into(), revision, epoch: 1, sequence };
    let ok = |value: &serde_json::Value| assert!(value.pointer("/receipt/result/Ok").is_some(), "{value}");
    // As the live adapter stands, with no Codex hooks pinned, the runtime can't launch a Codex model, and refuses the definition.
    let mut bare = config.clone();
    if let CodingHarness::Native { codex_hooks, .. } = &mut bare.harness { *codex_hooks = None; }
    runtime.coding = Some(bare.clone());
    let mut bare_definition = runtime.configured_definition().unwrap();
    bare_definition.models = definition.models.clone();
    let refusal = runtime.wire_command("w", "owner", Action::Publish { notifications: None, adapter: Some(Box::new(AdapterConfiguration::Coding(bare))),
        mutation: m("publish-bare", 0, 1), expected_pin: None, definition: bare_definition });
    assert!(matches!(refusal, Err(Error::Refused("a listed model's harness isn't set up on this runtime"))), "{refusal:?}");
    runtime.coding = Some(config.clone());
    let adapter = Some(Box::new(AdapterConfiguration::Coding(config.clone())));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    ok(&runtime.wire_command("w", "owner", Action::Publish { notifications: None, adapter, mutation: m("publish", 0, 1), expected_pin: None, definition }).unwrap());
    assert!(!config.repository.join(".codex").exists(), "published with no hooks file in the repository");
    assert_eq!(runtime.track_tickets(&[("w".into(), "ov-7".into(), "backlog".into())]).unwrap(), 1);
    let state = runtime.state("w").unwrap().unwrap(); let held = &state.items["ov-7"];
    let start = |model: &str, sequence| runtime.wire_command("w", "orchestrator", Action::Start { mutation: m(&format!("start-{sequence}"), state.revision, sequence), item: "ov-7".into(),
        run: held.run.clone(), pin: held.pin.clone(), start_pin: pin.clone(), models: [("build".to_string(), model.to_string())].into(), worktree: None, with: vec![] }).unwrap();
    let refused = start("codex:gpt-4", 2);
    assert_eq!(refused.pointer("/receipt/result/Err").and_then(|e| e.as_str()), Some("Invalid"), "a model the step doesn't list: {refused}");
    ok(&start("codex:gpt-6.1-sol", 3));
    assert_eq!(runtime.state("w").unwrap().unwrap().items["ov-7"].models["build"], "codex:gpt-6.1-sol");
    // Provision, then the build: the engine copies the hooks into the worktree, and Codex launches with the model.
    runtime.pump(2).await.unwrap(); // Only provision; never launch a real provider.
    let binding = admitted(&runtime).await;
    assert_eq!(binding.effect.model.as_deref(), Some("codex:gpt-6.1-sol"));
    let tree = runtime.tree(&binding);
    runtime.install_codex_hooks(&binding, &tree).await.unwrap();
    assert_eq!(std::fs::read_to_string(tree.join(".codex/hooks.json")).unwrap(), crate::hook_install::merge_codex("", &host));
    let launch = runtime.coding_command(&binding).unwrap().argv[2].clone();
    assert!(launch.contains("codex") && launch.contains("--model gpt-6.1-sol"), "{launch}");
}
/// The bundle at `FARCOOLER_OV1087_BUNDLE`, read as `workspace set-workflow --file` reads it, with its JSON.
fn ov1087_bundle() -> (serde_json::Value, Ov1087Bundle) {
    let path = std::env::var("FARCOOLER_OV1087_BUNDLE").expect("FARCOOLER_OV1087_BUNDLE names the bundle file");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let bundle: Ov1087Bundle = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&bundle).unwrap(), value, "the CLI refuses a bundle that doesn't round-trip");
    (value, bundle)
}
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Ov1087Bundle { definition: farcooler_protocol::workflow_commands::Definition, adapter: Option<farcooler_protocol::workflow_config::AdapterConfiguration> }
/// What a daemon with ov-1088 writes as `workflow-codex-hooks.json` beside Claude's engine settings at `claude`
/// (`workflow_starter::engine_codex_hooks`): Codex's hooks reporting to the engine's socket, run by the same
/// `farcooler` Claude's are. This test's own binary stands in for it, so its path is swapped for that one.
fn live_codex_hooks(claude: &Path) -> String {
    let binary = |text: &str| { let at = text.find(" hook --agent ").unwrap(); let start = text[..at].rfind("\"command\": \"").unwrap() + 12; text[start..at].to_string() };
    let ours = crate::hook_install::merge_codex("", &claude.parent().unwrap().join("workflow/h.sock"));
    let theirs = serde_json::to_string_pretty(&serde_json::from_slice::<serde_json::Value>(&std::fs::read(claude).unwrap()).unwrap()).unwrap();
    ours.replace(&binary(&ours), &binary(&theirs))
}
fn live_process(live: &farcooler_protocol::workflow_config::CodingConfig) -> farcooler_protocol::workflow_config::CodingCommand {
    match &live.harness { farcooler_protocol::workflow_config::CodingHarness::Native { process, .. } => process.clone(), _ => unreachable!() }
}

// ov-1122: use the same ticket through cancel/reopen/start, as the app does.
fn rerun_command(runtime: &Runtime, workspace: &str, command: Command) {
    let state = runtime.state(workspace).unwrap();
    let sequence = state.as_ref().and_then(|s| s.source_sequences.get("controller").copied()).unwrap_or(0) + 1;
    let receipt = runtime.apply(Request { id: format!("rerun-{sequence}"), workspace: workspace.into(),
        expected_revision: state.map_or(0, |s| s.revision), sequence, command }, "controller", Grant::Controller, 1).unwrap();
    assert!(receipt.result.is_ok(), "{:?}", receipt.result);
}
async fn rerun_fixture() -> (tempfile::TempDir, Runtime, String) {
    let (dir, mut runtime) = configured(false, "pass").await;
    let host = Uuid::now_v7();
    let repo = runtime.repository();
    let root = runtime.store.create_repository_root(host, repo.to_str().unwrap(), 0).unwrap();
    let repository = runtime.store.create_repository(host, root.id, "rerun", repo.join(".git").to_str().unwrap(), "").unwrap();
    let workspace = runtime.store.create_workspace(repository.id, "Rerun", "rr").unwrap().id.to_string();
    runtime.host = Some(WorktreeHost { dir: dir.path().join("trees"), install_id: "test".into(), locks: Default::default() });
    rerun_command(&runtime, &workspace, Command::Enqueue { item: "ticket".into(), definition: Definition::ticket(),
        policy_pin: "p".into(), resource: Exec::item_resource(&workspace, "ticket"), dependencies: BTreeSet::new(),
        initial_candidate: Some("ticket".into()) });
    (dir, runtime, workspace)
}
fn rerun_start(runtime: &Runtime, workspace: &str, tree: RunTree) {
    rerun_command(runtime, workspace, Command::Start { item: "ticket".into(), definition: runtime.configured_definition().unwrap(),
        policy_pin: "p".into(), resource: Exec::item_resource(workspace, "ticket"), dependencies: BTreeSet::new(),
        initial_candidate: None, models: Default::default(), tree: Some(Box::new(tree)), with: vec![] });
}
fn rerun_tree(dir: &Path, name: &str, base: &str) -> RunTree {
    RunTree { name: name.into(), branch: name.into(), path: dir.join(name).to_string_lossy().into_owned(), base: base.into(), adopted: false }
}
#[tokio::test]
async fn workflow_coding_second_run_of_ticket_provisions_its_own_base() {
    let (dir, mut runtime, workspace) = rerun_fixture().await;
    let base = runtime.base.clone();
    rerun_start(&runtime, &workspace, rerun_tree(dir.path(), "first", &base));
    runtime.pump(2).await.unwrap();
    assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["ticket"].stage, 1);
    rerun_command(&runtime, &workspace, Command::Cancel { item: "ticket".into(), reason: None });
    runtime.pump(3).await.unwrap();
    assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["ticket"].phase, Phase::Cancelled);
    rerun_command(&runtime, &workspace, Command::Reopen { item: "ticket".into(), definition: Definition::ticket(),
        policy_pin: "p".into(), resource: Exec::item_resource(&workspace, "ticket") });
    adapters::git(&runtime.repository(), &["-c", "user.name=Test", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false",
        "commit", "--allow-empty", "-m", "new main"]).await.unwrap();
    let base = adapters::git(&runtime.repository(), &["rev-parse", "HEAD"]).await.unwrap();
    rerun_start(&runtime, &workspace, rerun_tree(dir.path(), "second", &base));
    runtime.pump(4).await.unwrap();
    let state = runtime.state(&workspace).unwrap().unwrap();
    assert_eq!(state.items["ticket"].stage, 1, "second Provision: {:?}", state.items["ticket"]);
    assert_eq!(adapters::git(&dir.path().join("second"), &["rev-parse", "HEAD"]).await.unwrap(), base);
}

async fn mismatched_provision() -> (tempfile::TempDir, Runtime, String, Binding) {
    let (dir, runtime, workspace) = rerun_fixture().await;
    let old = runtime.base.clone();
    adapters::git(&runtime.repository(), &["-c", "user.name=Test", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false",
        "commit", "--allow-empty", "-m", "new base"]).await.unwrap();
    let base = adapters::git(&runtime.repository(), &["rev-parse", "HEAD"]).await.unwrap();
    rerun_start(&runtime, &workspace, rerun_tree(dir.path(), "mismatch", &base));
    // Exactly the pre-update state: a legacy item pin, and an admitted Provision of the next run.
    durable(&runtime.root.join(format!("base-{}.json", digest(&(&workspace, "ticket")))), &serde_json::to_vec(&old).unwrap()).unwrap();
    let state = runtime.state(&workspace).unwrap().unwrap();
    let stored = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap();
    let claim = runtime.store.workflow_claim_effect(&workspace, &stored.effect.id, "old-daemon", state.revision, state.epoch, 2, 10).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 2).unwrap());
    let binding = Binding::new(&workspace, &stored.effect, claim.resource_generation, 0, &old);
    (dir, runtime, workspace, binding)
}
#[tokio::test]
async fn workflow_coding_base_mismatch_writes_unstarted_and_fails_safely() {
    let (dir, runtime, workspace, binding) = mismatched_provision().await;
    let (_, cancel) = tokio::sync::watch::channel(false);
    let outcome = runtime.execute_coding(&workspace, &binding, cancel).await.unwrap();
    assert!(matches!(outcome, Outcome::SafeFailureBecause(_)), "{outcome:?}");
    assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(), outcome);
    let path = runtime.root.join("receipts").join(format!("{}.provision-unstarted.json", digest(&binding.effect)));
    assert!(path.exists(), "durable Unstarted receipt");
    assert!(!dir.path().join("mismatch").exists());
}
#[tokio::test]
async fn workflow_coding_legacy_uncertain_provision_recovers_without_execution() {
    let (dir, runtime, workspace, binding) = mismatched_provision().await;
    runtime.observe(&workspace, &binding.effect, Outcome::Uncertain, false, 3).unwrap();
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(4).await.unwrap();
    assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["ticket"].phase, Phase::Failed);
    assert!(!dir.path().join("mismatch").exists());
    rerun_command(&runtime, &workspace, Command::Cancel { item: "ticket".into(), reason: None });
    assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["ticket"].phase, Phase::Cancelled);
    rerun_command(&runtime, &workspace, Command::Reopen { item: "ticket".into(), definition: Definition::ticket(),
        policy_pin: "p".into(), resource: Exec::item_resource(&workspace, "ticket") });
    assert!(runtime.state(&workspace).unwrap().unwrap().items["ticket"].definition.is_ticket());
}
#[tokio::test]
async fn workflow_coding_legacy_uncertain_stop_recovers_to_cancelled() {
    let (dir, runtime, workspace, binding) = mismatched_provision().await;
    runtime.observe(&workspace, &binding.effect, Outcome::Uncertain, false, 3).unwrap();
    rerun_command(&runtime, &workspace, Command::Cancel { item: "ticket".into(), reason: None });
    let state = runtime.state(&workspace).unwrap().unwrap();
    let stop = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap();
    let claim = runtime.store.workflow_claim_effect(&workspace, &stop.effect.id, "old-stop", state.revision, state.epoch, 4, 10).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 4).unwrap());
    runtime.observe(&workspace, &stop.effect, Outcome::Uncertain, false, 5).unwrap();
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(6).await.unwrap();
    assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["ticket"].phase, Phase::Cancelled);
    assert!(!dir.path().join("mismatch").exists());
}

#[tokio::test]
async fn workflow_coding_stop_uncertain_answer_with_settled_process_completes() {
    let (_dir, mut runtime) = configured(false, "ci-wrong").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    for time in 2..15 { runtime.pump(time).await.unwrap(); }
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    rerun_command(&runtime, "w", Command::Cancel { item: "one".into(), reason: None });
    runtime.pump(20).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Cancelled);
}

#[tokio::test]
async fn workflow_coding_legacy_admitted_base_and_receipts_keep_working() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let provision = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .find(|s| matches!(s.effect.kind, EffectKind::Execute(StageKind::Provision))).unwrap().effect;
    let legacy = runtime.root.join(format!("base-{}.json", digest(&("w", &provision.item))));
    std::fs::rename(runtime.base_path("w", &provision).unwrap(), &legacy).unwrap();
    let bytes = std::fs::read(&legacy).unwrap();
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done);
    assert_eq!(std::fs::read(legacy).unwrap(), bytes);
    assert!(!runtime.base_path("w", &provision).unwrap().exists(), "never rebind an admitted legacy run");
}

/// ov-1074: cancelling a completed Provision needs proof that its Git operation ended, not a tree
/// that still passes the candidate checks. A branch moved after provisioning must not trap Cancel.
#[tokio::test]
async fn workflow_coding_cancel_uncertain_provision_uses_quiescence_not_candidate_validity() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    let binding = admitted(&runtime).await;
    let (_, cancel) = tokio::sync::watch::channel(false);
    assert!(matches!(runtime.execute_coding("w", &binding, cancel).await.unwrap(), Outcome::Passed(_)));
    let tree = runtime.tree(&binding);
    adapters::git(&tree, &["checkout", "-q", "-b", "moved-by-person"]).await.unwrap();
    std::fs::write(runtime.repository().join(".git/info/exclude"), "mine.txt\n").unwrap();
    std::fs::write(tree.join("mine.txt"), "keep my work").unwrap();
    assert!(runtime.observe("w", &binding.effect, Outcome::Uncertain, false, 20).unwrap());
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert!(runtime.submit(&runtime.controller(), Request { id: "cancel-provision".into(), workspace: "w".into(),
        expected_revision: state.revision, sequence: 2,
        command: Command::Cancel { item: "one".into(), reason: None } }, 21).unwrap().result.is_ok());
    runtime.pump(22).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Cancelled);
    assert_eq!(std::fs::read_to_string(tree.join("mine.txt")).unwrap(), "keep my work");
    assert_eq!(adapters::git(&tree, &["symbolic-ref", "--short", "HEAD"]).await.unwrap(), "moved-by-person");
    uncertain_provision_without_quiescence_stays_uncertain().await;
}

/// Missing or mismatched settlement means no proof the old Provision stopped. Preserve uncertainty.
async fn uncertain_provision_without_quiescence_stays_uncertain() {
    for foreign in [false, true] {
        let (_dir, mut runtime) = configured(false, "pass").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        let binding = admitted(&runtime).await;
        if foreign {
            let mut other = binding.clone(); other.generation += 1;
            durable(&runtime.git_settlement(&binding), &serde_json::to_vec(&serde_json::json!({ "binding": other })).unwrap()).unwrap();
        }
        assert!(runtime.observe("w", &binding.effect, Outcome::Uncertain, false, 20).unwrap());
        let state = runtime.state("w").unwrap().unwrap();
        assert!(runtime.submit(&runtime.controller(), Request { id: "cancel-provision".into(), workspace: "w".into(),
            expected_revision: state.revision, sequence: 2,
            command: Command::Cancel { item: "one".into(), reason: None } }, 21).unwrap().result.is_ok());
        runtime.pump(22).await.unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
        assert!(!runtime.tree(&binding).exists(), "recovery must never run Provision again");
    }
}
