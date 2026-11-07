//! The landing verbs against a real git repository and a local bare repository standing in for the
//! remote. No network push is ever made (ov-956).
use super::*;
use super::coding_tests::{admitted, configured};
use super::concurrency_tests::enqueue_item;
use super::coding_verbs::{classify_push, PushVerdict};
use std::collections::BTreeMap;

const GATE: &str = r#"elif stage == 'gate':
    if mode == 'gate-fail': sys.exit(5)
    if mode == 'gate-dirty': pathlib.Path('stray.out').write_text('left behind')
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
        .replace("conclusion='failure' if mode == 'ci-fail' else 'success',", "conclusion='failure' if mode == 'ci-fail' else ('cancelled' if mode == 'ci-cancel' else 'success'),");
    assert!(text.contains("elif stage == 'gate'") && text.contains("name = 'shared.txt'") && text.contains("'cancelled'"));
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
fn batched_push(runtime: &Runtime, base_mode: &str, patience_ms: u64) -> Definition {
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
        groups: [("landing".to_string(), GroupDef { limit: 4, exclusive: c.landing_resource().unwrap(), steps })].into() }
}
pub(super) fn command(runtime: &Runtime, command: Command) -> Receipt {
    let controller = runtime.controller();
    let state = runtime.state("w").unwrap().unwrap();
    let sequence = state.source_sequences.get("controller").copied().unwrap_or(0) + 1;
    runtime.submit(&controller, Request { id: format!("test-{sequence}"), workspace: "w".into(),
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
async fn a_conflict_fails_the_group_and_frees_the_target() {
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
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(remote_head(&runtime).await, theirs, "a conflicting group pushes nothing");
    assert_eq!(outbox(&runtime, StageKind::Push), 0);
    assert_eq!(outbox(&runtime, StageKind::Run), 0, "the fold stopped the group; the gate never ran on a half-folded tree");
    let group = state.items.iter().find(|(_, i)| i.group_of.is_some()).map(|(_, i)| i.phase.clone());
    assert_eq!(group, Some(Phase::Failed));
    // The conflicted tree was cleaned up by the abort, not left mid-pick.
    let tree = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.effect.kind == EffectKind::Execute(StageKind::Fold)).unwrap();
    let tree = runtime.tree(&Binding::new("w", &tree.effect, 1, 0, &runtime.base));
    assert!(adapters::git(&tree, &["status", "--porcelain"]).await.unwrap().is_empty());
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
}

#[tokio::test]
async fn waiting_for_a_base_whose_ci_is_red_times_out_and_waiting_for_ci_that_never_ends_times_out() {
    for (mode, base_mode, label) in [("ci-fail", "green", "base"), ("ci-pending", "any", "ci")] {
        let (_dir, mut runtime) = lander(mode).await;
        let definition = batched_push(&runtime, base_mode, 300);
        at_join(&mut runtime, "one", &definition).await;
        command(&runtime, Command::Queue { item: "one".into() });
        until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
        // `timeout` is an ordinary label here, routed to fail.
        let landed = outbox(&runtime, StageKind::Push);
        assert_eq!(landed, usize::from(label == "ci"), "{mode}: a red base never pushes; a CI timeout comes after the push");
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
async fn a_wait_leaves_the_effect_cap_alone() {
    let (_dir, mut runtime) = lander("ci-pending").await;
    runtime.set_effect_cap(1);
    let definition = batched_push(&runtime, "any", 800);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..6 { runtime.pump(time).await.unwrap(); }
    enqueue_item(&mut runtime, "w", "two", definition);
    // One sweep: CI is polled for the group while the new ticket's first step runs beside it.
    runtime.pump(9).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["two"].stage, 1, "the ticket moved on while the group waited: {state:?}");
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
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(outbox(&runtime, StageKind::Run), 1, "a failing exit is the candidate's answer: one run, not one per attempt");
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
