//! Builds that end with no new commit: which of them fail, with what sentence, and which keep earlier work.
use super::*;
use super::tests::native_agent;
use super::super::{coding_tests::admitted, tests::{enqueue, finish}};

#[tokio::test]
async fn ov_1217_build_without_a_commit_reports_ended_without_changes_and_last_message() {
    let (_dir, mut runtime, _fake) = native_agent("no-changes", "codex").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    let state = finish(&mut runtime, "w").await;
    let item = &state.items["one"];
    assert_eq!(item.phase, Phase::Failed, "{state:?}");
    assert_eq!(item.failure.as_deref(), Some("The build ended without changes: it made no new commit.\n\nThe agent's last message:\nI could not read the ticket; the runner request hung."));
    assert!(item.candidate.is_none());
}
#[tokio::test]
async fn ov_1235_cursor_build_without_a_commit_reports_the_last_message_from_its_transcript() {
    let (_dir, mut runtime, _fake) = native_agent("no-changes", "cursor").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    let state = finish(&mut runtime, "w").await;
    let item = &state.items["one"];
    assert_eq!(item.phase, Phase::Failed, "{state:?}");
    assert_eq!(item.failure.as_deref(), Some("The build ended without changes: it made no new commit.\n\nThe agent's last message:\nI could not read the ticket; the runner request hung."));
}
#[tokio::test]
async fn ov_1235_cursor_build_without_a_transcript_says_its_last_message_is_unavailable() {
    let (_dir, mut runtime, _fake) = native_agent("no-changes-bare", "cursor").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    let state = finish(&mut runtime, "w").await;
    let item = &state.items["one"];
    assert_eq!(item.phase, Phase::Failed, "{state:?}");
    assert_eq!(item.failure.as_deref(), Some("The build ended without changes: it made no new commit.\n\nThe agent's last message: unavailable for this agent."));
}
#[tokio::test]
async fn ov_1217_dirty_build_without_a_commit_reports_reason_files_and_last_message() {
    let (_dir, mut runtime, _fake) = native_agent("dirty-no-commit", "codex").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    let state = finish(&mut runtime, "w").await;
    let item = &state.items["one"];
    assert_eq!(item.phase, Phase::Failed, "{state:?}");
    assert_eq!(item.failure.as_deref(), Some("The build ended without changes: it made no new commit.\n\n\
        The build ended its turn with changes it hadn't committed: result.txt. \
        They're still in its worktree, and the build carries on from them.\n\n\
        The agent's last message:\nI could not read the ticket; the runner request hung."));
    assert!(item.candidate.is_none());
    assert_eq!(std::fs::read_to_string(runtime.item_tree("w", "one").join("result.txt")).unwrap(), "work kept without a commit");
}
/// An adopted branch can sit behind the run's base: its HEAD differs from the base, yet it holds nothing beyond it.
#[tokio::test]
async fn ov_1492_build_on_a_tree_behind_the_base_still_ends_without_changes() {
    let (_dir, mut runtime, _fake) = native_agent("no-changes", "codex").await;
    let repo = runtime.repository();
    adapters::git(&repo, &["-c", "user.name=Test", "-c", "user.email=test@localhost", "-c", "commit.gpgsign=false",
        "commit", "--allow-empty", "--quiet", "-m", "base moved on"]).await.unwrap();
    let base = adapters::git(&repo, &["rev-parse", "HEAD"]).await.unwrap();
    adapters::git(&repo, &["update-ref", runtime.landing_ref(), &base]).await.unwrap();
    runtime.coding.as_mut().unwrap().base = base;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let tree = runtime.tree(&binding);
    // The same files, one commit back, so the tree stays clean.
    adapters::git(&tree, &["update-ref", "HEAD", &format!("{}~1", binding.base)]).await.unwrap();
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let outcome = runtime.execute_coding("w", &binding, cancelled).await.unwrap();
    assert!(matches!(outcome, Outcome::SafeFailureBecause(ref why)
        if why.starts_with("The build ended without changes: it made no new commit.")), "{outcome:?}");
}
#[tokio::test]
async fn ov_1492_retry_keeps_committed_work_from_interrupted_attempt() {
    use super::super::{Command, Grant};

    let (dir, mut runtime, _fake) = native_agent("commit-then-interrupt", "codex").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let tree = runtime.tree(&binding);
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let interrupt = async {
        let committed = super::super::wait_until(tokio::time::Instant::now() + Duration::from_secs(10),
            Duration::from_millis(10), || dir.path().join("committed").exists()).await;
        cancel.send_replace(true);
        committed
    };
    let (outcome, committed_before_interrupt) = tokio::join!(runtime.execute_coding("w", &binding, cancelled), interrupt);
    let outcome = outcome.unwrap();
    assert!(committed_before_interrupt, "the first build did not commit: {outcome:?}");
    assert_eq!(outcome, Outcome::SafeFailure);
    let committed = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
    assert_ne!(committed, binding.base);
    assert!(runtime.observe("w", &binding.effect, outcome, false, 11).unwrap());
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Failed);

    let retry = runtime.internal("w", Command::Retry { item: "one".into() }, Grant::Controller, 12).unwrap();
    assert!(retry.result.is_ok(), "{retry:?}");
    runtime.pump(13).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    let item = &state.items["one"];
    assert_eq!(item.phase, Phase::Running, "{item:?}");
    assert_eq!(item.stage, 2);
    assert_eq!(item.candidate.as_deref(), Some(committed.as_str()));
}
