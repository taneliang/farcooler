//! ov-1178: accepted settings must deliver through the CLI and the real native turn host.
use super::*;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_store::models::Actor;

#[tokio::test]
async fn accepted_native_claude_delivers_message_and_acceptance_before_turn_ends() {
    let exe = std::env::current_exe().unwrap();
    let cli = farcooler_core::scratch::cargo_profile(&exe).unwrap().join("farcooler");
    assert!(cli.is_file(), "build the real hook CLI first: cargo build -p farcooler-cli --bin farcooler");
    let (dir, mut runtime, _fake) = tests::native("pass").await;
    runtime.exec.validate_native_host().unwrap();
    let (_service_dir, svc, repo) = crate::test_support::fixture().await;
    runtime.store = svc.store.clone(); // The native host and watcher share the runner's durable message queue.
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    let primary = svc.store.create_task(ws, "native primary", Actor::Manager).unwrap();
    let task = svc.store.create_task(ws, "native grouped ticket", Actor::Manager).unwrap();
    let watcher = crate::watch::Watcher::new(svc.clone());

    // Provision and admit a real build, then include a secondary ticket in this run's publication.
    let workspace = ws.to_string();
    let definition = runtime.configured_definition().unwrap();
    super::super::tests::enqueue(&mut runtime, &workspace, &primary.key, definition);
    runtime.pump(2).await.unwrap();
    let state = runtime.state(&workspace).unwrap().unwrap();
    let mut effect = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect(&workspace, &effect.id, "native-delivery", state.revision,
        state.epoch, 10, 100).unwrap().unwrap();
    let base = runtime.pin_base(&workspace, &effect).await.unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 10).unwrap());
    effect.with.push(task.key.clone());
    let binding = Binding::new(&workspace, &effect, claim.resource_generation, state.items[&primary.key].iteration, &base);

    // The validated command may use the channel's bare CLI name. Resolve it only inside this
    // scratch agent's PATH, to the binary built from this checkout, never an installed CLI.
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let hook_binary = crate::hook_install::binary_path();
    assert!(!Path::new(&hook_binary).is_absolute(), "unit harness should use the channel CLI name");
    std::os::unix::fs::symlink(&cli, bin.join(hook_binary)).unwrap();
    let agent = dir.path().join("claude");
    std::fs::copy(farcooler_core::programs::find("perl").unwrap(), &agent).unwrap();
    let script = dir.path().join("delivery.pl");
    std::fs::write(&script, include_str!("coding_native_delivery_test.pl")).unwrap();
    let CodingHarness::Native { settings: Some(settings), .. } = &runtime.coding.as_ref().unwrap().harness else { unreachable!() };
    TEST_ARGV.with(|p| *p.borrow_mut() = Some(vec![agent.to_string_lossy().into_owned(),
        script.to_string_lossy().into_owned(), settings.path.to_string_lossy().into_owned()]));
    let mut command = runtime.exec.coding_command(&binding).unwrap();
    command.timeout_ms = 20_000;
    command.environment.insert("PATH".into(), format!("{}:/usr/bin:/bin", bin.display()));
    command.environment.insert("CLAUDE_CONFIG_DIR".into(), dir.path().join("config").to_string_lossy().into_owned());
    let log = dir.path().join("context.jsonl");
    let finish = dir.path().join("finish");
    command.environment.insert("CONTEXT_LOG".into(), log.to_string_lossy().into_owned());
    command.environment.insert("FINISH".into(), finish.to_string_lossy().into_owned());
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let run = runtime.exec.run_native(&binding, &command, cancelled);
    tokio::pin!(run);
    let delivery = async {
        let started = dir.path().join("started");
        assert!(super::wait_until(tokio::time::Instant::now() + Duration::from_secs(60),
            Duration::from_millis(20), || started.exists()).await, "native agent never submitted its prompt");
        for key in [&primary.key, &task.key] {
            assert_eq!(crate::watch_view::lookup(&workspace, key).unwrap().1.terminal_id, terminal(&binding));
        }
        let sent = watcher.message_send(pb::Request { method: "message.send".into(),
            payload: Some(request::Payload::MessageSend(pb::MessageSend {
                to: task.key.clone(), text: "Rebase while this turn is active".into(), actor: "manager".into(),
                workspace_id: ws.as_bytes().to_vec().into(), ..Default::default()
            })), ..Default::default()
        }, crate::people::Person::Owner).await.unwrap();
        assert!(matches!(sent, result::Value::MessageSent(_)));
        for text in ["Rebase while this turn is active", "Acceptance changed on"] {
            if text.starts_with("Acceptance") {
                crate::task_ops::update(&svc, &watcher, &pb::TaskUpdate {
                    task_id: task.id.as_bytes().to_vec().into(), expected_version: task.resource_version,
                    title: task.title.clone(), intent: task.intent.clone(), actor: "manager".into(),
                    acceptance: vec![pb::TaskAcceptanceItem { text: "New mid-turn requirement".into(), ..Default::default() }],
                    ..Default::default()
                }).unwrap();
            }
            assert!(wait_until_async!(tokio::time::Instant::now() + Duration::from_secs(60),
                Duration::from_millis(20), async || {
                    watcher.pump_wakes().await;
                    std::fs::read_to_string(&log).unwrap_or_default().contains(text)
                }), "CLI never delivered {text}");
            let received = std::fs::read_to_string(&log).unwrap_or_default();
            let replies: Vec<serde_json::Value> = received.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
            let field = if text.starts_with("Acceptance") { "additionalContext" } else { "permissionDecisionReason" };
            let contexts: Vec<_> = replies.iter().filter(|r|
                r["hookSpecificOutput"][field].as_str().is_some_and(|c| c.contains(text))).collect();
            assert_eq!(contexts.len(), 1, "CLI never delivered {text}: {received}");
            assert_eq!(contexts[0]["hookSpecificOutput"]["hookEventName"], "PreToolUse");
            if text.starts_with("Acceptance") {
                assert!(contexts[0]["hookSpecificOutput"].get("permissionDecision").is_none());
            } else {
                assert_eq!(contexts[0]["hookSpecificOutput"]["permissionDecision"], "deny");
            }
            assert!(svc.store.pending_message_wakes().unwrap().is_empty());
            assert!(!finish.exists(), "delivery must precede Stop");
            let pid: i32 = std::fs::read_to_string(&started).unwrap().parse().unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the native turn must still be running");
        }
        std::fs::write(&finish, "end the turn").unwrap();
    };
    tokio::select! {
        result = &mut run => panic!("native turn ended before both deliveries: {result:?}"),
        _ = delivery => {}
    }
    assert_eq!(run.await.unwrap().code, Some(0));
    let settled: NativeSettlement = serde_json::from_slice(&std::fs::read(runtime.exec.native_path(&binding, "settled")).unwrap()).unwrap();
    let NativeSettlement::Launched { hooks, .. } = settled else { panic!("never launched") };
    assert!(hooks.iter().any(|h| h.event == "PreToolUse" && h.takes.iter().any(|t| t == "additional_context")));
    assert!(hooks.iter().any(|h| h.event == "Stop"));
    for key in [&primary.key, &task.key] { assert!(crate::watch_view::lookup(&workspace, key).is_none()); }
}

#[tokio::test]
async fn ov_1464_tool_hook_reply_timeout_does_not_end_the_build() {
    disconnected_hook_does_not_end_the_build(HookExit::Timeout).await;
}

#[tokio::test]
async fn ov_1464_killed_stop_hook_does_not_end_the_build() {
    disconnected_hook_does_not_end_the_build(HookExit::Killed).await;
}

#[tokio::test]
async fn ov_1464_non_gating_tool_hook_does_not_end_the_build() {
    disconnected_hook_does_not_end_the_build(HookExit::FireAndForget).await;
}

#[tokio::test]
async fn ov_1464_disconnected_duplicate_stop_preserves_the_continuation() {
    disconnected_hook_does_not_end_the_build(HookExit::DuplicateStop).await;
}

enum HookExit { Timeout, Killed, FireAndForget, DuplicateStop }

async fn disconnected_hook_does_not_end_the_build(exit: HookExit) {
    let (dir, mut runtime, _fake) = tests::native_agent("hook-disconnect", "codex").await;
    let host = Uuid::now_v7();
    let repository = runtime.repository();
    let root = runtime.store.create_repository_root(host, dir.path().to_str().unwrap(), 0).unwrap();
    let repo = runtime.store.create_repository(host, root.id, "Native", repository.join(".git").to_str().unwrap(), "").unwrap();
    let board = runtime.store.ensure_main_workspace(repo.id).unwrap();
    let task = runtime.store.create_task(board.id, "Steer this", Actor::User).unwrap();
    let workspace = board.id.to_string();
    let definition = runtime.configured_definition().unwrap();
    super::super::tests::enqueue(&mut runtime, &workspace, &task.key, definition);
    runtime.pump(2).await.unwrap();
    let binding = super::super::coding_tests::admitted_in(&runtime, &workspace).await;
    let (_cancel, cancel) = tokio::sync::watch::channel(false);
    let run = runtime.execute_coding(&workspace, &binding, cancel);
    tokio::pin!(run);

    let id = terminal(&binding);
    let started = dir.path().join("started");
    tokio::select! {
        outcome = &mut run => panic!("the build ended before the disconnect: {outcome:?}"),
        ready = super::wait_until(tokio::time::Instant::now() + Duration::from_secs(10),
            Duration::from_millis(10), || started.exists()) => assert!(ready, "the build did not start"),
    }
    runtime.store.send_manual_message(task.id, Actor::Manager, "Use the new API", Some(id), serde_json::json!({})).unwrap();
    let socket = runtime.native_socket(&binding);
    let payload = serde_json::json!({"session_id": id, "cwd": runtime.tree(&binding), "tool_name": "Write"});
    if matches!(exit, HookExit::DuplicateStop) {
        let line = serde_json::json!({"agent": "codex", "event": "Stop", "takes": ["followup"],
            "payload": {"session_id": id, "cwd": runtime.tree(&binding), "turn_id": "same-turn"}});
        let frame = format!("{line}\n");
        let read_reply = || async {
            let mut stream = tokio::net::UnixStream::connect(&socket).await.unwrap();
            stream.write_all(frame.as_bytes()).await.unwrap();
            let mut reply = String::new();
            tokio::io::BufReader::new(stream).read_line(&mut reply).await.unwrap();
            assert!(reply.contains("Use the new API"), "missing Stop continuation: {reply}");
        };
        tokio::select! {
            outcome = &mut run => panic!("the build ended before the Stop continuation: {outcome:?}"),
            reply = tokio::time::timeout(Duration::from_secs(5), read_reply()) => reply.unwrap(),
        }
        // The turn isn't polled between enqueueing the replay and closing its socket.
        let mut duplicate = tokio::net::UnixStream::connect(&socket).await.unwrap();
        duplicate.write_all(frame.as_bytes()).await.unwrap();
        drop(duplicate);
        tokio::select! {
            outcome = &mut run => panic!("replaying to a disconnected duplicate ended the build: {outcome:?}"),
            reply = tokio::time::timeout(Duration::from_secs(5), read_reply()) => reply.unwrap(),
        }
        std::fs::write(dir.path().join("direction-ready"), "").unwrap();
    } else {
        close_steering_hook(exit, &socket, payload).await;
        let finish = async {
            assert!(super::wait_until(tokio::time::Instant::now() + Duration::from_secs(10),
                Duration::from_millis(10), || runtime.store.notes_for(task.id, None).unwrap().iter()
                    .any(|n| n.body.contains("Couldn't confirm the engine agent got the message"))).await,
                "the failed delivery must be recorded");
            std::fs::write(dir.path().join("direction-ready"), "").unwrap();
        };
        tokio::select! {
            outcome = &mut run => panic!("the disconnected hook ended the build: {outcome:?}"),
            _ = finish => {}
        }
    }
    let outcome = run.await;
    assert!(matches!(outcome, Ok(Outcome::Passed(_))), "the same build must commit and pass: {outcome:?}");
    assert!(runtime.store.pending_message_wakes().unwrap().is_empty());
}

async fn close_steering_hook(exit: HookExit, socket: &Path, payload: serde_json::Value) {
    // Leave the listener bound without polling the turn until the complete frame's
    // sender has gone away. The delivery note below proves the frame was accepted.
    match exit {
        HookExit::Timeout | HookExit::FireAndForget => {
            let exe = std::env::current_exe().unwrap();
            let cli = farcooler_core::scratch::cargo_profile(&exe).unwrap().join("farcooler");
            assert!(cli.is_file(), "build the real hook CLI first: cargo build -p farcooler-cli --bin farcooler");
            let mut command = tokio::process::Command::new(cli);
            command.args(["hook", "--agent", "codex", "--event", "PreToolUse"]);
            if matches!(exit, HookExit::Timeout) { command.arg("--gating"); }
            let called = tokio::time::Instant::now();
            let mut hook = command.arg("--socket").arg(socket)
                .env("FARCOOLER_NATIVE_HOOK_SOCKET", socket)
                .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
                .kill_on_drop(true).spawn().unwrap();
            hook.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).await.unwrap();
            let output = tokio::time::timeout(Duration::from_secs(5), hook.wait_with_output()).await.unwrap().unwrap();
            assert!(output.status.success());
            assert!(output.stdout.is_empty(), "the hook must let Codex proceed");
            if matches!(exit, HookExit::Timeout) {
                assert!(called.elapsed() >= Duration::from_millis(400));
            }
        }
        HookExit::Killed => {
            let line = serde_json::json!({"agent": "codex", "event": "Stop", "takes": ["followup"], "payload": payload});
            // Use the same system interpreter as the native agent fixture.
            let mut hook = tokio::process::Command::new("/usr/bin/python3").arg("-c").arg(r#"
import socket, sys
with socket.socket(socket.AF_UNIX) as sock:
    sock.connect(sys.argv[1])
    sock.sendall((sys.argv[2] + '\n').encode())
    print('waiting for reply', flush=True)
    sock.recv(1)
"#).arg(socket).arg(line.to_string()).stdout(std::process::Stdio::piped()).kill_on_drop(true).spawn().unwrap();
            let mut stdout = tokio::io::BufReader::new(hook.stdout.take().unwrap());
            let mut ready = String::new();
            tokio::time::timeout(Duration::from_secs(5), stdout.read_line(&mut ready)).await.unwrap().unwrap();
            assert_eq!(ready.trim_end(), "waiting for reply");
            hook.start_kill().unwrap();
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(hook.wait().await.unwrap().signal(), Some(libc::SIGKILL));
        }
        HookExit::DuplicateStop => unreachable!(),
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_native_launch_refuses_a_quarantined_binary_and_lists_the_ticket() {
    let (dir, mut runtime, _fake) = tests::native_agent("pass", "codex-engine").await;
    let executable = dir.path().join("quarantined codex");
    std::fs::write(&executable, r#"#!/bin/sh
touch "$FC_TEST_RAN"
"#).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    quarantine(&executable);

    let link = dir.path().join("agent link");
    std::os::unix::fs::symlink(&executable, &link).unwrap();
    assert_quarantine_refused(&mut runtime, &link, &link, &dir.path().join("ran")).await;
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_native_launch_refuses_the_quarantined_binary_behind_an_npm_codex_launcher() {
    let (dir, mut runtime, _fake) = tests::native_agent("pass", "codex-engine").await;
    let node = farcooler_core::programs::find("node").expect("the npm launcher fixture requires Node.js");
    let arch = std::process::Command::new(&node).args(["-p", "process.arch"]).output().unwrap();
    assert!(arch.status.success());
    let (package, triple) = match std::str::from_utf8(&arch.stdout).unwrap().trim() {
        "arm64" => ("codex-darwin-arm64", "aarch64-apple-darwin"),
        "x64" => ("codex-darwin-x64", "x86_64-apple-darwin"),
        other => panic!("unexpected Node.js architecture {other}"),
    };
    let scope = dir.path().join("npm install/node_modules/@openai");
    let launcher = scope.join("codex/bin/codex.js");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    std::fs::write(&launcher, r#"#!/usr/bin/env node
const path = require("node:path");
const triple = process.arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
const pkg = require.resolve(`@openai/codex-darwin-${process.arch}/package.json`);
const binary = path.join(path.dirname(pkg), "vendor", triple, "bin", "codex");
require("node:fs").writeFileSync(process.env.FC_TEST_RAN, "launcher entered");
const result = require("node:child_process").spawnSync(binary, ["-c", "exit 0"], {stdio: "inherit"});
process.exit(result.status ?? 1);
"#).unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let platform = scope.join(package);
    let executable = platform.join("vendor").join(triple).join("bin/codex");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(platform.join("package.json"), "{}").unwrap();
    std::fs::copy("/bin/sh", &executable).unwrap();
    quarantine(&executable);

    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(&node, bin.join("node")).unwrap();
    let entry = bin.join("codex");
    std::os::unix::fs::symlink(&launcher, &entry).unwrap();
    let CodingHarness::Native { process, .. } = &mut runtime.coding.as_mut().unwrap().harness else { unreachable!() };
    process.environment.insert("PATH".into(), format!("{}:/usr/bin:/bin", bin.display()));

    assert_quarantine_refused(&mut runtime, &entry, &executable, &dir.path().join("ran")).await;
}

#[cfg(target_os = "macos")]
fn quarantine(executable: &Path) {
    assert!(std::process::Command::new("/usr/bin/xattr")
        .args(["-w", "com.apple.quarantine", "0081;00000000;test;"])
        .arg(executable).status().unwrap().success());
}

#[cfg(target_os = "macos")]
async fn assert_quarantine_refused(runtime: &mut Runtime, entry: &Path, refused: &Path, ran: &Path) {
    use super::super::{coding_tests::admitted, tests::enqueue};

    let _stand_in = crate::service::test_agent::stand_in(entry);
    TEST_ARGV.with(|p| p.borrow_mut().take());
    let CodingHarness::Native { process, .. } = &mut runtime.coding.as_mut().unwrap().harness else { unreachable!() };
    process.environment.insert("FC_TEST_RAN".into(), ran.to_string_lossy().into_owned());
    let mut definition = runtime.configured_definition().unwrap();
    definition.stages.iter_mut().find(|s| s.kind == StageKind::Build).unwrap().max_attempts = 2;
    let ticket = "ov-1469";
    enqueue(runtime, "w", ticket, definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(runtime).await;
    assert!(matches!(binding.effect.kind, EffectKind::Execute(StageKind::Build)));
    let (_cancel, orders) = tokio::sync::watch::channel(false);
    let outcome = runtime.execute_coding("w", &binding, orders).await.unwrap();
    let Outcome::SafeFailureBecause(reason) = &outcome else { panic!("{outcome:?}") };
    // macOS may spell the fixture's root /tmp or /private/tmp.
    let canonical_refused = refused.canonicalize().unwrap();
    let names_executable = |text: &str| [refused, canonical_refused.as_path()].iter()
        .any(|path| text.contains(path.to_str().unwrap()));
    assert!(reason.contains("com.apple.quarantine") && names_executable(reason), "{reason}");
    assert!(!ran.exists(), "the refused launch ran");
    assert!(runtime.observe("w", &binding.effect, outcome, false, 10).unwrap());
    let item = &runtime.state("w").unwrap().unwrap().items[ticket];
    assert_eq!(item.attempt, 2, "a queued retry still has to show the refusal");
    let mut items = Vec::new();
    crate::engine_stuck::include_refusals(&runtime.store, &mut items);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].item, ticket);
    assert!(items[0].question.contains("com.apple.quarantine"));
    assert!(names_executable(&items[0].question));
}
