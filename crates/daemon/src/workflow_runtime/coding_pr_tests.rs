//! The pull request verbs against a real git repository, a local bare repository standing in for the
//! remote, and a stub `gh` that keeps its pull requests in a file. No GitHub call is ever made (ov-957).
use super::*;
use super::coding_tests::configured;
use super::concurrency_tests::enqueue_item;
use std::collections::BTreeMap;

/// A stand-in for `gh`. Its "GitHub" is one JSON file; every call is appended to a log. Behavior is
/// chosen by `GH_MODE`, and a pull request is merged or reviewed after a number of looks at it.
const GH: &str = r#"#!/usr/bin/python3
import json, os, subprocess, sys
state_path, log_path, remote = os.environ['GH_STATE'], os.environ['GH_LOG'], os.environ['GH_REMOTE']
mode = os.environ.get('GH_MODE', 'merge')
args = sys.argv[1:]
open(log_path, 'a').write(json.dumps(dict(args=args, env=sorted(os.environ))) + '\n')
try: state = json.load(open(state_path))
except FileNotFoundError: state = dict(prs=[], looks=0)
def save(): json.dump(state, open(state_path, 'w'))
def opt(name): return args[args.index(name) + 1]
def head_oid(branch):
    r = subprocess.run(['/usr/bin/git', '-C', remote, 'rev-parse', '--verify', '--quiet', 'refs/heads/' + branch], capture_output=True, text=True)
    return r.stdout.strip()
assert '--repo' in args and opt('--repo') == 'private/fixture', args
if args[:2] == ['pr', 'list']:
    head = opt('--head')
    state['looks'] += 1
    for pr in state['prs']:
        if pr['headRefName'] == head and pr['state'] == 'OPEN':
            pr['headRefOid'] = head_oid(head)
            if mode == 'merge' and state['looks'] >= int(os.environ.get('GH_MERGE_AT', '5')):
                pr['state'] = 'MERGED'; pr['mergeCommit'] = dict(oid='c' * 40)
            if mode == 'wrong-head' and state['looks'] >= 3:
                pr['state'] = 'MERGED'; pr['mergeCommit'] = dict(oid='c' * 40); pr['headRefOid'] = 'd' * 40
            if mode == 'close' and state['looks'] >= 3: pr['state'] = 'CLOSED'
            if mode == 'rework' and state['looks'] >= 3 and not pr.get('reworked'):
                pr['latestReviews'] = [dict(state='CHANGES_REQUESTED', commit=dict(oid=pr['headRefOid']))]
            if mode == 'rework' and pr.get('reworked') and state['looks'] >= 6:
                pr['state'] = 'MERGED'; pr['mergeCommit'] = dict(oid='c' * 40)
    save()
    print(json.dumps([p for p in state['prs'] if p['headRefName'] == head]))
elif args[:2] == ['pr', 'create']:
    head = opt('--head')
    if mode == 'two' : state['prs'].append(dict(number=90, state='OPEN', isDraft=False, headRefName=head, headRefOid='', mergeCommit=None, latestReviews=[]))
    state['prs'].append(dict(number=len(state['prs']) + 100, state='OPEN', isDraft='--draft' in args, headRefName=head,
        headRefOid=head_oid(head), mergeCommit=None, latestReviews=[], title=opt('--title'), base=opt('--base')))
    save()
    if mode == 'create-lost': sys.exit(1)
    print('https://github.invalid/private/fixture/pull/%d' % state['prs'][-1]['number'])
elif args[:2] == ['pr', 'ready']:
    for pr in state['prs']:
        if str(pr['number']) == args[2]: pr['isDraft'] = False
    save()
else:
    sys.exit(2)
"#;

struct Gh { state: PathBuf, log: PathBuf }

/// A configured runtime with a private bare repository as the remote and the stub `gh`.
async fn hub(mode: &str, gh_mode: &str) -> (tempfile::TempDir, Runtime, Gh) {
    let (dir, runtime) = configured(false, mode).await;
    let bare = dir.path().join("private-remote.git"); std::fs::create_dir(&bare).unwrap();
    adapters::git(&bare, &["init", "--bare", "--quiet"]).await.unwrap();
    let repo = runtime.repository();
    let status = tokio::process::Command::new("/usr/bin/git").current_dir(&repo)
        .args(["-c", "core.hooksPath=/dev/null", "push", bare.to_str().unwrap(), "HEAD:refs/heads/main"]).status().await.unwrap();
    assert!(status.success());
    let (gh, state, log) = (dir.path().join("gh"), dir.path().join("gh-state.json"), dir.path().join("gh-log.jsonl"));
    std::fs::write(&gh, GH).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = runtime.root.clone();
    let mut config = runtime.coding.clone().unwrap();
    config.landing = Some(CodingLanding { job: "landing-957".into(), repository: bare.canonicalize().unwrap(),
        target: "refs/heads/main".into(), config_digest: digest(&std::fs::read(bare.join("config")).unwrap()) });
    config.pull_request = Some(CodingPullRequest { repository: "private/fixture".into(), base: "main".into(),
        gh: CodingCommand { argv: vec![gh.to_string_lossy().into_owned()], timeout_ms: 5000,
            environment: [("GH_STATE", state.clone()), ("GH_LOG", log.clone()), ("GH_REMOTE", bare.clone())].into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string_lossy().into_owned()))
                .chain([("GH_MODE".to_string(), gh_mode.to_string())]).collect() } });
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    drop(runtime);
    (dir, Runtime::open(&root).unwrap(), Gh { state, log })
}

fn stage(config: &CodingConfig, id: &str, kind: StageKind, version: &str, argument: &str) -> Stage {
    let mut s = config.stage(id, kind, version, argument); s.max_attempts = 2; s
}
fn waiting(config: &CodingConfig, id: &str, kind: StageKind, version: &str, argument: &str, patience_ms: u64) -> Stage {
    Stage { operation: config.wait_operation(version, argument, 20, patience_ms), max_attempts: 5, ..stage(config, id, kind, version, "") }
}
fn route(mut s: Stage, label: &str, to: Target, limit: u8) -> Stage { s.routes.insert(label.into(), to); s.limit = limit; s }
/// provision, build, check, push-branch, open-pr, pr-ready, await-pr (a requested change goes back to build), await-ci.
fn per_ticket_pull_request(runtime: &Runtime, draft: bool, patience_ms: u64) -> Definition {
    let c = runtime.coding.as_ref().unwrap();
    let wait = route(route(waiting(c, "merged", StageKind::AwaitPr, "coding-await-pr/v1", "", patience_ms),
        "closed", Target::Fail, 0), "changes_requested", Target::Step("build".into()), 2);
    let mut stages = vec![stage(c, "provision", StageKind::Provision, "coding-worktree/v1", ""),
        stage(c, "build", StageKind::Build, "coding-harness/v1", ""),
        stage(c, "check", StageKind::Check, "coding-check/v1", "0"),
        stage(c, "branch", StageKind::PushBranch, "coding-push-branch/v1", ""),
        stage(c, "pr", StageKind::OpenPr, "coding-open-pr/v1", if draft { "draft" } else { "ready" })];
    if draft { stages.push(stage(c, "ready", StageKind::PrReady, "coding-pr-ready/v1", "")); }
    stages.push(wait);
    // The merge commit exists only on GitHub, so CI is read for it by the same hook the other CI steps use.
    stages.push(waiting(c, "ci", StageKind::AwaitCi, "coding-await-ci/v1", "offline/repository", patience_ms));
    Definition { schema: 2, source: "pr-test/v1".into(), max_reworks: 0, rework_target: None, stages, groups: BTreeMap::new() }
}
fn phase_of(state: &State, id: &str) -> Option<Phase> {
    state.items.get(id).map(|i| i.phase.clone()).or_else(|| state.archive.get(id).map(|a| a.phase.clone()))
}
async fn until(runtime: &mut Runtime, what: impl Fn(&State) -> bool) -> State {
    for time in 2..400 {
        runtime.pump(time).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        if what(&state) { return state; }
    }
    let state = runtime.state("w").unwrap().unwrap();
    let items: Vec<_> = state.items.iter().map(|(id, i)| format!("{id}: {:?} at {}", i.phase, i.definition.stages[i.stage.min(i.definition.stages.len() - 1)].id)).collect();
    let effects: Vec<_> = runtime.store.workflow_outbox("w").unwrap().iter().map(|s| format!("{:?} {}", s.effect.kind, s.dispatch_state)).collect();
    panic!("runtime failed to settle:\n{}\n{}", items.join("\n"), effects.join("\n"));
}
fn calls(f: &Gh) -> Vec<Vec<String>> {
    std::fs::read_to_string(&f.log).unwrap_or_default().lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect()).collect()
}
fn count(f: &Gh, verb: &str) -> usize { calls(f).iter().filter(|c| c[1] == verb).count() }
fn prs(f: &Gh) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&std::fs::read(&f.state).unwrap()).unwrap()["prs"].as_array().unwrap().clone()
}
fn head_of(item: &str) -> String { format!("farcooler/{}", &digest(&("w", item))[4..20]) }
fn outbox(runtime: &Runtime, kind: StageKind) -> usize {
    runtime.store.workflow_outbox("w").unwrap().iter().filter(|s| s.effect.kind == EffectKind::Execute(kind.clone())).count()
}

#[tokio::test]
async fn a_ticket_gets_one_pull_request_that_is_made_ready_and_waited_on() {
    let (_dir_f, mut runtime_f, f) = hub("pass", "merge").await;
    let definition = per_ticket_pull_request(&runtime_f, true, 5000);
    enqueue_item(&mut runtime_f, "w", "one", definition);
    let state = until(&mut runtime_f, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let head = head_of("one");
    // The branch is on the remote, at the candidate the build made.
    let landing = runtime_f.coding.as_ref().unwrap().landing.clone().unwrap();
    let on_remote = adapters::git(&landing.repository, &["rev-parse", &format!("refs/heads/{head}")]).await.unwrap();
    // The ticket's candidate is the commit GitHub made, not the branch's.
    assert_eq!(state.items.get("one").and_then(|i| i.candidate.as_deref()).or_else(|| state.archive.get("one").and_then(|a| a.candidate.as_deref())), Some("cccccccccccccccccccccccccccccccccccccccc"));
    assert_ne!(on_remote, "cccccccccccccccccccccccccccccccccccccccc");
    let all = prs(&f);
    assert_eq!(all.len(), 1, "exactly one pull request: {all:?}");
    assert_eq!(all[0]["headRefName"], head);
    assert_eq!(all[0]["base"], "main");
    assert_eq!(all[0]["isDraft"], false, "it was opened as a draft and then made ready");
    assert_eq!(all[0]["title"], "build 0");
    assert_eq!((count(&f, "create"), count(&f, "ready")), (1, 1));
    // Merging is a person's act; the engine never asked for one.
    assert_eq!(calls(&f).iter().filter(|c| c[1] == "merge").count(), 0);
}

#[tokio::test]
async fn a_pull_request_that_is_already_ready_is_not_made_ready_again() {
    let (_dir_f, mut runtime_f, f) = hub("pass", "merge").await;
    let definition = per_ticket_pull_request(&runtime_f, false, 5000);
    let mut with_ready = definition.clone();
    let c = runtime_f.coding.clone().unwrap();
    with_ready.stages.insert(5, stage(&c, "ready", StageKind::PrReady, "coding-pr-ready/v1", ""));
    enqueue_item(&mut runtime_f, "w", "one", with_ready);
    until(&mut runtime_f, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_eq!((count(&f, "create"), count(&f, "ready")), (1, 0));
}

#[tokio::test]
async fn a_create_that_failed_after_it_worked_does_not_make_a_second_pull_request() {
    // `gh pr create` made the pull request and then exited non-zero: the lookup that follows is the answer.
    let (_dir_f, mut runtime_f, f) = hub("pass", "create-lost").await;
    let c = runtime_f.coding.clone().unwrap();
    let mut definition = per_ticket_pull_request(&runtime_f, false, 200);
    definition.stages[5] = route(route(waiting(&c, "merged", StageKind::AwaitPr, "coding-await-pr/v1", "", 200), "closed", Target::Fail, 0), "timeout", Target::Done, 0);
    definition.stages.truncate(6);
    enqueue_item(&mut runtime_f, "w", "one", definition);
    until(&mut runtime_f, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_eq!(prs(&f).len(), 1);
    assert_eq!(count(&f, "create"), 1);
}

#[tokio::test]
async fn a_step_run_again_after_a_crash_finds_the_pull_request_and_opens_none() {
    let (_dir_f, mut runtime_f, f) = hub("pass", "merge").await;
    let definition = per_ticket_pull_request(&runtime_f, false, 5000);
    enqueue_item(&mut runtime_f, "w", "one", definition);
    // Run to the open-pr step, open it, and lose the runner before it wrote its result.
    until(&mut runtime_f, |s| s.items.get("one").is_some_and(|i| i.definition.stages[i.stage].kind == StageKind::OpenPr && i.pending.is_some())).await;
    let binding = super::coding_tests::admitted(&runtime_f).await;
    assert!(matches!(binding.effect.kind, EffectKind::Execute(StageKind::OpenPr)));
    let (_hold, cancelled) = tokio::sync::watch::channel(false);
    runtime_f.execute_coding("w", &binding, cancelled).await.unwrap();
    assert_eq!(prs(&f).len(), 1);
    let root = runtime_f.root.clone(); drop(runtime_f);
    let mut runtime = Runtime::open(&root).unwrap();
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_eq!(prs(&f).len(), 1);
    assert_eq!(count(&f, "create"), 1);
}

#[tokio::test]
async fn a_requested_change_reworks_the_ticket_and_the_same_pull_request_is_waited_on_again() {
    let (_dir_f, mut runtime_f, f) = hub("pass", "rework").await;
    let definition = per_ticket_pull_request(&runtime_f, false, 5000);
    enqueue_item(&mut runtime_f, "w", "one", definition);
    // The reviewer asks for a change on the first commit. Mark the pull request reworked when the second commit is pushed.
    let first = until(&mut runtime_f, |s| s.items.get("one").is_some_and(|i| i.iteration == 1)).await;
    assert_eq!(first.items["one"].iteration, 1);
    let mut state: serde_json::Value = serde_json::from_slice(&std::fs::read(&f.state).unwrap()).unwrap();
    state["prs"][0]["reworked"] = true.into();
    state["prs"][0]["latestReviews"] = serde_json::json!([]);
    std::fs::write(&f.state, serde_json::to_vec(&state).unwrap()).unwrap();
    until(&mut runtime_f, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let all = prs(&f);
    assert_eq!(all.len(), 1, "the rework pushed to the same branch and found the same pull request");
    assert_eq!(count(&f, "create"), 1);
    assert_eq!(outbox(&runtime_f, StageKind::PushBranch), 2);
    assert_eq!(outbox(&runtime_f, StageKind::OpenPr), 2);
}

#[tokio::test]
async fn a_closed_pull_request_fails_the_ticket_and_a_merge_of_something_else_is_refused() {
    let (_dir_f, mut runtime_f, _f) = hub("pass", "close").await;
    let definition = per_ticket_pull_request(&runtime_f, false, 5000);
    enqueue_item(&mut runtime_f, "w", "one", definition);
    until(&mut runtime_f, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(outbox(&runtime_f, StageKind::AwaitCi), 0, "nothing waits on CI for a pull request that was never merged");

    let (_dir_g, mut runtime_g, _g) = hub("pass", "wrong-head").await;
    let definition = per_ticket_pull_request(&runtime_g, false, 5000);
    enqueue_item(&mut runtime_g, "w", "one", definition);
    for time in 2..200 { runtime_g.pump(time).await.unwrap(); }
    let state = runtime_g.state("w").unwrap().unwrap();
    assert_ne!(phase_of(&state, "one"), Some(Phase::Done), "a pull request whose head is not what was pushed never becomes the ticket's candidate");
    assert_eq!(outbox(&runtime_g, StageKind::AwaitCi), 0);
}

#[tokio::test]
async fn two_pull_requests_for_one_head_branch_are_refused_and_nothing_is_chosen() {
    let (_dir_f, mut runtime_f, f) = hub("pass", "two").await;
    let definition = per_ticket_pull_request(&runtime_f, false, 5000);
    enqueue_item(&mut runtime_f, "w", "one", definition);
    for time in 2..200 { runtime_f.pump(time).await.unwrap(); }
    let state = runtime_f.state("w").unwrap().unwrap();
    assert_ne!(phase_of(&state, "one"), Some(Phase::Done));
    assert_eq!(count(&f, "ready"), 0);
}

#[tokio::test]
async fn the_wait_gives_up_at_its_patience() {
    let (_dir_f, mut runtime_f, f) = hub("pass", "never").await;
    let mut definition = per_ticket_pull_request(&runtime_f, false, 150);
    definition.stages[5].routes.insert("timeout".into(), Target::Fail);
    enqueue_item(&mut runtime_f, "w", "one", definition);
    until(&mut runtime_f, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(prs(&f).len(), 1);
}

#[tokio::test]
async fn gh_runs_with_the_pinned_environment_and_nothing_inherited() {
    let (_dir_f, mut runtime_f, f) = hub("pass", "merge").await;
    unsafe { std::env::set_var("GH_TOKEN_LEAK_TEST", "secret"); }
    let definition = per_ticket_pull_request(&runtime_f, false, 5000);
    let mut definition = definition;
    definition.stages.truncate(6);
    definition.stages[5].routes.insert("timeout".into(), Target::Done);
    enqueue_item(&mut runtime_f, "w", "one", definition);
    until(&mut runtime_f, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    for line in std::fs::read_to_string(&f.log).unwrap().lines() {
        let env: Vec<String> = serde_json::from_str::<serde_json::Value>(line).unwrap()["env"].as_array().unwrap()
            .iter().map(|e| e.as_str().unwrap().to_string()).collect();
        assert!(!env.iter().any(|k| k.contains("LEAK")), "{env:?}");
        assert!(env.contains(&"GH_MODE".to_string()));
    }
    unsafe { std::env::remove_var("GH_TOKEN_LEAK_TEST"); }
}

#[tokio::test]
async fn a_definition_with_pull_request_steps_needs_the_pull_request_identity() {
    let (_dir, runtime) = configured(false, "pass").await;
    let c = runtime.coding.as_ref().unwrap();
    let mut with_branch = per_ticket_pull_request(&runtime, false, 5000);
    assert!(c.validate_definition(&with_branch).is_err(), "no pull request identity, no landing identity");
    with_branch.stages.truncate(4);
    assert!(c.validate_definition(&with_branch).is_err());
}
