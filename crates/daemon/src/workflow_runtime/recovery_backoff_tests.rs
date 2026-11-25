//! An Uncertain effect recovery can't settle is looked at once, then less and less often (ov-1301).
use super::{Runtime, Command, Grant, Outcome};
use super::recovery_backoff::FIRST_MS;
use super::coding_tests::{admitted, configured};
use farcooler_workflow::Phase;
use super::tests::enqueue;

/// The Canary daemon re-checked eight such builds every second: 5,000 warnings an hour at 70 percent CPU.
#[tokio::test]
async fn an_uncertain_build_whose_receipt_is_missing_is_checked_once_then_backed_off() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap(); // Provision; the build is next.
    // The command wrote its log before the daemon lost it. Without an owner or
    // process receipt, repeated checks cannot prove that it has exited.
    let binding = admitted(&runtime).await;
    std::fs::write(runtime.process_log(&binding), b"command started").unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    let mut per_sweep = Vec::new();
    for tick in 0..100 {
        let before = runtime.recovery.looks;
        runtime.pump(1_000 + tick * 1_000).await.unwrap();
        per_sweep.push(runtime.recovery.looks - before);
    }
    let item = &runtime.state("w").unwrap().unwrap().items["one"];
    assert_eq!(item.phase, Phase::Uncertain, "a process may still be running");
    let needs = crate::engine_stuck::uncertain(&runtime.store);
    assert_eq!(needs.len(), 1, "one item after repeated inconclusive checks");
    assert_eq!(needs[0].item, "one");
    assert!(needs[0].question.contains("one") && needs[0].question.contains("command may still be running"), "{needs:?}");
    assert!(per_sweep.iter().all(|&n| n <= 1), "one look per sweep at most: {per_sweep:?}");
    let looks: usize = per_sweep.iter().sum();
    // At 0, 5, 15, 35 and 75 seconds: five looks in a hundred one-second sweeps, where every sweep looked before.
    assert!((2..=6).contains(&looks), "{looks} looks in 100 sweeps: {per_sweep:?}");
    assert_eq!(per_sweep[0], 1, "the first sweep looks at once");
    assert!(runtime.recovery.warnings <= 2, "{} warnings in 100 sweeps", runtime.recovery.warnings);
    // The orchestrator's word (any command) has the next sweep look again at once.
    runtime.recovery.wake();
    let before = runtime.recovery.looks;
    runtime.pump(101_500).await.unwrap();
    assert_eq!(runtime.recovery.looks - before, 1);
}

#[tokio::test]
async fn an_uncertain_build_that_never_started_fails_after_the_grace_period_and_can_retry() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(1_000).await.unwrap();
    runtime.recovery.wake(); // Even an early wake for receipts cannot shorten the grace period.
    runtime.pump(1_000 + FIRST_MS - 1_000).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].phase, Phase::Uncertain);
    assert!(crate::engine_stuck::uncertain(&runtime.store).is_empty());
    runtime.pump(1_000 + FIRST_MS + 1_000).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items["one"].phase, Phase::Failed);
    assert!(state.items["one"].failure.as_deref().unwrap().contains("nothing of it is still running"));
    assert!(runtime.tree(&binding).is_dir());
    let retry = runtime.internal("w", Command::Retry { item: "one".into() }, Grant::Controller, 10_000).unwrap();
    assert!(retry.result.is_ok());
    runtime.pump(10_001).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].stage, 2);
}

#[tokio::test]
async fn an_uncertain_provision_checks_the_checkout_and_settles_after_the_grace_period() {
    for intact in [false, true] {
        let (_dir, mut runtime) = configured(false, "pass").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        let binding = admitted(&runtime).await;
        if intact {
            let (_cancel, cancelled) = tokio::sync::watch::channel(false);
            assert!(matches!(runtime.execute_coding("w", &binding, cancelled).await.unwrap(), Outcome::Passed(_)));
            std::fs::remove_file(runtime.git_settlement(&binding)).unwrap();
        }
        runtime.pump(1_000).await.unwrap();
        runtime.pump(1_000 + FIRST_MS - 1_000).await.unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
        runtime.pump(1_000 + FIRST_MS + 1_000).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        if intact {
            assert_eq!((state.items["one"].stage, &state.items["one"].phase), (1, &Phase::Running));
        } else {
            assert_eq!(state.items["one"].phase, Phase::Failed);
            assert!(state.items["one"].failure.as_deref().unwrap().contains("nothing was made"));
            let retry = runtime.internal("w", Command::Retry { item: "one".into() }, Grant::Controller, 10_000).unwrap();
            assert!(retry.result.is_ok());
            runtime.pump(10_001).await.unwrap();
            assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].stage, 1);
        }
    }
}
