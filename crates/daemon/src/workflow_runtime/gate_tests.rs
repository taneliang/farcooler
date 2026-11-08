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
prompt = os.environ['TEST_LAUNCHED_PROMPT']
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
    if len(log.read_text().splitlines()) > 1 and mode != 'never':
        pathlib.Path(os.environ['FARCOOLER_RECEIPT']).write_text(json.dumps(dict(binding=os.environ['FARCOOLER_OPERATION_BINDING'],
            identity='agent', candidate=os.environ['FARCOOLER_CANDIDATE'], passed=mode != 'reject')))
else:
    pathlib.Path('result.txt').write_text('candidate native PTY')
    subprocess.check_call(['/usr/bin/git','add','--','result.txt'])
    subprocess.check_call(['/usr/bin/git','-c','commit.gpgsign=false','commit','--quiet','-m','native build'])
hook('Stop')
time.sleep(30)
"#;

async fn three_gates(mode: &str) -> (tempfile::TempDir, Runtime, super::coding_native::tests::Fake) {
    let (dir, runtime, fake) = native("pass").await;
    let script = dir.path().join("gates.py");
    std::fs::write(&script, AGENT).unwrap();
    super::coding_native::TEST_ARGV.with(|p| *p.borrow_mut() = Some(vec!["/usr/bin/python3".into(),
        script.to_string_lossy().into_owned(), mode.into()]));
    (dir, runtime, fake)
}
/// The configured steps, with the three gates after the checks and before landing.
fn definition(runtime: &Runtime) -> Definition {
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
fn at(state: &State) -> (String, Phase) {
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
    write_agent_verdict(env, false).unwrap();
    assert!(matches!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::Rejected(_)));
    let _ = dir;
    // A verdict for another effect, or another candidate, is no verdict for this one.
    for (key, value) in [("FARCOOLER_OPERATION_BINDING", "foreign"), ("FARCOOLER_CANDIDATE", "a".repeat(40).as_str())] {
        let mut other = command.environment.clone();
        other.insert(key.into(), value.into());
        write_agent_verdict(|k| other.get(k).cloned(), true).unwrap();
        assert!(runtime.reconcile_coding(&binding).await.is_err(), "{key}");
    }
}

#[tokio::test]
async fn on_a_runner_that_starts_agents_a_drafted_agent_gate_is_one_the_engine_starts() {
    let (_dir, runtime, _fake) = three_gates("pass").await;
    let config = runtime.coding.as_ref().unwrap();
    let draft = crate::landing_templates::template("ticket-pr", config, &["make check".into()],
        &[crate::landing_templates::Extra::Agent], &crate::landing_templates::Options::default()).unwrap();
    assert!(draft.definition.stages.iter().any(|s| s.kind == StageKind::Review(Reviewer::Agent)), "{:?}", draft.definition.stages);
}
