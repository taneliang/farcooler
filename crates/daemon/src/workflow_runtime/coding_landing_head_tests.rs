//! Landing behavior when the target branch moves, before queueing or during a gate.
use super::{adapters, CodingConfigExt, CodingReviewer, Request, Runtime};
use super::coding_verbs_tests::{
    at_join, batched_push, candidate_of, command, lander, outbox, phase_of, remote_head,
    resolve_fixture_conflicts, settled, until,
};
use farcooler_protocol::workflow_commands::{Action, Mutation};
use farcooler_workflow::{step, Command, Definition, EffectKind, Phase, Refusal, Reviewer, StageKind, State, Target};
use farcooler_workflow::scheduling::{Budget, StartPolicy};

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

fn waiting_definition(runtime: &Runtime, auto_queue: bool) -> Definition {
    let mut definition = batched_push(runtime, "any", 5000);
    let join = definition.stages.last_mut().unwrap();
    join.limit = 1;
    join.routes.insert("rejected".into(), Target::Step("build".into()));
    if auto_queue {
        definition.start_policy = Some(StartPolicy {
            columns: ["todo".into()].into(), budget: Budget::Fixed(1), files: Default::default(),
            dependencies: Default::default(), rotate_models: false, auto_queue: true,
        });
    }
    definition
}

#[tokio::test]
async fn workflow_send_back_restarts_a_waiting_builder_and_charges_the_join_limit() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = waiting_definition(&runtime, false);
    at_join(&mut runtime, "ticket", &definition).await;
    let reason = "Fix the compile error against the new main.";
    let send_back = |runtime: &Runtime, sequence| {
        let state = runtime.state("w").unwrap().unwrap();
        let ticket = &state.items["ticket"];
        runtime.wire_command("w", "orchestrator", Action::SendBack {
            mutation: Mutation { id: format!("send-back-{sequence}"), revision: state.revision, epoch: state.epoch, sequence },
            item: "ticket".into(), run: ticket.run.clone(), pin: ticket.pin.clone(), reason: reason.into(),
        }).unwrap()
    };
    let reply = send_back(&runtime, 1);
    assert!(reply.pointer("/receipt/result/Ok").is_some(), "{reply}");
    let state = until(&mut runtime, |s| s.items["ticket"].phase == Phase::WaitingJoin
        && s.items["ticket"].sent_back.len() == 1).await;
    assert_eq!(state.items["ticket"].back_edges["join"], 1);
    let builds: Vec<_> = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .filter(|s| s.effect.kind == EffectKind::Execute(StageKind::Build)).collect();
    assert_eq!(builds.len(), 2);
    assert_ne!(builds[0].effect.run, builds[1].effect.run);
    assert_eq!(builds[1].effect.brief.as_deref(), Some(reason));
    let refused = send_back(&runtime, 2);
    assert_eq!(refused.pointer("/receipt/result/Err/SendBackRefused").and_then(|v| v.as_str()),
        Some("join: The back-edge limit of 1 has been reached."), "{refused}");
}

#[tokio::test]
async fn automatic_queue_sends_two_conflicts_back_and_leaves_the_join_limit_for_a_gate_rejection() {
    let (dir, mut runtime) = lander("conflict").await;
    let definition = waiting_definition(&runtime, true);
    at_join(&mut runtime, "ticket", &definition).await;
    let tree = runtime.item_tree("w", "ticket");
    let script = dir.path().join("adapter.py");
    let remote = &runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository;
    resolve_fixture_conflicts(&script, remote);
    let source = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, source.replace("elif stage == 'gate':", "elif stage == 'gate':\n    sys.exit(1)")).unwrap();
    for (n, change) in ["first change to shared.txt", "second change to shared.txt"].into_iter().enumerate() {
        let before = runtime.state("w").unwrap().unwrap();
        let candidate = before.items["ticket"].candidate.as_deref().unwrap();
        let main = move_waiting_ticket_main(&dir, &runtime, "shared.txt", change).await;
        assert!(runtime.check_auto_queues(&["w".into()], 10).await.unwrap().is_empty());

        let state = runtime.state("w").unwrap().unwrap();
        let ticket = &state.items["ticket"];
        assert_eq!(ticket.definition.stages[ticket.stage].kind, StageKind::Build, "conflict with {main}");
        assert!(state.items.values().all(|item| item.group_of.is_none()));
        assert_eq!(outbox(&runtime, StageKind::Fold), 0);
        assert_eq!(adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap(), candidate);
        assert!(adapters::git(&tree, &["status", "--porcelain"]).await.unwrap().is_empty());
        let builds: Vec<_> = runtime.store.workflow_outbox("w").unwrap().into_iter()
            .filter(|s| s.effect.kind == EffectKind::Execute(StageKind::Build)).collect();
        assert_eq!(builds.len(), n + 2);
        let brief = builds[n + 1].effect.brief.as_deref().unwrap();
        assert!(brief.contains(&main) && brief.contains("shared.txt"), "{brief}");
        until(&mut runtime, |s| phase_of(s, "ticket") == Some(Phase::WaitingJoin)).await;
    }
    let state = until(&mut runtime, |s| {
        let ticket = &s.items["ticket"];
        ticket.phase == Phase::Failed || ticket.back_edges.get("join") == Some(&1)
    }).await;
    let ticket = &state.items["ticket"];
    assert_eq!(ticket.definition.stages[ticket.stage].kind, StageKind::Build);
    assert_ne!(ticket.phase, Phase::Failed);
    assert_eq!(ticket.back_edges.get("join"), Some(&1));
    assert_eq!(ticket.iteration, 1);
    assert_eq!(outbox(&runtime, StageKind::Run), 1);

    let mut replay = State::empty("w".into(), 1);
    for event in runtime.events("w").unwrap() { replay = step(&replay, &event).unwrap().state; }
    assert_eq!(state, replay);
}

#[tokio::test]
async fn waiting_ticket_queue_checks_main_before_admitting_a_conflict() {
    let (dir, mut runtime) = lander("conflict").await;
    let definition = waiting_definition(&runtime, false);
    at_join(&mut runtime, "ticket", &definition).await;
    let before = runtime.state("w").unwrap().unwrap();
    let main = move_waiting_ticket_main(&dir, &runtime, "shared.txt", "another landing changed this file").await;
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
async fn review_after_a_landing_send_back_only_sees_the_rebased_ticket() {
    let (dir, mut runtime) = lander("conflict").await;
    let script = dir.path().join("adapter.py");
    let seen = dir.path().join("review-range");
    let mut config = runtime.coding.clone().unwrap();
    let mut review = config.checks[0].clone();
    review.argv[2] = "review".into();
    review.environment.insert("REVIEW_RANGE".into(), seen.to_string_lossy().into_owned());
    config.reviewer = Some(CodingReviewer { identity: "reviewer".into(), command: review });
    let root = runtime.root.clone();
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    drop(runtime);
    runtime = Runtime::open(&root).unwrap();
    let mut definition = waiting_definition(&runtime, false);
    definition.stages.insert(3, config.stage("review", StageKind::Review(Reviewer::Dispatch), "coding-review/v1", "reviewer"));
    let source = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, source.replace("elif stage == 'review':\n", r#"elif stage == 'review':
    base = os.environ['FARCOOLER_BASE']
    g = lambda *args: subprocess.check_output(['/usr/bin/git', *args], text=True).strip()
    pathlib.Path(os.environ['REVIEW_RANGE']).write_text(json.dumps([
        base, g('log', '--format=%s', base + '..' + candidate), g('diff', '--name-only', base, candidate)]))
"#).replace("passed=iteration != '0' and mode != 'review-reject'", "passed=True")).unwrap();
    at_join(&mut runtime, "ticket", &definition).await;
    let original = remote_head(&runtime).await;
    assert_eq!(serde_json::from_slice::<Vec<String>>(&std::fs::read(&seen).unwrap()).unwrap(),
        [original, "build 0".into(), "shared.txt".into()]);

    let rebased_onto = rebase_after_queue_send_back(&dir, &mut runtime).await;
    assert_eq!(outbox(&runtime, StageKind::Review(Reviewer::Dispatch)), 2);
    assert_eq!(serde_json::from_slice::<Vec<String>>(&std::fs::read(&seen).unwrap()).unwrap(),
        [rebased_onto, "build 0\nbuild 0".into(), "shared.txt".into()]);
}

#[tokio::test]
async fn agent_review_and_parallel_check_see_the_same_rebased_ticket() {
    use super::{coding::AGENT_REVIEWER, coding_native::{TEST_ARGV, tests::native_in}, gate_tests::AGENT};
    use crate::review_agent::ReviewOptions;

    let (dir, runtime) = lander("conflict").await;
    let (dir, mut runtime, _fake) = native_in(dir, runtime, "pass", "claude").await;
    let agent = dir.path().join("range-agent.py");
    let record = dir.path().join("record-range.py");
    std::fs::write(&record, r#"import json, os, pathlib, subprocess, sys
base = os.environ['FARCOOLER_BASE']
candidate = os.environ['FARCOOLER_CANDIDATE']
def git(*args):
    return subprocess.check_output(['/usr/bin/git', *args], text=True).strip()
pathlib.Path(__file__).with_name(sys.argv[1]).write_text(json.dumps([
    base, git('log', '--format=%s', base + '..' + candidate), git('diff', '--name-only', base, candidate)]))
"#).unwrap();
    let (review, _) = AGENT.split_once("\nelse:\n").unwrap();
    let review = review.replace("    log =", concat!(
        "    subprocess.check_call([sys.executable, pathlib.Path(__file__).with_name('record-range.py'), 'agent-range'])\n",
        "    log =",
    ));
    std::fs::write(&agent, format!("{review}{}", r#"
else:
    subprocess.check_call([sys.executable, pathlib.Path(__file__).with_name('adapter.py'), 'build'])
hook('Stop')
time.sleep(30)
"#)).unwrap();
    TEST_ARGV.with(|argv| *argv.borrow_mut() = Some(vec![
        "/usr/bin/python3".into(), agent.to_string_lossy().into_owned(), "subdir".into(),
    ]));

    let config = runtime.coding.as_mut().unwrap();
    let index = config.checks.len();
    let mut check = config.checks[0].clone();
    check.argv = vec!["/usr/bin/python3".into(), record.to_string_lossy().into_owned(), "check-range".into()];
    config.checks.push(check);
    let mut definition = waiting_definition(&runtime, false);
    let mut review = runtime.coding.as_ref().unwrap().stage("review", StageKind::Review(Reviewer::Agent), "coding-review/v1", AGENT_REVIEWER);
    review.operation.arguments.push(ReviewOptions { check: Some(index), ..Default::default() }.argument());
    definition.stages.insert(3, review);
    at_join(&mut runtime, "ticket", &definition).await;
    let observed = |name| serde_json::from_slice::<Vec<String>>(&std::fs::read(dir.path().join(name)).unwrap()).unwrap();
    let original = [remote_head(&runtime).await, "build 0".into(), "shared.txt".into()];
    assert_eq!(observed("agent-range"), original);
    assert_eq!(observed("check-range"), original);

    let rebased_onto = rebase_after_queue_send_back(&dir, &mut runtime).await;
    assert_eq!(outbox(&runtime, StageKind::Review(Reviewer::Agent)), 2);
    let expected = [rebased_onto, "build 0\nbuild 0".into(), "shared.txt".into()];
    assert_eq!(observed("agent-range"), expected);
    assert_eq!(observed("check-range"), expected);
}

async fn rebase_after_queue_send_back(dir: &tempfile::TempDir, runtime: &mut Runtime) -> String {
    let script = dir.path().join("adapter.py");
    let before = runtime.state("w").unwrap().unwrap();
    move_waiting_ticket_main(dir, runtime, "independent.txt", "landed elsewhere").await;
    let rebased_onto = move_waiting_ticket_main(dir, runtime, "shared.txt", "conflicting landing").await;
    let receipt = runtime.submit(&runtime.controller(), Request {
        id: "queue-before-review-rebase".into(), workspace: "w".into(), expected_revision: before.revision,
        sequence: 2, command: Command::Queue { item: "ticket".into() },
    }, 10).await.unwrap();
    assert_eq!(receipt.result, Err(Refusal::Stale));
    // Advance main again after the rebase target was named. Build onto the named SHA,
    // so the review needs the common ancestor rather than the latest target tip.
    let source = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, source.replace("if stage == 'build':",
        &format!("if stage == 'build':\n    subprocess.check_call(['/usr/bin/git', 'rebase', '--no-autostash', '-X', 'theirs', {rebased_onto:?}])"))).unwrap();
    move_waiting_ticket_main(dir, runtime, "later.txt", "landed after the send-back").await;
    until(runtime, |s| phase_of(s, "ticket") == Some(Phase::WaitingJoin)).await;
    rebased_onto
}

#[tokio::test]
async fn waiting_ticket_queues_a_signed_candidate_unchanged_after_main_moves() {
    let (dir, mut runtime) = lander("pass").await;
    let key = dir.path().join("signing-key");
    assert!(tokio::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&key).status().await.unwrap().success());
    let repository = runtime.repository();
    adapters::git(&repository, &["config", "gpg.format", "ssh"]).await.unwrap();
    adapters::git(&repository, &["config", "user.signingkey", key.to_str().unwrap()]).await.unwrap();
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
    std::fs::write(&file, "local edit").unwrap();
    let main = move_waiting_ticket_main(&dir, &runtime, "independent.txt", "another landing changed this file").await;

    assert_ne!(adapters::git(&tree, &["rev-parse", "HEAD^"]).await.unwrap(), main);
    let worktrees = adapters::git(&repository, &["worktree", "list", "--porcelain"]).await.unwrap();
    let receipt = runtime.submit(&runtime.controller(), Request { id: "queue-after-independent-change".into(),
        workspace: "w".into(), expected_revision: before.revision, sequence: 2,
        command: Command::Queue { item: "ticket".into() } }, 12).await.unwrap();
    assert!(receipt.result.is_ok(), "{receipt:?}");
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["ticket"].phase, Phase::Joined);
    assert_eq!(candidate_of(&state, "ticket"), Some(previous));
    let member = state.items.values().find_map(|i| i.members.first()).unwrap();
    assert_eq!(member.candidate, previous);
    assert_eq!(adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap(), previous);
    assert_eq!(adapters::git(&repository, &["worktree", "list", "--porcelain"]).await.unwrap(), worktrees);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "local edit");
    let allowed_signers = dir.path().join("allowed-signers");
    std::fs::write(&allowed_signers, format!("test@localhost {}", std::fs::read_to_string(key.with_extension("pub")).unwrap())).unwrap();
    assert!(tokio::process::Command::new("/usr/bin/git").current_dir(&tree)
        .args(["-c", &format!("gpg.ssh.allowedSignersFile={}", allowed_signers.display()), "verify-commit", previous])
        .status().await.unwrap().success());
    let mut replay = State::empty("w".into(), 1);
    for event in runtime.events("w").unwrap() { replay = step(&replay, &event).unwrap().state; }
    assert_eq!(state, replay);
}

async fn move_waiting_ticket_main(dir: &tempfile::TempDir, runtime: &Runtime, path: &str, content: &str) -> String {
    let other = dir.path().join("other-main");
    let remote = &runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository;
    if !other.exists() {
        assert!(tokio::process::Command::new("/usr/bin/git")
            .args(["clone", "-q", "-b", "main", remote.to_str().unwrap(), other.to_str().unwrap()])
            .status().await.unwrap().success());
    }
    std::fs::write(other.join(path), content).unwrap();
    adapters::git(&other, &["add", "--", path]).await.unwrap();
    adapters::git(&other, &["-c", "user.name=Other", "-c", "user.email=o@localhost",
        "-c", "commit.gpgsign=false", "commit", "-qm", "other landing"]).await.unwrap();
    assert!(tokio::process::Command::new("/usr/bin/git").current_dir(&other)
        .args(["push", "-q", "origin", "HEAD:refs/heads/main"]).status().await.unwrap().success());
    adapters::git(&other, &["rev-parse", "HEAD"]).await.unwrap()
}

#[tokio::test]
async fn a_stale_index_lock_in_the_reviewed_checkout_does_not_hold_queue() {
    let (dir, mut runtime) = lander("pass").await;
    let definition = waiting_definition(&runtime, false);
    at_join(&mut runtime, "ticket", &definition).await;
    let before = runtime.state("w").unwrap().unwrap();
    let candidate = candidate_of(&before, "ticket").unwrap();
    let main = move_waiting_ticket_main(&dir, &runtime, "independent.txt", "another landing changed this file").await;
    let tree = runtime.item_tree("w", "ticket");
    // Reproduce ov-1419: the worktree is clean, but an in-place rebase cannot write its index.
    let lock = adapters::git(&tree, &["rev-parse", "--path-format=absolute", "--git-path", "index.lock"]).await.unwrap();
    std::fs::write(&lock, b"").unwrap();
    assert!(adapters::git(&tree, &["status", "--porcelain"]).await.unwrap().is_empty());

    let receipt = runtime.submit(&runtime.controller(), Request {
        id: "queue-with-locked-index".into(), workspace: "w".into(), expected_revision: before.revision,
        sequence: 2, command: Command::Queue { item: "ticket".into() },
    }, 10).await.unwrap();
    assert!(receipt.result.is_ok(), "{receipt:?}");
    let after = runtime.state("w").unwrap().unwrap();
    assert_eq!(after.items["ticket"].phase, Phase::Joined);
    assert_eq!(candidate_of(&after, "ticket"), Some(candidate));
    assert_eq!(adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap(), candidate);
    assert_ne!(adapters::git(&tree, &["merge-base", &main, candidate]).await.unwrap(), main);
    assert!(std::path::Path::new(&lock).is_file());
}

#[tokio::test]
async fn a_held_queue_explains_its_missing_target_notifies_once_and_retries() {
    use super::wire_commands::Prepared;
    use farcooler_protocol::workflow_config::NoticePoint;

    let (_dir, mut runtime) = lander("pass").await;
    let definition = waiting_definition(&runtime, false);
    at_join(&mut runtime, "ticket", &definition).await;
    let head = remote_head(&runtime).await;
    let landing = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().clone();
    adapters::git(&landing.repository, &["update-ref", "-d", &landing.target, &head]).await.unwrap();
    let before = runtime.state("w").unwrap().unwrap();
    let item = &before.items["ticket"];
    let action = Action::Queue {
        mutation: Mutation { id: "queue-held".into(), epoch: before.epoch, revision: before.revision, sequence: 1 },
        item: "ticket".into(), run: item.run.clone(), pin: item.pin.clone(),
    };
    let check = runtime.wire_queue_check("w", "orchestrator", &action).await.unwrap();
    let queue_hold = runtime.apply_queue_check("w", "ticket", check, super::now()).unwrap();
    let reply = runtime.wire_command_with("w", "orchestrator", &["orchestrator".into()], None, action.clone(),
        Prepared { queue_hold, ..Prepared::default() }).unwrap();
    let why = "Could not read or fetch the landing target head (local landing target could not be resolved).";
    assert_eq!(reply.pointer("/receipt/result/Err/Held").and_then(|v| v.as_str()), Some(why), "{reply}");
    assert_eq!(runtime.wire_command("w", "orchestrator", action).unwrap(), reply);
    let get = runtime.wire_command("w", "orchestrator", Action::Get).unwrap();
    assert_eq!(get.pointer("/state/items/ticket/queue_hold/reason").and_then(|v| v.as_str()), Some(why), "{get}");

    let since = runtime.state("w").unwrap().unwrap().items["ticket"].queue_hold.as_ref().unwrap().since_ms;
    let notices = |runtime: &Runtime| runtime.store.workflow_notice_deliveries("w").unwrap().into_iter()
        .filter(|d| d.notice.point == NoticePoint::QueueHeld).collect::<Vec<_>>();
    // Restart before retry to verify that the clock and requested queue survive the runner.
    let root = runtime.root.clone();
    drop(runtime);
    let runtime = Runtime::open(&root).unwrap();
    let workspaces = ["w".into()];
    let deadline = since + farcooler_workflow::QUEUE_HOLD_NOTICE_MS;
    assert!(runtime.check_auto_queues(&workspaces, deadline - 60_000).await.unwrap().is_empty());
    assert!(notices(&runtime).is_empty());
    for attempt in [deadline, deadline + 60_000] {
        assert!(runtime.check_auto_queues(&workspaces, attempt).await.unwrap().is_empty());
        let notices = notices(&runtime);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].notice.reason.as_deref(), Some(why));
    }
    adapters::git(&landing.repository, &["update-ref", &landing.target, &head]).await.unwrap();
    let now_ms = deadline + 120_000;
    let ready = runtime.check_auto_queues(&workspaces, now_ms).await.unwrap();
    assert!(ready.contains(&("w".into(), "ticket".into())));
    runtime.schedule_ready(&workspaces, now_ms, &ready).unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["ticket"].phase, Phase::Joined);
    assert!(state.items["ticket"].queue_hold.is_none());
    assert_eq!(candidate_of(&state, "ticket"), item.candidate.as_deref());
}
