//! The orchestrator settles a build left Uncertain once its process is gone, and a cancel of one ends (ov-1301).
//! On the Canary, ov-1296 was Uncertain after a restart and neither resolve nor send-back settled it; cancelling
//! ov-1241 and ov-1255 left them Uncertain while stopping, which only an owner's resolve-cancellation cleared.
use super::*;
use super::super::{coding_tests::{admitted, configured}, tests::enqueue};

/// A command build admitted by a daemon that died before it started: Uncertain, with no receipt to recover from.
async fn uncertain_build() -> (tempfile::TempDir, Runtime) {
    let (dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    admitted(&runtime).await;
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(1_000).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    (dir, runtime)
}
fn log_of(runtime: &Runtime) -> PathBuf {
    let state = runtime.state("w").unwrap().unwrap();
    let item = &state.items["one"];
    let outbox = runtime.store.workflow_outbox("w").unwrap();
    let mut stored = outbox.iter().find(|s| Some(&s.effect.id) == item.pending.as_ref()).unwrap();
    if let EffectKind::Stop { operation } = &stored.effect.kind { stored = outbox.iter().find(|s| &s.effect.id == operation).unwrap(); }
    runtime.root.join("receipts").join(format!("{}.output", digest(&stored.effect)))
}

#[tokio::test]
async fn the_orchestrator_resolves_an_uncertain_build_once_nothing_of_it_runs() {
    let (_dir, mut runtime) = uncertain_build().await;
    let controller = runtime.controller();
    // A command build that wrote its log may still be running: the runner can't prove it ended, so it won't say so.
    std::fs::write(log_of(&runtime), b"").unwrap();
    let error = runtime.resolve_provision(&controller, "w", "one").await.expect_err("an unproved process is refused");
    assert!(error.to_string().contains("can't be resolved yet"), "{error}");
    std::fs::remove_file(log_of(&runtime)).unwrap();
    runtime.resolve_provision(&controller, "w", "one").await.unwrap();
    let error = runtime.resolve_provision(&controller, "w", "one").await.expect_err("resolved once");
    assert!(error.to_string().contains("already resolved"), "{error}");
    // The next sweep fails the step with why, so the orchestrator can retry or send it back.
    runtime.recovery.wake();
    runtime.pump(2_000).await.unwrap();
    let item = &runtime.state("w").unwrap().unwrap().items["one"];
    assert_ne!(item.phase, Phase::Uncertain);
    assert!(item.failure.as_deref().is_some_and(|f| f.contains("the orchestrator resolved it")), "{item:?}");
}

#[tokio::test]
async fn a_cancelled_uncertain_build_ends_once_resolved_with_no_owner_step() {
    let (_dir, mut runtime) = uncertain_build().await;
    runtime.internal("w", Command::Cancel { item: "one".into(), reason: None }, Grant::Controller, 1_100).unwrap();
    runtime.pump(1_200).await.unwrap();
    let item = &runtime.state("w").unwrap().unwrap().items["one"];
    assert_eq!((item.phase.clone(), item.stopping.is_some()), (Phase::Uncertain, true), "the stop can't prove the build ended");
    runtime.resolve_provision(&runtime.controller(), "w", "one").await.unwrap();
    runtime.recovery.wake();
    runtime.pump(1_300).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    let phase = state.items.get("one").map(|i| i.phase.clone()).or_else(|| state.archive.get("one").map(|a| a.phase.clone()));
    assert_eq!(phase, Some(Phase::Cancelled));
}

/// A native build whose daemon died mid-turn: its pane went with the daemon's tmux server, and its log is gone too.
async fn vanished_native_build() -> (tempfile::TempDir, Runtime, super::super::coding_native::tests::Fake) {
    let (dir, mut runtime, fake) = super::super::coding_native::tests::native("cancel").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let mut run = Box::pin(runtime.execute_coding("w", &binding, cancelled));
    let started = dir.path().join("started");
    tokio::select! {
        result = run.as_mut() => panic!("the build ended before its agent started: {result:?}"),
        _ = async { while !started.exists() { tokio::time::sleep(Duration::from_millis(10)).await; } } => {}
    }
    drop(run); // The daemon dies: its private tmux server is reaped with it.
    let pid: i32 = std::fs::read_to_string(&started).unwrap().parse().unwrap();
    for _ in 0..300 { if unsafe { libc::kill(pid, 0) } != 0 { break; } tokio::time::sleep(Duration::from_millis(10)).await; }
    // The output capture outlives the build's future and writes the log once more as the pane closes.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let _ = std::fs::remove_file(log_of(&runtime));
    let root = runtime.root.clone();
    drop(runtime);
    (dir, Runtime::open(&root).unwrap(), fake)
}

#[tokio::test]
async fn cancelling_a_native_build_whose_pane_and_log_are_gone_reaches_cancelled() {
    let (_dir, mut runtime, _fake) = vanished_native_build().await;
    assert!(!log_of(&runtime).exists(), "the log is gone");
    runtime.internal("w", Command::Cancel { item: "one".into(), reason: None }, Grant::Controller, 1_100).unwrap();
    for time in 0..5 { runtime.pump(1_200 + time * 10_000).await.unwrap(); }
    let state = runtime.state("w").unwrap().unwrap();
    let phase = state.items.get("one").map(|i| i.phase.clone()).or_else(|| state.archive.get("one").map(|a| a.phase.clone()));
    assert_eq!(phase, Some(Phase::Cancelled), "{:?}", state.items.get("one"));
}

#[tokio::test]
async fn a_native_build_whose_pane_went_with_the_daemon_resumes_with_why() {
    let (_dir, mut runtime, _fake) = vanished_native_build().await;
    runtime.pump(1_200).await.unwrap();
    let item = &runtime.state("w").unwrap().unwrap().items["one"];
    assert_eq!(item.phase, Phase::Running, "{item:?}");
    assert_eq!(item.attempt, 1);
    assert!(item.reason.as_deref().is_some_and(|f| f.contains("interrupted") && f.contains("resumed")), "{item:?}");
}

#[tokio::test]
async fn ov1427_restart_reaps_a_native_owner_that_survived_the_runner() {
    let (dir, mut runtime, _fake) = super::super::coding_native::tests::native("cancel").await;
    let mut definition = runtime.configured_definition().unwrap();
    for stage in &mut definition.stages { stage.max_attempts = 1; }
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let (_stop, signal) = tokio::sync::watch::channel(false);
    let mut abandoned = Box::pin(runtime.execute_coding("w", &binding, signal));
    let started = dir.path().join("started");
    tokio::select! {
        result = abandoned.as_mut() => panic!("native build ended early: {result:?}"),
        _ = async { while !started.exists() { tokio::time::sleep(Duration::from_millis(10)).await; } } => {}
    }
    let pid: i32 = std::fs::read_to_string(&started).unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    // Keep the old future alive and unpolled: unlike dropping it, this does not reap its private server.
    assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::Interrupted);
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    drop(abandoned);
}
