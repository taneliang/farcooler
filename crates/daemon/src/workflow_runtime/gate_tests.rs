//! ov-969: one workflow with an agent gate, an orchestrator gate and a person gate, run by the real runtime.
use super::*;
use super::coding_native::tests::native;
use super::tests::{enqueue, finish};
use std::collections::BTreeMap;

/// Builds as the native fixture does. Reviews: the first attempt ends its turn with no verdict, the second
/// writes one through the same receipt `farcooler workflow verdict` writes.
const AGENT: &str = r#"import json, os, pathlib, socket, subprocess, sys, time
mode = sys.argv[1]
session = os.environ['FARCOOLER_NATIVE_SESSION']
prompt = pathlib.Path(os.environ['TEST_LAUNCHED_PROMPT_FILE']).read_text() if 'TEST_LAUNCHED_PROMPT_FILE' in os.environ else os.environ['TEST_LAUNCHED_PROMPT']
def hook(event, **extra):
    with socket.socket(socket.AF_UNIX) as s:
        s.connect(os.environ['FARCOOLER_NATIVE_HOOK_SOCKET'])
        s.sendall((json.dumps(dict(agent='claude', event=event, payload=dict(session_id=session, cwd=os.getcwd(), **extra)))+'\n').encode())
        assert s.recv(64).strip() == b'{}'
hook('SessionStart')
hook('UserPromptSubmit', prompt=prompt)
if prompt.startswith('You are reviewing'):
    log = pathlib.Path(os.environ['STARTED']).parent / 'reviews'
    with log.open('a') as f: f.write(os.environ.get('FARCOOLER_WORKFLOW_MODEL', '-') + '\n')
    # 'subdir' and 'nostop' review well the first time: one ends its turn from a directory it moved into, as ov-1064's
    # reviewer did after a `cd apps/android`, and one records its verdict and never ends its turn (ov-1082).
    first = mode in ('subdir', 'nostop')
    if (first or len(log.read_text().splitlines()) > 1) and mode != 'never':
        pathlib.Path(os.environ['FARCOOLER_RECEIPT']).write_text(json.dumps(dict(binding=os.environ['FARCOOLER_OPERATION_BINDING'],
            identity='agent', candidate=os.environ['FARCOOLER_CANDIDATE'], passed=mode != 'reject')))
    if mode == 'subdir':
        os.makedirs('apps/android', exist_ok=True)
        os.chdir('apps/android')
    if mode == 'nostop':
        time.sleep(30)
else:
    pathlib.Path('result.txt').write_text('candidate native PTY')
    subprocess.check_call(['/usr/bin/git','add','--','result.txt'])
    subprocess.check_call(['/usr/bin/git','-c','commit.gpgsign=false','commit','--quiet','-m','native build'])
hook('Stop')
time.sleep(30)
"#;

pub(super) async fn three_gates(mode: &str) -> (tempfile::TempDir, Runtime, super::coding_native::tests::Fake) {
    let (dir, runtime, fake) = native("pass").await;
    let script = dir.path().join("gates.py");
    std::fs::write(&script, AGENT).unwrap();
    super::coding_native::TEST_ARGV.with(|p| *p.borrow_mut() = Some(vec!["/usr/bin/python3".into(),
        script.to_string_lossy().into_owned(), mode.into()]));
    (dir, runtime, fake)
}
/// The configured steps, with the three gates after the checks and before landing.
pub(super) fn definition(runtime: &Runtime) -> Definition {
    let mut definition = runtime.configured_definition().unwrap();
    let operation = |argument: &str| runtime.coding.as_ref().unwrap().operation("coding-review/v1", argument);
    let gate = |id: &str, reviewer: Reviewer, argument: &str| Stage { id: id.into(), kind: StageKind::Review(reviewer),
        progression: Progression::Automatic, max_attempts: 2, operation: operation(argument), routes: BTreeMap::new(), limit: 0 };
    let land = definition.stages.iter().position(|s| s.kind == StageKind::Land).unwrap();
    definition.stages.splice(land..land, [gate("agent-review", Reviewer::Agent, AGENT_REVIEWER),
        gate("orchestrator-review", Reviewer::Orchestrator("orchestrator".into()), "orchestrator:orchestrator"),
        gate("person-review", Reviewer::Person("owner".into()), "person:owner")]);
    definition.models = BTreeMap::from([("agent-review".into(), vec!["claude:sonnet".into()])]);
    definition
}
pub(super) fn at(state: &State) -> (String, Phase) {
    let item = &state.items["one"];
    (item.definition.stages.get(item.stage).map_or("end".into(), |s| s.id.clone()), item.phase.clone())
}
async fn settle(runtime: &mut Runtime, from: i64) -> State {
    for time in from..from + 40 {
        runtime.pump(time).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        if !matches!(state.items["one"].phase, Phase::Running | Phase::Blocked) { return state; }
    }
    panic!("never settled: {:?}", runtime.state("w").unwrap());
}
fn review(runtime: &Runtime, principal: &str, now: i64) -> std::result::Result<(), Refusal> {
    let state = runtime.state("w").unwrap().unwrap();
    let item = &state.items["one"];
    let session = runtime.decision_session(&runtime.controller(), principal).unwrap();
    let sequence = state.source_sequences.get(&format!("decision:{principal}")).copied().unwrap_or(0) + 1;
    runtime.decide(&session, Request { id: format!("review-{now}"), workspace: "w".into(), expected_revision: state.revision,
        sequence, command: Command::Review { item: "one".into(), run: item.run.clone(),
            evidence: item.last_evidence.clone().unwrap(), passed: true, reason: None } }, now).unwrap().result.map(|_| ())
}

#[tokio::test]
async fn an_agent_gate_then_the_orchestrator_then_the_owner_each_hold_the_run_until_they_decide() {
    let (dir, mut runtime, _fake) = three_gates("pass").await;
    let definition = definition(&runtime);
    enqueue(&mut runtime, "w", "one", definition);
    let state = settle(&mut runtime, 2).await;
    // The agent ran twice with the model the step lists: once with no verdict, which didn't pass, then with one.
    assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), "claude:sonnet\nclaude:sonnet\n");
    assert_eq!(at(&state), ("orchestrator-review".into(), Phase::WaitingReview));
    // Nothing moves a waiting gate but its own decider.
    assert_eq!(runtime.pump(100).await.unwrap(), 0);
    assert_eq!(review(&runtime, "owner", 101), Err(Refusal::Unauthorized));
    review(&runtime, "orchestrator", 102).unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(at(&state), ("person-review".into(), Phase::WaitingReview));
    assert_eq!(review(&runtime, "orchestrator", 103), Err(Refusal::Unauthorized));
    review(&runtime, "owner", 104).unwrap();
    let state = finish(&mut runtime, "w").await;
    assert_eq!(at(&state), ("end".into(), Phase::Done));
    // It landed what the agent reviewed.
    assert_eq!(adapters::git(&runtime.repository(), &["rev-parse", runtime.landing_ref()]).await.unwrap(),
        state.items["one"].candidate.clone().unwrap());
}

#[tokio::test]
async fn an_agent_that_never_gives_a_verdict_fails_the_run_and_one_that_rejects_stops_it() {
    for mode in ["never", "reject"] {
        let (_dir, mut runtime, _fake) = three_gates(mode).await;
        let definition = definition(&runtime);
        enqueue(&mut runtime, "w", "one", definition);
        let state = settle(&mut runtime, 2).await;
        assert_eq!(at(&state), ("agent-review".into(), Phase::Failed), "{mode}");
        assert_eq!(adapters::git(&runtime.repository(), &["rev-parse", runtime.landing_ref()]).await.unwrap(),
            runtime.coding.as_ref().unwrap().base, "{mode}: nothing landed");
    }
}

#[tokio::test]
async fn the_verdict_command_writes_the_receipt_the_gate_reads() {
    let (dir, mut runtime, _fake) = three_gates("never").await;
    let definition = definition(&runtime);
    enqueue(&mut runtime, "w", "one", definition);
    // Provision and build, then hold the review effect without running it.
    for time in 2..20 {
        let state = runtime.state("w").unwrap().unwrap();
        if at(&state).0 == "agent-review" { break; }
        runtime.pump(time).await.unwrap();
    }
    let binding = super::coding_tests::admitted(&runtime).await;
    assert_eq!(binding.effect.kind, EffectKind::Execute(StageKind::Review(Reviewer::Agent)));
    // The agent runs and ends its turn with no verdict: that is no pass.
    let (_hold, cancelled) = tokio::sync::watch::channel(false);
    assert_eq!(runtime.execute_coding("w", &binding, cancelled).await.unwrap(), Outcome::SafeFailure);
    // What the agent's environment held, as the step pinned it.
    let command = runtime.coding_command(&binding).unwrap();
    let env = |key: &str| command.environment.get(key).cloned();
    assert_eq!(env("FARCOOLER_WORKFLOW_MODEL").as_deref(), Some("claude:sonnet"));
    // Outside a review the command refuses.
    assert!(matches!(write_agent_verdict(|_| None, true), Err(Error::Refused(_))));
    // The verdict command's receipt is the one the gate reads.
    let receipt = write_agent_verdict(env, true).unwrap();
    assert_eq!(receipt, PathBuf::from(env("FARCOOLER_RECEIPT").unwrap()));
    assert!(matches!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::Passed(_)));
    // A verdict for another effect, or another candidate, is no verdict for this one.
    for (key, value) in [("FARCOOLER_OPERATION_BINDING", "foreign"), ("FARCOOLER_CANDIDATE", "a".repeat(40).as_str())] {
        let mut other = command.environment.clone();
        other.insert(key.into(), value.into());
        write_agent_verdict(|k| other.get(k).cloned(), true).unwrap();
        assert!(runtime.reconcile_coding(&binding).await.is_err(), "{key}");
    }
    let finding = "Retry discards the caller's deadline.";
    write_agent_verdict_with_summary(env, false, Some(finding.into())).unwrap();
    let rejected = runtime.reconcile_coding(&binding).await.unwrap();
    assert!(matches!(rejected, Outcome::ReviewRejected { .. }));
    assert!(runtime.observe("w", &binding.effect, rejected, false, 40).unwrap());
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].phase, Phase::Failed);
    let get = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    let reason = get["state"]["items"]["one"]["reason"].as_str().unwrap();
    assert!(reason.starts_with("agent-review:") && reason.contains(finding)
        && reason.contains(receipt.file_name().unwrap().to_str().unwrap()), "{get}");
    assert_eq!(crate::plan_run::run_of(&state, "one").unwrap().reason.as_deref(), Some(reason));
    drop(dir);
}

#[tokio::test]
async fn on_a_runner_that_starts_agents_a_drafted_agent_gate_is_one_the_engine_starts() {
    let (_dir, runtime, _fake) = three_gates("pass").await;
    let config = runtime.coding.as_ref().unwrap();
    let draft = crate::landing_templates::template("ticket-pr", config, &["make check".into()],
        &[crate::landing_templates::Extra::Agent], &crate::landing_templates::Options::default()).unwrap();
    assert!(draft.definition.stages.iter().any(|s| s.kind == StageKind::Review(Reviewer::Agent)), "{:?}", draft.definition.stages);
}

/// ov-1082: ov-1064's reviewer recorded its verdict, moved into `apps/android` and ended its turn there. Its Stop
/// named that directory, not the worktree, so the turn was never seen to end.
#[tokio::test]
async fn a_reviewer_that_ends_its_turn_in_a_subdirectory_settles_from_its_verdict() {
    let (dir, mut runtime, _fake) = three_gates("subdir").await;
    let definition = definition(&runtime);
    enqueue(&mut runtime, "w", "one", definition);
    let state = settle(&mut runtime, 2).await;
    assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), "claude:sonnet\n", "it reviewed once");
    assert_eq!(at(&state), ("orchestrator-review".into(), Phase::WaitingReview));
    // Its Stop ended the turn, not the quiet after its verdict.
    let heard = std::fs::read_dir(runtime.root.join("receipts")).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".native-settled.json"))
        .flat_map(|p| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).unwrap()).unwrap()["hooks"].as_array().unwrap().clone())
        .any(|hook| hook["event"] == "Stop" && hook["payload"]["cwd"].as_str().unwrap().ends_with("/apps/android"));
    assert!(heard, "the Stop from apps/android was not heard");
}

/// The review effect, admitted and not yet run.
pub(super) async fn held_review(runtime: &mut Runtime) -> Binding {
    let definition = definition(runtime);
    enqueue(runtime, "w", "one", definition);
    for time in 2..20 {
        if at(&runtime.state("w").unwrap().unwrap()).0 == "agent-review" { break; }
        runtime.pump(time).await.unwrap();
    }
    super::coding_tests::admitted(runtime).await
}

/// ov-1082: a Stop that never arrives (a hook lost under load) doesn't hold a recorded verdict for the turn's
/// three hours. Once the reviewer has gone quiet, the verdict ends the turn.
#[tokio::test]
async fn a_recorded_verdict_settles_a_turn_whose_end_never_arrives() {
    let (_dir, mut runtime, _fake) = three_gates("nostop").await;
    let binding = held_review(&mut runtime).await;
    let started = std::time::Instant::now();
    let (_hold, cancelled) = tokio::sync::watch::channel(false);
    assert!(matches!(runtime.execute_coding("w", &binding, cancelled).await.unwrap(), Outcome::Passed(_)));
    // Not the fixture's five-second turn limit: the quiet after the verdict.
    assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
}

/// ov-1082: a daemon that stops (an install) while a reviewer that has given its verdict is still in its turn
/// keeps the verdict, rather than counting the review as never done.
#[tokio::test]
async fn a_verdict_survives_the_daemon_stopping_before_the_turn_ends() {
    let (_dir, mut runtime, _fake) = three_gates("nostop").await;
    let binding = held_review(&mut runtime).await;
    let receipt = PathBuf::from(runtime.coding_command(&binding).unwrap().environment["FARCOOLER_RECEIPT"].clone());
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let stop = async {
        assert!(super::wait_until(tokio::time::Instant::now() + Duration::from_secs(60),
            Duration::from_millis(10), || receipt.exists()).await, "the reviewer gave no verdict");
        cancel.send_replace(true);
    };
    let (outcome, ()) = tokio::join!(runtime.execute_coding("w", &binding, cancelled), stop);
    assert!(matches!(outcome.unwrap(), Outcome::Passed(_)));
}

/// ov-1071: an agent gate's options give its reviewer the workflow's words, and choose its model by the paths the
/// ticket changed. The build here changes `result.txt`.
fn with_options(runtime: &Runtime, rule: &str) -> Definition {
    let mut definition = definition(runtime);
    let options = crate::review_agent::ReviewOptions { prompt: Some("Read the lane rules first.".into()),
        check: None, models: vec![(rule.into(), "claude:opus".into())] };
    let gate = definition.stages.iter_mut().find(|s| s.id == "agent-review").unwrap();
    gate.operation.arguments.push(options.argument());
    definition.models.insert("agent-review".into(), vec!["claude:sonnet".into(), "claude:opus".into()]);
    definition
}

#[tokio::test]
async fn a_review_runs_with_the_model_its_changed_paths_earn_and_the_workflows_words() {
    for (rule, model) in [("result.txt", "claude:opus"), ("crates/**", "claude:sonnet")] {
        let (dir, mut runtime, _fake) = three_gates("pass").await;
        let definition = with_options(&runtime, rule);
        runtime.coding.as_ref().unwrap().validate_definition(&definition).unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        let state = settle(&mut runtime, 2).await;
        assert_eq!(at(&state), ("orchestrator-review".into(), Phase::WaitingReview), "{rule}");
        assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), format!("{model}\n{model}\n"), "{rule}: both attempts");
    }
    let (_dir, mut runtime, _fake) = three_gates("never").await;
    let definition = with_options(&runtime, "result.txt");
    enqueue(&mut runtime, "w", "one", definition);
    let binding = {
        for time in 2..20 {
            if at(&runtime.state("w").unwrap().unwrap()).0 == "agent-review" { break; }
            runtime.pump(time).await.unwrap();
        }
        super::coding_tests::admitted(&runtime).await
    };
    let (_hold, cancelled) = tokio::sync::watch::channel(false);
    runtime.execute_coding("w", &binding, cancelled).await.unwrap();
    // The launch, and a rebuild of it, name the chosen model and carry the workflow's words after the engine's brief.
    let command = runtime.coding_command(&binding).unwrap();
    assert_eq!(command.environment.get("FARCOOLER_WORKFLOW_MODEL").map(String::as_str), Some("claude:opus"));
    let prompt = command.environment.get("TEST_LAUNCHED_PROMPT").unwrap();
    assert!(prompt.starts_with("You are reviewing ticket one.") && prompt.ends_with("\n\nRead the lane rules first."), "{prompt}");
    assert!(prompt.contains(" task note one --kind finding") && !prompt.contains("`farcooler "), "the reviewer is told where its reasons go: {prompt}");
    assert_eq!(runtime.coding_command(&binding).unwrap(), command);
}

#[tokio::test]
async fn a_review_option_naming_a_model_its_step_doesnt_list_is_refused() {
    let (_dir, runtime, _fake) = three_gates("pass").await;
    let config = runtime.coding.as_ref().unwrap();
    let mut definition = with_options(&runtime, "result.txt");
    definition.models.insert("agent-review".into(), vec!["claude:sonnet".into()]);
    assert!(config.validate_definition(&definition).is_err(), "opus isn't listed");
    let mut garbled = with_options(&runtime, "result.txt");
    garbled.stages.iter_mut().find(|s| s.id == "agent-review").unwrap().operation.arguments[2] = "{}".into();
    assert!(config.validate_definition(&garbled).is_err(), "options that say nothing");
}

/// ov-1194: use a two-party barrier, so sequential execution fails even on a fast machine.
async fn parallel_fixture(fail: bool, parallel: bool) -> (tempfile::TempDir, Runtime, super::coding_native::tests::Fake, Definition) {
    use crate::landing_templates::Host;
    use farcooler_workflow::templates::{self, Gate, Plan, Template};
    let (dir, mut runtime, fake) = three_gates("subdir").await;
    let agent = dir.path().join("gates.py");
    let text = std::fs::read_to_string(&agent).unwrap();
    let barrier = r#"
    d = pathlib.Path(os.environ['STARTED']).parent
    (d / 'review-ready').touch()
    deadline = time.time() + 4
    while not (d / 'gate-ready').exists():
        if time.time() > deadline: sys.exit(9)
        time.sleep(0.02)
"#;
    std::fs::write(&agent, text.replace("    log = pathlib.Path", &format!("{barrier}    log = pathlib.Path"))).unwrap();
    let check = dir.path().join("parallel.py");
    std::fs::write(&check, r#"import pathlib, sys, time
d = pathlib.Path(sys.argv[1])
(d / 'gate-ready').touch()
deadline = time.time() + 4
while not (d / 'review-ready').exists():
    if time.time() > deadline: sys.exit(9)
    time.sleep(0.02)
sys.exit(int(sys.argv[2]))
"#).unwrap();
    let config = runtime.coding.as_mut().unwrap();
    config.checks[0].argv = vec!["/usr/bin/python3".into(), check.to_string_lossy().into_owned(),
        dir.path().to_string_lossy().into_owned(), if fail { "7" } else { "0" }.into()];
    // The group never runs in these tests; its private bare target makes the whole definition publishable.
    let bare = dir.path().join("remote.git"); std::fs::create_dir(&bare).unwrap();
    adapters::git(&bare, &["init", "--bare", "--quiet"]).await.unwrap();
    config.landing = Some(CodingLanding { job: "parallel-test".into(), repository: bare.canonicalize().unwrap(),
        target: "refs/heads/main".into(), config_digest: digest(&std::fs::read(bare.join("config")).unwrap()) });
    config.security_reviews = vec![super::coding::SecurityReview {
        step: "full".into(), models: vec!["claude:opus".into()],
        options: parallel.then(|| crate::review_agent::ReviewOptions { prompt: None, models: vec![], check: Some(0) }.argument()),
    }];
    let plan = Plan::new(Template::BatchedPush, vec![Gate::Command(1)]);
    let mut d = templates::build(&plan, &Host(config)).unwrap();
    let mut gate = config.stage("light", StageKind::Review(Reviewer::Agent), "coding-review/v1", AGENT_REVIEWER);
    gate.max_attempts = 1;
    gate.operation.arguments.push(crate::review_agent::ReviewOptions {
        prompt: None, models: vec![], check: parallel.then_some(0) }.argument());
    // A sequential control for the barrier regression test.
    if !parallel { gate.operation.arguments.pop(); }
    let mut full = gate.clone(); full.id = "full".into();
    full.routes.insert("passed".into(), Target::Step("join".into()));
    let rules = serde_json::json!({"under_lines":100,"report_paths":["reports/**"],"full_paths":["auth/**"]}).to_string();
    let mut route = config.stage("review-route", StageKind::Route, "coding-route/v1", &rules);
    route.routes = BTreeMap::from([("light".into(), Target::Step("light".into())), ("full".into(), Target::Step("full".into()))]);
    d.stages.splice(2..2, [route, full, gate]);
    d.models = BTreeMap::from([("light".into(), vec!["claude:sonnet".into()]), ("full".into(), vec!["claude:opus".into()])]);
    config.validate_definition(&d).unwrap();
    (dir, runtime, fake, d)
}

#[tokio::test]
async fn light_review_and_gate_overlap_and_both_are_required_before_join() {
    for fail in [false, true] {
        let (dir, mut runtime, _fake, d) = parallel_fixture(fail, true).await;
        enqueue(&mut runtime, "w", "one", d);
        let state = settle(&mut runtime, 2).await;
        assert_eq!(state.items["one"].phase, if fail { Phase::Failed } else { Phase::WaitingJoin });
        assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), "claude:sonnet\n", "one fast review");
        // Recovery reads both facts; it never launches either command again.
        let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.effect.model.as_deref() == Some("claude:sonnet")).unwrap();
        let binding = Binding::new("w", &effect.effect, effect.resource_generation.unwrap(), 0, &runtime.coding.as_ref().unwrap().base);
        let outcome = runtime.reconcile_coding(&binding).await.unwrap();
        assert_eq!(matches!(outcome, Outcome::Passed(_)), !fail);
        assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap().lines().count(), 1);
    }
}

/// Controlled before/after measurement using the engine's own stage timestamps, not a wall timer
/// around test setup. No external model calls; each stand-in spends 500ms doing its work.
#[tokio::test]
#[ignore = "ov-1194 stage timing experiment; run with --ignored --nocapture"]
async fn light_review_stage_timing_before_and_after() {
    for parallel in [false, true] {
        let mut samples = Vec::new();
        for _ in 0..5 {
            let (dir, mut runtime, _fake, mut d) = parallel_fixture(false, parallel).await;
            let agent = dir.path().join("gates.py");
            let text = std::fs::read_to_string(&agent).unwrap();
            let start = text.find("    d = pathlib.Path(os.environ['STARTED']).parent\n").unwrap();
            let end = text[start..].find("    log = pathlib.Path").unwrap() + start;
            std::fs::write(&agent, format!("{}    time.sleep(0.5)\n{}", &text[..start], &text[end..])).unwrap();
            std::fs::write(dir.path().join("parallel.py"), "import time\ntime.sleep(0.5)\n").unwrap();
            if !parallel {
                let config = runtime.coding.as_ref().unwrap();
                d.stages.insert(2, config.stage("ticket-gate", StageKind::Check, "coding-check/v1", "0"));
            }
            enqueue(&mut runtime, "w", "one", d);
            assert!(wait_until_async!(tokio::time::Instant::now() + Duration::from_secs(60),
                Duration::from_millis(10), async || {
                runtime.pump(super::now()).await.unwrap();
                let state = runtime.state("w").unwrap().unwrap();
                assert!(!matches!(state.items["one"].phase, Phase::Failed | Phase::Uncertain));
                if state.items["one"].phase == Phase::WaitingJoin {
                    let effects = runtime.store.workflow_outbox("w").unwrap();
                    let next = effects.iter().find(|s| s.effect.operation == state.items["one"].definition.stages[2].operation).unwrap();
                    let ms = runtime.store.workflow_events("w").unwrap().last().unwrap().now_ms - next.effect.created_ms;
                    samples.push(ms);
                    true
                } else { false }
            }), "timing sample never reached WaitingJoin");
        }
        assert_eq!(samples.len(), 5);
        samples.sort();
        println!("ov-1194 {} build-done-to-WaitingJoin ms: {:?}; median {}", if parallel { "after" } else { "before" }, samples, samples[2]);
    }
}

#[tokio::test]
async fn full_review_route_and_reviewer_rejection_remain_effective() {
    for reject in [false, true] {
        let (dir, mut runtime, _fake, mut d) = parallel_fixture(false, true).await;
        d.stages[2].operation.arguments[1] = serde_json::json!({"under_lines":1,"report_paths":[],"full_paths":["auth/**"]}).to_string();
        if reject {
            let agent = dir.path().join("gates.py");
            let text = std::fs::read_to_string(&agent).unwrap().replace("passed=mode != 'reject'", "passed=False");
            std::fs::write(agent, text).unwrap();
        }
        enqueue(&mut runtime, "w", "one", d);
        let state = settle(&mut runtime, 2).await;
        assert_eq!(state.items["one"].phase, if reject { Phase::Failed } else { Phase::WaitingJoin });
        assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), "claude:opus\n");
    }
}

#[tokio::test]
async fn parallel_review_requires_its_gate_receipt_on_recovery_and_a_known_check_at_publication() {
    let (dir, mut runtime, _fake, mut d) = parallel_fixture(false, true).await;
    let gate = d.stages.iter_mut().find(|s| s.id == "light").unwrap();
    let options = crate::review_agent::ReviewOptions { check: Some(999), ..Default::default() };
    let good = std::mem::replace(&mut gate.operation.arguments[2], options.argument());
    assert!(runtime.coding.as_ref().unwrap().validate_definition(&d).is_err());
    d.stages.iter_mut().find(|s| s.id == "light").unwrap().operation.arguments[2] = good;
    enqueue(&mut runtime, "w", "one", d);
    let state = settle(&mut runtime, 2).await;
    assert_eq!(state.items["one"].phase, Phase::WaitingJoin);
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.effect.model.as_deref() == Some("claude:sonnet")).unwrap();
    let binding = Binding::new("w", &effect.effect, effect.resource_generation.unwrap(), 0, &runtime.coding.as_ref().unwrap().base);
    let check = runtime.review_check(&binding).unwrap();
    std::fs::remove_file(runtime.root.join("receipts").join(format!("{}.process.json", digest(&check.effect)))).unwrap();
    assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::SafeFailure);
    assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap().lines().count(), 1, "recovery doesn't relaunch");
}

#[tokio::test]
async fn cancelling_parallel_review_ends_the_owned_gate_before_returning() {
    let (dir, mut runtime, _fake, d) = parallel_fixture(false, true).await;
    let script = dir.path().join("parallel.py");
    let text = std::fs::read_to_string(&script).unwrap().replace("sys.exit(int(sys.argv[2]))", "time.sleep(60)");
    std::fs::write(script, text).unwrap();
    enqueue(&mut runtime, "w", "one", d);
    for time in 2..20 {
        if at(&runtime.state("w").unwrap().unwrap()).0 == "light" { break; }
        runtime.pump(time).await.unwrap();
    }
    let binding = super::coding_tests::admitted(&runtime).await;
    let check = runtime.review_check(&binding).unwrap();
    let owner = runtime.owner_path(&check);
    let receipt = runtime.provider_path(&binding);
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let stop = async {
        assert!(super::wait_until(tokio::time::Instant::now() + Duration::from_secs(60),
            Duration::from_millis(10), || owner.exists() && receipt.exists()).await, "parallel processes never started");
        cancel.send_replace(true);
    };
    let (result, ()) = tokio::join!(runtime.execute_coding("w", &binding, cancelled), stop);
    assert_eq!(result.unwrap(), Outcome::SafeFailure);
    let owner: crate::effect_runner::Owner = serde_json::from_slice(&std::fs::read(owner).unwrap()).unwrap();
    assert!(crate::effect_runner::Group::gone(owner.pid).unwrap());
}

#[tokio::test]
async fn review_routing_reads_real_git_report_paths_and_binary_numstat() {
    for binary in [false, true] {
        let (dir, mut runtime, _fake, mut d) = parallel_fixture(false, true).await;
        d.stages[2].operation.arguments[1] = serde_json::json!({"under_lines":1,"report_paths":["reports/**"],"full_paths":["auth/**"]}).to_string();
        let script = dir.path().join("gates.py");
        let mut text = std::fs::read_to_string(&script).unwrap();
        if binary {
            text = text.replace(".write_text('candidate native PTY')", ".write_bytes(b'candidate\\x00binary')");
        } else {
            text = text.replace("    pathlib.Path('result.txt').write_text", "    pathlib.Path('reports').mkdir()\n    pathlib.Path('result.txt').write_text")
                .replace("'result.txt'", "'reports/a\\tb\\nc.txt'");
        }
        std::fs::write(script, text).unwrap();
        enqueue(&mut runtime, "w", "one", d);
        let state = settle(&mut runtime, 2).await;
        assert_eq!(state.items["one"].phase, Phase::WaitingJoin);
        assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), if binary { "claude:opus\n" } else { "claude:sonnet\n" });
    }
}

/// A protected path wins even when both light-review conditions hold on the real Git diff.
#[tokio::test]
async fn security_paths_require_full_review_even_for_small_report_only_changes() {
    let (dir, mut runtime, _fake, mut d) = parallel_fixture(false, true).await;
    d.stages[2].operation.arguments[1] = serde_json::json!({
        "under_lines":100,"report_paths":["**"],"full_paths":["result.txt"] }).to_string();
    runtime.coding.as_ref().unwrap().validate_definition(&d).unwrap();
    enqueue(&mut runtime, "w", "one", d);
    let state = settle(&mut runtime, 2).await;
    assert_eq!(state.items["one"].phase, Phase::WaitingJoin);
    assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), "claude:opus\n");
}

/// The actual candidate diff obeys the repository policy even when the definition omits it.
#[tokio::test]
async fn omitted_keychain_path_and_documented_definition_require_full_review() {
    let docs = include_str!("../../../../docs/light-review.md");
    let documented = docs.split("```json\n").nth(1).unwrap().split("\n```").next().unwrap();
    let omitted = r#"{"under_lines":100,"report_paths":["**"],"full_paths":["unrelated/**"]}"#;
    for argument in [omitted, documented] {
        let (dir, mut runtime, _fake, mut d) = parallel_fixture(false, true).await;
        d.stages[2].operation.arguments[1] = argument.into();
        runtime.coding.as_ref().unwrap().validate_definition(&d).unwrap();
        let script = dir.path().join("gates.py");
        let text = std::fs::read_to_string(&script).unwrap()
            .replace("    pathlib.Path('result.txt').write_text",
                "    pathlib.Path('apps/shared/AgentKit/Tests/AgentKitTests').mkdir(parents=True)\n    pathlib.Path('result.txt').write_text")
            .replace("'result.txt'", "'apps/shared/AgentKit/Tests/AgentKitTests/AccountKeychainWriteTests.swift'");
        std::fs::write(script, text).unwrap();
        enqueue(&mut runtime, "w", "one", d);
        let state = settle(&mut runtime, 2).await;
        assert_eq!(state.items["one"].phase, Phase::WaitingJoin);
        assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), "claude:opus\n", "{argument}");
    }
}

#[tokio::test]
async fn stopping_a_parallel_review_after_restart_reaps_its_gate() {
    let (_dir, mut runtime, _fake, d) = parallel_fixture(false, true).await;
    enqueue(&mut runtime, "w", "one", d);
    for time in 2..20 {
        if at(&runtime.state("w").unwrap().unwrap()).0 == "light" { break; }
        runtime.pump(time).await.unwrap();
    }
    let binding = super::coding_tests::admitted(&runtime).await;
    let check = runtime.review_check(&binding).unwrap();
    // A child left by the old daemon with an unreadable process fact. Cancellation still
    // needs to end the child before falling back to the reviewer's quiescence receipt.
    let mut child = tokio::process::Command::new("/bin/sleep").arg("60").process_group(0).kill_on_drop(true).spawn().unwrap();
    let pid = child.id().unwrap();
    crate::effect_runner::Owner::write(&runtime.owner_path(&check), pid).unwrap();
    std::fs::write(runtime.root.join("receipts").join(format!("{}.process.json", digest(&check.effect))), b"{").unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert!(runtime.submit_now(&runtime.controller(), Request { id: "cancel".into(), workspace: "w".into(),
        expected_revision: state.revision, sequence: 2, command: Command::Cancel { item: "one".into(), reason: None } }, 200).unwrap().result.is_ok());
    // Reap the simulated orphan concurrently: on macOS a zombie can still hold its group.
    let (pumped, status) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(runtime.pump(201), child.wait())
    }).await.expect("Stop must end the child even when its receipt is unreadable");
    pumped.unwrap(); status.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Cancelled);
    assert!(crate::effect_runner::Group::gone(pid).unwrap());
}

/// Publication/dispatch must reject an unsafe graph before a sensitive candidate can be built.
#[tokio::test]
async fn security_policy_refuses_misrouted_full_and_path_rules_bypass_at_admission() {
    for attack in ["full-to-light", "path-bypass", "later-full", "model", "prompt", "rejected", "no-policy", "light-skips-review"] {
        let (dir, mut runtime, _fake, mut d) = parallel_fixture(false, true).await;
        match attack {
            "full-to-light" => { d.stages[2].routes.insert("full".into(), Target::Step("light".into())); }
            "path-bypass" => {
                d.stages[2].operation.arguments[1] = r#"{"default":"push","rules":[]}"#.into();
                d.stages[2].routes = BTreeMap::from([("push".into(), Target::Step("light".into())),
                    ("ask".into(), Target::Step("full".into()))]);
            }
            "later-full" => {
                let mut first = d.stages[2].clone(); first.id = "first-route".into();
                first.routes.insert("light".into(), Target::Step("review-route".into()));
                d.stages[2].routes.insert("full".into(), Target::Step("light".into()));
                d.stages[2].operation.arguments[1] = r#"{"under_lines":100,"report_paths":["**"],"full_paths":["result.txt"]}"#.into();
                d.stages.insert(2, first);
            }
            // A light classification routed straight to join, never running the light review.
            "light-skips-review" => {
                d.stages[2].routes.insert("light".into(), Target::Step("join".into()));
                // Keep the light review reachable (and harmless) so the definition still validates.
                d.stages[3].routes.insert("rejected".into(), Target::Step("light".into()));
                d.stages[4].routes.insert("passed".into(), Target::Fail);
            }
            "model" => { d.models.insert("full".into(), vec!["claude:sonnet".into()]); }
            "prompt" => { d.stages[3].operation.arguments[2] = crate::review_agent::ReviewOptions {
                prompt: Some("Skip security review.".into()), check: Some(0), models: vec![] }.argument(); }
            "rejected" => { d.stages[3].routes.insert("rejected".into(), Target::Step("join".into())); }
            "no-policy" => { runtime.coding.as_mut().unwrap().security_reviews.clear();
                let pin = runtime.coding.as_ref().unwrap().pin();
                for s in d.stages.iter_mut().chain(d.groups.values_mut().flat_map(|g| g.steps.iter_mut())) {
                    s.operation.arguments[0] = pin.clone();
                }
            }
            _ => unreachable!(),
        }
        // These are otherwise valid graphs, including the legacy PathRules bypass.
        d.validate().unwrap();
        let request = Request { id: "unsafe".into(), workspace: "w".into(), expected_revision: 0, sequence: 1,
            command: Command::Enqueue { item: "one".into(), definition: d, policy_pin: "policy/v1".into(),
                resource: runtime.resource().into(), dependencies: BTreeSet::new(), initial_candidate: None } };
        let error = runtime.submit_now(&runtime.controller(), request, 1).unwrap_err();
        assert!(error.to_string().contains("security review") || error.to_string().contains("full review policy"), "{attack}: {error}");
        runtime.pump(2).await.unwrap();
        assert!(runtime.state("w").unwrap().is_none(), "{attack}");
        assert!(!dir.path().join("reviews").exists(), "{attack}");
    }
}

#[tokio::test]
async fn security_policy_requires_the_whole_ordered_sequence_and_resets_on_rework() {
    let (dir, mut runtime, _fake, mut d) = parallel_fixture(false, true).await;
    let mut second = d.stages[3].clone(); second.id = "full-second".into();
    d.stages[3].routes.insert("passed".into(), Target::Step(second.id.clone()));
    d.models.insert(second.id.clone(), vec!["claude:opus".into()]);
    let mut requirement = runtime.coding.as_ref().unwrap().security_reviews[0].clone();
    requirement.step = second.id.clone();
    runtime.coding.as_mut().unwrap().security_reviews.push(requirement);
    d.stages.insert(4, second);
    let pin = runtime.coding.as_ref().unwrap().pin();
    for s in d.stages.iter_mut().chain(d.groups.values_mut().flat_map(|g| g.steps.iter_mut())) {
        s.operation.arguments[0] = pin.clone();
    }
    runtime.coding.as_ref().unwrap().validate_definition(&d).unwrap();
    let mut reversed = runtime.coding.as_ref().unwrap().clone();
    reversed.security_reviews.reverse();
    assert!(super::coding_review_policy::validate(&reversed, &d).is_err());
    let mut missing = d.clone(); missing.stages[3].routes.insert("passed".into(), Target::Step("join".into()));
    assert!(runtime.coding.as_ref().unwrap().validate_definition(&missing).is_err());
    // Only the rework path bypasses classification: it cannot reuse the first candidate's certificate.
    let mut rework = d.clone();
    let mut rebuild = rework.stages[1].clone(); rebuild.id = "rebuild".into();
    rebuild.routes.insert("passed".into(), Target::Step("light".into())); rebuild.limit = 1;
    let light = rework.stages.iter_mut().find(|s| s.id == "light").unwrap();
    light.routes.insert("passed".into(), Target::Step("join".into()));
    light.routes.insert("rejected".into(), Target::Step("rebuild".into()));
    let join = rework.stages.iter().position(|s| s.id == "join").unwrap();
    rework.stages.insert(join, rebuild);
    rework.validate().unwrap();
    assert!(runtime.coding.as_ref().unwrap().validate_definition(&rework).is_err());
    // Exercise both required reviewers on a sensitive diff, through the actual runtime.
    d.stages[2].operation.arguments[1] = r#"{"under_lines":100,"report_paths":["**"],"full_paths":["result.txt"]}"#.into();
    enqueue(&mut runtime, "w", "one", d);
    let state = settle(&mut runtime, 2).await;
    assert_eq!(state.items["one"].phase, Phase::WaitingJoin);
    assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), "claude:opus\nclaude:opus\n");
    let outbox = runtime.store.workflow_outbox("w").unwrap();
    for id in ["full", "full-second"] {
        assert!(outbox.iter().any(|e| e.effect.operation == state.items["one"].definition.stages.iter().find(|s| s.id == id).unwrap().operation
            && e.dispatch_state == "settled"));
    }
}
