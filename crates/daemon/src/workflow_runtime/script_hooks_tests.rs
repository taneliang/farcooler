use super::*;

/// For a rule a test expects to answer. The deadline runs in real time and starts before the rule's
/// checkout is made, so on a busy Mac a short one expires before the script can write its receipt.
const ANSWERS_MS: u64 = 60_000;
/// For the tests about expiry.
const EXPIRES_MS: u64 = 200;

fn script(dir: &Path, body: &str, timeout_ms: u64) -> ScriptPolicy {
    let path = dir.join("rule.sh");
    let bytes = format!("#!/bin/sh\n{body}\n").into_bytes();
    std::fs::write(&path, &bytes).unwrap();
    ScriptPolicy { point: HookPoint::RunFails, script: path.to_str().unwrap().into(), digest: digest(&bytes),
        timeout_ms, max_calls: 1, fallback: HookMove::Leave }
}
/// The fixture clock is explicit: neither firing, retry nor replay reads wall time.
async fn failed(body: &str) -> (tempfile::TempDir, Runtime) {
    failed_within(body, ANSWERS_MS).await
}
async fn failed_within(body: &str, timeout_ms: u64) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let policy = script(dir.path(), body, timeout_ms);
    let mut definition = coding_definition(false);
    definition.stages.truncate(1); definition.stages[0].max_attempts = 1;
    definition.script_policies.insert("triage".into(), policy);
    super::tests::enqueue(&mut runtime, "w", "coding", definition);
    let effect = runtime.store.workflow_outbox("w").unwrap()[0].effect.clone();
    let state = runtime.state("w").unwrap().unwrap();
    let claim = runtime.store.workflow_claim_effect("w", &effect.id, "fixture", state.revision, state.epoch, 2, 20).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 2).unwrap());
    assert!(runtime.observe("w", &effect, Outcome::SafeFailure, false, 10).unwrap());
    assert!(runtime.state("w").unwrap().unwrap().items["coding"].deciding.is_some());
    (dir, runtime)
}
async fn wait_for_decision(runtime: &mut Runtime, now_ms: i64) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        runtime.pump(now_ms).await.unwrap();
        if runtime.state("w").unwrap().unwrap().items["coding"].deciding.is_none() { return; }
        assert!(std::time::Instant::now() < deadline, "hook decision did not settle within 60 seconds");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
fn replay(runtime: &Runtime) {
    let mut state = State::empty("w".into(), 1);
    for event in runtime.events("w").unwrap() { state = step(&state, &event).unwrap().state; }
    assert_eq!(Some(state), runtime.state("w").unwrap());
}
fn response(decision: &str) -> String {
    format!("call=$(/usr/bin/sed -n 's/.*\"call\":\"\\([^\"]*\\)\".*/\\1/p'); printf '{{\"schema\":\"hook-decision/v1\",\"call\":\"%s\",\"decision\":\"{decision}\"}}' \"$call\" > \"$FARCOOLER_RECEIPT\"")
}
#[tokio::test]
async fn script_hooks_good_bad_and_illegal_decisions_are_logged_and_replayed() {
    for (body, good) in [(response("retry"), true), ("printf 'broken' > \"$FARCOOLER_RECEIPT\"".into(), false),
        (response("retry").replace("\"decision\":", "\"decision\":\"leave\",\"decision\":"), false),
        (response("accept"), false), (response("resolve"), false)] {
        let (_dir, mut runtime) = failed(&body).await;
        // With no build slots a decision still runs. A good retry queues its build behind the cap.
        runtime.places().agent = 0;
        wait_for_decision(&mut runtime, 20).await;
        let state = runtime.state("w").unwrap().unwrap();
        let item = &state.items["coding"];
        assert!(item.deciding.is_none());
        assert_eq!(item.retries, u8::from(good));
        assert_eq!(item.hook_refused.is_none(), good);
        assert!(runtime.events("w").unwrap().iter().any(|e| matches!(e.command, Command::Decide { .. })));
        replay(&runtime);
    }
}
#[tokio::test]
async fn script_hooks_timeout_and_no_receipt_retry_once_then_fall_back() {
    for (body, timeout_ms) in [("exit 1", ANSWERS_MS), ("/bin/sleep 2", EXPIRES_MS)] {
        let (_dir, mut runtime) = failed_within(body, timeout_ms).await;
        runtime.pump(20).await.unwrap();
        wait_for_decision(&mut runtime, 30).await;
        let events = runtime.events("w").unwrap();
        let answers: Vec<_> = events.iter().filter_map(|e| match &e.command { Command::Decide { result, .. } => Some(result), _ => None }).collect();
        assert_eq!(answers.len(), 2, "{answers:?}");
        assert!(answers.iter().all(|a| if body == "exit 1" { **a == HookResult::Missing } else { **a == HookResult::Timeout }));
        let state = runtime.state("w").unwrap().unwrap();
        assert!(state.items["coding"].deciding.is_none());
        assert!(state.items["coding"].failure.as_ref().unwrap().contains("fallback"));
        replay(&runtime);
    }
}
#[tokio::test]
async fn script_hooks_restart_mid_call_recovers_receipt_or_spends_one_retry() {
    for receipt in [false, true] {
        let (dir, mut runtime) = failed(&response("leave")).await;
        let state = runtime.state("w").unwrap().unwrap();
        let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
        let claim = runtime.store.workflow_claim_effect("w", &effect.id, "test", state.revision, state.epoch, 20, 30).unwrap().unwrap();
        assert_eq!(claim.resource_generation, 0);
        assert!(runtime.store.workflow_admit_effect(&claim, 20).unwrap());
        let binding = Binding::new("w", &effect, 0, 0, &runtime.base);
        if receipt {
            runtime.exec.launched = Some((20, std::time::Instant::now()));
            let outcome = runtime.execute_hook(&binding, None).await.unwrap();
            assert!(matches!(outcome, Outcome::Hook(HookResult::Receipt { .. })), "{outcome:?}");
        }
        if receipt {
            // Recovery consumes the pinned answer, even if the published source is no longer there.
            std::fs::remove_file(dir.path().join("rule.sh")).unwrap();
        }
        drop(runtime);
        let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
        runtime.pump(40).await.unwrap();
        wait_for_decision(&mut runtime, 50).await;
        let state = runtime.state("w").unwrap().unwrap();
        assert!(state.items["coding"].deciding.is_none());
        assert_eq!(state.items["coding"].phase, Phase::Failed);
        let count = runtime.events("w").unwrap().iter().filter(|e| matches!(e.command, Command::Decide { .. })).count();
        assert_eq!(count, if receipt { 1 } else { 2 });
        replay(&runtime);
    }
}

#[tokio::test]
async fn script_hooks_changed_script_falls_back_and_retry_budget_remains_authoritative() {
    let (dir, mut runtime) = failed(&response("retry")).await;
    let mut state = runtime.state("w").unwrap().unwrap();
    state.items.get_mut("coding").unwrap().retries = MAX_RETRIES;
    let item = &state.items["coding"];
    let call = item.deciding.as_ref().unwrap().input.call.clone();
    let input = Input { schema: 1, id: "exhausted".into(), workspace: "w".into(), expected_revision: state.revision,
        epoch: state.epoch, source: "test".into(), source_sequence: 1, now_ms: 20, busy_resources: BTreeSet::new(),
        grant: Grant::Operation { effect: item.pending.clone().unwrap(), run: item.run.clone(), fence: item.fence },
        command: Command::Decide { item: "coding".into(), call: call.clone(), fence: item.fence, policy_pin: item.policy_pin.clone(),
            result: HookResult::Receipt { manifest: "m".into(), raw: serde_json::json!({"schema":"hook-decision/v1","call":call,"decision":"retry"}).to_string() } } };
    let refused = step(&state, &input).unwrap().state;
    assert_eq!(refused.items["coding"].retries, MAX_RETRIES);
    assert!(refused.items["coding"].hook_refused.is_some(), "current legality overrides the firing snapshot");
    std::fs::write(dir.path().join("rule.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
    wait_for_decision(&mut runtime, 20).await;
    assert_eq!(runtime.state("w").unwrap().unwrap().items["coding"].retries, 0);
    assert!(runtime.state("w").unwrap().unwrap().items["coding"].hook_refused.is_some());
    replay(&runtime);

    let (_dir, mut runtime) = failed(&response("retry")).await;
    runtime.places().agent = 0;
    wait_for_decision(&mut runtime, 20).await;
    let state = runtime.state("w").unwrap().unwrap();
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect("w", &effect.id, "fixture", state.revision, state.epoch, 30, 40).unwrap().unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 30).unwrap());
    assert!(runtime.observe("w", &effect, Outcome::SafeFailure, false, 30).unwrap());
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["coding"].phase, Phase::Failed);
    assert!(state.items["coding"].deciding.is_none(), "the published call budget survives a retry");
    replay(&runtime);
}

#[tokio::test]
async fn script_hooks_fake_clock_expiry_is_logged_and_keeps_the_retry_deadline() {
    let (dir, mut runtime) = failed_within(&response("retry"), EXPIRES_MS).await;
    runtime.pump(1_000).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    let open = state.items["coding"].deciding.as_ref().unwrap();
    assert_eq!(open.attempt, 2);
    let events = runtime.events("w").unwrap();
    let first_timeout = events.iter().find(|e| matches!(e.command, Command::Decide { result: HookResult::Timeout, .. })).unwrap().now_ms;
    assert_eq!(open.deadline_ms, first_timeout + EXPIRES_MS as i64, "the retry retains its published timeout from the observed expiry");
    drop(runtime);
    let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
    wait_for_decision(&mut runtime, 2_000).await;
    let events = runtime.events("w").unwrap();
    let times: Vec<_> = events.iter().filter(|e| matches!(e.command, Command::Decide { result: HookResult::Timeout, .. }))
        .map(|e| e.now_ms).collect();
    assert_eq!(times.len(), 2);
    assert_eq!(times[0], first_timeout);
    assert!(times[0] >= 1_000 && times[1] >= 2_000, "the supplied clock is the lower bound; dispatch also accounts for elapsed time");
    assert!(runtime.state("w").unwrap().unwrap().items["coding"].deciding.is_none());
    replay(&runtime);
}

#[tokio::test]
async fn spawn_failure_removes_checkout_and_worktree_registration() {
    let (_dir, mut runtime) = failed("exit 0").await;
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let binding = Binding::new("w", &effect, 0, 0, &runtime.base);
    let path = Path::new(&match &effect.kind { EffectKind::Decide { policy, .. } => policy.script.clone(), _ => panic!() }).to_owned();
    let bytes = b"#!/nonexistent/farcooler-hook-interpreter\n".to_vec();
    std::fs::write(&path, &bytes).unwrap();
    let mut binding = binding;
    if let EffectKind::Decide { policy, .. } = &mut binding.effect.kind { policy.digest = digest(&bytes); }
    runtime.exec.launched = Some((20, std::time::Instant::now()));
    assert!(matches!(runtime.execute_hook(&binding, None).await.unwrap(), Outcome::Hook(HookResult::Missing)));
    let checkout = runtime.exec.root.join(format!("hook-tree-{}", effect.id));
    assert!(!checkout.exists());
    let output = std::process::Command::new("git").arg("-C").arg(runtime.exec.repository()).args(["worktree", "list", "--porcelain"]).output().unwrap();
    assert!(!String::from_utf8_lossy(&output.stdout).contains(checkout.to_str().unwrap()));
}
