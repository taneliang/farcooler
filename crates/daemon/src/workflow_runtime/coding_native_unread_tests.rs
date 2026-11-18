//! A runner too busy to read an agent's pane in time neither ends its turn nor fails it without a reason (ov-1301).
//! On the Canary, a tmux that didn't answer within its one-second deadline read as a lost session: four builds ended
//! in a minute, each recorded Failed with no reason, and the engine's retry ended the same way.
use super::*;
use super::super::{coding_tests::admitted, tests::enqueue};
use super::tests::native;

#[test]
fn a_pane_read_that_fails_is_unknown_until_two_minutes_pass_and_a_closed_pane_is_read_twice() {
    use farcooler_tmux::PaneLife;
    let mut unread = PaneUnread::default();
    let timed_out = || Err(farcooler_core::DomainError::TmuxTimedOut);
    assert!(matches!(pane_look(timed_out(), &mut unread, Duration::ZERO), PaneLook::Unread));
    assert!(matches!(pane_look(timed_out(), &mut unread, UNREADABLE_PANE_FOR / 2), PaneLook::Unread));
    assert!(matches!(pane_look(Ok(PaneLife::Live), &mut unread, UNREADABLE_PANE_FOR), PaneLook::Live), "a read that answers resets the wait");
    assert!(matches!(pane_look(timed_out(), &mut unread, UNREADABLE_PANE_FOR), PaneLook::Unread));
    assert!(matches!(pane_look(timed_out(), &mut unread, UNREADABLE_PANE_FOR * 2),
        PaneLook::Lost(why) if why.contains("could not read the agent's pane")));
    let mut unread = PaneUnread::default();
    assert!(matches!(pane_look(Ok(PaneLife::Gone), &mut unread, Duration::ZERO), PaneLook::Unread), "one closed read is looked at again");
    assert!(matches!(pane_look(Ok(PaneLife::Dead), &mut unread, Duration::ZERO), PaneLook::Lost(why) if why.contains("lost without a receipt")));
}

/// Start the native build and wait until its agent runs, polling the build all the while.
async fn started<'a>(dir: &tempfile::TempDir, run: &mut std::pin::Pin<&mut (impl std::future::Future<Output = Result<Outcome>> + 'a)>) {
    let started = dir.path().join("started");
    tokio::select! {
        result = run.as_mut() => panic!("the build ended before its agent started: {result:?}"),
        _ = async { while !started.exists() { tokio::time::sleep(Duration::from_millis(10)).await; } } => {}
    }
}

#[tokio::test]
async fn a_pane_the_runner_cannot_read_for_a_while_leaves_the_build_running_and_a_lost_one_fails_it_with_why() {
    let (dir, mut runtime, _fake) = native("cancel").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let run = runtime.execute_coding("w", &binding, cancelled);
    tokio::pin!(run);
    started(&dir, &mut run).await;
    // A second of reads that time out, as a busy runner's tmux does: the build goes on.
    UNREADABLE_PANE.with(|n| n.set(4));
    tokio::select! {
        result = run.as_mut() => panic!("a pane that couldn't be read ended the turn: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(1_300)) => {}
    }
    assert_eq!(UNREADABLE_PANE.with(|n| n.get()), 0, "every failing read was tried");
    // The agent really ends: the step fails, and says why.
    let pid: i32 = std::fs::read_to_string(dir.path().join("started")).unwrap().parse().unwrap();
    unsafe { libc::kill(pid, libc::SIGKILL); }
    let outcome = tokio::time::timeout(Duration::from_secs(3), run).await.expect("a lost session ends the turn").unwrap();
    let Outcome::SafeFailureBecause(why) = outcome else { panic!("a lost session failed without a reason: {outcome:?}") };
    assert!(why.contains("lost without a receipt"), "{why}");
    assert!(valid_rework_reason(&why), "the reason can go on the item: {why}");
}

#[tokio::test]
async fn a_pane_the_runner_never_reads_again_fails_the_build_with_why_on_the_item() {
    let (dir, mut runtime, _fake) = native("cancel").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let run = runtime.execute_coding("w", &binding, cancelled);
    tokio::pin!(run);
    started(&dir, &mut run).await;
    UNREADABLE_PANE.with(|n| n.set(u32::MAX));
    let outcome = tokio::time::timeout(UNREADABLE_PANE_FOR + Duration::from_secs(2), run).await
        .expect("the step gives up once the pane has gone unread too long").unwrap();
    UNREADABLE_PANE.with(|n| n.set(0));
    let Outcome::SafeFailureBecause(why) = outcome.clone() else { panic!("ended without a reason: {outcome:?}") };
    assert!(why.contains("could not read the agent's pane"), "{why}");
    // The reason is what the orchestrator reads on the item.
    let state = runtime.state("w").unwrap().unwrap();
    let effect = state.items["one"].pending.clone().unwrap();
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.effect.id == effect).unwrap().effect;
    assert!(runtime.observe("w", &effect, outcome, false, 10).unwrap());
    let item = &runtime.state("w").unwrap().unwrap().items["one"];
    assert!(item.failure.as_deref().is_some_and(|f| f.contains("could not read the agent's pane")), "{item:?}");
}

/// A real retained builder survives both a gate already running at launch and one arriving mid-turn.
#[tokio::test]
async fn a_landing_gate_keeps_a_native_builds_pane_until_it_resumes_and_finishes() {
    // A child harness lets the actual tmux client fail without changing this process's
    // environment or the fanout daemon's probes. The two failures used to report Gone,
    // causing native_turn to fail and quiesce_native to kill the still-live builder.
    if std::env::var_os("FC_PROBE_INJECTED").is_none() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tmux = farcooler_core::programs::find("tmux").unwrap();
        let wrapper = dir.path().join("tmux");
        let script = r#"#!/bin/sh
if [ "$PPID" = "$FC_PROBE_OWNER" ] && [ "$5" = display-message ]; then
    case "$9" in '#{pane_dead}'*)
        count="$FC_PROBE_DIR/$2"
        n=$(cat "$count" 2>/dev/null || echo 0)
        if [ "$n" -lt 2 ]; then
            echo $((n + 1)) > "$count"
            echo injected >> "$FC_PROBE_DIR/injected"
            echo 'error connecting to server (Interrupted system call)' >&2
            exit 1
        fi;;
    esac
fi
exec REAL_TMUX "$@"
"#;
        std::fs::write(&wrapper, script.replace("REAL_TMUX", &crate::service::shell_quote(tmux.to_str().unwrap()))).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let test = "workflow_runtime::coding_native::unread_tests::a_landing_gate_keeps_a_native_builds_pane_until_it_resumes_and_finishes";
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", "export FC_PROBE_OWNER=$$; exec \"$@\"", "probe-owner"])
            .arg(std::env::current_exe().unwrap()).args(["--exact", test, "--nocapture"])
            .env("FC_PROBE_INJECTED", "1").env("FC_PROBE_DIR", dir.path())
            .env("PATH", format!("{}:{}", dir.path().display(), std::env::var("PATH").unwrap_or_default()))
            .output().unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert_eq!(std::fs::read_to_string(dir.path().join("injected")).unwrap().lines().count(), 8,
            "both pane probes failed in all four native gate scenarios");
        return;
    }
    native_gate_scenarios().await;
}

/// Exercise the real pause and tmux reads independently of the failed-client
/// regression above. A live-pane assertion alone misses tmux's automatic
/// SIGCONT of a stopped pane supervisor: the agent must actually stay stopped.
#[tokio::test]
async fn a_native_builder_pauses_and_finishes_during_a_gate_without_probe_injection() {
    native_gate_scenarios().await;
}

async fn native_gate_scenarios() {
    for (agent, gate_first) in [("claude", false), ("claude", true), ("codex", false), ("codex", true)] {
        let (dir, mut runtime, _fake) = super::tests::native_agent("pass", agent).await;
        let script = dir.path().join("interactive.py");
        let source = std::fs::read_to_string(&script).unwrap();
        let barrier = "while not pathlib.Path(os.environ['STARTED']).with_name('finish').exists():\n    pathlib.Path(os.environ['STARTED']).with_name('heartbeat').write_text(str(time.monotonic_ns()))\n    time.sleep(0.01)\n";
        std::fs::write(&script, source.replace("if mode in ('steering', 'active-steering'):\n    ready",
            &format!("{barrier}if mode in ('steering', 'active-steering'):\n    ready"))).unwrap();
        if let CodingHarness::Native { process, .. } = &mut runtime.coding.as_mut().unwrap().harness { process.timeout_ms = 30_000; }
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap();
        let binding = admitted(&runtime).await;
        let yielding = runtime.exec.yielding.clone();
        let mut gate = gate_first.then(|| yielding.gate());
        let (_cancel, cancelled) = tokio::sync::watch::channel(false);
        let run = runtime.execute_coding("w", &binding, cancelled);
        tokio::pin!(run);
        if !gate_first { started(&dir, &mut run).await; gate = Some(yielding.gate()); }
        tokio::select! {
            result = run.as_mut() => panic!("gate ended the builder: {result:?}"),
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
        let owner: NativeOwner = serde_json::from_slice(&std::fs::read(runtime.exec.native_path(&binding, "owner")).unwrap()).unwrap();
        assert_eq!(server(&runtime.root, &binding).pane_life(&owner.pane, owner.terminal).await.unwrap(), farcooler_tmux::PaneLife::Live);
        let heartbeat = dir.path().join("heartbeat");
        let held = std::fs::read(&heartbeat).ok();
        tokio::select! {
            result = run.as_mut() => panic!("gate ended the builder while paused: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(700)) => {}
        }
        assert_eq!(std::fs::read(&heartbeat).ok(), held, "the agent must stay paused across a gate sweep, not run between SIGSTOP and tmux's automatic SIGCONT");
        if dir.path().join("started").exists() {
            let pid = std::fs::read_to_string(dir.path().join("started")).unwrap();
            let state = std::process::Command::new("ps").args(["-o", "stat=", "-p", pid.trim()]).output().unwrap();
            assert!(String::from_utf8_lossy(&state.stdout).trim_start().starts_with('T'),
                "the gate must actually pause the native builder: {}", String::from_utf8_lossy(&state.stdout));
        }
        std::fs::write(dir.path().join("finish"), "go").unwrap();
        drop(gate);
        let outcome = tokio::time::timeout(Duration::from_secs(10), run).await.unwrap().unwrap();
        assert!(matches!(outcome, Outcome::Passed(_)), "{outcome:?}");
    }
}
