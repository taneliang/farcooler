//! An Uncertain effect recovery can't settle is looked at once, then less and less often (ov-1301).
use super::*;
use super::coding_tests::{admitted, configured};
use super::tests::enqueue;

/// The Canary daemon re-checked eight such builds every second: 5,000 warnings an hour at 70 percent CPU.
#[tokio::test]
async fn an_uncertain_build_whose_receipt_is_missing_is_checked_once_then_backed_off() {
    let (_dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap(); // Provision; the build is next.
    // The build is admitted and the daemon dies before it starts: no process fact, no log, no owner.
    admitted(&runtime).await;
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
    assert_eq!(item.phase, Phase::Uncertain, "held for the orchestrator, never guessed");
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
