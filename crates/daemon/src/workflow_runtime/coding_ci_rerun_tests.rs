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
