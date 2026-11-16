//! ov-1280: Nov 16's runner outage, replayed. Group 2895's CI (run 43745907131) went red only because a runner
//! couldn't download the Rust toolchain, and the next job found no artifact. The engine now reruns those jobs once by
//! itself, and group 2924, which gated on 2895's head, waits at its push for that rerun and pushes on its green with
//! no second gate. Never contacts GitHub: the run's jobs and failed-log lines come from the saved fixture.
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use farcooler_workflow::{digest, setup_only, CiRun, Command, EffectKind, FailedStep, Phase, StageKind, State, SETUP_RERUN};
use super::Runtime;
use super::coding_verbs_tests::{lander, batched_push, at_join, command, until, phase_of, outbox, settled, remote_head};

const RUN: &str = include_str!("../../../../test/fixtures/landing/run43745907131-jobs.json");
const PREVIOUS: [&str; 2] = ["ov-1266", "ov-1210"];
const GATED: [&str; 3] = ["ov-1199", "ov-1267", "ov-1277"];

/// The run's failed jobs as the CI look reports them: each one's failed step, and the first action its failed log
/// ends in failure.
fn outage() -> CiRun {
    let fixture: serde_json::Value = serde_json::from_str(RUN).unwrap();
    let failed: Vec<&serde_json::Value> = fixture["jobs"].as_array().unwrap().iter().filter(|j| j["conclusion"] == "failure").collect();
    let failed_steps = failed.iter().map(|j| FailedStep {
        job: j["name"].as_str().unwrap().into(),
        step: j["steps"].as_array().unwrap().iter().find(|s| s["conclusion"] == "failure").unwrap()["name"].as_str().unwrap().into(),
        action: j["log_failed"].as_array().unwrap().iter().filter_map(|l| l.as_str()?.split("##[end-action id=").nth(1))
            .find_map(|rest| rest.split_once(';').filter(|(_, t)| t.starts_with("outcome=failure;")).map(|(id, _)| id.to_owned())),
    }).collect();
    CiRun { id: fixture["run"]["id"].as_u64().unwrap(), attempt: 1,
        failed_jobs: failed.iter().map(|j| j["name"].as_str().unwrap().into()).collect(), failed_steps }
}

/// 2895's pushed candidate (the second CI sees) is still running until `release-ci` exists; then attempt 1 is
/// `red` (the outage, or a real test failure) and a rerun's attempt 2 is green. Every other candidate's CI passes.
async fn replay(red: &CiRun) -> (tempfile::TempDir, Runtime) {
    let (dir, runtime) = lander("pass").await;
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(dir.path().join("red.json"), serde_json::to_vec(red).unwrap()).unwrap();
    let replayed = r#"elif stage == 'ci':
    control = pathlib.Path(__file__).parent
    seen = control / 'ci-seen'
    order = seen.read_text().split() if seen.exists() else []
    if candidate != os.environ['RED'] and candidate not in order: order.append(candidate); seen.write_text(' '.join(order))
    if len(order) > 1 and candidate == order[1]:
        red = json.loads((control / 'red.json').read_text())
        attempt = int((control / 'attempt').read_text()) if (control / 'attempt').exists() else 1
        released = (control / 'release-ci').exists()
        failing = released and attempt == 1
        receipt.write_text(json.dumps(dict(binding=binding, repository='offline/repository',
            failed_jobs=red['failed_jobs'] if failing else [], failed_steps=red.get('failed_steps', []) if failing else [],
            run=dict(id=red['id'], name='configured-check', head_sha=candidate, event='push', run_attempt=attempt,
                status='completed' if released else 'in_progress', conclusion=('failure' if failing else 'success') if released else None,
                html_url='fixture://ci/' + str(red['id']), created_at='2026-11-15T18:17:23Z'))))
        sys.exit(0)
"#;
    assert!(text.contains("elif stage == 'ci':\n"));
    std::fs::write(&script, text.replace("elif stage == 'ci':\n", replayed)).unwrap();
    // The rerun goes through the pinned gh: it records the send and starts attempt 2 of the same run on 2895's head.
    let gh = dir.path().join("gh");
    std::fs::write(&gh, r#"#!/usr/bin/python3
import json, pathlib, sys
control = pathlib.Path(__file__).parent
args = sys.argv[1:]
run = json.loads((control / 'red.json').read_text())['id']
attempt = control / 'attempt'
if args[:2] == ['run', 'rerun']:
    assert args == ['run', 'rerun', str(run), '--failed', '--repo', 'offline/repository'], args
    with open(control / 'sends', 'a') as f: f.write('rerun\n')
    attempt.write_text('2')
elif args[:3] == ['api', '-X', 'GET']:
    assert args[3] == 'repos/offline/repository/actions/runs/' + str(run), args
    print(json.dumps(dict(id=run, name='configured-check', head_sha=(control / 'ci-seen').read_text().split()[1], event='push',
        run_attempt=int(attempt.read_text()) if attempt.exists() else 1, status='completed', conclusion='failure',
        html_url='fixture://ci/' + str(run), created_at='2026-11-15T18:17:23Z')))
else:
    raise AssertionError(args)
"#).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut config = runtime.coding.clone().unwrap();
    config.ci.command.environment.insert("CI_GH".into(), gh.to_string_lossy().into_owned());
    let root = runtime.root.clone(); drop(runtime);
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    (dir, Runtime::open(&root).unwrap())
}

/// Queue 2895 and then 2924 behind it, as on Nov 16, and run until every member of 2924 has landed. Returns the
/// two groups, 2924's candidate at its push, and the board.
async fn land(dir: &std::path::Path, runtime: &mut Runtime) -> (String, String, String, State) {
    let definition = batched_push(runtime, "green", 30_000);
    // A landing ahead holds the landing while 2895's two members queue into one group; 2895 holds it while 2924's do.
    for item in ["ahead"].iter().chain(&PREVIOUS).chain(&GATED) { at_join(runtime, item, &definition).await; }
    for item in ["ahead"].into_iter().chain(PREVIOUS) { command(runtime, Command::Queue { item: item.into() }); }
    let previous = runtime.state("w").unwrap().unwrap().items[PREVIOUS[0]].grouped_in.clone().unwrap();
    let holding = |s: &State| s.items.get(&previous).is_some_and(|g| g.group_of.is_some()
        && matches!(g.definition.stages[g.stage].kind, StageKind::Fold | StageKind::Run));
    until(runtime, holding).await;
    for item in GATED { command(runtime, Command::Queue { item: item.into() }); }
    let gated = runtime.state("w").unwrap().unwrap().items[GATED[0]].grouped_in.clone().unwrap();
    assert_ne!(gated, previous, "2924 is a group of its own");
    // 2924 folds on 2895's head and gates while that CI runs; at its push, the CI comes back red.
    let (store, release, watched) = (runtime.store.clone(), dir.join("release-ci"), gated.clone());
    let observer = tokio::spawn(async move {
        loop {
            let group = store.workflow_state("w").unwrap().unwrap().items[&watched].clone();
            if group.definition.stages.get(group.stage).is_some_and(|s| s.kind == StageKind::Push) {
                std::fs::write(&release, "red").unwrap();
                return group.candidate;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    runtime.continuous = true;
    let state = until(runtime, |s| GATED.iter().all(|m| phase_of(s, m) == Some(Phase::Done))).await;
    let gated_on = observer.await.unwrap().expect("the gated group had a candidate at its push");
    (previous, gated, gated_on, state)
}
fn effects(runtime: &Runtime, group: &str, kind: StageKind) -> usize {
    runtime.store.workflow_outbox("w").unwrap().iter().filter(|s| s.effect.item == group && s.effect.kind == EffectKind::Execute(kind.clone())).count()
}
fn push_fact(runtime: &Runtime, group: &str) -> serde_json::Value {
    let push = runtime.store.workflow_outbox("w").unwrap().into_iter().rev()
        .find(|s| s.effect.item == group && s.effect.kind == EffectKind::Execute(StageKind::Push)).unwrap();
    serde_json::from_slice(&std::fs::read(runtime.root.join("receipts").join(format!("{}.verb.json", digest(&push.effect)))).unwrap()).unwrap()
}
fn sends(dir: &std::path::Path) -> usize { std::fs::read_to_string(dir.join("sends")).unwrap_or_default().lines().count() }

#[tokio::test]
async fn nov16_runner_outage_reruns_2895s_ci_and_2924_pushes_on_its_green_without_a_second_gate() {
    let red = outage();
    assert!(setup_only(&red), "the fixture is the runner outage");
    let (dir, mut runtime) = replay(&red).await;
    let (previous, gated, gated_on, state) = land(dir.path(), &mut runtime).await;
    // 2895: its red CI was rerun once, by the engine, and the rerun's green landed it.
    assert_eq!(sends(dir.path()), 1, "one rerun sent");
    assert_eq!(effects(&runtime, &previous, StageKind::RerunCi), 1);
    assert_eq!(effects(&runtime, &previous, StageKind::AwaitCi), 2, "the first look, and the look at the rerun");
    assert!(PREVIOUS.iter().all(|m| phase_of(&state, m) == Some(Phase::Done)), "2895 landed on its rerun's green");
    let rerun = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .find(|s| s.effect.item == previous && s.effect.kind == EffectKind::Execute(StageKind::RerunCi)).unwrap();
    assert_eq!(rerun.effect.ci_retry.as_ref().map(|r| (r.policy.as_str(), r.run.attempt)), Some((SETUP_RERUN, 1)));
    // 2924: one base step, one gate, and a push that waited for the rerun and went over a green head.
    assert_eq!(effects(&runtime, &gated, StageKind::AwaitBase), 1, "no back edge to the base step");
    assert_eq!(effects(&runtime, &gated, StageKind::Run), 1, "the change is gated once");
    let fact = push_fact(&runtime, &gated);
    assert_eq!(fact["label"].as_str(), Some("passed"));
    assert!(fact["detail"].get("red_base").is_none(), "the push waited for the rerun's green: {fact}");
    assert_eq!(remote_head(&runtime).await, gated_on, "2924's gated candidate is what landed");
    assert_eq!(settled(&runtime, StageKind::AwaitCi)["label"].as_str(), Some("passed"), "2924's own CI decided");
    assert_eq!(outbox(&runtime, StageKind::Build), 1 + PREVIOUS.len() + GATED.len(), "no member was sent back to build");
}

#[tokio::test]
async fn a_real_test_failure_in_the_previous_ci_is_not_rerun_and_the_gated_group_goes_as_before() {
    // The same run, but the iOS job failed in the repository's own build command.
    let mut red = outage();
    red.failed_steps[0].action = Some("__self.__run_2".into());
    assert!(!setup_only(&red));
    let (dir, mut runtime) = replay(&red).await;
    let (previous, gated, gated_on, state) = land(dir.path(), &mut runtime).await;
    assert_eq!(sends(dir.path()), 0, "nothing was rerun");
    assert_eq!(effects(&runtime, &previous, StageKind::RerunCi), 0);
    assert!(PREVIOUS.iter().all(|m| phase_of(&state, m) == Some(Phase::Failed)), "2895's red CI fails 2895");
    // What a red predecessor does to the gated group today (ov-1291): it pushes over the red head it gated on.
    assert_eq!(effects(&runtime, &gated, StageKind::Run), 1);
    assert_eq!(push_fact(&runtime, &gated)["detail"]["red_base"], true);
    assert_eq!(remote_head(&runtime).await, gated_on);
}
