//! Real engine effects against private git and a deterministic gh stand-in. Never contacts GitHub.
use super::*;
use super::coding_tests::admitted;
use super::coding_verbs_tests::{lander, batched_push, at_join, command, until, phase_of, remote_head, outbox};

async fn fixture(red_twice: bool, jobs: &[&str]) -> (tempfile::TempDir, Runtime) {
    let (dir, runtime) = lander("ci-fail").await;
    let adapter = dir.path().join("adapter.py");
    let mut text = std::fs::read_to_string(&adapter).unwrap();
    text.truncate(text.find("elif stage == 'ci':").unwrap());
    text.push_str(r#"elif stage == 'ci':
    attempt_file = pathlib.Path(os.environ['ATTEMPT'])
    attempt = int(attempt_file.read_text()) if attempt_file.exists() else 1
    red = attempt == 1 or os.environ['RED_TWICE'] == 'yes'
    receipt.write_text(json.dumps(dict(binding=binding, repository='offline/repository',
        failed_jobs=json.loads(os.environ['FAILED_JOBS']) if red else [], run=dict(
        id=73, name='configured-check', head_sha=candidate, status='completed', run_attempt=attempt,
        conclusion='failure' if red else 'success', html_url='fixture://ci/73',
        created_at='2026-01-01T00:00:00Z', event='push'))))
"#);
    std::fs::write(&adapter, text).unwrap();
    let gh = dir.path().join("gh");
    std::fs::write(&gh, r#"#!/usr/bin/python3
import json, os, pathlib, subprocess, sys
args = sys.argv[1:]
attempt = pathlib.Path(os.environ['ATTEMPT'])
if args[:2] == ['run', 'rerun']:
    assert args == ['run', 'rerun', '73', '--failed', '--repo', 'offline/repository'], args
    with open(os.environ['SENDS'], 'a') as f: f.write('rerun\n')
    attempt.write_text('2')
elif args[:3] == ['api', '-X', 'GET']:
    assert args[3] == 'repos/offline/repository/actions/runs/73', args
    sha = subprocess.check_output(['/usr/bin/git', '--git-dir', os.environ['REMOTE'], 'rev-parse', 'refs/heads/main']).decode().strip()
    print(json.dumps(dict(id=73, name='configured-check', head_sha=sha, run_attempt=int(attempt.read_text()) if attempt.exists() else 1,
        status='completed', conclusion='success', html_url='fixture://ci/73', created_at='2026-01-01T00:00:00Z', event='push')))
else:
    raise AssertionError(args)
"#).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut config = runtime.coding.clone().unwrap();
    let remote = config.landing.as_ref().unwrap().repository.to_string_lossy().into_owned();
    config.ci.command.environment.extend([
        ("CI_GH".into(), gh.to_string_lossy().into_owned()),
        ("ATTEMPT".into(), dir.path().join("attempt").to_string_lossy().into_owned()),
        ("SENDS".into(), dir.path().join("sends").to_string_lossy().into_owned()),
        ("REMOTE".into(), remote), ("RED_TWICE".into(), if red_twice { "yes" } else { "no" }.into()),
        ("FAILED_JOBS".into(), serde_json::to_string(jobs).unwrap()),
    ]);
    let root = runtime.root.clone(); drop(runtime);
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    (dir, Runtime::open(&root).unwrap())
}
fn definition(runtime: &Runtime) -> Definition {
    let mut d = batched_push(runtime, "any", 5000);
    d.policies.insert("flaky-ci".into(), CiPolicy { point: CiPoint::Result, step: "ci".into(),
        rule: CiRule { rerun_failed: RerunFailed { jobs: vec!["iOS UI tests".into()], times: 1 } }, fallback: None });
    d
}
fn sends(dir: &Path) -> usize { std::fs::read_to_string(dir.join("sends")).unwrap_or_default().lines().count() }

#[tokio::test]
async fn ci_rerun_red_then_green_lands_and_red_twice_routes_rejected() {
    for red_twice in [false, true] {
        let (dir, mut runtime) = fixture(red_twice, &["iOS UI tests"]).await;
        let d = definition(&runtime);
        at_join(&mut runtime, "one", &d).await;
        assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
        let state = until(&mut runtime, |s| s.items.values().any(|i| i.group_of.is_some()
            && i.phase == if red_twice { Phase::Failed } else { Phase::Done })).await;
        let group = state.items.values().find(|i| i.group_of.is_some()).unwrap();
        assert_eq!(group.ci_reruns["ci"], 1);
        assert_eq!(sends(dir.path()), 1);
        assert_eq!(outbox(&runtime, StageKind::RerunCi), 1);
        assert_eq!(outbox(&runtime, StageKind::AwaitCi), 2);
        assert_eq!(outbox(&runtime, StageKind::Push), 1);
        assert_eq!(group.candidate.as_deref(), Some(remote_head(&runtime).await.as_str()));
        let mut replay = State::empty("w".into(), 1);
        for input in runtime.events("w").unwrap() { replay = step(&replay, &input).unwrap().state; }
        assert_eq!(replay, state);
    }
}

#[tokio::test]
async fn ci_rerun_unlisted_job_is_rejected_without_sending() {
    let (dir, mut runtime) = fixture(false, &["iOS UI tests", "Rust"]).await;
    let d = definition(&runtime);
    at_join(&mut runtime, "one", &d).await;
    command(&runtime, Command::Queue { item: "one".into() });
    until(&mut runtime, |s| s.items.values().any(|i| i.group_of.is_some() && i.phase == Phase::Failed)).await;
    assert_eq!(sends(dir.path()), 0);
    assert_eq!(outbox(&runtime, StageKind::RerunCi), 0);
}

#[tokio::test]
async fn ci_rerun_restart_after_send_before_receipt_reads_attempt_without_resending() {
    let (dir, mut runtime) = fixture(false, &["iOS UI tests"]).await;
    let d = definition(&runtime);
    at_join(&mut runtime, "one", &d).await;
    command(&runtime, Command::Queue { item: "one".into() });
    until(&mut runtime, |s| s.items.values().any(|i| i.ci_retry.as_ref().is_some_and(|r| r.pending))).await;
    let binding = admitted(&runtime).await;
    assert_eq!(binding.effect.kind, EffectKind::Execute(StageKind::RerunCi));
    let (_hold, cancel) = tokio::sync::watch::channel(false);
    assert!(matches!(runtime.execute_coding("w", &binding, cancel).await.unwrap(), Outcome::Passed(_)));
    assert_eq!(sends(dir.path()), 1);
    // The process completed, but the daemon died before the verb receipt and Observe reached disk.
    std::fs::remove_file(runtime.verb_path(&binding)).unwrap();
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_eq!(sends(dir.path()), 1);
    assert_eq!(outbox(&runtime, StageKind::RerunCi), 1);
}

#[tokio::test]
async fn ci_rerun_restart_with_no_send_proof_stays_uncertain_and_does_not_send() {
    let (dir, mut runtime) = fixture(false, &["iOS UI tests"]).await;
    let d = definition(&runtime);
    at_join(&mut runtime, "one", &d).await;
    command(&runtime, Command::Queue { item: "one".into() });
    until(&mut runtime, |s| s.items.values().any(|i| i.ci_retry.as_ref().is_some_and(|r| r.pending))).await;
    let binding = admitted(&runtime).await;
    let group = binding.effect.item.clone();
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    until(&mut runtime, |s| phase_of(s, &group) == Some(Phase::Uncertain)).await;
    assert_eq!(sends(dir.path()), 0);
    for t in 200..203 { runtime.pump(t).await.unwrap(); }
    assert_eq!(sends(dir.path()), 0);
    assert_eq!(outbox(&runtime, StageKind::RerunCi), 1);
}

#[tokio::test]
async fn pushed_landing_waits_for_runner_retry_and_keeps_the_reason_when_it_is_exhausted() {
    use std::os::unix::fs::PermissionsExt;
    for final_conclusion in ["success", "cancelled", "failure"] {
        let (dir, runtime) = lander("pass").await;
        let script = dir.path().join("ci-retry");
        std::fs::write(&script, r#"#!/usr/bin/python3
import json, os, pathlib, subprocess, sys
counter = pathlib.Path(os.environ['POLL'])
poll = int(counter.read_text()) if counter.exists() else 0
if len(sys.argv) == 1:
    poll += 1
    counter.write_text(str(poll))
terminal = os.environ['FINAL']
attempt, status, conclusion = {
    1: (1, 'completed', 'failure'),
    2: (2, 'completed', 'cancelled'),
    3: (3, 'in_progress', None),
}.get(poll, (3 if terminal == 'success' else 8, 'completed', terminal))
candidate = subprocess.check_output(['/usr/bin/git', '--git-dir', os.environ['REMOTE'], 'rev-parse', 'refs/heads/main']).decode().strip()
run = dict(id=45191956221, name='CI', head_sha=candidate, status=status, conclusion=conclusion,
           head_branch='main', run_attempt=attempt, html_url='fixture://ci/45191956221',
           created_at='2026-11-23T14:13:54Z', updated_at='2026-11-23T14:54:35Z', event='push')
if len(sys.argv) == 1:
    assert candidate == os.environ['FARCOOLER_CANDIDATE']
    pathlib.Path(os.environ['FARCOOLER_RECEIPT']).write_text(json.dumps(
        dict(binding=os.environ['FARCOOLER_OPERATION_BINDING'], repository='offline/repository', run=run)))
elif 'ci-runner-retry.yml' in sys.argv[4]:
    print(json.dumps(dict(workflow_runs=[dict(id=123, display_title=f'Retry CI 45191956221 attempt {attempt}',
        status='in_progress' if poll <= 2 else 'completed')])))
elif '/attempts/8/jobs?' in sys.argv[4]:
    print('Swift (app build + bundle checks)')
else:
    assert sys.argv[4].endswith('/actions/runs/45191956221'), sys.argv
    print(json.dumps(run))
"#).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let polls = dir.path().join("poll");
        let mut config = runtime.coding.clone().unwrap();
        config.ci.workflow = "CI".into();
        config.ci.command.argv = vec![script.to_string_lossy().into_owned()];
        config.ci.command.environment.extend([
            ("CI_GH".into(), script.to_string_lossy().into_owned()),
            ("POLL".into(), polls.to_string_lossy().into_owned()),
            ("FINAL".into(), final_conclusion.into()),
        ]);
        let root = runtime.root.clone();
        drop(runtime);
        std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
        let mut runtime = Runtime::open(&root).unwrap();
        let definition = batched_push(&runtime, "any", 5000);
        at_join(&mut runtime, "one", &definition).await;
        command(&runtime, Command::Queue { item: "one".into() });
        until(&mut runtime, |s| s.items.values().any(|i| i.pushed.is_some())).await;
        let ci = runtime.store.workflow_outbox("w").unwrap().into_iter()
            .find(|s| s.active && s.effect.kind == EffectKind::Execute(StageKind::AwaitCi)).unwrap();
        // The original deadline has elapsed while the first controller is still active.
        runtime.pump(ci.effect.created_ms + 5001).await.unwrap();
        let terminal = if final_conclusion == "success" { Phase::Done } else { Phase::Failed };
        assert_eq!(std::fs::read_to_string(polls).unwrap(), "4", "interim attempts cannot settle the wait");
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(phase_of(&state, "one"), Some(terminal));
        assert_eq!(outbox(&runtime, StageKind::Push), 1);
        assert_eq!(outbox(&runtime, StageKind::AwaitCi), 1);
        if final_conclusion != "success" {
            let group = state.items.values().find(|item| item.group_of.is_some()).unwrap();
            assert_eq!(group.pushed.as_deref(), Some(remote_head(&runtime).await.as_str()));
            let failure = group.group_failure.as_ref().unwrap();
            assert_eq!(failure.job, "Swift (app build + bundle checks)");
            let verdict = if final_conclusion == "cancelled" { "was cancelled" } else { "failed" };
            assert_eq!(failure.first_line, format!("CI run 45191956221, attempt 8 {verdict}. This job never acquired a runner."));
        }
    }
}
