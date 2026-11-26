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
    for (intact, legacy) in [(false, false), (true, false), (false, true), (true, true)] {
        let (_dir, mut runtime) = configured(false, "pass").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        let binding = admitted(&runtime).await;
        if intact {
            let (_cancel, cancelled) = tokio::sync::watch::channel(false);
            assert!(matches!(runtime.execute_coding("w", &binding, cancelled).await.unwrap(), Outcome::Passed(_)));
            std::fs::remove_file(runtime.git_settlement(&binding)).unwrap();
        }
        if legacy { std::fs::remove_file(runtime.provision_lock_path(&binding.effect)).unwrap(); }
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

/// The Git child outlives the process that launched it. Recovery must pause
/// even when the checkout looks ready or when Git hasn't created anything yet.
#[tokio::test]
async fn provision_recovery_waits_for_git_that_survived_the_runner() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    for (intact, legacy) in [(false, false), (true, false), (false, true), (true, true)] {
        let (dir, mut runtime) = configured(false, "pass").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        let binding = admitted(&runtime).await;
        let root = runtime.root.clone();
        let tree = runtime.tree(&binding);
        let script = dir.path().join("delayed-git");
        std::fs::write(&script, r#"#!/bin/sh
for arg do
    if [ "$arg" = "$STOP_AT" ]; then
        exec >"$CONTROL/git.log" 2>&1
        if [ "$STOP_AT" = reset ]; then
            "$REAL_GIT" "$@" || exit
        fi
        touch "$CONTROL/ready"
        for i in $(seq 1 300); do
            [ -d "$CONTROL" ] || exit 1
            if [ -f "$CONTROL/release" ]; then
                "$REAL_GIT" "$@" || exit
                if [ "$STOP_AT" = add ]; then
                    unset GIT_WORK_TREE
                    "$REAL_GIT" -C "$TREE" reset -q --hard || exit
                fi
                touch "$CONTROL/done"
                exit
            fi
            sleep .1
        done
        exit 1
    fi
done
exec "$REAL_GIT" "$@"
"#).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        drop(runtime);
        let mut runner = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", concat!(module_path!(), "::provision_crash_worker")
                .trim_start_matches("farcooler_daemon::"), "--ignored", "--nocapture"])
            .env("PROVISION_TEST_ROOT", &root)
            .env("PROVISION_TEST_GIT", &script)
            .env("REAL_GIT", crate::git::absolute_git().unwrap())
            .env("STOP_AT", if intact { "reset" } else { "add" })
            .env("CONTROL", dir.path())
            .env("TREE", &tree)
            .kill_on_drop(true)
            .spawn().unwrap();
        let wait_for = |name| {
            let path = dir.path().join(name);
            let log = dir.path().join("git.log");
            async move {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while !path.exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
                }).await.unwrap_or_else(|_| panic!("Git didn't write {}: {}", path.display(),
                    std::fs::read_to_string(log).unwrap_or_default()));
            }
        };
        wait_for("ready").await;
        assert_eq!(tree.is_dir(), intact);
        runner.kill().await.unwrap();

        let mut runtime = Runtime::open(&root).unwrap();
        if legacy { std::fs::remove_file(runtime.provision_lock_path(&binding.effect)).unwrap(); }
        runtime.pump(1_000).await.unwrap();
        // The first sweep accounts for time spent before it reaches the effect.
        runtime.pump(1_000 + FIRST_MS + 1_000).await.unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
        assert!(!runtime.git_settlement(&binding).exists());
        assert!(!runtime.unstarted_recorded(&binding).unwrap());
        let needs = crate::engine_stuck::uncertain(&runtime.store);
        assert_eq!(needs.len(), 1);
        let finding = if legacy { "a process in the repository or worktree may still be running" }
            else { "provisioning Git process may still be running" };
        assert!(needs[0].question.contains(finding), "{needs:?}");

        std::fs::write(dir.path().join("release"), "").unwrap();
        wait_for("done").await;
        for tick in 1..=5 {
            runtime.pump(1_000 + tick * 4 * FIRST_MS).await.unwrap();
            if runtime.state("w").unwrap().unwrap().items["one"].phase != Phase::Uncertain { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!((state.items["one"].stage, &state.items["one"].phase), (1, &Phase::Running));
        assert!(crate::engine_stuck::uncertain(&runtime.store).is_empty());
    }
}

#[tokio::test]
#[ignore = "subprocess of provision_recovery_waits_for_git_that_survived_the_runner"]
async fn provision_crash_worker() {
    use super::{Binding, StageKind};
    let Some(root) = std::env::var_os("PROVISION_TEST_ROOT").map(std::path::PathBuf::from) else { return };
    let runtime = Runtime::open(&root).unwrap();
    let stored = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap();
    assert_eq!(stored.effect.kind, super::EffectKind::Execute(StageKind::Provision));
    let base = runtime.item_base("w", &stored.effect).unwrap();
    let binding = Binding::new("w", &stored.effect, stored.resource_generation.unwrap(), 0, &base);
    crate::git::PROGRAM.with(|program| *program.borrow_mut() = std::env::var_os("PROVISION_TEST_GIT"));
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let result = runtime.execute_coding("w", &binding, cancelled).await;
    panic!("the runner must be killed before Provision returns: {result:?}");
}
