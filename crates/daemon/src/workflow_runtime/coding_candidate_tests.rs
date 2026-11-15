//! CI on a landing group's candidate branch before the push (ov-1269), against a local bare repository standing in
//! for the remote. No network push is ever made.
use super::*;
use super::coding_verbs_tests::{lander, command, phase_of, until, at_join, remote_head, outbox, candidate_of};

/// The group of `batched_push`, with the candidate put on its branch after the fold and CI waited for before the push.
fn candidate_push(runtime: &Runtime, patience_ms: u64) -> Definition {
    let mut definition = super::coding_verbs_tests::batched_push(runtime, "any", patience_ms);
    let c = runtime.coding.as_ref().unwrap();
    let steps = &mut definition.groups.get_mut("landing").unwrap().steps;
    let ci = steps.pop().unwrap();
    let push = steps.pop().unwrap();
    let mut candidate = c.stage("candidate", StageKind::PushCandidate, "coding-push-candidate/v1", "");
    candidate.max_attempts = 2;
    steps.insert(2, candidate);
    steps.extend([ci, push]);
    definition
}

/// A lander whose CI writes down, each time it is asked, the commit it judged, the target's head, and every
/// `landing/` branch on the remote then. `ci-fail` names a failing job.
async fn watched(mode: &str) -> (tempfile::TempDir, Runtime, PathBuf) {
    let (dir, runtime) = lander(mode).await;
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    let seen = "elif stage == 'ci':\n    if os.environ.get('CI_SEEN'):\n        refs = subprocess.run(['/usr/bin/git', '--git-dir', os.environ['REMOTE'], 'for-each-ref', '--format=%(refname) %(objectname)', 'refs/heads/landing/'], capture_output=True, text=True).stdout.split()\n        main = subprocess.run(['/usr/bin/git', '--git-dir', os.environ['REMOTE'], 'rev-parse', 'refs/heads/main'], capture_output=True, text=True).stdout.strip()\n        with open(os.environ['CI_SEEN'], 'a') as f: f.write(json.dumps(dict(candidate=candidate, main=main, branches=dict(zip(refs[0::2], refs[1::2])))) + '\\n')\n";
    let text = text.replacen("elif stage == 'ci':\n", seen, 1)
        .replace("created_at='2026-01-01T00:00:00Z', event='push'))))",
            "created_at='2026-01-01T00:00:00Z', event='push'), failure=dict(job='Rust tests (Linux, shard 2)', firstLine='assertion failed') if mode == 'ci-fail' else None)))");
    assert!(text.contains("CI_SEEN") && text.contains("shard 2"));
    std::fs::write(&script, text).unwrap();
    let root = runtime.root.clone();
    let log = dir.path().join("ci-seen.jsonl");
    let mut config = runtime.coding.clone().unwrap();
    config.ci.command.environment.insert("CI_SEEN".into(), log.to_string_lossy().into_owned());
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    drop(runtime);
    (dir, Runtime::open(&root).unwrap(), log)
}
fn seen(log: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}
async fn landing_branches(runtime: &Runtime) -> String {
    let landing = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap();
    adapters::git(&landing.repository, &["for-each-ref", "--format=%(refname)", "refs/heads/landing/"]).await.unwrap()
}
/// Pump until every candidate branch has left the remote and its record is gone: the sweep deletes off the loop.
async fn until_deleted(runtime: &mut Runtime) {
    for time in 1000..1200 {
        runtime.pump(time).await.unwrap();
        let records = std::fs::read_dir(runtime.root.join("candidate-branches")).map_or(0, |d| d.count());
        if landing_branches(runtime).await.is_empty() && records == 0 { return; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the candidate branch was never deleted: {}", landing_branches(runtime).await);
}

#[tokio::test]
async fn a_green_candidate_lands_that_exact_commit_after_ci_ran_on_its_branch_and_the_branch_goes() {
    let (_dir, mut runtime, log) = watched("pass").await;
    let definition = candidate_push(&runtime, 5000);
    runtime.coding.as_ref().unwrap().validate_definition(&definition).unwrap();
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let pushed = remote_head(&runtime).await;
    assert_ne!(pushed, first);
    assert_eq!(candidate_of(&state, "one"), Some(pushed.as_str()));
    // CI judged the very commit the target now holds, on its branch, while the target still sat at the base.
    let looks = seen(&log);
    assert!(!looks.is_empty());
    for look in &looks {
        assert_eq!(look["candidate"], pushed.as_str(), "{look}");
        assert_eq!(look["main"], first.as_str(), "CI ran before the push moved the target: {look}");
        let branches = look["branches"].as_object().unwrap();
        assert_eq!(branches.len(), 1, "{look}");
        let (name, head) = branches.iter().next().unwrap();
        assert!(name.starts_with("refs/heads/landing/"), "{name}");
        assert_eq!(head, pushed.as_str(), "the branch holds the candidate");
    }
    // One CI wait, and none after the push: the candidate's verdict is the target's.
    for kind in [StageKind::PushCandidate, StageKind::AwaitCi, StageKind::Push] {
        assert_eq!(outbox(&runtime, kind.clone()), 1, "{kind:?} ran once");
    }
    let order: Vec<_> = runtime.store.workflow_outbox("w").unwrap().into_iter().map(|s| s.effect.kind)
        .filter(|k| matches!(k, EffectKind::Execute(StageKind::AwaitCi | StageKind::Push))).collect();
    assert_eq!(order, [EffectKind::Execute(StageKind::AwaitCi), EffectKind::Execute(StageKind::Push)]);
    until_deleted(&mut runtime).await;
}

#[tokio::test]
async fn a_red_candidate_fails_the_group_naming_the_job_and_the_target_never_moves() {
    let (_dir, mut runtime, log) = watched("ci-fail").await;
    let definition = candidate_push(&runtime, 5000);
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| s.items.values().any(|i| i.group_of.is_some() && i.phase == Phase::Failed)).await;
    let group = state.items.values().find(|i| i.group_of.is_some()).unwrap();
    assert_eq!(group.group_failure.as_ref().map(|f| f.job.as_str()), Some("Rust tests (Linux, shard 2)"));
    assert_eq!(group.pushed, None);
    assert_eq!(remote_head(&runtime).await, first, "main is untouched");
    assert_eq!(outbox(&runtime, StageKind::Push), 0, "nothing was pushed to the target");
    assert!(!seen(&log).is_empty(), "CI ran on the branch");
    // The ticket is told `rejected`, which this definition's join doesn't route: it fails, and nothing landed.
    assert_eq!(phase_of(&state, "one"), Some(Phase::Failed));
    assert_eq!(state.items["one"].landed, None);
    until_deleted(&mut runtime).await;
}

#[tokio::test]
async fn a_cancelled_candidate_ci_fails_the_group_and_the_target_never_moves() {
    let (_dir, mut runtime, _log) = watched("ci-cancel").await;
    let definition = candidate_push(&runtime, 5000);
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    until(&mut runtime, |s| s.items.values().any(|i| i.group_of.is_some() && i.phase == Phase::Failed)).await;
    assert_eq!(remote_head(&runtime).await, first);
    assert_eq!(outbox(&runtime, StageKind::Push), 0);
    until_deleted(&mut runtime).await;
}

#[tokio::test]
async fn a_group_that_branches_its_candidate_must_wait_for_ci_before_the_push() {
    let (_dir, runtime, _log) = watched("pass").await;
    let mut definition = candidate_push(&runtime, 5000);
    let steps = &mut definition.groups.get_mut("landing").unwrap().steps;
    let push = steps.pop().unwrap();
    let ci = steps.pop().unwrap();
    steps.extend([push, ci]);
    assert!(runtime.coding.as_ref().unwrap().validate_definition(&definition).is_err(), "the push would move the target before CI");
}

#[tokio::test]
async fn a_live_group_keeps_its_candidate_branch() {
    let (_dir, mut runtime, _log) = watched("ci-pending").await;
    let definition = candidate_push(&runtime, 2_000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    until(&mut runtime, |s| s.items.values().any(|i| i.group_of.is_some() && i.definition.stages[i.stage].kind == StageKind::AwaitCi)).await;
    // While its group waits for CI, the sweep leaves the branch where it is.
    let branches = landing_branches(&runtime).await;
    assert_eq!(branches.lines().count(), 1, "{branches}");
    assert_eq!(runtime.ended_candidate_branches().unwrap().len(), 0);
    // Once the wait gives up, the group has ended and its branch goes.
    until(&mut runtime, |s| s.items.values().any(|i| i.group_of.is_some() && i.phase == Phase::Failed)).await;
    until_deleted(&mut runtime).await;
}
