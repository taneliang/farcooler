//! Landing behavior when the target branch moves, before queueing or during a gate.
use super::{adapters, Request, Runtime};
use super::coding_verbs_tests::{
    at_join, batched_push, candidate_of, command, lander, outbox, phase_of, remote_head, settled, until,
};
use farcooler_workflow::{step, Command, EffectKind, Phase, Refusal, StageKind, State};

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
async fn waiting_ticket_holds_the_rebase_for_a_dirty_submodule() {
    let (dir, mut runtime) = lander("pass").await;
    let source = dir.path().join("module-source");
    std::fs::create_dir(&source).unwrap();
    adapters::git(&source, &["init", "-q"]).await.unwrap();
    std::fs::write(source.join("tracked.txt"), "committed").unwrap();
    adapters::git(&source, &["add", "tracked.txt"]).await.unwrap();
    adapters::git(&source, &["-c", "user.name=Test", "-c", "user.email=test@localhost",
        "-c", "commit.gpgsign=false", "commit", "-qm", "submodule base"]).await.unwrap();
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap().replace("if stage == 'build':", &format!(r#"if stage == 'build':
    subprocess.check_call(['/usr/bin/git', '-c', 'protocol.file.allow=always', 'submodule', 'add', '-q', {}, 'module'])
"#, serde_json::to_string(&source).unwrap()));
    std::fs::write(&script, text).unwrap();
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "ticket", &definition).await;
    let before = runtime.state("w").unwrap().unwrap();
    let candidate = before.items["ticket"].candidate.as_deref().unwrap();
    let tree = runtime.item_tree("w", "ticket");
    let tracked = tree.join("module/tracked.txt");
    let untracked = tree.join("module/untracked.txt");
    let main = move_waiting_ticket_main(&dir, &runtime, "independent.txt").await;

    std::fs::write(&tracked, "local edit").unwrap();
    assert!(adapters::git(&tree, &["status", "--porcelain"]).await.unwrap().is_empty());
    assert!(!runtime.prepare_queue("w", "ticket", before.revision, None, None, 10).await);
    assert_eq!(std::fs::read_to_string(&tracked).unwrap(), "local edit");
    std::fs::write(&tracked, "committed").unwrap();

    std::fs::write(&untracked, "local file").unwrap();
    runtime.pump(11).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap(), before);
    assert_eq!(adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap(), candidate);
    assert_eq!(std::fs::read_to_string(&untracked).unwrap(), "local file");
    std::fs::remove_file(&untracked).unwrap();

    runtime.pump(12).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["ticket"].phase, Phase::WaitingJoin);
    let head = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
    assert_ne!(head, candidate);
    assert_eq!(candidate_of(&state, "ticket"), Some(head.as_str()));
    assert_eq!(adapters::git(&tree, &["rev-parse", "HEAD^"]).await.unwrap(), main);
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
