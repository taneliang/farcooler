//! The landing verbs against a real git repository and a local bare repository standing in for the
//! remote. No network push is ever made (ov-956).
use super::*;
use super::coding_tests::{admitted, configured};
use super::concurrency_tests::{enqueue_item, hold_builds, watch};
use super::coding_verbs::{classify_push, PushVerdict};
use std::collections::BTreeMap;

const GATE: &str = r#"elif stage == 'gate':
    if mode == 'resource-hold' and not pathlib.Path(os.environ['MARKER']).exists():
        print('lane-gate: resource hold: insufficient disk; retry later', flush=True)
        sys.exit(75)
    if mode == 'gate-fail':
        print('landing gate stdout', flush=True)
        print('landing gate stderr', file=sys.stderr, flush=True)
        print('starting gate\nerror: first gate failure\nerror: later gate failure')
        sys.exit(1)
    if mode == 'named-breaker' and pathlib.Path('services/web/src/client/saved-copies.ts').exists():
        sys.stdout.buffer.write(pathlib.Path(os.environ['FAILURE_FIXTURE']).read_bytes())
        sys.exit(1)
    if mode == 'gate-dirty': pathlib.Path('stray.out').write_text('left behind')
    if mode == 'gate-slow':
        pathlib.Path('half-built.out').write_text('left by the gate')
        __import__('time').sleep(60)
    if mode == 'batch-fail':
        head = subprocess.check_output(['/usr/bin/git', '--git-dir', os.environ['REMOTE'], 'rev-parse', 'refs/heads/main']).decode().strip()
        if int(subprocess.check_output(['/usr/bin/git', 'rev-list', '--count', head + '..HEAD']).decode().strip()) > 1: sys.exit(5)
    if mode == 'move-base' and not pathlib.Path(os.environ['MARKER']).exists():
        pathlib.Path(os.environ['MARKER']).write_text('x')
        d = os.environ['SCRATCH']
        g = lambda *a: subprocess.check_call(['/usr/bin/git', '-C', d, *a])
        subprocess.check_call(['/usr/bin/git', 'clone', '-q', '-b', 'main', os.environ['REMOTE'], d])
        pathlib.Path(d, 'other.txt').write_text('other lander')
        g('add', 'other.txt'); g('-c', 'commit.gpgsign=false', 'commit', '-q', '-m', 'other lander'); g('push', '-q', 'origin', 'HEAD:refs/heads/main')
    if mode == 'path-join':
        tools = pathlib.Path.cwd() / 'node_modules' / '.bin'
        tools.mkdir(parents=True); (tools / 'tool').write_text('#!/bin/sh\nexit 0\n'); (tools / 'tool').chmod(0o755)
        found = subprocess.call('tool', shell=True, env={'PATH': str(tools) + os.pathsep + '/usr/bin:/bin'})
        __import__('shutil').rmtree(tools.parent)
        if found != 0: sys.exit(found)
    if mode == 'build':
        warm = pathlib.Path('target', 'built').exists()
        pathlib.Path('target').mkdir(exist_ok=True); pathlib.Path('target', 'built').write_text('x')
        with open(os.environ['MARKER'], 'a') as f: f.write('warm\n' if warm else 'cold\n')
    if mode == 'lane-env':
        seen = {k: os.environ.get(k) for k in ('USER', 'LOGNAME')}
        seen['python3'] = __import__('shutil').which('python3')
        seen['origin/main'] = subprocess.call(['/usr/bin/git', 'rev-parse', '--verify', '--quiet', 'refs/remotes/origin/main'], stdout=subprocess.DEVNULL)
        pathlib.Path(os.environ['MARKER']).write_text(json.dumps(seen))
    assert any(p.read_text().startswith('candidate') for p in pathlib.Path('.').glob('*.txt'))
"#;

/// A configured runtime whose landing target is a private bare repository. Every ticket's build writes a file of
/// its own, so tickets fold without clashing, and the second check is a gate that can move the remote under the group.
pub(super) async fn lander(mode: &str) -> (tempfile::TempDir, Runtime) {
    let (dir, runtime) = configured(false, mode).await;
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    let text = text
        .replace("    pathlib.Path('result.txt').write_text('candidate iteration", "    name = 'shared.txt' if mode == 'conflict' else 'result-' + os.environ['FARCOOLER_WORKFLOW_RUN'][:8] + '.txt'\n    pathlib.Path(name).write_text('candidate iteration")
        .replace("['/usr/bin/git', 'add', '--', 'result.txt']", "['/usr/bin/git', 'add', '--', name]")
        .replace("    assert pathlib.Path('result.txt').read_text().startswith('candidate')", "    assert any(p.read_text().startswith('candidate') for p in pathlib.Path('.').glob('*.txt'))")
        .replace("elif stage == 'review':\n", &format!("{GATE}elif stage == 'review':\n"))
        .replace("conclusion='failure' if mode == 'ci-fail' else 'success',", "conclusion='failure' if mode == 'ci-fail' else ('cancelled' if mode == 'ci-cancel' else 'success'),")
        // `UNRUN_SUBJECT`: a commit with this subject on the remote gets no CI run, as a push GitHub's path filter skips.
        .replace("    if mode == 'ci-missing': sys.exit(0)", "    if mode == 'ci-missing': sys.exit(0)\n    if os.environ.get('UNRUN_SUBJECT') and subprocess.run(['/usr/bin/git', '--git-dir', os.environ['REMOTE'], 'log', '-1', '--format=%s', candidate], capture_output=True, text=True).stdout.strip() == os.environ['UNRUN_SUBJECT']: sys.exit(0)")
        // `base-red`: CI failed on the base the remote starts at, and passes on anything pushed after it.
        .replace("conclusion='failure' if mode == 'ci-fail'", "conclusion='failure' if mode == 'ci-fail' or (mode == 'base-red' and candidate == os.environ['RED'])")
        // `base-pending`: CI on that base never finishes.
        .replace("status='in_progress' if mode == 'ci-pending'", "status='in_progress' if mode == 'ci-pending' or (mode == 'base-pending' and candidate == os.environ['RED'])");
    let text = text.replace("    pathlib.Path(name).write_text('candidate iteration",
        "    if mode == 'named-breaker' and os.environ['FARCOOLER_WORKFLOW_ITEM'] == 'breaker':\n        bad = pathlib.Path('services/web/src/client/saved-copies.ts')\n        bad.parent.mkdir(parents=True, exist_ok=True)\n        bad.write_text('extra copy')\n        subprocess.check_call(['/usr/bin/git', 'add', '--', str(bad)])\n    pathlib.Path(name).write_text('candidate iteration");
    assert!(text.contains("elif stage == 'gate'") && text.contains("name = 'shared.txt'") && text.contains("'cancelled'")
        && text.contains("UNRUN_SUBJECT") && text.contains("mode == 'base-red'") && text.contains("mode == 'base-pending'"));
    std::fs::write(&script, text).unwrap();
    let bare = dir.path().join("private-remote.git"); std::fs::create_dir(&bare).unwrap();
    adapters::git(&bare, &["init", "--bare", "--quiet"]).await.unwrap();
    let repo = runtime.repository();
    // Initial private remote population is test setup, never an external push.
    let status = tokio::process::Command::new("/usr/bin/git").current_dir(&repo)
        .args(["-c", "core.hooksPath=/dev/null", "push", bare.to_str().unwrap(), "HEAD:refs/heads/main"]).status().await.unwrap();
    assert!(status.success());
    let root = runtime.root.clone();
    let mut config = runtime.coding.clone().unwrap();
    config.landing = Some(CodingLanding { job: "landing-956".into(), repository: bare.canonicalize().unwrap(),
        target: "refs/heads/main".into(), config_digest: digest(&std::fs::read(bare.join("config")).unwrap()) });
    config.checks[1].argv[2] = "gate".into();
    config.ci.command.environment.insert("RED".into(), config.base.clone());
    config.ci.command.environment.insert("REMOTE".into(), bare.to_string_lossy().into_owned());
    for (key, value) in [("MARKER", dir.path().join("marker")), ("REMOTE", bare.clone()), ("SCRATCH", dir.path().join("other-lander"))] {
        config.checks[1].environment.insert(key.into(), value.to_string_lossy().into_owned());
    }
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    drop(runtime);
    (dir, Runtime::open(&root).unwrap())
}

fn stage(config: &CodingConfig, id: &str, kind: StageKind, version: &str, argument: &str) -> Stage {
    let mut s = config.stage(id, kind, version, argument); s.max_attempts = 2; s
}
fn waiting(config: &CodingConfig, id: &str, kind: StageKind, version: &str, argument: &str, patience_ms: u64) -> Stage {
    Stage { operation: config.wait_operation(version, argument, 20, patience_ms), max_attempts: 5, ..stage(config, id, kind, version, "") }
}
fn route(mut s: Stage, label: &str, to: Target, limit: u8) -> Stage { s.routes.insert(label.into(), to); s.limit = limit; s }
fn split() -> Target { Target::Split { step: "base".into(), otherwise: Box::new(Target::Fail) } }
/// provision, build, check, join(landing); and the group: base, fold, gate, push, ci.
pub(super) fn batched_push(runtime: &Runtime, base_mode: &str, patience_ms: u64) -> Definition {
    let c = runtime.coding.as_ref().unwrap();
    let mut join = stage(c, "join", StageKind::Join { group: "landing".into(), adopt: true }, "coding-join/v1", "landing");
    join.max_attempts = 1;
    let steps = vec![
        route(waiting(c, "base", StageKind::AwaitBase, "coding-await-base/v1", base_mode, patience_ms), "timeout", Target::Fail, 0),
        route(stage(c, "fold", StageKind::Fold, "coding-fold/v1", ""), "conflict", split(), 3),
        route(stage(c, "gate", StageKind::Run, "coding-gate/v1", "1"), "rejected", split(), 3),
        route(stage(c, "push", StageKind::Push, "coding-push/v1", "refs/heads/main"), "base_moved", Target::Step("base".into()), 2),
        route(route(route(waiting(c, "ci", StageKind::AwaitCi, "coding-await-ci/v1", "offline/repository", patience_ms),
            "rejected", Target::Fail, 0), "timeout", Target::Fail, 0), "cancelled", Target::Fail, 0),
    ];
    Definition { schema: 2, source: "landing-test/v1".into(), max_reworks: 0, rework_target: None,
        stages: vec![stage(c, "provision", StageKind::Provision, "coding-worktree/v1", ""),
            stage(c, "build", StageKind::Build, "coding-harness/v1", ""),
            stage(c, "check", StageKind::Check, "coding-check/v1", "0"), join],
        groups: [("landing".to_string(), GroupDef { limit: Some(4), exclusive: c.landing_resource().unwrap(), steps })].into(), models: Default::default(), send_backs: None, build_folders: None, script_policies: Default::default(), start_policy: None, limits: None, policies: Default::default() }
}
pub(super) fn command(runtime: &Runtime, command: Command) -> Receipt {
    let controller = runtime.controller();
    let state = runtime.state("w").unwrap().unwrap();
    let sequence = state.source_sequences.get("controller").copied().unwrap_or(0) + 1;
    // Fold fixtures can deliberately put a stale candidate into a group to exercise its fallback.
    runtime.submit_now(&controller, Request { id: format!("test-{sequence}"), workspace: "w".into(),
        expected_revision: state.revision, sequence, command }, 1).unwrap()
}
pub(super) fn phase_of(state: &State, id: &str) -> Option<Phase> {
    state.items.get(id).map(|i| i.phase.clone()).or_else(|| state.archive.get(id).map(|a| a.phase.clone()))
}
pub(super) async fn until(runtime: &mut Runtime, what: impl Fn(&State) -> bool) -> State {
    for time in 2..200 {
        runtime.pump(time).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        if what(&state) { return state; }
    }
    let state = runtime.state("w").unwrap().unwrap();
    let items: Vec<_> = state.items.iter().map(|(id, i)| format!("{id}: {:?} at {} attempt {}", i.phase, i.definition.stages[i.stage.min(i.definition.stages.len() - 1)].id, i.attempt)).collect();
    let effects: Vec<_> = runtime.store.workflow_outbox("w").unwrap().iter().map(|s| format!("{:?} {} {}", s.effect.kind, s.dispatch_state, s.active)).collect();
    let facts: Vec<_> = std::fs::read_dir(runtime.root.join("receipts")).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".verb.json"))
        .map(|p| String::from_utf8_lossy(&std::fs::read(p).unwrap()).chars().filter(|c| *c != '\n').collect::<String>().replace(|c: char| c == '{' && false, "")).collect();
    let fetch: Vec<_> = std::fs::read_dir(runtime.root.join("receipts")).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with("poll-0.fetch.output")).map(|p| std::fs::read_to_string(p).unwrap()).collect();
    eprintln!("FETCH {fetch:?}");
    panic!("runtime failed to settle:\n{}\n{}\n{}", items.join("\n"), effects.join("\n"), facts.iter().map(|f| f.split("\"detail\"").last().unwrap().to_string()).collect::<Vec<_>>().join("\n"));
}
/// Enqueue a ticket and run it to its join step.
pub(super) async fn at_join(runtime: &mut Runtime, item: &str, definition: &Definition) {
    enqueue_item(runtime, "w", item, definition.clone());
    until(runtime, |s| phase_of(s, item) == Some(Phase::WaitingJoin)).await;
}
pub(super) fn candidate_of<'a>(state: &'a State, id: &str) -> Option<&'a str> {
    state.items.get(id).and_then(|i| i.candidate.as_deref()).or_else(|| state.archive.get(id).and_then(|a| a.candidate.as_deref()))
}
fn group_id(state: &State) -> String {
    state.items.iter().chain(std::iter::empty()).find(|(_, i)| i.group_of.is_some()).map(|(id, _)| id.clone()).expect("a group is live")
}
pub(super) async fn remote_head(runtime: &Runtime) -> String {
    let landing = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap();
    adapters::git(&landing.repository, &["rev-parse", &landing.target]).await.unwrap()
}
pub(super) fn outbox(runtime: &Runtime, kind: StageKind) -> usize {
    runtime.store.workflow_outbox("w").unwrap().iter().filter(|s| s.effect.kind == EffectKind::Execute(kind.clone())).count()
}

#[tokio::test]
async fn a_ticket_lands_through_a_group_and_adopts_the_pushed_commit() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let pushed = remote_head(&runtime).await;
    assert_ne!(pushed, first);
    assert_eq!(candidate_of(&state, "one"), Some(pushed.as_str()), "the ticket adopts what was pushed");
    let parent = adapters::git(&runtime.repository(), &["rev-parse", &format!("{pushed}^")]).await.unwrap();
    assert_eq!(parent, first);
    let message = adapters::git(&runtime.repository(), &["log", "-1", "--format=%B", &pushed]).await.unwrap();
    assert!(message.contains("(cherry picked from commit"), "{message}");
    assert_eq!(adapters::git(&runtime.repository(), &["rev-parse", runtime.landing_ref()]).await.unwrap(), pushed,
        "the local copy of the target follows a push that started from it");
    let landing = runtime.coding.as_ref().unwrap().landing.clone().unwrap();
    assert_eq!(adapters::git(&landing.repository, &["rev-list", "--count", &landing.target]).await.unwrap(), "2", "the base and the ticket's one commit");
    for kind in [StageKind::AwaitBase, StageKind::Fold, StageKind::Run, StageKind::Push, StageKind::AwaitCi] {
        assert_eq!(outbox(&runtime, kind.clone()), 1, "{kind:?} ran once");
    }
}

#[tokio::test]
async fn a_base_that_moves_under_the_gate_is_retried_from_the_new_base_and_wedges_nothing() {
    let (_dir, mut runtime) = lander("move-base").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    // The other lander's commit and this ticket's both reached the target, the other one first.
    let pushed = remote_head(&runtime).await;
    let landing = runtime.coding.as_ref().unwrap().landing.clone().unwrap();
    let history = adapters::git(&landing.repository, &["log", "--format=%s", &landing.target]).await.unwrap();
    assert!(history.lines().any(|l| l == "other lander"), "{history}");
    assert_eq!(candidate_of(&state, "one"), Some(pushed.as_str()));
    assert_eq!(outbox(&runtime, StageKind::Push), 2, "the first push was refused before it was sent, the second landed");
    assert_eq!(outbox(&runtime, StageKind::AwaitBase), 2);
    // The repository is free: the next ticket lands without anyone clearing anything.
    at_join(&mut runtime, "two", &definition).await;
    command(&runtime, Command::Queue { item: "two".into() });
    until(&mut runtime, |s| phase_of(s, "two") == Some(Phase::Done)).await;
    assert_ne!(remote_head(&runtime).await, pushed);
}

#[tokio::test]
async fn a_push_the_remote_refuses_is_a_moved_base_and_a_push_that_may_have_landed_is_uncertain() {
    // Refused at the remote after the lease check passed: nothing landed, so it is safe to start over.
    assert_eq!(classify_push(Some(1), "To x\n!\tabc:refs/heads/main\t[rejected] (stale info)\n", Some("b0"), "c1"), PushVerdict::Refused);
    // Refused, yet the candidate is there: we cannot say what happened.
    assert_eq!(classify_push(Some(1), "!\tabc:refs/heads/main\t[remote rejected]\n", Some("c1"), "c1"), PushVerdict::Unknown);
    // The remote cannot be read afterwards.
    assert_eq!(classify_push(Some(1), "!\tx\n", None, "c1"), PushVerdict::Unknown);
    // A failure with no porcelain refusal (a lost connection) and a killed push stay unknown.
    assert_eq!(classify_push(Some(128), "fatal: unable to access\n", Some("b0"), "c1"), PushVerdict::Unknown);
    assert_eq!(classify_push(None, "", Some("b0"), "c1"), PushVerdict::Unknown);
    // A clean exit counts only if the remote has the candidate.
    assert_eq!(classify_push(Some(0), "", Some("c1"), "c1"), PushVerdict::Landed);
    assert_eq!(classify_push(Some(0), "", Some("b0"), "c1"), PushVerdict::Unknown);
}

#[tokio::test]
async fn a_conflict_returns_the_ticket_to_build_with_paths_and_frees_the_target() {
    let (dir, mut runtime) = lander("conflict").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    // Someone lands a different `shared.txt` first.
    let other = dir.path().join("other");
    let landing = runtime.coding.as_ref().unwrap().landing.clone().unwrap();
    for args in [vec!["clone", "-q", "-b", "main", landing.repository.to_str().unwrap(), other.to_str().unwrap()]] {
        assert!(tokio::process::Command::new("/usr/bin/git").args(args).status().await.unwrap().success());
    }
    std::fs::write(other.join("shared.txt"), "someone else's").unwrap();
    for args in [vec!["add", "shared.txt"], vec!["-c", "user.name=O", "-c", "user.email=o@localhost", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "someone else"],
        vec!["push", "-q", "origin", "HEAD:refs/heads/main"]] {
        assert!(tokio::process::Command::new("/usr/bin/git").current_dir(&other).args(args).status().await.unwrap().success());
    }
    let theirs = remote_head(&runtime).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| s.items["one"].brief.is_some()).await;
    let ticket = &state.items["one"];
    assert_eq!(ticket.phase, Phase::Running);
    assert_eq!(ticket.definition.stages[ticket.stage].kind, StageKind::Build);
    assert!(ticket.back_edges.is_empty());
    assert_eq!(ticket.iteration, 0);
    let build = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .find(|s| s.effect.item == "one" && s.effect.brief.is_some()).unwrap();
    let brief = build.effect.brief.unwrap();
    assert!(brief.contains("shared.txt") && brief.contains(&theirs), "{brief}");
    assert_eq!(remote_head(&runtime).await, theirs, "a conflicting group pushes nothing");
    assert_eq!(outbox(&runtime, StageKind::Push), 0);
    assert_eq!(outbox(&runtime, StageKind::Run), 0, "the fold stopped the group; the gate never ran on a half-folded tree");
    let group = state.items.iter().find(|(_, i)| i.group_of.is_some()).map(|(_, i)| i.phase.clone());
    assert_eq!(group, Some(Phase::Failed));
    // A failed fold releases its checkout; its durable receipt still reconciles (ov-1351).
    let tree = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.effect.kind == EffectKind::Execute(StageKind::Fold)).unwrap();
    let binding = Binding::new("w", &tree.effect, tree.resource_generation.unwrap(), 0,
        &runtime.item_base("w", &tree.effect).unwrap());
    let recovered = runtime.reconcile_verb(&binding, &StageKind::Fold).await.unwrap();
    assert!(matches!(recovered, Outcome::FoldConflict { ref evidence, ref files, .. }
        if evidence.candidate == theirs && files == &["shared.txt".to_string()]), "{recovered:?}");
    let tree = runtime.tree(&binding);
    assert!(!tree.exists(), "{}", tree.display());
}

#[tokio::test]
async fn a_failing_gate_stops_the_group_before_any_push() {
    let (_dir, mut runtime) = lander("gate-fail").await;
    let definition = batched_push(&runtime, "any", 5000);
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(remote_head(&runtime).await, first);
    assert_eq!(outbox(&runtime, StageKind::Push), 0);
}

#[tokio::test]
async fn a_push_recovered_after_a_crash_settles_from_its_fact_and_is_never_repeated() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    // base, fold, gate have run; the push is next.
    for time in 2..5 { runtime.pump(time).await.unwrap(); }
    let binding = admitted(&runtime).await;
    assert!(matches!(binding.effect.kind, EffectKind::Execute(StageKind::Push)));
    let (_hold, cancelled) = tokio::sync::watch::channel(false);
    runtime.execute_coding("w", &binding, cancelled).await.unwrap();
    let pushed = remote_head(&runtime).await;
    let log = std::fs::read(runtime.remote_log(&binding)).unwrap();
    // The runner died before it wrote the result.
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_eq!(remote_head(&runtime).await, pushed, "recovery pushed nothing more");
    assert_eq!(std::fs::read(runtime.remote_log(&binding)).unwrap(), log);
    assert_eq!(candidate_of(&state, "one"), Some(pushed.as_str()));
}

#[tokio::test]
async fn a_push_with_no_settlement_after_a_crash_stays_uncertain_and_sends_nothing() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..5 { runtime.pump(time).await.unwrap(); }
    let binding = admitted(&runtime).await; // admitted, then the runner died
    assert!(matches!(binding.effect.kind, EffectKind::Execute(StageKind::Push)));
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(50).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items[&group_id(&state)].phase, Phase::Uncertain);
    assert_eq!(remote_head(&runtime).await, first);
    assert_eq!(runtime.pump(51).await.unwrap(), 0);
    let waiting = runtime.root.join("receipts").join(format!("{}.push-waiting.json", digest(&binding.effect)));
    let mut foreign = binding.clone(); foreign.workspace = "other".into();
    for raw in [b"{".to_vec(), serde_json::to_vec(&foreign).unwrap()] {
        std::fs::write(&waiting, raw).unwrap();
        assert!(runtime.reconcile_verb(&binding, &StageKind::Push).await.is_err(), "damaged or foreign pre-send records prove nothing");
    }
    std::fs::write(&waiting, serde_json::to_vec(&binding).unwrap()).unwrap();
    assert!(matches!(runtime.reconcile_verb(&binding, &StageKind::Push).await.unwrap(), Outcome::Interrupted));
    // Even a partial send-intent write must close recovery's retry path.
    std::fs::write(runtime.root.join("receipts").join(format!("{}.push-send.json", digest(&binding.effect))), b"{").unwrap();
    assert!(runtime.reconcile_verb(&binding, &StageKind::Push).await.is_err());
    let (_hold, cancel) = tokio::sync::watch::channel(false);
    assert!(runtime.execute_coding("w", &binding, cancel).await.is_err(), "re-entry cannot resend an attempted push");
    assert_eq!(remote_head(&runtime).await, first);
}

#[tokio::test]
async fn waiting_for_a_base_whose_ci_is_pending_times_out_and_waiting_for_ci_that_never_ends_times_out() {
    for (mode, base_mode, label) in [("base-pending", "green", "base"), ("ci-pending", "any", "ci")] {
        let (_dir, mut runtime) = lander(mode).await;
        let first = remote_head(&runtime).await;
        let definition = batched_push(&runtime, base_mode, 300);
        at_join(&mut runtime, "one", &definition).await;
        command(&runtime, Command::Queue { item: "one".into() });
        until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
        // `timeout` is an ordinary label here, routed to fail.
        let landed = outbox(&runtime, StageKind::Push);
        assert_eq!(landed, usize::from(label == "ci"), "{mode}: a pending base never pushes; a CI timeout comes after the push");
        assert_eq!(remote_head(&runtime).await == first, label == "base", "{mode}");
    }
}

/// A fix for red main: the base's CI failed, so waiting longer can't turn it green. The group folds
/// onto it anyway, the gate judges the folded commit, and CI after the push decides (ov-1076).
#[tokio::test]
async fn a_batch_on_a_base_whose_ci_failed_folds_gates_and_lands_and_ci_after_the_push_decides() {
    for (mode, after) in [("base-red", Phase::Done), ("ci-fail", Phase::Failed)] {
        let (_dir, mut runtime) = lander(mode).await;
        let first = remote_head(&runtime).await;
        let definition = batched_push(&runtime, "green", 5000);
        at_join(&mut runtime, "one", &definition).await;
        command(&runtime, Command::Queue { item: "one".into() });
        let state = until(&mut runtime, |s| phase_of(s, "one") == Some(after.clone())).await;
        for kind in [StageKind::Fold, StageKind::Run, StageKind::Push, StageKind::AwaitCi] {
            assert_eq!(outbox(&runtime, kind.clone()), 1, "{mode}: {kind:?} ran once on the red base");
        }
        let pushed = remote_head(&runtime).await;
        assert_ne!(pushed, first, "{mode}: the fix reached the remote");
        let remote = &runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository;
        assert!(adapters::git(remote, &["merge-base", "--is-ancestor", &first, &pushed]).await.is_ok(), "{mode}: folded onto the red base");
        if after == Phase::Done { assert_eq!(candidate_of(&state, "one"), Some(pushed.as_str())); }
    }
}

#[tokio::test]
async fn ci_canceled_after_the_push_is_the_cancelled_label() {
    let (_dir, mut runtime) = lander("ci-cancel").await;
    let mut definition = batched_push(&runtime, "any", 5000);
    // Route cancelled back to nothing useful: the point is that the label reaches the definition's route.
    let steps = &mut definition.groups.get_mut("landing").unwrap().steps;
    steps[4].routes.insert("cancelled".into(), Target::Done);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_eq!(candidate_of(&state, "one"), Some(remote_head(&runtime).await.as_str()));
}

#[tokio::test]
async fn a_wait_leaves_the_work_pool_alone() {
    let (dir, mut runtime) = lander("ci-pending").await;
    // One work place, and a CI wait that stays held for seconds. A wait does no work, so it must never take a place:
    // if `Pool::of` gave it one, the second ticket's check, a work effect, could not start until the wait ended.
    runtime.places.lock().unwrap().work = 1;
    let definition = batched_push(&runtime, "any", 6000);
    at_join(&mut runtime, "one", &definition).await;
    let (hold, go) = hold_builds(dir.path());
    std::fs::write(&hold, "x").unwrap();
    enqueue_item(&mut runtime, "w", "two", definition);
    command(&runtime, Command::Queue { item: "one".into() });
    runtime.continuous = true;
    // Two's build is held until one's group is at its CI wait, then released: its check must run beside the wait.
    let send = runtime.send.clone();
    let watcher = tokio::spawn(async move {
        let mut released = false;
        let mut ended = false;
        let seen = wait_until_async!(tokio::time::Instant::now() + Duration::from_secs(60),
            Duration::from_millis(50), async || {
                if ended { return true; }
                let (reply, state) = oneshot::channel();
                send.send(Inbox::State("w".into(), reply)).await.ok().unwrap();
                let Ok(state) = tokio::time::timeout(Duration::from_secs(2), state).await else { ended = true; return true };
                if let Some(s) = state.unwrap().unwrap() {
                    let waiting = s.items.values().any(|i| i.group_of.is_some() && i.stage >= 4);
                    if waiting && !released { std::fs::write(&go, "x").unwrap(); released = true; }
                    return released && waiting && phase_of(&s, "two") == Some(Phase::WaitingJoin);
                }
                false
            }) && !ended;
        std::fs::write(go, "x").unwrap();
        seen
    });
    // Bounded, so a wait that wrongly held the work place fails this test instead of stalling the sweep.
    let sweep = tokio::time::timeout(Duration::from_secs(70), runtime.pump(301)).await;
    assert!(watcher.await.unwrap(), "two's check ran while one's CI wait was held");
    sweep.expect("the sweep ended").unwrap();
}

#[tokio::test]
async fn a_landing_group_folds_while_a_build_holds_every_agent_place() {
    let (dir, mut runtime) = lander("pass").await;
    let (hold, go) = hold_builds(dir.path());
    runtime.set_effect_cap(1);
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    enqueue_item(&mut runtime, "w", "two", definition);
    runtime.pump(300).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["two"].stage, 1, "two's build is next");
    std::fs::write(&hold, "x").unwrap();
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    // The build was queued first and holds the one agent place; the group's fold, gate and push must not wait on it.
    runtime.continuous = true;
    let watcher = watch(&runtime, go, |s| s.items.values().any(|i| i.group_of.is_some() && i.stage >= 4));
    runtime.pump(301).await.unwrap();
    assert!(watcher.await.unwrap(), "the group reached CI while the build held its place");
}

#[tokio::test]
async fn a_definition_must_pin_the_landing_target_and_sound_waits() {
    let (_dir, runtime) = lander("pass").await;
    let config = runtime.coding.clone().unwrap();
    let good = batched_push(&runtime, "any", 5000);
    assert!(config.validate_definition(&good).is_ok());
    let mut other_target = good.clone();
    other_target.groups.get_mut("landing").unwrap().exclusive = "ref:origin:main".into();
    assert!(config.validate_definition(&other_target).is_err(), "a group must hold the landing target's own resource");
    let mut hurried = good.clone();
    hurried.groups.get_mut("landing").unwrap().steps[0].operation = config.wait_operation("coding-await-base/v1", "any", 1, 5000);
    assert!(config.validate_definition(&hurried).is_err(), "polling every millisecond is not allowed");
    let mut forever = good.clone();
    forever.groups.get_mut("landing").unwrap().steps[4].operation = config.wait_operation("coding-await-ci/v1", "offline/repository", 20, u64::MAX / 2);
    assert!(config.validate_definition(&forever).is_err());
    let mut foreign = good.clone();
    foreign.groups.get_mut("landing").unwrap().steps[3].operation.arguments[1] = "refs/heads/other".into();
    assert!(config.validate_definition(&foreign).is_err(), "a push is pinned to the landing target");
    let mut ungated = good.clone();
    ungated.groups.get_mut("landing").unwrap().steps[2].operation = config.operation("coding-gate/v1", "9");
    assert!(config.validate_definition(&ungated).is_err(), "a gate names a configured check");
    // Without a landing identity there is nothing to wait for or push to.
    let mut local = config.clone(); local.landing = None;
    assert!(local.validate_definition(&good).is_err());
    let _: BTreeMap<String, String> = BTreeMap::new();
}

/// Run the group to its push, admit it and leave the runner "crashed" with no settlement.
async fn uncertain_push(sent: bool) -> (tempfile::TempDir, Runtime, String) {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..5 { runtime.pump(time).await.unwrap(); }
    let binding = admitted(&runtime).await;
    let (_hold, cancelled) = tokio::sync::watch::channel(false);
    if sent {
        runtime.execute_coding("w", &binding, cancelled).await.unwrap();
        std::fs::remove_file(runtime.verb_path(&binding)).unwrap(); // the push landed, the word that it did was lost
    }
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(50).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    let group = group_id(&state);
    assert_eq!(state.items[&group].phase, Phase::Uncertain);
    (_dir, runtime, group)
}

#[tokio::test]
async fn a_person_settles_a_lost_push_that_did_land() {
    let (_dir, mut runtime, group) = uncertain_push(true).await;
    let controller = runtime.controller();
    assert!(runtime.settle_push(&controller, "w", &group, false).await.is_err(), "the commit is on the remote, so it did not fail to land");
    runtime.settle_push(&controller, "w", &group, true).await.unwrap();
    let pushed = remote_head(&runtime).await;
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_eq!(candidate_of(&state, "one"), Some(pushed.as_str()));
    assert_eq!(outbox(&runtime, StageKind::Push), 1, "settling pushed nothing");
}

#[tokio::test]
async fn a_person_settles_a_lost_push_that_did_not_land_and_the_group_starts_over() {
    let (_dir, mut runtime, group) = uncertain_push(false).await;
    let controller = runtime.controller();
    assert!(runtime.settle_push(&controller, "w", &group, true).await.is_err(), "the remote does not have the commit, so it did not land");
    let first = remote_head(&runtime).await;
    runtime.settle_push(&controller, "w", &group, false).await.unwrap();
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_ne!(remote_head(&runtime).await, first);
    assert_eq!(outbox(&runtime, StageKind::Push), 2, "the second push is a new one from a fresh base");
    assert_eq!(candidate_of(&state, "one"), Some(remote_head(&runtime).await.as_str()));
}

#[tokio::test]
async fn only_an_uncertain_push_can_be_settled() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = runtime.state("w").unwrap().unwrap();
    let group = group_id(&state);
    let controller = runtime.controller();
    assert!(runtime.settle_push(&controller, "w", &group, true).await.is_err(), "a running group is not uncertain");
    assert!(runtime.settle_push(&controller, "w", "one", true).await.is_err(), "a ticket is not a push");
    assert!(runtime.settle_push(&controller, "w", "nobody", true).await.is_err());
}

// ov-956 review blockers.

#[tokio::test]
async fn a_rejected_gate_splits_a_batch_and_both_halves_land() {
    let (_dir, mut runtime) = lander("batch-fail").await;
    let definition = batched_push(&runtime, "any", 5000);
    let first = remote_head(&runtime).await;
    for item in ["x", "a", "b"] { at_join(&mut runtime, item, &definition).await; }
    // `x` takes the target; `a` and `b` queue behind it and form one group.
    for item in ["x", "a", "b"] { assert!(command(&runtime, Command::Queue { item: item.into() }).result.is_ok()); }
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items[&state.items["a"].grouped_in.clone().unwrap()].members.len(), 2);
    for item in ["x", "a", "b"] { until(&mut runtime, |s| phase_of(s, item) == Some(Phase::Done)).await; }
    let landing = runtime.coding.as_ref().unwrap().landing.clone().unwrap();
    assert_eq!(adapters::git(&landing.repository, &["rev-list", "--count", &format!("{first}..{}", landing.target)]).await.unwrap(), "3",
        "each ticket's commit landed");
    assert_eq!(outbox(&runtime, StageKind::Run), 4, "x, then the pair (rejected), then each half");
    assert_eq!(outbox(&runtime, StageKind::Push), 3);
}

#[tokio::test]
async fn a_failing_gate_is_rejected_on_the_first_run_and_not_retried_as_a_crash() {
    let (_dir, mut runtime) = lander("gate-fail").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    let failure = state.items.values().find(|i| i.group_of.is_some()).unwrap().group_failure.as_ref().unwrap();
    assert!(failure.job.contains("gate"), "the configured gate command is the job");
    assert_eq!(failure.first_line, "error: first gate failure");
    assert!(!failure.split, "a one-member fallback failed without splitting");
    assert_eq!(outbox(&runtime, StageKind::Run), 1, "a failing exit is the candidate's answer: one run, not one per attempt");
    let reply = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    let logs = reply["process_logs"].as_array().unwrap();
    let log = logs.iter().find(|log| log["stage"] == serde_json::json!({"Execute":"Run"})).unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(log["item"], group_id(&state));
    assert_eq!(log["exit"], 1);
    let output = std::fs::read_to_string(log["log_path"].as_str().unwrap()).unwrap();
    assert!(output.contains("landing gate stdout") && output.contains("landing gate stderr"));
    assert_eq!(log["tail"], output.trim_end());
}

#[tokio::test]
async fn a_gate_that_leaves_the_tree_dirty_rejects_the_candidate_and_wedges_nothing() {
    let (_dir, mut runtime) = lander("gate-dirty").await;
    let definition = batched_push(&runtime, "any", 5000);
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(remote_head(&runtime).await, first);
    assert_eq!(state.items[&group_id(&state)].phase, Phase::Failed, "failed, not uncertain: the target is free");
}

#[tokio::test]
async fn a_gate_left_running_by_a_dead_daemon_is_ended_and_run_again() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..4 { runtime.pump(time).await.unwrap(); }
    let binding = admitted(&runtime).await;
    assert!(matches!(binding.effect.kind, EffectKind::Execute(StageKind::Run)));
    // The daemon died with the gate running: its process group lives on, recorded, with no result.
    let mut gate = tokio::process::Command::new("/bin/sleep").arg("600").process_group(0).spawn().unwrap();
    let pid = gate.id().unwrap();
    crate::effect_runner::Owner::write(&runtime.owner_path(&binding), pid).unwrap();
    std::fs::write(runtime.tree(&binding).join("half-built.txt"), "left by the gate").unwrap();
    let reaped = tokio::spawn(async move { gate.wait().await });
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    // Waiting on it proves it ended: a gate nobody ended would sleep for ten minutes.
    tokio::time::timeout(Duration::from_secs(5), reaped).await.expect("the old gate was not ended").unwrap().unwrap();
    assert_ne!(unsafe { libc::killpg(pid as i32, 0) }, 0, "the old gate's process group is gone");
    assert_eq!(outbox(&runtime, StageKind::Run), 2, "the gate ran again on a clean tree");
}

/// ov-979: a Run gate in a tree the app lists has what changes noted as it runs. A person works in the tree
/// while the gate runs, then the daemon dies. On restart the run stops and lists every change, names what the
/// ledger saw (the person's file too: a look can't tell who wrote it) as changed while the gate ran, and removes nothing.
/// Once the person has discarded them, the run goes on. Removing the call in `execute_coding` turns the
/// private-tree test above red; this one runs the named branch.
#[tokio::test]
async fn a_run_gate_restarting_in_a_named_tree_keeps_a_persons_changes() {
    let (_dir, mut runtime) = lander("gate-slow").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..4 { runtime.pump(time).await.unwrap(); }
    let mut binding = admitted(&runtime).await;
    let path = runtime.tree(&binding);
    binding.effect.tree = Some(RunTree { name: "n".into(), branch: "b".into(), path: path.to_string_lossy().into_owned(), base: binding.base.clone(), adopted: false });
    let ledger = runtime.gate_files_path(&binding);
    let (exec, running) = (runtime.exec.clone(), binding.clone());
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let gate = tokio::spawn(async move { exec.execute_coding("w", &running, cancelled).await });
    let noted = |name: &str| std::fs::read_to_string(&ledger).is_ok_and(|l| l.contains(name));
    tokio::time::timeout(Duration::from_secs(30), async { loop {
        if noted("half-built.out") { break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }}).await.expect("the running gate's file was noted");
    // A person works in the tree while the gate runs, and the daemon notes it.
    std::fs::write(path.join("mine.txt"), "a person's work").unwrap();
    tokio::time::timeout(Duration::from_secs(30), async { loop {
        if noted("mine.txt") { break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }}).await.expect("the person's file was noted");
    // The daemon dies mid-gate; its gate is ended on restart. The ledger outlives them both.
    gate.abort();
    assert!(gate.await.unwrap_err().is_cancelled());
    assert!(crate::effect_runner::Owner::reap(&runtime.owner_path(&binding)).await.unwrap());
    assert!(ledger.exists());
    let error = runtime.settle_run_tree(&binding, &path).await.unwrap_err().to_string();
    assert!(error.contains("has changes, and it left them as they are: half-built.out, mine.txt. Changed while its gate was running, and not since: half-built.out, mine.txt."), "{error}");
    assert_eq!(std::fs::read_to_string(path.join("mine.txt")).unwrap(), "a person's work", "the person's work stays");
    assert!(path.join("half-built.out").exists(), "and nothing is cleaned");
    // The app reads why the run stopped, with the list, beside the run's state.
    let stops = |runtime: &Runtime| runtime.wire_command("w", "p", farcooler_protocol::workflow_commands::Action::Get).unwrap()["stops"].clone();
    let stop = &stops(&runtime)[&binding.effect.item];
    assert_eq!(stop["run"], binding.effect.run.as_str());
    assert!(stop["text"].as_str().is_some_and(|t| t.starts_with("The run stopped") && t.contains("half-built.out, mine.txt")), "{stop}");
    // The person discards both.
    std::fs::remove_file(path.join("mine.txt")).unwrap();
    std::fs::remove_file(path.join("half-built.out")).unwrap();
    runtime.settle_run_tree(&binding, &path).await.unwrap();
    assert!(!ledger.exists());
    assert!(stops(&runtime).get(&binding.effect.item).is_none(), "a clean tree takes the stop away");
}

/// ov-979: a gate that ends, however it ends, takes its ledger with it, so a later gate never reads a stale one.
#[tokio::test]
async fn a_named_tree_s_gate_that_ends_leaves_no_ledger() {
    let (_dir, mut runtime) = lander("gate-dirty").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..4 { runtime.pump(time).await.unwrap(); }
    let mut binding = admitted(&runtime).await;
    let path = runtime.tree(&binding);
    binding.effect.tree = Some(RunTree { name: "n".into(), branch: "b".into(), path: path.to_string_lossy().into_owned(), base: binding.base.clone(), adopted: false });
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    runtime.execute_coding("w", &binding, cancelled).await.unwrap();
    assert!(path.join("stray.out").exists());
    assert!(!runtime.gate_files_path(&binding).exists());
    assert!(runtime.settle_run_tree(&binding, &path).await.is_err(), "what a finished gate left is a person's to judge");
    assert!(path.join("stray.out").exists());
}

#[tokio::test]
async fn a_group_whose_leader_is_killed_but_not_yet_reaped_is_not_an_error() {
    // macOS answers EPERM to killpg(pid, 0) while only a zombie is left; that is a group on its way out.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owner.json");
    let mut gate = tokio::process::Command::new("/bin/sleep").arg("600").process_group(0).spawn().unwrap();
    let pid = gate.id().unwrap();
    crate::effect_runner::Owner::write(&path, pid).unwrap();
    unsafe { libc::kill(pid as i32, libc::SIGKILL); }
    // Wait until the zombie shows (killpg no longer says alive), without reaping it.
    for _ in 0..200 { if unsafe { libc::killpg(pid as i32, 0) } != 0 { break; } tokio::time::sleep(Duration::from_millis(5)).await; }
    let reaper = tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(100)).await; gate.wait().await });
    assert!(crate::effect_runner::Owner::reap(&path).await.is_ok(), "a zombie-only group is not an error");
    reaper.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_pid_that_belongs_to_someone_else_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owner.json");
    let mut other = tokio::process::Command::new("/bin/sleep").arg("600").process_group(0).spawn().unwrap();
    let pid = other.id().unwrap();
    std::fs::write(&path, serde_json::to_vec(&serde_json::json!({ "pid": pid, "started": "Thu Jan  1 00:00:00 1970" })).unwrap()).unwrap();
    assert!(crate::effect_runner::Owner::reap(&path).await.unwrap(), "nothing of ours is left");
    assert_eq!(unsafe { libc::killpg(pid as i32, 0) }, 0, "a process that started at another time is not ours to kill");
    other.kill().await.unwrap();
}

#[tokio::test]
async fn a_restart_during_ci_looks_again_and_the_pushed_group_ends_as_landed() {
    let (_dir, mut runtime) = lander("ci-pending").await;
    let mut definition = batched_push(&runtime, "any", 400);
    definition.groups.get_mut("landing").unwrap().steps[4].max_attempts = 1;
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..6 { runtime.pump(time).await.unwrap(); }
    let binding = admitted(&runtime).await;
    assert!(matches!(binding.effect.kind, EffectKind::Execute(StageKind::AwaitCi)));
    let pushed = remote_head(&runtime).await;
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    // CI never finishes, so the wait times out: the group fails, after the push.
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(outbox(&runtime, StageKind::AwaitCi), 2, "the restart cost no attempt: CI was looked at again");
    assert_eq!(candidate_of(&state, "one"), Some(pushed.as_str()), "the ticket carries what landed, and was not told `rejected`");
    assert_eq!(state.items["one"].back_edges.get("join"), None);
}

#[tokio::test]
async fn a_push_that_fails_its_checks_before_sending_is_tried_again_and_never_uncertain() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..5 { runtime.pump(time).await.unwrap(); }
    // The fold's record is gone: the push cannot show what it was built on, so it sends nothing.
    for entry in std::fs::read_dir(runtime.root.join("receipts")).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap().to_string_lossy().starts_with("fold-") { std::fs::remove_file(path).unwrap(); }
    }
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(remote_head(&runtime).await, first);
    assert_eq!(state.items[&group_id(&state)].phase, Phase::Failed, "failed and free, not uncertain");
    assert_eq!(outbox(&runtime, StageKind::Push), 2, "tried again, up to its attempts");
}

#[tokio::test]
async fn a_remote_that_cannot_be_read_settles_nothing() {
    let (dir, runtime, group) = uncertain_push(false).await;
    let landing = runtime.coding.as_ref().unwrap().landing.clone().unwrap();
    std::fs::rename(&landing.repository, dir.path().join("moved-away.git")).unwrap();
    let controller = runtime.controller();
    // Unreadable is not "absent": saying the push did not land must not be believed.
    assert!(runtime.settle_push(&controller, "w", &group, false).await.is_err());
    assert!(runtime.settle_push(&controller, "w", &group, true).await.is_err());
    std::fs::rename(dir.path().join("moved-away.git"), &landing.repository).unwrap();
    runtime.settle_push(&controller, "w", &group, false).await.unwrap();
}

#[tokio::test]
async fn the_wire_settles_a_lost_push_for_the_exact_run_and_refuses_a_stale_one() {
    let (_dir, mut runtime, group) = uncertain_push(true).await;
    let state = runtime.state("w").unwrap().unwrap();
    let item = &state.items[&group];
    let stale = runtime.settle_wire("w", &group, "old-run", &item.pin, true, state.epoch).await.unwrap();
    assert_eq!(stale.pointer("/receipt/result/Err").and_then(|v| v.as_str()), Some("Stale"), "{stale}");
    let wrong_remote = runtime.settle_wire("w", &group, &item.run, &item.pin, false, state.epoch).await.unwrap();
    assert!(wrong_remote.pointer("/receipt/result/Err").is_some(), "the remote has the commit: {wrong_remote}");
    let settled = runtime.settle_wire("w", &group, &item.run, &item.pin, true, state.epoch).await.unwrap();
    assert!(settled.pointer("/receipt/result/Ok").is_some(), "{settled}");
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
}

/// The gate runs a tool from `node_modules/.bin`, found through PATH as npm finds it. A group that was mid-flight in a
/// tree under the old `tree-wf1:<hex>` name is folded again under the new one, so its gate can find it (ov-1081).
#[tokio::test]
async fn a_gate_finds_a_tool_on_a_path_through_its_tree_even_after_an_old_tree() {
    let (_dir, mut runtime) = lander("path-join").await;
    let definition = batched_push(&runtime, "any", 5000);
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| s.items.values().any(|i| i.group_of.is_some())).await;
    let group = group_id(&state);
    let legacy = runtime.root.join(format!("tree-{}", digest(&("w", &group))));
    assert!(!legacy.exists() && !runtime.item_tree("w", &group).exists(), "planted before the fold ran");
    adapters::git(&runtime.repository(), &["worktree", "add", "--detach", legacy.to_str().unwrap(), &first]).await.unwrap();
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_ne!(remote_head(&runtime).await, first, "the gate passed and the group landed");
    assert!(!legacy.exists(), "the fold removed the tree under the old name");
}

/// A group's gate runs `scripts/landing-gate` in the fold tree, so it sees what a hand-run lane gate sees there
/// (ov-1083): the runner's account in USER and LOGNAME, which a test that ssh-es back to this runner reads; the
/// account's own tools in ~/.local/bin ahead of the system's, so `python3` is the one a login shell finds and not
/// macOS's 3.9; and the repository's own refs, `origin/main` among them, which lane-gate.py diffs against.
#[tokio::test]
async fn a_gate_in_the_fold_tree_sees_the_account_and_the_refs_a_lane_gate_reads() {
    let (dir, runtime) = lander("lane-env").await;
    let tools = dir.path().join("home/.local/bin");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(tools.join("python3"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(tools.join("python3"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut config = runtime.coding.clone().unwrap();
    // As a published workflow pins it: a HOME, and a PATH with the system's python3 on it.
    config.checks[1].environment.insert("HOME".into(), dir.path().join("home").to_string_lossy().into_owned());
    config.checks[1].environment.insert("PATH".into(), "/opt/homebrew/bin:/usr/bin:/bin".into());
    let root = runtime.root.clone();
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    let repo = runtime.repository();
    adapters::git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]).await.unwrap();
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    until(&mut runtime, |s| matches!(phase_of(s, "one"), Some(Phase::Done | Phase::Failed))).await;
    let seen: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("marker")).expect("the gate ran")).unwrap();
    let account = super::coding_execute::runner_account().expect("the test runs as an account");
    assert_eq!(seen["USER"], account.as_str(), "the gate sees who the runner is: {seen}");
    assert_eq!(seen["LOGNAME"], account.as_str(), "{seen}");
    assert_eq!(seen["python3"], tools.join("python3").to_str().unwrap(), "the account's python3 comes first: {seen}");
    assert_eq!(seen["origin/main"], 0, "the fold tree reads the repository's refs: {seen}");
}

/// A CI provider that answers for whichever workflow `FARCOOLER_CI_WORKFLOW` names, from `CI_RUNS`: `success`,
/// `failure`, `pending`, or `none` (no run: the push didn't start it).
const CI_BY_WORKFLOW: &str = r#"import json, os, pathlib, sys
name = os.environ['FARCOOLER_CI_WORKFLOW']
state = json.loads(os.environ['CI_RUNS']).get(name, 'success')
if state == 'none': sys.exit(0)
pathlib.Path(os.environ['FARCOOLER_RECEIPT']).write_text(json.dumps(dict(binding=os.environ['FARCOOLER_OPERATION_BINDING'],
    repository='offline/repository', run=dict(id=74, name=name, head_sha=os.environ['FARCOOLER_CANDIDATE'],
    status='in_progress' if state == 'pending' else 'completed', conclusion='failure' if state == 'failure' else 'success',
    html_url='fixture://ci/74', created_at='2026-01-01T00:00:00Z', event='push'))))
"#;

/// The label the group's CI wait settled on.
fn ci_label(runtime: &Runtime) -> String {
    std::fs::read_dir(runtime.root.join("receipts")).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".verb.json"))
        .map(|p| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).unwrap()).unwrap())
        .find(|fact| fact["binding"]["effect"]["kind"]["Execute"] == "AwaitCi")
        .map(|fact| fact["label"].as_str().unwrap().to_string()).unwrap_or_default()
}

/// ov-1071: a CI wait can wait on other workflows besides the configured one. Red anywhere is red, one still going is
/// waited for, and one the push never started (a path filter) doesn't hold the landing.
#[tokio::test]
async fn a_ci_wait_waits_on_every_workflow_it_names_that_the_push_started() {
    let also = serde_json::to_string(&["Web", "Doc comments"]).unwrap();
    for (runs, named, label, phase) in [
        (r#"{"Web":"success","Doc comments":"none"}"#, true, "passed", Phase::Done),
        (r#"{"Web":"failure"}"#, true, "rejected", Phase::Failed),
        (r#"{"Doc comments":"pending"}"#, true, "timeout", Phase::Failed),
        (r#"{"configured-check":"none"}"#, true, "timeout", Phase::Failed),
        // Without the list, only the configured workflow counts, as before.
        (r#"{"Web":"failure"}"#, false, "passed", Phase::Done),
    ] {
        let (dir, runtime) = lander("pass").await;
        let script = dir.path().join("ci-by-workflow.py");
        std::fs::write(&script, CI_BY_WORKFLOW).unwrap();
        let root = runtime.root.clone();
        let mut config = runtime.coding.clone().unwrap();
        config.ci.command.argv = vec!["/usr/bin/python3".into(), script.to_string_lossy().into_owned()];
        config.ci.command.environment.insert("CI_RUNS".into(), runs.into());
        std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
        drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        let mut definition = batched_push(&runtime, "any", 800);
        if named { definition.groups.get_mut("landing").unwrap().steps[4].operation.arguments.push(also.clone()); }
        runtime.coding.as_ref().unwrap().validate_definition(&definition).unwrap();
        at_join(&mut runtime, "one", &definition).await;
        command(&runtime, Command::Queue { item: "one".into() });
        until(&mut runtime, |s| phase_of(s, "one") == Some(phase.clone())).await;
        assert_eq!(ci_label(&runtime), label, "{runs} named={named}");
    }
}

#[tokio::test]
async fn a_ci_wait_that_names_its_own_workflow_again_or_nothing_sound_is_refused() {
    let (_dir, runtime) = lander("pass").await;
    let config = runtime.coding.as_ref().unwrap();
    for bad in ["[]", "\"Web\"", r#"["Web","Web"]"#, r#"["configured-check"]"#, r#"[" "]"#] {
        let mut definition = batched_push(&runtime, "any", 800);
        definition.groups.get_mut("landing").unwrap().steps[4].operation.arguments.push(bad.into());
        assert!(config.validate_definition(&definition).is_err(), "{bad}");
    }
    assert_eq!(super::coding_verbs::combined(&[]), farcooler_store::board_ci::CiStatus::Unknown);
}

/// ov-1209: a controlled first CI stays pending until the second gate has run. The remote must
/// stay at the first push in that interval. A red first CI still lets the second group push, since it
/// was gated on top of that red head, and its own CI decides (ov-1291).
#[tokio::test]
async fn next_group_gates_during_ci_and_pushes_over_a_red_one() {
    for red in [true, false] {
        let (dir, mut runtime) = lander("pass").await;
        let script = dir.path().join("adapter.py");
        let text = std::fs::read_to_string(&script).unwrap();
        let controlled = r#"elif stage == 'ci':
    control = pathlib.Path(__file__).parent
    first = control / 'first-ci'
    if candidate != os.environ['RED']:
        if not first.exists(): first.write_text(candidate)
        if candidate == first.read_text():
            mode = 'ci-pending' if not (control / 'release-ci').exists() else ('ci-fail' if (control / 'red-ci').exists() else 'pass')
"#;
        std::fs::write(&script, text.replace("elif stage == 'ci':\n", controlled)).unwrap();
        if red { std::fs::write(dir.path().join("red-ci"), "red").unwrap(); }
        let definition = batched_push(&runtime, "green", 30_000);
        at_join(&mut runtime, "one", &definition).await;
        at_join(&mut runtime, "two", &definition).await;
        command(&runtime, Command::Queue { item: "one".into() });
        command(&runtime, Command::Queue { item: "two".into() });
        let state = runtime.state("w").unwrap().unwrap();
        let second = state.items["two"].grouped_in.clone().unwrap();
        let store = runtime.store.clone();
        let release = dir.path().join("release-ci");
        let first_ci = dir.path().join("first-ci");
        let remote = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository.clone();
        let second_id = second.clone();
        let recovery = runtime.exec.clone();
        let mut observer = tokio::spawn(async move {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            loop {
                let state = store.workflow_state("w").unwrap().unwrap();
                let second = &state.items[&second_id];
                if second.definition.stages.get(second.stage).is_some_and(|s| s.kind == StageKind::Push) {
                    // Leave the push runnable long enough to expose a missing CI barrier.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let first = std::fs::read_to_string(&first_ci).unwrap();
                    assert_eq!(adapters::git(&remote, &["rev-parse", "refs/heads/main"]).await.unwrap(), first,
                        "the second gate finished while the first CI was pending, without a second push");
                    let stored = store.workflow_outbox("w").unwrap().into_iter().find(|s|
                        s.active && s.effect.item == second_id && s.effect.kind == EffectKind::Execute(StageKind::Push)).unwrap();
                    let binding = Binding::new("w", &stored.effect, stored.resource_generation.unwrap(), second.iteration,
                        &recovery.item_base("w", &stored.effect).unwrap());
                    assert!(matches!(recovery.reconcile_verb(&binding, &StageKind::Push).await.unwrap(), Outcome::Interrupted),
                        "recovery during the pre-send CI wait must safely resume, without spending an attempt");
                    assert!(!recovery.remote_log(&binding).exists(), "no push process started during the wait");
                    return true;
                }
                if std::time::Instant::now() >= deadline {
                    std::fs::write(&release, "go").unwrap();
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        runtime.continuous = true;
        let overlapped = tokio::select! {
            result = &mut observer => result.unwrap(),
            result = runtime.pump(2) => { result.unwrap(); panic!("CI finished before the restart"); }
        };
        assert!(overlapped, "the next group must finish its gate before the previous CI is released");
        // Drop the live futures as a daemon crash would, then recover from the admitted effects.
        let root = runtime.root.clone(); drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        std::fs::write(dir.path().join("release-ci"), "go").unwrap();
        runtime.continuous = true;
        let state = until(&mut runtime, |s| phase_of(s, "two") == Some(Phase::Done)).await;
        assert_eq!(phase_of(&state, "two"), Some(Phase::Done));
        let bases = runtime.store.workflow_outbox("w").unwrap().iter().filter(|s|
            s.effect.item == second && s.effect.kind == EffectKind::Execute(StageKind::AwaitBase)).count();
        assert_eq!(bases, 1, "a red previous CI doesn't send the gated group back to its base step");
        assert_eq!(outbox(&runtime, StageKind::Push), 3, "only the interrupted pre-send effect was retried");
        assert_eq!(settled(&runtime, StageKind::Push).pointer("/detail/red_base").is_some(), red, "the push says it went over a red head");
        assert_eq!(outbox(&runtime, StageKind::Build), 2, "no member was sent back to build");
        let sends = std::fs::read_dir(runtime.root.join("receipts")).unwrap().filter(|e|
            e.as_ref().unwrap().path().to_string_lossy().ends_with(".push-process.json")).count();
        assert_eq!(sends, 2, "exactly one actual push per group, including after recovery");
        let mut replay = State::empty("w".into(), state.epoch);
        for event in runtime.store.workflow_events("w").unwrap() {
            replay = farcooler_workflow::step(&replay, &event).unwrap().state;
        }
        assert_eq!(replay, state, "the resource handoff and red-CI retry replay exactly");
    }
}

#[tokio::test]
async fn a_reviewed_ticket_joins_automatically_and_a_landing_gate_leaves_build_capacity() {
    let (dir, mut runtime) = lander("pass").await;
    runtime.set_effect_cap(1);
    let mut definition = batched_push(&runtime, "any", 5000);
    definition.start_policy = Some(farcooler_workflow::scheduling::StartPolicy {
        columns: ["todo".into()].into(), budget: farcooler_workflow::scheduling::Budget::Fixed(1),
        files: Default::default(), dependencies: Default::default(), rotate_models: false, auto_queue: true,
    });
    at_join(&mut runtime, "one", &definition).await;
    // Removing automatic starts must not strand a reviewed run admitted under auto_queue.
    let mut manual = definition.clone();
    manual.start_policy = None;
    let state = runtime.state("w").unwrap().unwrap();
    let publication = farcooler_protocol::workflow_commands::Action::Publish { reason: None,
        mutation: farcooler_protocol::workflow_commands::Mutation {
            id: "remove-start-policy".into(), revision: state.revision, epoch: state.epoch, sequence: 1,
        }, expected_pin: None, definition: manual, adapter: None, notifications: None,
    };
    let answer = runtime.wire_command("w", "owner", publication).unwrap();
    assert!(answer.pointer("/receipt/result/Ok").is_some(), "{answer}");
    assert!(runtime.store.workflow_configuration("w").unwrap().unwrap().definition.start_policy.is_none());
    // No Queue command: the next owned sweep admits it to its group.
    runtime.pump(10).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].phase, Phase::Joined);
    let group = group_id(&state);
    until(&mut runtime, |s| s.items.get(&group).is_some_and(|g| g.definition.stages[g.stage].kind == StageKind::Run)).await;
    // Both pools admit work even with one build slot. The builder may yield its CPU to the gate (ov-1214).
    let script = dir.path().join("adapter.py");
    let source = std::fs::read_to_string(&script).unwrap();
    let marker = dir.path().join("build-alongside-gate");
    let admitted = dir.path().join("build-admitted-alongside-gate");
    let gate = format!("elif stage == 'gate':\n    import time\n    marker = pathlib.Path({:?})\n    until = time.time() + 4\n    while not marker.exists() and time.time() < until: time.sleep(0.02)\n    if not marker.exists(): sys.exit(8)\n", admitted.to_str().unwrap());
    let build = format!("if stage == 'build':\n    pathlib.Path({:?}).write_text('started')\n", marker.to_str().unwrap());
    std::fs::write(script, source.replace("elif stage == 'gate':\n", &gate).replace("if stage == 'build':\n", &build)).unwrap();
    runtime.continuous = true;
    enqueue_item(&mut runtime, "w", "two", definition);
    let places = runtime.places.clone();
    let gate_id = group.clone();
    let observer = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
        loop {
            let both = {
                let places = places.lock().unwrap();
                places.holding.iter().any(|p| p.item == gate_id && p.kind == EffectKind::Execute(StageKind::Run))
                    && places.holding.iter().any(|p| p.item == "two" && p.kind == EffectKind::Execute(StageKind::Build))
            };
            if both { std::fs::write(admitted, "admitted").unwrap(); return true; }
            if tokio::time::Instant::now() >= deadline { return false; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    runtime.pump(11).await.unwrap();
    assert!(observer.await.unwrap(), "the landing gate must not consume the builder's place");
    let state = runtime.state("w").unwrap().unwrap();
    assert!(marker.exists(), "the admitted builder resumes after yielding to the gate");
    assert_eq!(phase_of(&state, &group), Some(Phase::Done), "landing gate consumed the build slot");
}

/// A failed fold diagnoses conflicts with main and between tickets in one pass. The remaining
/// members land together, without a chain of split groups or repeated conflict discovery.
#[tokio::test]
async fn a_fold_conflict_isolates_all_bad_tickets_and_keeps_the_healthy_batch_together() {
    let (dir, mut runtime) = lander("pass").await;
    let mut definition = batched_push(&runtime, "any", 5000);
    definition.groups.get_mut("landing").unwrap().limit = None;
    at_join(&mut runtime, "ahead", &definition).await;
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, text.replace("name = 'shared.txt' if mode == 'conflict' else 'result-' + os.environ['FARCOOLER_WORKFLOW_RUN'][:8] + '.txt'", "name = 'shared.txt'")).unwrap();
    for item in ["good-one", "bad-pair"] { at_join(&mut runtime, item, &definition).await; }
    std::fs::write(&script, text).unwrap();
    for item in ["bad-one", "good-two", "bad-two"] {
        at_join(&mut runtime, item, &definition).await;
    }
    let before = runtime.state("w").unwrap().unwrap();
    let other = dir.path().join("other");
    let remote = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository.clone();
    assert!(tokio::process::Command::new("/usr/bin/git")
        .args(["clone", "-q", "-b", "main", remote.to_str().unwrap(), other.to_str().unwrap()])
        .status().await.unwrap().success());
    for item in ["bad-one", "bad-two"] {
        let candidate = before.items[item].candidate.as_ref().unwrap();
        let path = adapters::git(&runtime.repository(), &["diff-tree", "--no-commit-id", "--name-only", "-r", candidate]).await.unwrap();
        std::fs::write(other.join(path.trim()), "another landing changed this file").unwrap();
    }
    for args in [vec!["add", "."], vec!["-c", "user.name=Other", "-c", "user.email=o@localhost", "-c", "commit.gpgsign=false", "commit", "-qm", "conflicting landing"],
        vec!["push", "-q", "origin", "HEAD:refs/heads/main"]] {
        assert!(tokio::process::Command::new("/usr/bin/git").current_dir(&other)
            .args(args).status().await.unwrap().success());
    }
    // The first queue owns the target; the other five join one waiting group before any fold runs.
    for item in ["ahead", "good-one", "bad-one", "good-two", "bad-two", "bad-pair"] {
        assert!(command(&runtime, Command::Queue { item: item.into() }).result.is_ok());
    }
    resolve_fixture_conflicts(&script, &remote);
    let state = until(&mut runtime, |s| ["ahead", "good-one", "good-two"].iter().all(|t| phase_of(s, t) == Some(Phase::Done))
        && ["bad-one", "bad-two", "bad-pair"].iter().all(|t| phase_of(s, t) == Some(Phase::WaitingJoin))).await;
    assert_eq!(fold_rebuilds(&runtime), ["bad-one", "bad-pair", "bad-two"]);
    assert_eq!(outbox(&runtime, StageKind::Check), outbox(&runtime, StageKind::Build), "rebuilt candidates run checks again");
    for name in ["bad-one", "bad-two", "bad-pair"] {
        assert!(state.items[name].back_edges.is_empty(), "a conflict spends no join review budget");
        assert_eq!(state.items[name].iteration, 0);
        assert!(state.items[name].brief.is_none(), "the rebuild consumed its rebase brief");
    }
    assert_eq!(outbox(&runtime, StageKind::Fold), 3, "ahead, diagnosis, one healthy fold");
    assert_eq!(outbox(&runtime, StageKind::Run), 2, "one gate for each healthy landing");
    assert_eq!(outbox(&runtime, StageKind::AwaitCi), 2, "one CI watch for each healthy landing");
    let folds: Vec<_> = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .filter(|s| s.effect.kind == EffectKind::Execute(StageKind::Fold)).collect();
    assert_eq!(folds.iter().map(|s| s.effect.item.as_str()).collect::<std::collections::BTreeSet<_>>().len(), 2, "no split groups");
    let diagnosed = folds.iter().find(|s| s.effect.members.len() == 5).unwrap();
    let fact = runtime.root.join("receipts").join(format!("{}.verb.json", digest(&diagnosed.effect)));
    let fact: serde_json::Value = serde_json::from_slice(&std::fs::read(fact).unwrap()).unwrap();
    let binding: Binding = serde_json::from_value(fact["binding"].clone()).unwrap();
    let recovered = runtime.reconcile_verb(&binding, &StageKind::Fold).await.unwrap();
    assert!(matches!(recovered, Outcome::FoldConflict { members: Some(members), .. } if members == ["bad-one", "bad-two", "bad-pair"]));
    let mut replay = State::empty("w".into(), 1);
    for event in runtime.events("w").unwrap() { replay = step(&replay, &event).unwrap().state; }
    assert_eq!(state, replay);
}

#[tokio::test]
async fn tickets_that_apply_alone_but_conflict_together_return_the_conflicting_member() {
    let (dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "ahead", &definition).await;
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, text.replace("name = 'shared.txt' if mode == 'conflict' else 'result-' + os.environ['FARCOOLER_WORKFLOW_RUN'][:8] + '.txt'", "name = 'shared.txt'")).unwrap();
    at_join(&mut runtime, "first", &definition).await;
    let shared = std::fs::read_to_string(&script).unwrap();
    // The conflicting member has a clean first commit: diagnosis must roll back the whole member.
    std::fs::write(&script, shared.replace("    name = 'shared.txt'", "    pathlib.Path('partial.txt').write_text('must not land')\n    subprocess.check_call(['/usr/bin/git', 'add', 'partial.txt'])\n    subprocess.check_call(['/usr/bin/git', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'partial member'])\n    name = 'shared.txt'")).unwrap();
    at_join(&mut runtime, "second", &definition).await;
    std::fs::write(&script, &shared).unwrap();
    at_join(&mut runtime, "third", &definition).await;
    std::fs::write(&script, shared.replace("name = 'shared.txt'", "name = 'partial.txt'")).unwrap();
    at_join(&mut runtime, "tail", &definition).await;
    let tail = runtime.state("w").unwrap().unwrap().items["tail"].candidate.clone().unwrap();
    for item in ["ahead", "first", "second", "third", "tail"] {
        assert!(command(&runtime, Command::Queue { item: item.into() }).result.is_ok());
    }
    let remote = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository.clone();
    resolve_fixture_conflicts(&script, &remote);
    until(&mut runtime, |s| ["first", "tail"].iter().all(|t| phase_of(s, t) == Some(Phase::Done))
        && ["second", "third"].iter().all(|t| phase_of(s, t) == Some(Phase::WaitingJoin))).await;
    assert_eq!(fold_rebuilds(&runtime), ["second", "third"]);
    assert_eq!(outbox(&runtime, StageKind::Check), outbox(&runtime, StageKind::Build), "their checks rerun before joining");
    assert_eq!(outbox(&runtime, StageKind::Fold), 3);
    assert_eq!(outbox(&runtime, StageKind::Run), 2);
    assert_eq!(outbox(&runtime, StageKind::AwaitCi), 2);
    assert_eq!(adapters::git(&remote, &["show", "refs/heads/main:partial.txt"]).await.unwrap(),
        adapters::git(&runtime.repository(), &["show", &format!("{tail}:partial.txt")]).await.unwrap());
}

/// Once the original candidates are queued, the fixture builder handles its brief by replaying
/// its edits over main and resolving overlapping lines in favor of its own work.
fn resolve_fixture_conflicts(script: &Path, remote: &Path) {
    let source = std::fs::read_to_string(script).unwrap();
    let remote = serde_json::to_string(remote.to_str().unwrap()).unwrap();
    let rebuild = format!("if stage == 'build':\n    main = subprocess.check_output(['/usr/bin/git', '--git-dir', {remote}, 'rev-parse', 'refs/heads/main']).decode().strip()\n    subprocess.check_call(['/usr/bin/git', 'rebase', '--no-autostash', '-X', 'theirs', main])");
    std::fs::write(script, source.replace("if stage == 'build':", &rebuild)).unwrap();
}

fn fold_rebuilds(runtime: &Runtime) -> Vec<String> {
    let mut items: Vec<_> = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .filter(|s| s.effect.kind == EffectKind::Execute(StageKind::Build)
            && s.effect.brief.as_ref().is_some_and(|b| b.contains("resolve the cherry-pick conflicts")))
        .map(|s| s.effect.item).collect();
    items.sort();
    items
}

#[tokio::test]
async fn interrupted_ci_later_green_finishes_on_the_pushed_sha() {
    let (dir, mut runtime) = lander("ci-pending").await;
    use std::os::unix::fs::PermissionsExt;
    let definition = batched_push(&runtime, "any", 5000);
    // Check the post-push timestamp directly, leaving real Python CI polls enough
    // patience under shared-host load rather than depending on a 400 ms deadline.
    let hook = dir.path().join("private-remote.git/hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\nsleep 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..5 { runtime.pump(time).await.unwrap(); }
    let started = std::time::Instant::now();
    runtime.pump(5).await.unwrap();
    let resume_at = 5 + i64::try_from(started.elapsed().as_millis()).unwrap() + 10;
    let binding = admitted(&runtime).await;
    assert_eq!(binding.effect.kind, EffectKind::Execute(StageKind::AwaitCi));
    assert!(binding.effect.created_ms >= 1005, "CI must start after the one-second push: {:?}", binding.effect);
    let pushed = remote_head(&runtime).await;
    let root = runtime.root.clone(); drop(runtime);
    // CI is pending on the first resumed poll, then green. The original, accurately
    // stamped CI deadline still has patience left: a restart never refunds it.
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    let marker = dir.path().join("resumed-ci");
    let text = text.replace("elif stage == 'ci':", &format!("elif stage == 'ci':\n    marker = pathlib.Path({:?})\n    if marker.exists(): mode = 'pass'\n    marker.write_text('poll')", marker.to_str().unwrap()));
    std::fs::write(script, text).unwrap();
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(resume_at).await.unwrap();
    runtime.pump(resume_at + 1).await.unwrap();
    let state = until(&mut runtime, |s| matches!(phase_of(s, "one"), Some(Phase::Done | Phase::Failed))).await;
    assert_eq!(phase_of(&state, "one"), Some(Phase::Done));
    assert_eq!(candidate_of(&state, "one"), Some(pushed.as_str()));
}

#[tokio::test]
async fn enospc_during_cherry_pick_retries_the_fold_without_split_or_rework() {
    use std::os::unix::fs::PermissionsExt;
    struct RestoreGit;
    impl Drop for RestoreGit {
        fn drop(&mut self) { crate::git::PROGRAM.with(|p| *p.borrow_mut() = None); }
    }
    for staged in [false, true] {
        let (dir, mut runtime) = lander("pass").await;
        let definition = batched_push(&runtime, "any", 5000);
        for item in ["ahead", "one", "two"] { at_join(&mut runtime, item, &definition).await; }
        // Fail exactly one fold pick, either before it writes anything or after it
        // stages the patch but before it can commit. Other Git calls remain real.
        let wrapper = dir.path().join("git-enospc.py");
        let marker = dir.path().join("enospc");
        std::fs::write(&wrapper, format!(r#"#!/usr/bin/python3
import os, pathlib, subprocess, sys
args = sys.argv[1:]
marker = pathlib.Path({marker:?})
if 'cherry-pick' in args and '-x' in args and not marker.exists():
    marker.write_text('injected')
    if {staged}:
        subprocess.check_call(['/usr/bin/git', *args[:-1], '--no-commit', args[-1]])
    print('fatal: cannot write index: No space left on device', file=sys.stderr)
    sys.exit(128)
os.execv('/usr/bin/git', ['/usr/bin/git', *args])
"#, marker = marker.to_str().unwrap(), staged = if staged { "True" } else { "False" })).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _restore = RestoreGit;
        crate::git::PROGRAM.with(|p| *p.borrow_mut() = Some(wrapper.into_os_string()));
        for item in ["ahead", "one", "two"] { command(&runtime, Command::Queue { item: item.into() }); }
        let state = until(&mut runtime, |s| matches!(phase_of(s, "two"), Some(Phase::Done | Phase::Failed))).await;
        assert!(marker.exists(), "the cherry-pick fault was exercised");
        for item in ["ahead", "one", "two"] {
            assert_eq!(phase_of(&state, item), Some(Phase::Done), "staged={staged}, {item}");
        }
        assert_eq!(outbox(&runtime, StageKind::Fold), 3, "two original groups and one fold retry, no Split");
        assert_eq!(outbox(&runtime, StageKind::Build), 3, "no member was sent to rework");
        let facts: Vec<_> = std::fs::read_dir(runtime.root.join("receipts")).unwrap()
            .map(|e| e.unwrap().path()).filter(|p| p.to_string_lossy().ends_with(".verb.json"))
            .map(|p| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).unwrap()).unwrap()).collect();
        assert!(facts.iter().all(|f| f["label"] != "conflict"), "infrastructure writes no conflict fact");
    }
}

#[tokio::test]
async fn a_fold_tree_deleted_mid_gate_retries_the_gate_without_split() {
    let (dir, mut runtime) = lander("pass").await;
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    let marker = dir.path().join("deleted-once");
    let text = text.replace("elif stage == 'gate':", &format!("elif stage == 'gate':\n    marker = pathlib.Path({:?})\n    if not marker.exists() and len(list(pathlib.Path('.').glob('result-*.txt'))) > 1:\n        marker.write_text('deleted')\n        __import__('shutil').rmtree(pathlib.Path.cwd())\n        sys.exit(5)\n    if marker.exists() and marker.read_text() == 'deleted':\n        marker.write_text('disk-full')\n        print('OSError: [Errno 28] No space left on device', file=sys.stderr)\n        sys.exit(5)", marker.to_str().unwrap()));
    std::fs::write(script, text).unwrap();
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "ahead", &definition).await;
    at_join(&mut runtime, "one", &definition).await;
    at_join(&mut runtime, "two", &definition).await;
    command(&runtime, Command::Queue { item: "ahead".into() });
    command(&runtime, Command::Queue { item: "one".into() });
    command(&runtime, Command::Queue { item: "two".into() });
    let state = until(&mut runtime, |s| matches!(phase_of(s, "two"), Some(Phase::Done | Phase::Failed))).await;
    assert_eq!(phase_of(&state, "one"), Some(Phase::Done));
    assert_eq!(phase_of(&state, "two"), Some(Phase::Done));
    assert_eq!(outbox(&runtime, StageKind::Fold), 2, "one fold per original group, no split");
    assert_eq!(outbox(&runtime, StageKind::Run), 4, "the deleted tree and ENOSPC each retry only the gate");
}

#[tokio::test]
async fn prelaunch_gate_enospc_retries_boundedly_without_split_or_rework() {
    use std::os::unix::fs::PermissionsExt;
    struct RestoreGit;
    impl Drop for RestoreGit {
        fn drop(&mut self) { crate::git::PROGRAM.with(|p| *p.borrow_mut() = None); }
    }
    for persistent in [false, true] {
        let (dir, mut runtime) = lander("pass").await;
        let definition = batched_push(&runtime, "any", 5000);
        for item in ["ahead", "one", "two"] { at_join(&mut runtime, item, &definition).await; }
        let wrapper = dir.path().join("git-prelaunch-enospc.py");
        let marker = dir.path().join("prelaunch-failures");
        // Only the batch's pre-launch settlement fails. No gate process has started,
        // and the real Git error is returned through git_recorded as Error::Refused.
        std::fs::write(&wrapper, format!(r#"#!/usr/bin/python3
import os, pathlib, sys
args = sys.argv[1:]
marker = pathlib.Path({marker:?})
if args == ['reset', '--hard', '--quiet'] and len(list(pathlib.Path.cwd().glob('result-*.txt'))) > 1:
    count = int(marker.read_text()) if marker.exists() else 0
    if {persistent} or count == 0:
        marker.write_text(str(count + 1))
        print('fatal: cannot write index: No space left on device', file=sys.stderr)
        sys.exit(128)
os.execv('/usr/bin/git', ['/usr/bin/git', *args])
"#, marker = marker.to_str().unwrap(), persistent = if persistent { "True" } else { "False" })).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _restore = RestoreGit;
        crate::git::PROGRAM.with(|p| *p.borrow_mut() = Some(wrapper.into_os_string()));
        for item in ["ahead", "one", "two"] { command(&runtime, Command::Queue { item: item.into() }); }
        let mut finished = None;
        for time in 2..200 {
            runtime.pump(time).await.unwrap();
            let effects = runtime.store.workflow_outbox("w").unwrap();
            assert!(effects.iter().filter(|s| s.effect.kind == EffectKind::Execute(StageKind::Run))
                .all(|s| s.dispatch_state != "uncertain"), "a pre-launch failure must be settled as safe immediately");
            let state = runtime.state("w").unwrap().unwrap();
            if matches!(phase_of(&state, "two"), Some(Phase::Done | Phase::Failed)) { finished = Some(state); break; }
        }
        let state = finished.expect("bounded gate retries must settle");
        assert_eq!(std::fs::read_to_string(marker).unwrap(), if persistent { "3" } else { "1" });
        assert_eq!(phase_of(&state, "ahead"), Some(Phase::Done));
        for item in ["one", "two"] {
            assert_eq!(phase_of(&state, item), Some(if persistent { Phase::Failed } else { Phase::Done }));
        }
        assert_eq!(outbox(&runtime, StageKind::Run), if persistent { 4 } else { 3 }, "at most two gate retries");
        assert_eq!(outbox(&runtime, StageKind::Fold), 2, "only the original groups fold, no Split");
        assert_eq!(outbox(&runtime, StageKind::Build), 3, "no member rework");
    }
}

/// ov-1164: an I/O error after launch is not permission to run another writer.
#[tokio::test]
async fn an_unproved_gate_settlement_refuses_retry_until_ownership_is_settled() {
    let (_dir, mut runtime) = lander("gate-slow").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..4 { runtime.pump(time).await.unwrap(); }
    let binding = admitted(&runtime).await;
    assert_eq!(binding.effect.kind, EffectKind::Execute(StageKind::Run));
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let (result, mut child) = crate::effect_runner::TEST_UNSETTLED.scope(std::cell::RefCell::new(None), async {
        let result = runtime.execute_coding("w", &binding, cancelled).await;
        let child = crate::effect_runner::TEST_UNSETTLED.with(|held| held.borrow_mut().take().unwrap());
        (result, child)
    }).await;
    let pid = child.id().unwrap();
    let alive = !crate::effect_runner::Group::gone(pid).unwrap();
    // Make preparation fail deterministically if it is reached before checking the
    // existing launch. The old gate is still alive, so even this failure is unsafe.
    let git_link = runtime.tree(&binding).join(".git");
    let original = std::fs::read(&git_link).unwrap();
    std::fs::write(&git_link, "gitdir: missing-gate-git-dir\n").unwrap();
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let repeated = runtime.execute_coding("w", &binding, cancelled).await;
    std::fs::write(&git_link, original).unwrap();
    // Keep the leader reaped as ownership reconciliation stops the old group.
    let reaped = tokio::spawn(async move { child.wait().await.unwrap() });
    let outcome = result.unwrap_or(Outcome::Uncertain);
    runtime.observe("w", &binding.effect, outcome.clone(), false, 20).unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    let phase = state.items[&binding.effect.item].phase.clone();
    let runs = outbox(&runtime, StageKind::Run);
    let settled = runtime.reconcile_coding(&binding).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), reaped).await.unwrap().unwrap();
    assert!(alive, "fault must leave a real gate alive");
    assert_eq!(repeated.unwrap_or(Outcome::Uncertain), Outcome::Uncertain,
        "existing launch evidence also requires reconciliation, never a safe retry");
    assert_eq!(outcome, Outcome::Uncertain, "unproved settlement must not authorize a retry");
    assert_eq!(phase, Phase::Uncertain);
    assert_eq!(runs, 1, "no second gate may be queued while the old gate lives");
    assert_eq!(settled, Outcome::SafeFailure, "retry is safe only after ownership reconciliation");
    assert!(crate::effect_runner::Group::gone(pid).unwrap());
    runtime.observe("w", &binding.effect, settled, true, 21).unwrap();
    assert_eq!(outbox(&runtime, StageKind::Run), 2, "settlement permits the retry");
}

#[tokio::test]
async fn a_gate_owner_write_failure_keeps_recovery_uncertain() {
    let (_dir, mut runtime) = lander("gate-slow").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..4 { runtime.pump(time).await.unwrap(); }
    let binding = admitted(&runtime).await;
    assert_eq!(binding.effect.kind, EffectKind::Execute(StageKind::Run));
    // The child exists before the atomic owner write fails. No durable PID remains.
    std::fs::create_dir(runtime.owner_path(&binding).with_extension("tmp")).unwrap();
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let result = runtime.execute_coding("w", &binding, cancelled).await;
    assert_eq!(result.unwrap_or(Outcome::Uncertain), Outcome::Uncertain);
    assert!(runtime.reconcile_coding(&binding).await.is_err(),
        "a missing owner record after a possible launch is not proof that writers stopped");
}

/// ov-1164: neither an empty saved identity nor a failed identity lookup proves quiescence.
#[tokio::test]
async fn a_gate_with_missing_start_identity_refuses_retry_until_its_group_is_gone() {
    gate_with_missing_start_identity(true).await;
}

#[tokio::test]
async fn a_gate_with_unreadable_start_identity_refuses_retry_until_its_group_is_gone() {
    gate_with_missing_start_identity(false).await;
}

async fn gate_with_missing_start_identity(empty_record: bool) {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..4 { runtime.pump(time).await.unwrap(); }
    let binding = admitted(&runtime).await;
    assert_eq!(binding.effect.kind, EffectKind::Execute(StageKind::Run));
    let mut gate = tokio::process::Command::new("/bin/sleep").arg("600")
        .process_group(0).kill_on_drop(true).spawn().unwrap();
    let pid = gate.id().unwrap();
    let path = runtime.owner_path(&binding);
    crate::effect_runner::Owner::write(&path, pid).unwrap();
    if empty_record {
        std::fs::write(&path, serde_json::to_vec(&crate::effect_runner::Owner {
            pid, started: String::new(),
        }).unwrap()).unwrap();
    }
    let root = runtime.root.clone();
    drop(runtime);
    let runtime = Runtime::open(&root).unwrap();
    let recovered = if empty_record {
        runtime.reconcile_coding(&binding).await
    } else {
        crate::effect_runner::TEST_START_UNAVAILABLE.scope((), runtime.reconcile_coding(&binding)).await
    };
    let alive = !crate::effect_runner::Group::gone(pid).unwrap();
    let outcome = recovered.unwrap_or(Outcome::Uncertain);
    runtime.observe("w", &binding.effect, outcome.clone(), false, 20).unwrap();
    let phase = runtime.state("w").unwrap().unwrap().items[&binding.effect.item].phase.clone();
    let runs = outbox(&runtime, StageKind::Run);
    // Clean up before assertions, including when run against the unsafe implementation.
    gate.kill().await.unwrap();
    assert!(crate::effect_runner::Group::gone(pid).unwrap());
    assert!(alive, "unknown identity must leave the live process alone");
    assert_eq!(outcome, Outcome::Uncertain, "missing identity cannot authorize a gate retry");
    assert_eq!(phase, Phase::Uncertain);
    assert_eq!(runs, 1, "no retry while ownership is uncertain");
    // Group disappearance is independent proof, even with the identity still unavailable.
    let settled = crate::effect_runner::TEST_START_UNAVAILABLE.scope((), runtime.reconcile_coding(&binding)).await.unwrap();
    assert_eq!(settled, Outcome::SafeFailure);
    runtime.observe("w", &binding.effect, settled, true, 21).unwrap();
    assert_eq!(outbox(&runtime, StageKind::Run), 2, "proven quiescence permits the retry");
}

#[tokio::test]
async fn a_gate_owner_write_requires_a_readable_start_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owner.json");
    let mut gate = tokio::process::Command::new("/bin/sleep").arg("600")
        .process_group(0).kill_on_drop(true).spawn().unwrap();
    let result = crate::effect_runner::TEST_START_UNAVAILABLE.scope((), async {
        crate::effect_runner::Owner::write(&path, gate.id().unwrap())
    }).await;
    gate.kill().await.unwrap();
    assert!(result.is_err(), "unreadable identity must fail the post-launch owner write");
    assert!(!path.exists(), "never persist an owner receipt with an empty identity");
}

/// Rewrite the runtime's pinned configuration, and open it again: the Nov 15 head no CI run was ever started on.
fn with_unrun(runtime: Runtime, subject: &str, unrun: farcooler_protocol::workflow_config::CiUnrun) -> Runtime {
    let root = runtime.root.clone();
    let mut config = runtime.coding.clone().unwrap();
    config.ci.command.environment.insert("UNRUN_SUBJECT".into(), subject.into());
    config.ci.unrun = Some(unrun);
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    drop(runtime);
    Runtime::open(&root).unwrap()
}
/// The newest settlement of a step's effect.
pub(super) fn settled(runtime: &Runtime, kind: StageKind) -> serde_json::Value {
    let stored = runtime.store.workflow_outbox("w").unwrap().into_iter().rev()
        .find(|s| s.effect.kind == EffectKind::Execute(kind.clone())).expect("the step ran");
    serde_json::from_slice(&std::fs::read(runtime.root.join("receipts").join(format!("{}.verb.json", digest(&stored.effect)))).unwrap()).unwrap()
}
fn unrun(grace_ms: u64, ignored: &[&str]) -> farcooler_protocol::workflow_config::CiUnrun {
    farcooler_protocol::workflow_config::CiUnrun { grace_ms, ignored: ignored.iter().map(|p| p.to_string()).collect() }
}
/// ov-1243, Nov 15: a group folded on main at 05:30:31, a bot pushed a commit CI ignores a minute later, the group's
/// gate passed at 06:09:24 and its push was refused as `base_moved` four seconds after. The group went back to
/// its base step, whose head now had no CI run and never would: it polled for its whole patience. Replayed here with
/// the bot's commit as the other lander the gate moves the remote for (the order is the same: fold, move, gate
/// passed, refused), and CI ignoring the files it touches. The second base step takes the verdict of the commit
/// behind the head after the grace, and the group lands.
#[tokio::test]
async fn a_bot_commit_ci_ignores_that_lands_under_a_gate_does_not_stall_the_next_base_step() {
    let (_dir, runtime) = lander("move-base").await;
    let mut runtime = with_unrun(runtime, "other lander", unrun(50, &["other.txt"]));
    let first = remote_head(&runtime).await;
    // Patience far beyond the test: only the grace can end the wait.
    let definition = batched_push(&runtime, "green", 60_000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_eq!(outbox(&runtime, StageKind::Push), 2, "refused as base_moved, then landed");
    assert_eq!(outbox(&runtime, StageKind::AwaitBase), 2);
    assert_eq!(outbox(&runtime, StageKind::Fold), 2, "the gate ran again: reuse is unsafe, see the ticket's report");
    let fact = settled(&runtime, StageKind::AwaitBase);
    assert_eq!(fact["label"], "passed", "{fact}");
    assert_eq!(fact["detail"]["ci_from"], first, "the verdict came from the commit behind the bot's: {fact}");
    assert_eq!(candidate_of(&state, "one"), Some(remote_head(&runtime).await.as_str()));
}

/// The same head with no ignored paths configured: CI will never run on it, so the step says so after the grace
/// instead of polling for its patience, and nothing is folded on top of a base nobody can vouch for.
#[tokio::test]
async fn a_head_with_no_ci_run_ends_the_base_step_after_the_grace_with_a_sentence() {
    for ignored in [&[][..], &["unrelated.txt"][..]] {
        let (_dir, runtime) = lander("move-base").await;
        let mut runtime = with_unrun(runtime, "other lander", unrun(50, ignored));
        let definition = batched_push(&runtime, "green", 60_000);
        at_join(&mut runtime, "one", &definition).await;
        command(&runtime, Command::Queue { item: "one".into() });
        until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
        let fact = settled(&runtime, StageKind::AwaitBase);
        assert_eq!(fact["label"], "timeout", "{fact}");
        let why = fact["detail"]["why"].as_str().expect("it says why");
        assert!(why.contains("no run") && why.contains("orchestrator"), "{why}");
        assert_eq!(outbox(&runtime, StageKind::Push), 1, "nothing landed on the unvouched base");
    }
}

#[tokio::test]
async fn waiting_ticket_conflicts_with_main_before_joining_a_group() {
    let (dir, mut runtime) = lander("conflict").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "ticket", &definition).await;
    let before = runtime.state("w").unwrap().unwrap();
    let candidate = before.items["ticket"].candidate.as_deref().unwrap();
    let main = move_waiting_ticket_main(&dir, &runtime, "shared.txt").await;
    runtime.pump(10).await.unwrap();

    let state = runtime.state("w").unwrap().unwrap();
    assert_ne!(state.items["ticket"].phase, Phase::WaitingJoin, "a stale conflicting candidate was still eligible to queue");
    assert!(state.items.values().all(|item| item.group_of.is_none()));
    assert_eq!(outbox(&runtime, StageKind::Fold), 0);
    let builds: Vec<_> = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .filter(|s| s.effect.kind == EffectKind::Execute(StageKind::Build)).collect();
    let brief = builds[1].effect.brief.as_deref().unwrap();
    assert!(brief.contains(&main) && brief.contains("shared.txt"), "{brief}");
    let tree = runtime.item_tree("w", "ticket");
    adapters::git(&tree, &["merge-base", "--is-ancestor", candidate, "HEAD"]).await.unwrap();
    assert!(adapters::git(&tree, &["status", "--porcelain"]).await.unwrap().is_empty());
}

#[tokio::test]
async fn waiting_ticket_queue_checks_main_before_admitting_a_conflict() {
    let (dir, mut runtime) = lander("conflict").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "ticket", &definition).await;
    let before = runtime.state("w").unwrap().unwrap();
    let main = move_waiting_ticket_main(&dir, &runtime, "shared.txt").await;
    let receipt = runtime.submit(&runtime.controller(), Request { id: "queue-after-main-moved".into(),
        workspace: "w".into(), expected_revision: before.revision, sequence: 2,
        command: Command::Queue { item: "ticket".into() } }, 10).await.unwrap();
    assert_eq!(receipt.result, Err(Refusal::Stale));
    let state = runtime.state("w").unwrap().unwrap();
    let ticket = &state.items["ticket"];
    assert_eq!(ticket.definition.stages[ticket.stage].kind, StageKind::Build);
    assert!(state.items.values().all(|item| item.group_of.is_none()));
    let brief = ticket.brief.as_deref().unwrap();
    assert!(brief.contains(&main) && brief.contains("shared.txt"), "{brief}");
}

#[tokio::test]
async fn waiting_ticket_rebases_a_signed_candidate_only_after_its_worktree_is_clean() {
    let (dir, mut runtime) = lander("pass").await;
    let key = dir.path().join("signing-key");
    assert!(tokio::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&key).status().await.unwrap().success());
    let repository = runtime.repository();
    adapters::git(&repository, &["config", "gpg.format", "ssh"]).await.unwrap();
    adapters::git(&repository, &["config", "user.signingkey", key.to_str().unwrap()]).await.unwrap();
    adapters::git(&repository, &["config", "rebase.autoStash", "true"]).await.unwrap();
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, text.replace("'commit', '--quiet', '-m', 'build '+iteration",
        "'commit', '-S', '--quiet', '-m', 'build '+iteration")).unwrap();
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "ticket", &definition).await;
    let before = runtime.state("w").unwrap().unwrap();
    let previous = before.items["ticket"].candidate.as_deref().unwrap();
    let tree = runtime.item_tree("w", "ticket");
    let path = adapters::git(&tree, &["diff-tree", "--no-commit-id", "--name-only", "-r", previous]).await.unwrap();
    let file = tree.join(path);
    let committed = std::fs::read(&file).unwrap();
    std::fs::write(&file, "local edit").unwrap();
    let main = move_waiting_ticket_main(&dir, &runtime, "independent.txt").await;

    assert!(!runtime.prepare_queue("w", "ticket", before.revision, None, None, 10).await);
    assert_eq!(runtime.state("w").unwrap().unwrap(), before);
    assert_eq!(adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap(), previous);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "local edit");
    std::fs::write(&file, &committed).unwrap();

    runtime.pump(11).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    let ticket = &state.items["ticket"];
    assert_eq!(ticket.phase, Phase::WaitingJoin);
    let rebased = ticket.candidate.as_deref().unwrap();
    assert_ne!(rebased, previous);
    assert_eq!(adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap(), rebased);
    assert_eq!(adapters::git(&tree, &["rev-parse", "HEAD^"]).await.unwrap(), main);
    assert!(adapters::git(&tree, &["status", "--porcelain"]).await.unwrap().is_empty());
    assert_eq!(std::fs::read(&file).unwrap(), committed);
    let allowed_signers = dir.path().join("allowed-signers");
    std::fs::write(&allowed_signers, format!("test@localhost {}", std::fs::read_to_string(key.with_extension("pub")).unwrap())).unwrap();
    for commit in [previous, rebased] {
        assert!(tokio::process::Command::new("/usr/bin/git").current_dir(&tree)
            .args(["-c", &format!("gpg.ssh.allowedSignersFile={}", allowed_signers.display()), "verify-commit", commit])
            .status().await.unwrap().success());
    }
    let receipt = runtime.submit(&runtime.controller(), Request { id: "queue-rebased".into(),
        workspace: "w".into(), expected_revision: state.revision, sequence: 2,
        command: Command::Queue { item: "ticket".into() } }, 12).await.unwrap();
    assert!(receipt.result.is_ok());
    let state = runtime.state("w").unwrap().unwrap();
    let member = state.items.values().find_map(|i| i.members.first()).unwrap();
    assert_eq!(member.candidate, rebased);
    let mut replay = State::empty("w".into(), 1);
    for event in runtime.events("w").unwrap() { replay = step(&replay, &event).unwrap().state; }
    assert_eq!(state, replay);
}

async fn move_waiting_ticket_main(dir: &tempfile::TempDir, runtime: &Runtime, path: &str) -> String {
    let other = dir.path().join("other-main");
    let remote = &runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository;
    assert!(tokio::process::Command::new("/usr/bin/git")
        .args(["clone", "-q", "-b", "main", remote.to_str().unwrap(), other.to_str().unwrap()])
        .status().await.unwrap().success());
    std::fs::write(other.join(path), "another landing changed this file").unwrap();
    adapters::git(&other, &["add", "--", path]).await.unwrap();
    adapters::git(&other, &["-c", "user.name=Other", "-c", "user.email=o@localhost",
        "-c", "commit.gpgsign=false", "commit", "-qm", "other landing"]).await.unwrap();
    assert!(tokio::process::Command::new("/usr/bin/git").current_dir(&other)
        .args(["push", "-q", "origin", "HEAD:refs/heads/main"]).status().await.unwrap().success());
    adapters::git(&other, &["rev-parse", "HEAD"]).await.unwrap()
}
